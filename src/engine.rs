//! Merge and normalization engine.
//!
//! Rule version `RULE_VERSION` identifies this exact behavior. Derived output
//! is deterministic for a given ordered input set so replay is reproducible.

use crate::domain::*;
use crate::ids::{parse_node_id, parse_qualified_name, value_as_string, value_kind};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const RULE_VERSION: &str = "compass-rules-1";

/// Builds the global namespace table in deterministic order: first occurrence
/// of a URI across files in import order.
pub fn build_namespace_table(files: &[ParsedFile]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for f in files {
        for uri in &f.namespace_table {
            if seen.insert(uri.clone()) {
                out.push(uri.clone());
            }
        }
    }
    out
}

pub fn build_namespace_map(files: &[ParsedFile]) -> Vec<NamespaceMapping> {
    let global = build_namespace_table(files);
    let mut out = Vec::new();
    for f in files {
        for (idx, uri) in f.namespace_table.iter().enumerate() {
            let global_index = global.iter().position(|u| u == uri).unwrap_or(0) as u16;
            out.push(NamespaceMapping {
                file_id: f.file_id.clone(),
                file_index: idx as u16,
                uri: uri.clone(),
                global_index,
            });
        }
    }
    out
}

pub struct ResolveCtx<'a> {
    pub file: &'a ParsedFile,
    pub alias_targets: &'a HashMap<String, String>,
    pub alias_cycle_members: &'a HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct EndpointResult {
    pub gid: Option<GlobalNodeId>,
    pub raw: String,
    pub reason: Option<String>,
}

pub fn global_id_of(raw: &RawNodeId, file: &ParsedFile) -> Option<GlobalNodeId> {
    let uri = file.namespace_table.get(raw.ns as usize)?.clone();
    Some(GlobalNodeId {
        uri,
        value: value_as_string(&raw.id),
        id_kind: value_kind(&raw.id).to_string(),
    })
}

/// Resolve a textual NodeId reference through the per-file alias table, then
/// remap the numeric namespace index to its URI.
pub fn resolve_endpoint(text: &str, ctx: &ResolveCtx) -> EndpointResult {
    let raw = text.trim();
    let mut current = raw.to_string();
    let mut hops = 0;
    loop {
        if let Some(target) = ctx.alias_targets.get(&current) {
            current = target.clone();
            hops += 1;
            if hops > 64 {
                return EndpointResult {
                    gid: None,
                    raw: raw.to_string(),
                    reason: Some(format!("alias chain exceeded hop limit from {raw}")),
                };
            }
            continue;
        }
        break;
    }
    if ctx.alias_cycle_members.iter().any(|m| *m == raw) {
        // If the original symbol is a known cycle member, still try to resolve
        // final text (it never resolves), and mark it.
    }
    match parse_node_id(&current) {
        Ok(nid) => match global_id_of(&nid, ctx.file) {
            Some(gid) => EndpointResult {
                gid: Some(gid),
                raw: raw.to_string(),
                reason: None,
            },
            None => EndpointResult {
                gid: None,
                raw: raw.to_string(),
                reason: Some(format!(
                    "namespace index {} absent from this file's NamespaceTable",
                    nid.ns
                )),
            },
        },
        Err(e) => EndpointResult {
            gid: None,
            raw: raw.to_string(),
            reason: Some(e.detail),
        },
    }
}

/// Detect alias cycles per file (alias -> alias only). Returns the alias map
/// and the set of alias names that participate in a cycle.
pub fn alias_graph_cycles(file: &ParsedFile) -> (HashMap<String, String>, HashSet<String>) {
    let map: HashMap<String, String> = file
        .aliases
        .iter()
        .map(|a| (a.name.clone(), a.target_raw.trim().to_string()))
        .collect();
    let mut cycle_members: HashSet<String> = HashSet::new();
    for start in map.keys() {
        let mut path: Vec<String> = Vec::new();
        let mut on_path: HashSet<String> = HashSet::new();
        let mut cur = start.clone();
        loop {
            if on_path.contains(&cur) {
                if let Some(pos) = path.iter().position(|x| *x == cur) {
                    for m in &path[pos..] {
                        cycle_members.insert(m.clone());
                    }
                }
                break;
            }
            match map.get(&cur) {
                Some(next) => {
                    path.push(cur.clone());
                    on_path.insert(cur);
                    cur = next.clone();
                }
                None => break,
            }
            if path.len() > 64 {
                break;
            }
        }
    }
    (map, cycle_members)
}

#[derive(Default)]
pub struct DeriveInputs {
    pub changed_files: Vec<String>,
}

/// Full derivation from an ordered set of parsed files.
pub fn derive(files: &[ParsedFile]) -> DerivedGraph {
    let namespace_map = build_namespace_map(files);
    let global_ns = build_namespace_table(files);

    let mut graph = DerivedGraph {
        rule_version: RULE_VERSION.to_string(),
        namespace_map,
        ..Default::default()
    };

    // Per-file alias analysis.
    let mut alias_ctx: HashMap<String, (HashMap<String, String>, HashSet<String>)> =
        HashMap::new();
    for f in files {
        let (map, cycles) = alias_graph_cycles(f);
        if !cycles.is_empty() {
            let mut cyc: Vec<String> = cycles.iter().cloned().collect();
            cyc.sort();
            graph.alias_cycles.push(AliasCycle {
                file_id: f.file_id.clone(),
                cycle: cyc,
            });
            graph.diagnostics.push(DiagnosticEvent {
                code: "ALIAS_CYCLE".to_string(),
                severity: "error".to_string(),
                file_id: f.file_id.clone(),
                subject: cycles.iter().cloned().collect::<Vec<_>>().join(","),
                message: "alias references form a cycle; those references stay unresolved"
                    .to_string(),
                basis: "aliases resolve within one file; a cycle has no terminal NodeId"
                    .to_string(),
                range: None,
            });
        }
        alias_ctx.insert(f.file_id.clone(), (map, cycles));
    }

    // Nodes, with duplicate global ids recorded but all definitions retained.
    let mut node_by_gid: BTreeMap<GlobalNodeId, Vec<usize>> = BTreeMap::new();
    let mut all_nodes: Vec<ResolvedNode> = Vec::new();
    let mut node_gids_by_file: HashMap<String, HashSet<GlobalNodeId>> = HashMap::new();
    let mut gid_file: HashMap<GlobalNodeId, String> = HashMap::new();

    for f in files {
        let (alias_map, cycle_members) = alias_ctx.get(&f.file_id).cloned().unwrap_or_default();
        let ctx = ResolveCtx {
            file: f,
            alias_targets: &alias_map,
            alias_cycle_members: &cycle_members,
        };
        for raw_node in &f.nodes {
            let nid = match parse_node_id(&raw_node.node_id_raw) {
                Ok(n) => n,
                Err(err) => {
                    graph.diagnostics.push(DiagnosticEvent {
                        code: "BAD_NODE_ID".to_string(),
                        severity: "error".to_string(),
                        file_id: f.file_id.clone(),
                        subject: raw_node.node_id_raw.clone(),
                        message: format!("node id not parseable: {}", err.detail),
                        basis: "NodeId must use i=/s=/g=/b= with file-local ns=".to_string(),
                        range: Some(raw_node.range),
                    });
                    continue;
                }
            };
            let gid = match global_id_of(&nid, f) {
                Some(g) => g,
                None => {
                    graph.diagnostics.push(DiagnosticEvent {
                        code: "NODE_ID_NS_OUT_OF_RANGE".to_string(),
                        severity: "error".to_string(),
                        file_id: f.file_id.clone(),
                        subject: raw_node.node_id_raw.clone(),
                        message: format!("ns={} not in NamespaceTable", nid.ns),
                        basis: "numeric namespace index is file-local and must index the file's NamespaceTable".to_string(),
                        range: Some(raw_node.range),
                    });
                    continue;
                }
            };
            let qn = parse_qualified_name(&raw_node.browse_name_raw);
            let (browse_uri, browse_name) = match &qn {
                Ok(q) => (
                    f.namespace_table.get(q.ns as usize).cloned().unwrap_or_default(),
                    q.name.clone(),
                ),
                Err(_) => (String::new(), raw_node.browse_name_raw.clone()),
            };
            let resolved = ResolvedNode {
                gid: gid.clone(),
                file_id: f.file_id.clone(),
                kind: raw_node.kind.as_str().to_string(),
                browse_uri,
                browse_name,
                display_name: raw_node.display_name.clone(),
                ordinal: raw_node.ordinal,
                raw_range: raw_node.range,
            };
            let idx = all_nodes.len();
            node_by_gid.entry(gid.clone()).or_default().push(idx);
            node_gids_by_file
                .entry(f.file_id.clone())
                .or_default()
                .insert(gid.clone());
            gid_file.entry(gid.clone()).or_insert_with(|| f.file_id.clone());
            all_nodes.push(resolved);

            // Quantities belong to the node (last quantity mapping etc.).
            for q in &raw_node.quantities {
                graph.quantities.push(normalize_quantity(gid.clone(), q));
            }

            let _ = &ctx;
        }
    }

    // Duplicate records.
    for (gid, indices) in &node_by_gid {
        if indices.len() > 1 {
            let mut file_ids: Vec<String> = indices
                .iter()
                .map(|i| all_nodes[*i].file_id.clone())
                .collect();
            file_ids.sort();
            file_ids.dedup();
            graph.duplicates.push(DuplicateRecord {
                gid: gid.clone(),
                file_ids,
                resolution: "all-definitions-retained-first-wins-identity".to_string(),
            });
        }
    }

    let _ = global_ns;
    graph.nodes = finalize_nodes(all_nodes, &node_by_gid);

    // Reference closure.
    build_edges(files, &alias_ctx, &mut graph, node_gids_by_file);

    // Dependencies.
    build_dependencies(files, &mut graph);

    // Sort deterministic outputs.
    sort_graph(&mut graph);
    graph
}

fn finalize_nodes(
    all: Vec<ResolvedNode>,
    by_gid: &BTreeMap<GlobalNodeId, Vec<usize>>,
) -> Vec<ResolvedNode> {
    // Canonical definition per gid = earliest import (files processed in order,
    // then node ordinal). Duplicate definitions remain recorded separately.
    let mut chosen: HashSet<GlobalNodeId> = HashSet::new();
    let mut out = Vec::new();
    for (_gid, indices) in by_gid {
        let first = *indices.first().unwrap();
        chosen.insert(all[first].gid.clone());
        out.push(all[first].clone());
    }
    let _ = chosen;
    out.sort_by(|a, b| {
        a.file_id
            .cmp(&b.file_id)
            .then(a.ordinal.cmp(&b.ordinal))
            .then(a.gid.cmp(&b.gid))
    });
    out
}

fn build_edges(
    files: &[ParsedFile],
    alias_ctx: &HashMap<String, (HashMap<String, String>, HashSet<String>)>,
    graph: &mut DerivedGraph,
    _node_gids: HashMap<String, HashSet<GlobalNodeId>>,
) {
    let present: HashSet<GlobalNodeId> = graph.nodes.iter().map(|n| n.gid.clone()).collect();
    let mut seen_edges: HashSet<EdgeKey> = HashSet::new();
    let mut absent_targets: Vec<(GlobalNodeId, GlobalNodeId, String)> = Vec::new();

    for f in files {
        let (alias_map, cycle_members) = alias_ctx.get(&f.file_id).cloned().unwrap_or_default();
        let ctx = ResolveCtx {
            file: f,
            alias_targets: &alias_map,
            alias_cycle_members: &cycle_members,
        };
        for raw_node in &f.nodes {
            let source_nid = match parse_node_id(&raw_node.node_id_raw) {
                Ok(n) => n,
                Err(_) => continue,
            };
            let source = match global_id_of(&source_nid, f) {
                Some(g) => g,
                None => continue,
            };
            for r in &raw_node.references {
                let type_ep = resolve_endpoint(&r.ref_type_raw, &ctx);
                let target_ep = resolve_endpoint(&r.target_raw, &ctx);
                let ref_type = type_ep.gid;
                let target_gid = target_ep.gid.clone();

                // Type unresolved -> diagnostic, no fabricated edge.
                if ref_type.is_none() {
                    graph.unresolved.push(UnresolvedRef {
                        source: Some(source.clone()),
                        endpoint_raw: r.ref_type_raw.clone(),
                        ref_type_raw: r.ref_type_raw.clone(),
                        reason: type_ep
                            .reason
                            .unwrap_or_else(|| "reference type unresolved".to_string()),
                        file_id: f.file_id.clone(),
                        range: r.range,
                    });
                    continue;
                }
                let ref_type = ref_type.unwrap();

                // Target unresolved (alias cycle, bad id, missing node): keep
                // an unresolved marker, never invent a node or edge.
                let target = match target_gid {
                    Some(g) => g,
                    None => {
                        let reason = if cycle_members.contains(r.target_raw.trim()) {
                            "alias cycle: target has no terminal NodeId".to_string()
                        } else {
                            target_ep.reason.unwrap_or_else(|| "target unresolved".to_string())
                        };
                        graph.unresolved.push(UnresolvedRef {
                            source: Some(source.clone()),
                            endpoint_raw: r.target_raw.clone(),
                            ref_type_raw: r.ref_type_raw.clone(),
                            reason,
                            file_id: f.file_id.clone(),
                            range: r.range,
                        });
                        continue;
                    }
                };

                if !present.contains(&target) {
                    // Dependency missing or target deleted: preserve unresolved
                    // node placeholder (recorded below) and no edge.
                    graph.unresolved.push(UnresolvedRef {
                        source: Some(source.clone()),
                        endpoint_raw: r.target_raw.clone(),
                        ref_type_raw: r.ref_type_raw.clone(),
                        reason: "target node absent from merged model (dangling)".to_string(),
                        file_id: f.file_id.clone(),
                        range: r.range,
                    });
                    absent_targets.push((
                        target.clone(),
                        source.clone(),
                        "target node absent from merged model (dangling)".to_string(),
                    ));
                    continue;
                }

                // Normalize direction to forward, without creating missing edges:
                // IsForward=false reverses endpoints of the exact stated edge.
                let (fwd_source, fwd_target) = if r.is_forward {
                    (source.clone(), target.clone())
                } else {
                    (target.clone(), source.clone())
                };
                let key = EdgeKey {
                    source: fwd_source.clone(),
                    target: fwd_target.clone(),
                    ref_type: ref_type.clone(),
                };
                if seen_edges.insert(key.clone()) {
                    graph.edges.push(ResolvedEdge {
                        key,
                        direction: "forward".to_string(),
                        source_file: f.file_id.clone(),
                        raw_is_forward: r.is_forward,
                        range: r.range,
                    });
                }
            }
        }
    }

    // Preserve unresolved *node* placeholders for referenced-but-absent
    // targets so a missing dependency / deleted node never silently disappears.
    let mut placeholders: BTreeMap<GlobalNodeId, UnresolvedNode> = BTreeMap::new();
    for (target_gid, source_gid, reason) in absent_targets.drain(..) {
        if present.contains(&target_gid) {
            continue;
        }
        let entry = placeholders.entry(target_gid.clone()).or_insert(UnresolvedNode {
            gid: target_gid.clone(),
            referenced_from: Vec::new(),
            reason,
        });
        if !entry.referenced_from.contains(&source_gid) {
            entry.referenced_from.push(source_gid);
        }
    }
    graph.unresolved_nodes = placeholders.into_values().collect();
    graph.unresolved_nodes.sort_by(|a, b| a.gid.cmp(&b.gid));
}


fn build_dependencies(files: &[ParsedFile], graph: &mut DerivedGraph) {
    fn available_for(consumer: &ParsedFile, files: &[ParsedFile]) -> HashMap<String, String> {
        let mut available: HashMap<String, String> = HashMap::new();
        for provider in files {
            if provider.model_uri.is_empty() || std::ptr::eq(provider, consumer) {
                continue;
            }
            let entry = available.entry(provider.model_uri.clone()).or_default();
            if entry.is_empty() || version_cmp(&provider.model_version, entry).is_gt() {
                *entry = provider.model_version.clone();
            }
        }
        available
    }

    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    for f in files {
        let available = available_for(f, files);
        for req in &f.requirements {
            let key = (f.file_id.clone(), req.model_uri.clone(), req.version.clone());
            if !seen.insert(key) {
                continue;
            }
            let avail = available.get(&req.model_uri).cloned();
            let (state, detail) = match &avail {
                None => (
                    "missing".to_string(),
                    "required model is not provided by another imported file".to_string(),
                ),
                Some(v) if version_cmp(v, &req.version).is_lt() => (
                    "conflict".to_string(),
                    format!(
                        "available {v} is older than required {} (version conflict)",
                        req.version
                    ),
                ),
                Some(v) => ("satisfied".to_string(), format!("available {v}")),
            };
            if state == "missing" {
                graph.diagnostics.push(DiagnosticEvent {
                    code: "MODEL_DEPENDENCY_MISSING".to_string(),
                    severity: "warning".to_string(),
                    file_id: f.file_id.clone(),
                    subject: req.model_uri.clone(),
                    message: format!("requires {} {} but it is not imported", req.model_uri, req.version),
                    basis: "missing dependencies are retained as unresolved, never guessed".to_string(),
                    range: Some(req.range),
                });
            }
            if state == "conflict" {
                graph.diagnostics.push(DiagnosticEvent {
                    code: "MODEL_VERSION_CONFLICT".to_string(),
                    severity: "error".to_string(),
                    file_id: f.file_id.clone(),
                    subject: req.model_uri.clone(),
                    message: detail.clone(),
                    basis: "required minimum version must be satisfied by an available model".to_string(),
                    range: Some(req.range),
                });
            }
            graph.dependencies.push(DependencyStatus {
                required_uri: req.model_uri.clone(),
                required_version: req.version.clone(),
                available_version: avail,
                state,
                detail,
            });
        }
    }
}


#[derive(PartialEq, Eq)]
enum Ord {
    Lt,
    Eq,
    Gt,
}
impl Ord {
    fn is_lt(&self) -> bool {
        matches!(self, Ord::Lt)
    }
    fn is_gt(&self) -> bool {
        matches!(self, Ord::Gt)
    }
}

/// Numeric dotted version compare; non-numeric tails fall back lexically.
fn version_cmp(a: &str, b: &str) -> Ord {
    if a == b {
        return Ord::Eq;
    }
    let pa: Vec<&str> = a.split('.').collect();
    let pb: Vec<&str> = b.split('.').collect();
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let x = pa.get(i).copied().unwrap_or("0");
        let y = pb.get(i).copied().unwrap_or("0");
        match (x.parse::<i64>(), y.parse::<i64>()) {
            (Ok(xn), Ok(yn)) => {
                if xn < yn {
                    return Ord::Lt;
                }
                if xn > yn {
                    return Ord::Gt;
                }
            }
            _ => {
                if x < y {
                    return Ord::Lt;
                }
                if x > y {
                    return Ord::Gt;
                }
            }
        }
    }
    Ord::Eq
}

fn normalize_quantity(gid: GlobalNodeId, q: &RawQuantity) -> Quantity {
    let val: Option<f64> = q.value_text.parse().ok();
    // Only a deliberately small, explicit set of units is normalized. Values
    // that cannot be converted stay in their original place with a diagnostic.
    let (normalized_value, normalized_unit, convertible, diagnostic) =
        match (val, q.unit.as_str()) {
            (Some(v), "kelvin") => (Some(v), Some("K".to_string()), true, None),
            (Some(v), "degC") | (Some(v), "celsius") => (
                Some(v + 273.15),
                Some("K".to_string()),
                true,
                None,
            ),
            (Some(v), "degF") => (
                Some((v - 32.0) * 5.0 / 9.0 + 273.15),
                Some("K".to_string()),
                true,
                None,
            ),
            (Some(v), "bar") => (Some(v * 100_000.0), Some("Pa".to_string()), true, None),
            (Some(v), "Pa") | (Some(v), "pascal") => {
                (Some(v), Some("Pa".to_string()), true, None)
            }
            (Some(_), other) => (
                None,
                None,
                false,
                Some(format!(
                    "unit '{other}' has no conversion rule; original value retained"
                )),
            ),
            (None, _) => (
                None,
                None,
                false,
                Some("quantity value is not numeric; original representation retained".to_string()),
            ),
        };
    Quantity {
        gid,
        value_text: q.value_text.clone(),
        unit: q.unit.clone(),
        coordinate_basis: q.coordinate_basis.clone(),
        normalized_value,
        normalized_unit,
        convertible,
        diagnostic,
        range: q.range,
    }
}

fn sort_graph(graph: &mut DerivedGraph) {
    graph.namespace_map.sort_by(|a, b| {
        a.file_id
            .cmp(&b.file_id)
            .then(a.file_index.cmp(&b.file_index))
    });
    graph.nodes.sort_by(|a, b| {
        a.file_id
            .cmp(&b.file_id)
            .then(a.ordinal.cmp(&b.ordinal))
            .then(a.gid.cmp(&b.gid))
    });
    graph.edges.sort_by(|a, b| a.key.cmp(&b.key));
    graph.unresolved.sort_by(|a, b| {
        a.file_id
            .cmp(&b.file_id)
            .then(a.range.start_byte.cmp(&b.range.start_byte))
    });
    graph.quantities.sort_by(|a, b| a.gid.cmp(&b.gid));
    graph.dependencies.sort_by(|a, b| {
        a.required_uri
            .cmp(&b.required_uri)
            .then(a.required_version.cmp(&b.required_version))
    });
    graph.duplicates.sort_by(|a, b| a.gid.cmp(&b.gid));
    graph.alias_cycles.sort_by(|a, b| a.file_id.cmp(&b.file_id));
    graph.diagnostics.sort_by(|a, b| {
        a.code
            .cmp(&b.code)
            .then(a.file_id.cmp(&b.file_id))
            .then(a.subject.cmp(&b.subject))
    });

}
