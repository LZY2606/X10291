//! Merge engine: file-local NodeSets -> one canonical derived model.
//!
//! Guaranteed semantics:
//! * numeric namespace indices are only used inside one file; merging is by URI
//! * forward/inverse references are normalized to one direction without
//!   inventing mirror edges
//! * missing dependencies, missing alias targets and dangling references keep
//!   unresolved stub nodes and are never silently dropped
//! * ambiguity (alias cycles, duplicate ids, version conflicts) is reported
//!   with *all* legal candidates, never collapsed

use crate::identity;
use crate::model::*;
use crate::nodeid;
use crate::raw;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Manual adjudication recorded against a conflict subject.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Decisions {
    /// subject (e.g. canonical node id) -> chosen candidate raw_node_id
    pub duplicates: BTreeMap<String, String>,
    /// edge id -> keep ("keep") or reject ("drop")
    pub edges: BTreeMap<String, String>,
    /// requirement subject "uri@file" -> accept version "accept:<v>"
    pub requirements: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecomputeReport {
    pub reused_nodes: Vec<String>,
    pub rebuilt_nodes: Vec<String>,
    pub new_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub reasons: Vec<String>,
}

struct Ctx {
    events: Vec<Event>,
    unresolved: Vec<Unresolved>,
    seq: u64,
    branch: String,
}

impl Ctx {
    fn new(branch: &str) -> Self {
        Ctx {
            events: Vec::new(),
            unresolved: Vec::new(),
            seq: 0,
            branch: branch.to_string(),
        }
    }

    fn event(
        &mut self,
        kind: &str,
        severity: &str,
        subject: impl Into<String>,
        message: impl Into<String>,
        evidence: Vec<SpanRef>,
        candidates: Vec<String>,
    ) -> Event {
        self.seq += 1;
        let subject = subject.into();
        Event {
            id: identity::event_id(&self.branch, self.seq, kind, &subject),
            seq: self.seq,
            kind: kind.to_string(),
            severity: severity.to_string(),
            subject,
            message: message.into(),
            rule: RULE_SET_VERSION.into(),
            evidence,
            candidates,
            branch: self.branch.clone(),
        }
    }

    fn push(
        &mut self,
        kind: &str,
        severity: &str,
        subject: impl Into<String>,
        message: impl Into<String>,
        evidence: Vec<SpanRef>,
        candidates: Vec<String>,
    ) {
        let e = self.event(kind, severity, subject, message, evidence, candidates);
        self.events.push(e);
    }
}

fn span(file: &raw::SourceFile, span: (usize, usize), label: &str) -> SpanRef {
    SpanRef {
        file_id: file.id.clone(),
        path: file.path.clone(),
        start: span.0,
        end: span.1,
        label: label.to_string(),
    }
}

fn content_hash(s: &str) -> String {
    identity::short_hash(&[s])
}

/// Resolve the namespace URI for a file-local numeric index.
fn uri_for_index(file: &raw::SourceFile, idx: u16) -> Option<String> {
    if idx == 0 {
        return Some(UA_NS_URI.to_string());
    }
    file.model
        .namespaces
        .iter()
        .find(|n| n.index == idx)
        .map(|n| n.uri.clone())
}

fn file_ns_span(file: &raw::SourceFile, idx: u16) -> Option<(usize, usize)> {
    file.model
        .namespaces
        .iter()
        .find(|n| n.index == idx)
        .map(|n| n.span)
}

pub fn build(files: Vec<raw::SourceFile>, branch: &str, decisions: &Decisions) -> DerivedModel {
    let mut ctx = Ctx::new(branch);
    let mut model = DerivedModel {
        rule_set_version: RULE_SET_VERSION.into(),
        branch: branch.to_string(),
        ..Default::default()
    };

    // ---- files (deterministic import order) ----
    model.files = files
        .iter()
        .enumerate()
        .map(|(order, f)| FileDigest {
            file_id: f.id.clone(),
            path: f.path.clone(),
            content_hash: content_hash(&f.content),
            order,
        })
        .collect();

    build_namespaces(&files, &mut model, &mut ctx);
    build_models(&files, &mut model, &mut ctx, decisions);
    build_aliases(&files, &mut model, &mut ctx);
    build_nodes(&files, &mut model, &mut ctx, decisions);
    build_edges(&files, &mut model, &mut ctx, decisions);
    build_quantities_and_coords(&files, &mut model, &mut ctx);
    build_extensions(&files, &mut model);

    // ---- stable ordering ----
    model.edges.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then(a.id.cmp(&b.id))
            .then(a.target.cmp(&b.target))
    });
    model.nodes.sort_by(|a, b| {
        a.status
            .starts_with("unresolved")
            .cmp(&b.status.starts_with("unresolved"))
            .then(a.canonical_node_id.cmp(&b.canonical_node_id))
    });
    model.events.sort_by_key(|e| e.seq);
    model.unresolved.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(a.subject.cmp(&b.subject))
            .then(a.id.cmp(&b.id))
    });

    model
}

fn build_namespaces(files: &[raw::SourceFile], model: &mut DerivedModel, ctx: &mut Ctx) {
    let mut order_seen: BTreeSet<String> = BTreeSet::new();
    for f in files {
        // index 0 is implicit UA namespace
        if !order_seen.contains(UA_NS_URI) {
            order_seen.insert(UA_NS_URI.to_string());
            model.namespaces.push(NsEntry {
                uri: UA_NS_URI.to_string(),
                canonical_index: 0,
                bindings: vec![],
            });
        }
        for ns in &f.model.namespaces {
            if !order_seen.contains(&ns.uri) {
                order_seen.insert(ns.uri.clone());
                model.namespaces.push(NsEntry {
                    uri: ns.uri.clone(),
                    canonical_index: 0,
                    bindings: vec![],
                });
            }
            let entry = model
                .namespaces
                .iter_mut()
                .find(|e| e.uri == ns.uri)
                .expect("uri inserted");
            entry.bindings.push(NsBinding {
                file_id: f.id.clone(),
                path: f.path.clone(),
                index: ns.index,
                span: Some(ns.span),
            });
        }
    }
    for (i, e) in model.namespaces.iter_mut().enumerate() {
        e.canonical_index = i as u16;
    }

    // namespace re-ordering: same URI bound to different numeric indices
    let mut by_uri: BTreeMap<String, Vec<(u16, String)>> = BTreeMap::new();
    for f in files {
        for ns in &f.model.namespaces {
            by_uri
                .entry(ns.uri.clone())
                .or_default()
                .push((ns.index, f.path.clone()));
        }
    }
    for (uri, bindings) in by_uri {
        let distinct: BTreeSet<u16> = bindings.iter().map(|(i, _)| *i).collect();
        if distinct.len() > 1 {
            let evidence: Vec<SpanRef> = files
                .iter()
                .flat_map(|f| {
                    f.model
                        .namespaces
                        .iter()
                        .filter(|n| n.uri == uri)
                        .map(|n| span(f, n.span, "namespace table Uri"))
                        .collect::<Vec<_>>()
                })
                .collect();
            ctx.push(
                "namespace_reindexed",
                "info",
                uri.clone(),
                format!(
                    "namespace URI appears at file-local indices {:?}; merged by URI with canonical index",
                    distinct.iter().collect::<Vec<_>>()
                ),
                evidence,
                vec![],
            );
        }
    }
}

fn build_models(files: &[raw::SourceFile], model: &mut DerivedModel, ctx: &mut Ctx, decisions: &Decisions) {
    // provided models
    let mut provided: BTreeMap<String, ModelEntry> = BTreeMap::new();
    for f in files {
        for m in &f.model.models {
            let id = identity::model_id(&m.uri, m.version.as_deref().unwrap_or("-"));
            let entry = provided.entry(m.uri.clone()).or_insert_with(|| ModelEntry {
                id,
                uri: m.uri.clone(),
                version: m.version.clone(),
                publication_date: m.publication_date.clone(),
                provided_by: vec![],
                requirements: vec![],
            });
            if !entry.provided_by.contains(&f.path) {
                entry.provided_by.push(f.path.clone());
            }
        }
    }

    // requirements
    for f in files {
        for m in &f.model.models {
            for r in &m.required {
                let subject = format!("{}@{}", r.uri, f.path);
                let sp = span(f, r.span, "RequiredModel");
                let provided_entry = provided.get(&r.uri);
                let status;
                let mut resolved_by = None;
                match provided_entry {
                    None => {
                        status = "missing".to_string();
                        ctx.push(
                            "model_dependency_missing",
                            "error",
                            subject.clone(),
                            format!(
                                "required model {} {} is not provided by any imported file; nodes referencing it remain unresolved",
                                r.uri,
                                r.version.as_deref().unwrap_or("(any)")
                            ),
                            vec![sp.clone()],
                            vec![],
                        );
                        model.unresolved.push(Unresolved {
                            id: format!("unres-{}", identity::short_hash(&["model", &subject])),
                            kind: "model".into(),
                            subject: subject.clone(),
                            reason: "required model missing from import set".into(),
                            evidence: vec![sp.clone()],
                        });
                    }
                    Some(p) => {
                        resolved_by = p.version.clone();
                        let req_v = r.version.clone().unwrap_or_default();
                        let got_v = p.version.clone().unwrap_or_default();
                        if req_v == got_v || req_v.is_empty() {
                            status = "satisfied".to_string();
                        } else if let Some(choice) = decisions.requirements.get(&subject) {
                            status = format!("accepted:{}", choice);
                            ctx.push(
                                "model_version_decision",
                                "info",
                                subject.clone(),
                                format!(
                                    "manual decision {} accepted for {} (wanted {}, imported {})",
                                    choice, r.uri, req_v, got_v
                                ),
                                vec![sp.clone()],
                                vec![format!("accept:{}", got_v), format!("reject:{}", got_v)],
                            );
                        } else {
                            status = "version_conflict".to_string();
                            ctx.push(
                                "model_version_conflict",
                                "warn",
                                subject.clone(),
                                format!(
                                    "{} requires {} but the import set provides {}",
                                    f.path, req_v, got_v
                                ),
                                vec![sp.clone()],
                                vec![format!("accept:{}", got_v), format!("reject:{}", got_v)],
                            );
                            model.unresolved.push(Unresolved {
                                id: format!("unres-{}", identity::short_hash(&["model", &subject])),
                                kind: "model".into(),
                                subject: subject.clone(),
                                reason: format!("wanted {}, provided {}", req_v, got_v),
                                evidence: vec![sp.clone()],
                            });
                        }
                    }
                }
                if let Some(entry) = provided.get_mut(&m.uri) {
                    entry.requirements.push(ModelRequirement {
                        uri: r.uri.clone(),
                        requested_version: r.version.clone(),
                        status,
                        resolved_by,
                        span: sp,
                    });
                }
            }
        }
    }

    let mut entries: Vec<ModelEntry> = provided.into_values().collect();
    entries.sort_by(|a, b| a.uri.cmp(&b.uri));
    model.models = entries;
}

fn build_aliases(files: &[raw::SourceFile], model: &mut DerivedModel, ctx: &mut Ctx) {
    for f in files {
        // map within this file only
        let map: HashMap<&str, &str> = f
            .model
            .aliases
            .iter()
            .map(|a| (a.name.as_str(), a.target.as_str()))
            .collect();

        for a in &f.model.aliases {
            let sp = span(f, a.span, "Alias");
            let mut chain = vec![a.name.clone(), a.target.clone()];
            let mut seen: BTreeSet<String> = BTreeSet::new();
            seen.insert(a.name.clone());
            let mut cur = a.target.as_str();
            let mut status;
            loop {
                if seen.contains(cur) {
                    status = "cycle".to_string();
                    chain.push(cur.to_string());
                    ctx.push(
                        "alias_cycle",
                        "error",
                        a.id.clone(),
                        format!("alias chain starting at '{}' closes back on '{}'; reference cannot be resolved and is kept unresolved", a.name, cur),
                        vec![sp.clone()],
                        vec![],
                    );
                    model.unresolved.push(Unresolved {
                        id: format!("unres-{}", identity::short_hash(&["alias", &a.id, cur])),
                        kind: "alias".into(),
                        subject: cur.to_string(),
                        reason: "alias cycle".into(),
                        evidence: vec![sp.clone()],
                    });
                    break;
                }
                match map.get(cur) {
                    Some(next) => {
                        seen.insert(cur.to_string());
                        chain.push((*next).to_string());
                        cur = next;
                    }
                    None => {
                        // terminal must be a NodeId literal
                        match nodeid::parse(cur) {
                            Ok(_) => status = "resolved".to_string(),
                            Err(e) => {
                                status = "unresolved".to_string();
                                ctx.push(
                                    "alias_dangling",
                                    "warn",
                                    a.id.clone(),
                                    format!("alias '{}' terminates at '{}' which is not a NodeId: {}", a.name, cur, e.reason),
                                    vec![sp.clone()],
                                    vec![],
                                );
                                model.unresolved.push(Unresolved {
                                    id: format!("unres-{}", identity::short_hash(&["alias", &a.id, cur])),
                                    kind: "alias".into(),
                                    subject: cur.to_string(),
                                    reason: e.reason,
                                    evidence: vec![sp.clone()],
                                });
                            }
                        }
                        break;
                    }
                }
            }
            model.aliases.push(AliasEntry {
                id: a.id.clone(),
                file_id: f.id.clone(),
                name: a.name.clone(),
                target: a.target.clone(),
                status,
                chain,
                span: sp,
            });
        }
    }
    model
        .aliases
        .sort_by(|a, b| a.file_id.cmp(&b.file_id).then(a.name.cmp(&b.name)));
}

/// Final node-id resolution inside one file: follows aliases, detects cycles.
struct ResolvedRefTarget {
    terminal: String,
    status: String,
    evidence: Vec<SpanRef>,
}

fn resolve_target(f: &raw::SourceFile, raw_target: &str) -> ResolvedRefTarget {
    // direct NodeId?
    if nodeid::parse(raw_target).is_ok() {
        return ResolvedRefTarget {
            terminal: raw_target.to_string(),
            status: "literal".into(),
            evidence: vec![],
        };
    }
    let map: HashMap<&str, &raw::RawAlias> = f
        .model
        .aliases
        .iter()
        .map(|a| (a.name.as_str(), a))
        .collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut cur = raw_target.to_string();
    let mut evidence = vec![];
    loop {
        if seen.contains(&cur) {
            return ResolvedRefTarget {
                terminal: cur,
                status: "alias_cycle".into(),
                evidence,
            };
        }
        match map.get(cur.as_str()) {
            Some(a) => {
                seen.insert(cur.clone());
                evidence.push(span(f, a.span, "Alias hop"));
                cur = a.target.clone();
            }
            None => {
                let status = if nodeid::parse(&cur).is_ok() {
                    "literal".into()
                } else {
                    "alias_unresolved".into()
                };
                return ResolvedRefTarget {
                    terminal: cur,
                    status,
                    evidence,
                };
            }
        }
    }
}

fn parse_browse_name(b: &str) -> (Option<String>, String) {
    match b.split_once(':') {
        Some((idx, name)) => (idx.parse::<u16>().ok().map(|i| i.to_string()), name.to_string()),
        None => (None, b.to_string()),
    }
}

fn build_nodes(files: &[raw::SourceFile], model: &mut DerivedModel, ctx: &mut Ctx, decisions: &Decisions) {
    // group raw nodes by canonical (uri, literal)
    let mut groups: BTreeMap<String, Vec<(usize, &raw::SourceFile, &raw::RawNode, String, String, String)>> =
        BTreeMap::new();

    for (fi, f) in files.iter().enumerate() {
        for n in &f.model.nodes {
            if n.kind == "__ExtensionCarrier__" {
                continue;
            }
            let parsed = match nodeid::parse(&n.node_id) {
                Ok(p) => p,
                Err(e) => {
                    ctx.push(
                        "bad_nodeid",
                        "error",
                        n.id.clone(),
                        format!("NodeId '{}' cannot be parsed: {}", n.node_id, e.reason),
                        vec![span(f, n.span, "node")],
                        vec![],
                    );
                    model.unresolved.push(Unresolved {
                        id: format!("unres-{}", identity::short_hash(&["nodeid", &n.id])),
                        kind: "node".into(),
                        subject: n.node_id.clone(),
                        reason: e.reason,
                        evidence: vec![span(f, n.span, "node")],
                    });
                    continue;
                }
            };
            let uri = match uri_for_index(f, parsed.ns_index) {
                Some(u) => u,
                None => {
                    ctx.push(
                        "namespace_index_unknown",
                        "error",
                        n.id.clone(),
                        format!(
                            "file-local ns={} has no Uri in this file's namespace table",
                            parsed.ns_index
                        ),
                        vec![span(f, n.span, "node")],
                        vec![],
                    );
                    model.unresolved.push(Unresolved {
                        id: format!("unres-{}", identity::short_hash(&["ns", &n.id])),
                        kind: "node".into(),
                        subject: n.node_id.clone(),
                        reason: format!("ns index {} not declared", parsed.ns_index),
                        evidence: vec![span(f, n.span, "node")],
                    });
                    continue;
                }
            };
            let gid = identity::global_node_id(&uri, &format!("{}={}", parsed.id_kind, parsed.literal));
            groups
                .entry(gid)
                .or_default()
                .push((fi, f, n, uri.clone(), parsed.id_kind.to_string(), parsed.literal.clone()));
        }
    }

    for (gid, group) in groups {
        let (_, _, _, uri, kind, literal) = &group[0];
        let mut sources = Vec::new();
        let mut local_ids = Vec::new();
        let mut candidate_ids = Vec::new();
        for (_, f, n, _, _, _) in &group {
            sources.push(NodeProvenance {
                file_id: f.id.clone(),
                path: f.path.clone(),
                raw_node_id: n.id.clone(),
                span: n.span,
                raw_node_id_attr: n.node_id.clone(),
            });
            local_ids.push(format!("{}!{}", f.path, n.node_id));
            candidate_ids.push(n.id.clone());
        }

        let mut status = "ok".to_string();
        let mut winner = None;
        if group.len() > 1 {
            status = "duplicate_conflict".to_string();
            if let Some(chosen_raw) = decisions.duplicates.get(&gid) {
                winner = Some(chosen_raw.clone());
                let winner_file = group
                    .iter()
                    .find(|(_, _, n, _, _, _)| &n.id == chosen_raw)
                    .map(|(_, f, _, _, _, _)| f.path.clone());
                ctx.push(
                    "duplicate_resolved",
                    "info",
                    gid.clone(),
                    format!(
                        "duplicate NodeId {} resolved manually in favour of {}",
                        canonical_display(uri, kind, literal),
                        winner_file.unwrap_or_default()
                    ),
                    group
                        .iter()
                        .map(|(_, f, n, _, _, _)| span(f, n.span, "duplicate node"))
                        .collect(),
                    candidate_ids.clone(),
                );
            } else {
                ctx.push(
                    "duplicate_nodeid",
                    "error",
                    gid.clone(),
                    format!(
                        "NodeId {} is declared by {} files; all candidates retained, no default winner chosen",
                        canonical_display(uri, kind, literal),
                        group.len()
                    ),
                    group
                        .iter()
                        .map(|(_, f, n, _, _, _)| span(f, n.span, "duplicate node"))
                        .collect(),
                    candidate_ids.clone(),
                );
                model.unresolved.push(Unresolved {
                    id: format!("unres-{}", identity::short_hash(&["dup", &gid])),
                    kind: "node".into(),
                    subject: canonical_display(uri, kind, literal),
                    reason: "duplicate NodeId across files".into(),
                    evidence: group
                        .iter()
                        .map(|(_, f, n, _, _, _)| span(f, n.span, "duplicate node"))
                        .collect(),
                });
            }
        }

        // choose representative: explicit winner, else first by order
        let chosen = if let Some(w) = &winner {
            group
                .iter()
                .find(|(_, _, n, _, _, _)| &n.id == w)
                .unwrap_or(&group[0])
        } else {
            &group[0]
        };
        let (_, cfile, cnode, _, _, _) = chosen;
        let (browse_uri, browse) = match &cnode.browse_name {
            Some(b) => {
                let (idx, name) = parse_browse_name(b);
                let bu = idx.and_then(|i| {
                    i.parse::<u16>()
                        .ok()
                        .and_then(|n| uri_for_index(cfile, n))
                });
                (bu, Some(name))
            }
            None => (None, None),
        };

        // fingerprint content of all raw occurrences, so any edit anywhere
        // that touches this node is detected by the recompute propagation
        let mut fp_parts: Vec<String> = sources
            .iter()
            .flat_map(|s| {
                vec![
                    s.file_id.clone(),
                    s.raw_node_id_attr.clone(),
                    format!("{}-{}", s.span.0, s.span.1),
                ]
            })
            .collect();
        fp_parts.push(status.clone());
        fp_parts.push(winner.clone().unwrap_or_default());
        let fp = identity::fingerprint(&fp_parts);

        model.nodes.push(DerivedNode {
            id: gid.clone(),
            ns_uri: uri.clone(),
            id_kind: kind.clone(),
            literal: literal.clone(),
            canonical_node_id: canonical_display(uri, kind, literal),
            local_node_ids: local_ids,
            browse_name: browse,
            browse_name_uri: browse_uri,
            display_name: cnode.display_name.clone(),
            description: cnode.description.clone(),
            kind: cnode.kind.clone(),
            status,
            fingerprint: fp,
            sources,
            winner,
            candidates: if group.len() > 1 { candidate_ids } else { vec![] },
            quantity: None,
            coordinate: None,
        });
    }

}

fn canonical_display(uri: &str, kind: &str, literal: &str) -> String {
    format!("uri={};{}={}", uri, kind, literal)
}

fn canonical_key_for(file: &raw::SourceFile, nodeid_literal: &str) -> Option<(String, String, String)> {
    let p = nodeid::parse(nodeid_literal).ok()?;
    let uri = uri_for_index(file, p.ns_index)?;
    Some((uri, p.id_kind.to_string(), p.literal))
}

fn build_edges(files: &[raw::SourceFile], model: &mut DerivedModel, ctx: &mut Ctx, decisions: &Decisions) {
    // index existing canonical nodes
    let mut have: BTreeSet<String> = model.nodes.iter().map(|n| n.id.clone()).collect();

    for f in files {
        for n in &f.model.nodes {
            // owner canonical endpoints: only for parseable node ids
            let owner_key = match canonical_key_for(f, &n.node_id) {
                Some(k) => k,
                None => continue,
            };
            let owner_gid = identity::global_node_id(
                &owner_key.0,
                &format!("{}={}", owner_key.1, owner_key.2),
            );

            for r in &n.refs {
                let sp = span(f, r.span, "Reference");

                // reference type: alias? numeric?
                let mut ref_type_uri = None;
                let mut ref_resolved = false;
                let p = nodeid::parse(&r.reference_type);
                if let Ok(p) = p {
                    if let Some(u) = uri_for_index(f, p.ns_index) {
                        ref_type_uri = Some(format!("{}|{}={}", u, p.id_kind, p.literal));
                        ref_resolved = true;
                    }
                } else {
                    // alias for reference type
                    let resolved = resolve_target(f, &r.reference_type);
                    if resolved.status == "literal" {
                        if let Some(k) = canonical_key_for(f, &resolved.terminal) {
                            ref_type_uri =
                                Some(format!("{}|{}={}", k.0, k.1, k.2));
                            ref_resolved = true;
                        }
                    }
                }
                let ref_type = ref_type_uri.unwrap_or_else(|| format!("unresolved|{}", r.reference_type));
                if !ref_resolved {
                    ctx.push(
                        "reference_type_unresolved",
                        "warn",
                        format!("{}/{}", owner_gid, r.reference_type),
                        format!(
                            "reference type '{}' could not be resolved to a namespace URI; edge retained",
                            r.reference_type
                        ),
                        vec![sp.clone()],
                        vec![],
                    );
                    model.unresolved.push(Unresolved {
                        id: format!(
                            "unres-{}",
                            identity::short_hash(&["reftype", &owner_gid, &r.reference_type])
                        ),
                        kind: "ref_type".into(),
                        subject: r.reference_type.clone(),
                        reason: "reference type alias/index unresolvable".into(),
                        evidence: vec![sp.clone()],
                    });
                }

                // target resolution through alias chain
                let target_res = resolve_target(f, &r.target);
                let mut edge_status = "resolved".to_string();
                let mut target_key: Option<(String, String, String)> = None;
                if target_res.status == "literal" {
                    target_key = canonical_key_for(f, &target_res.terminal);
                    if target_key.is_none() {
                        edge_status = "unresolved_alias".to_string();
                        ctx.push(
                            "reference_target_ns_unknown",
                            "error",
                            format!("{}/{}", owner_gid, r.target),
                            format!(
                                "reference target '{}' has a file-local namespace index with no Uri in this file; edge retained unresolved",
                                r.target
                            ),
                            vec![sp.clone()],
                            vec![],
                        );
                        model.unresolved.push(Unresolved {
                            id: format!(
                                "unres-{}",
                                identity::short_hash(&["ns-edge", &owner_gid, &r.target])
                            ),
                            kind: "node".into(),
                            subject: r.target.clone(),
                            reason: "target namespace index not declared in file".into(),
                            evidence: vec![sp.clone()],
                        });
                    }
                }
                if target_res.status != "literal" {
                    edge_status = "unresolved_alias".to_string();
                    ctx.push(
                        "reference_target_alias_broken",
                        "error",
                        format!("{}/{}", owner_gid, r.target),
                        format!(
                            "reference target '{}' is an alias that {}; edge retained with unresolved target",
                            r.target,
                            if target_res.status == "alias_cycle" { "is part of a cycle" } else { "does not terminate at a NodeId" }
                        ),
                        {
                            let mut ev = vec![sp.clone()];
                            ev.extend(target_res.evidence);
                            ev
                        },
                        vec![],
                    );
                    model.unresolved.push(Unresolved {
                        id: format!(
                            "unres-{}",
                            identity::short_hash(&["alias-edge", &owner_gid, &r.target])
                        ),
                        kind: "alias".into(),
                        subject: r.target.clone(),
                        reason: target_res.status.clone(),
                        evidence: vec![sp.clone()],
                    });
                }

                // canonical endpoints (inverse normalization; no mirror created)
                let (source_gid, target_gid, from_inverse) = if r.is_forward {
                    match &target_key {
                        Some(k) => (
                            owner_gid.clone(),
                            identity::global_node_id(&k.0, &format!("{}={}", k.1, k.2)),
                            false,
                        ),
                        None => (
                            owner_gid.clone(),
                            format!(
                                "pending-{}",
                                identity::short_hash(&[f.id.as_str(), &r.target, &r.reference_type])
                            ),
                            false,
                        ),
                    }
                } else {
                    // inverse: this node is the *target* of a forward edge from target node
                    match &target_key {
                        Some(k) => (
                            identity::global_node_id(&k.0, &format!("{}={}", k.1, k.2)),
                            owner_gid.clone(),
                            true,
                        ),
                        None => (
                            format!(
                                "pending-{}",
                                identity::short_hash(&[f.id.as_str(), &r.target, &r.reference_type])
                            ),
                            owner_gid.clone(),
                            true,
                        ),
                    }
                };

                if edge_status == "resolved" && !have.contains(&target_gid) {
                    // dangling after deletion: keep stub node + edge, do not invent content
                    edge_status = "dangling".to_string();
                    let stub_uri_lit = match &target_key {
                        Some(k) => (k.0.clone(), k.1.clone(), k.2.clone()),
                        None => (String::new(), "?".into(), String::new()),
                    };
                    have.insert(target_gid.clone());
                    model.nodes.push(DerivedNode {
                        id: target_gid.clone(),
                        ns_uri: stub_uri_lit.0.clone(),
                        id_kind: stub_uri_lit.1.clone(),
                        literal: stub_uri_lit.2.clone(),
                        canonical_node_id: canonical_display(&stub_uri_lit.0, &stub_uri_lit.1, &stub_uri_lit.2),
                        kind: "UnresolvedStub".into(),
                        status: "unresolved_stub".to_string(),
                        fingerprint: identity::fingerprint(&[
                            "stub".into(),
                            target_gid.clone(),
                        ]),
                        ..Default::default()
                    });
                    ctx.push(
                        "dangling_reference",
                        "error",
                        target_gid.clone(),
                        format!(
                            "reference points at {} which does not exist in the merged model; unresolved stub retained",
                            match &target_key {
                                Some(k) => canonical_display(&k.0, &k.1, &k.2),
                                None => r.target.clone(),
                            }
                        ),
                        vec![sp.clone()],
                        vec![],
                    );
                    model.unresolved.push(Unresolved {
                        id: format!("unres-{}", identity::short_hash(&["dangle", &target_gid])),
                        kind: "node".into(),
                        subject: r.target.clone(),
                        reason: "referenced node absent (deleted or not imported)".into(),
                        evidence: vec![sp.clone()],
                    });
                }

                if !ref_resolved {
                    edge_status = "unresolved_type".to_string();
                }

                let edge_id = identity::edge_id(&source_gid, &ref_type, &target_gid, if from_inverse { "inverse" } else { "forward" });

                // rejected by an earlier adjudication?
                if decisions.edges.get(&edge_id).map(String::as_str) == Some("drop") {
                    ctx.push(
                        "edge_rejected",
                        "info",
                        edge_id.clone(),
                        "edge removed by a retained manual decision (candidate not promoted)".to_string(),
                        vec![sp.clone()],
                        vec![],
                    );
                    continue;
                }

                let prov = RawRefProvenance {
                    file_id: f.id.clone(),
                    path: f.path.clone(),
                    owner_node: n.id.clone(),
                    raw_target: r.target.clone(),
                    raw_type: r.reference_type.clone(),
                    raw_is_forward: r.is_forward,
                    span: r.span,
                };
                if let Some(existing) = model.edges.iter_mut().find(|e| e.id == edge_id) {
                    if !existing.provenance.iter().any(|p| p.file_id == prov.file_id && p.span == prov.span) {
                        existing.provenance.push(prov);
                        existing.fingerprint = identity::fingerprint(&[
                            existing.fingerprint.clone(),
                            edge_id.clone(),
                        ]);
                    }
                    continue;
                }

                model.edges.push(Edge {
                    id: edge_id,
                    source: source_gid,
                    target: target_gid,
                    reference_type: ref_type,
                    reference_type_resolved: ref_resolved,
                    from_inverse,
                    status: edge_status,
                    provenance: vec![prov],
                    fingerprint: String::new(),
                });
                if from_inverse {
                    ctx.push(
                        "inverse_normalized",
                        "info",
                        identity::short_hash(&[&owner_gid, &r.target]),
                        format!(
                            "inverse reference '{}' normalized to forward direction; no reciprocal edge fabricated",
                            r.reference_type
                        ),
                        vec![sp.clone()],
                        vec![],
                    );
                }
            }
        }
    }

    // fill fingerprints
    for e in &mut model.edges {
        let parts: Vec<String> = e
            .provenance
            .iter()
            .map(|p| format!("{}:{}-{}", p.file_id, p.span.0, p.span.1))
            .collect();
        e.fingerprint = identity::fingerprint(&[e.id.clone(), identity::fingerprint(&parts)]);
    }
}

const CONVERSIONS: &[(&str, &str, f64, f64)] = &[
    // (from unit, canonical unit, scale, offset): canonical = value * scale + offset
    ("degC", "K", 1.0, 273.15),
    ("bar", "kPa", 100.0, 0.0),
    ("mm", "m", 0.001, 0.0),
    ("kPa", "kPa", 1.0, 0.0),
    ("K", "K", 1.0, 0.0),
    ("m", "m", 1.0, 0.0),
];

fn build_quantities_and_coords(files: &[raw::SourceFile], model: &mut DerivedModel, ctx: &mut Ctx) {
    #[allow(clippy::map_clone)]
    let node_meta: HashMap<String, (String, Option<String>)> = model
        .nodes
        .iter()
        .map(|n| (n.id.clone(), (n.status.clone(), n.winner.clone())))
        .collect();

    for f in files {
        for n in &f.model.nodes {
            let key = match canonical_key_for(f, &n.node_id) {
                Some(k) => k,
                None => continue,
            };
            let gid = identity::global_node_id(&key.0, &format!("{}={}", key.1, key.2));
            if !node_meta.contains_key(&gid) {
                continue;
            }
            // only annotate the winning occurrence
            let winner = node_meta.get(&gid).and_then(|(_, w)| w.clone());
            if winner
                .as_ref()
                .map(|w| w != &n.id)
                .unwrap_or(false)
            {
                continue;
            }

            let value_text = n
                .children_known
                .iter()
                .find(|c| c.local == "Value")
                .map(|c| c.text.trim().to_string());

            // nc:EngineeringUnit
            let unit_ext = n.unknown_extensions.iter().find(|e| e.tag.ends_with("EngineeringUnit"));
            if let (Some(raw_value), Some(unit_ext)) = (value_text.clone(), unit_ext) {
                // extract unit attr from raw_xml
                let raw_unit = extract_attr(&unit_ext.raw_xml, "unit").unwrap_or_else(|| "?".into());
                let parsed_value: Option<f64> = raw_value.parse().ok();
                let rule = CONVERSIONS.iter().find(|(from, _, _, _)| *from == raw_unit);
                let mut canonical_value = None;
                let mut canonical_unit = None;
                let mut converted = false;
                let mut diagnostic = None;
                match (parsed_value, rule) {
                    (Some(v), Some((_, to, scale, offset))) => {
                        canonical_value = Some(v * scale + offset);
                        canonical_unit = Some((*to).to_string());
                        converted = true;
                    }
                    (None, _) => {
                        diagnostic = Some(format!(
                            "value '{}' is not numeric in rule set {}; raw representation retained in place",
                            raw_value, RULE_SET_VERSION
                        ));
                    }
                    (Some(_), None) => {
                        diagnostic = Some(format!(
                            "no conversion rule for unit '{}' in rule set {}; raw value and unit retained, no normalization",
                            raw_unit, RULE_SET_VERSION
                        ));
                    }
                }
                let qid = format!("qty-{}", identity::short_hash(&[&gid, &raw_unit, &raw_value]));
                let sp = span(f, unit_ext.span, "EngineeringUnit");
                model.quantities.push(Quantity {
                    id: qid.clone(),
                    node: gid.clone(),
                    raw_value,
                    raw_unit: raw_unit.clone(),
                    canonical_value,
                    canonical_unit,
                    converted,
                    diagnostic: diagnostic.clone(),
                    rule_version: RULE_SET_VERSION.into(),
                    span: sp.clone(),
                });
                if let Some(msg) = diagnostic {
                    ctx.push(
                        "unit_unconvertible",
                        "warn",
                        qid,
                        msg,
                        vec![sp],
                        vec![],
                    );
                    model.unresolved.push(Unresolved {
                        id: format!("unres-{}", identity::short_hash(&["unit", &gid, &raw_unit])),
                        kind: "unit".into(),
                        subject: raw_unit,
                        reason: "no conversion rule".into(),
                        evidence: vec![span(f, unit_ext.span, "EngineeringUnit")],
                    });
                }
                if let Some(node) = model.nodes.iter_mut().find(|nn| nn.id == gid) {
                    node.quantity = Some(format!("qty:{}", gid));
                }
            }

            // nc:Coordinate
            if let Some(coord_ext) = n.unknown_extensions.iter().find(|e| e.tag.ends_with("Coordinate")) {
                let frame = extract_attr(&coord_ext.raw_xml, "frame").unwrap_or_else(|| "unspecified".into());
                let raw_values: Vec<String> = coord_ext
                    .raw_xml
                    .lines()
                    .last()
                    .unwrap_or("")
                    .split(|c| c == ';' || c == ' ')
                    .filter(|s| !s.is_empty() && !s.contains('<') && !s.contains('>'))
                    .map(|s| s.trim().to_string())
                    .collect();
                // Better: inner text via second parse is heavy; use attribute "values"
                let values_attr = extract_attr(&coord_ext.raw_xml, "values");
                let raw_values = match values_attr {
                    Some(v) => v.split(';').map(|s| s.trim().to_string()).collect(),
                    None => raw_values,
                };
                let mut values = Vec::new();
                let mut bad = Vec::new();
                for rv in &raw_values {
                    match rv.parse::<f64>() {
                        Ok(x) => values.push(x),
                        Err(_) => bad.push(rv.clone()),
                    }
                }
                let diagnostic = if !bad.is_empty() {
                    Some(format!(
                        "coordinate components {:?} not numeric; raw representation retained",
                        bad
                    ))
                } else if values.is_empty() {
                    Some("coordinate has no numeric components; raw representation retained".into())
                } else {
                    None
                };
                let cid = format!("crd-{}", identity::short_hash(&[&gid, &frame]));
                let sp = span(f, coord_ext.span, "Coordinate");
                model.coordinates.push(Coordinate {
                    id: cid.clone(),
                    node: gid.clone(),
                    frame,
                    raw_values,
                    values,
                    diagnostic: diagnostic.clone(),
                    span: sp.clone(),
                });
                if let Some(msg) = diagnostic {
                    ctx.push("coordinate_invalid", "warn", cid, msg, vec![sp], vec![]);
                }
                if let Some(node) = model.nodes.iter_mut().find(|nn| nn.id == gid) {
                    node.coordinate = Some(format!("crd:{}", gid));
                }
            }
        }
    }
    model.quantities.sort_by(|a, b| a.node.cmp(&b.node));
    model.coordinates.sort_by(|a, b| a.node.cmp(&b.node));
}

fn extract_attr(raw_xml: &str, name: &str) -> Option<String> {
    let needle = format!("{}=\"", name);
    let start = raw_xml.find(&needle)? + needle.len();
    let end = raw_xml[start..].find('"')? + start;
    Some(raw_xml[start..end].to_string())
}

fn build_extensions(files: &[raw::SourceFile], model: &mut DerivedModel) {
    for f in files {
        for e in &f.model.root_extensions {
            model.extensions.push(KeptExtension {
                id: e.id.clone(),
                container: e.container.clone(),
                tag: e.tag.clone(),
                recognized: is_recognized(&e.tag),
                node: None,
                span: span(f, e.span, "unknown top-level element"),
            });
        }
        for n in &f.model.nodes {
            for e in &n.unknown_extensions {
                model.extensions.push(KeptExtension {
                    id: e.id.clone(),
                    container: e.container.clone(),
                    tag: e.tag.clone(),
                    recognized: is_recognized(&e.tag),
                    node: Some(n.node_id.clone()),
                    span: span(f, e.span, "extension element"),
                });
            }
        }
    }
    model.extensions.sort_by(|a, b| {
        a.span
            .path
            .cmp(&b.span.path)
            .then(a.span.start.cmp(&b.span.start))
    });
}

fn is_recognized(tag: &str) -> bool {
    let local = tag.rsplit(':').next().unwrap_or(tag);
    matches!(local, "EngineeringUnit" | "Coordinate")
}

// ---------------------------------------------------------------------------
// Dependency-graph recompute: only rebuild nodes touched by changed sources.
// ---------------------------------------------------------------------------

pub fn dependency_graph(model: &DerivedModel) -> BTreeMap<String, Vec<String>> {
    // node -> nodes it depends on: type/subtype and reference endpoints
    let mut g: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in &model.nodes {
        g.entry(n.id.clone()).or_default();
    }
    for e in &model.edges {
        g.entry(e.source.clone())
            .or_default()
            .push(e.target.clone());
        g.entry(e.target.clone()).or_default().push(e.source.clone());
    }
    for deps in g.values_mut() {
        deps.sort();
        deps.dedup();
    }
    g
}

/// Explain, for a set of changed source-file ids, which nodes remain valid
/// and which must be rebuilt, following the dependency graph.
pub fn propagate(
    previous: &DerivedModel,
    changed_file_ids: &[String],
) -> (BTreeSet<String>, Vec<String>) {
    let graph = dependency_graph(previous);
    let mut dirty: BTreeSet<String> = BTreeSet::new();
    let mut reasons = Vec::new();

    // directly dirty: nodes with provenance in a changed file
    for n in &previous.nodes {
        let touched = n
            .sources
            .iter()
            .any(|s| changed_file_ids.contains(&s.file_id));
        if touched {
            dirty.insert(n.id.clone());
            reasons.push(format!(
                "node {} directly sourced from a changed file -> rebuild",
                n.canonical_node_id
            ));
        }
    }
    // edge-derived stubs whose edges live in a changed file
    for e in &previous.edges {
        let touched = e
            .provenance
            .iter()
            .any(|p| changed_file_ids.contains(&p.file_id));
        if touched {
            dirty.insert(e.source.clone());
            dirty.insert(e.target.clone());
            reasons.push(format!(
                "edge {} endpoints touched by changed source -> revalidate",
                e.id
            ));
        }
    }

    // transitive closure
    let mut frontier: Vec<String> = dirty.iter().cloned().collect();
    while let Some(node) = frontier.pop() {
        if let Some(neighbors) = graph.get(&node) {
            for nb in neighbors {
                if dirty.insert(nb.clone()) {
                    reasons.push(format!(
                        "node {} connected to changed node {} by the reference graph -> rebuild",
                        nb, node
                    ));
                    frontier.push(nb.clone());
                }
            }
        }
    }

    (dirty, reasons)
}

/// Rebuild a new derived model and annotate why each result was kept or changed.
pub fn recompute(
    previous: &DerivedModel,
    files: Vec<raw::SourceFile>,
    branch: &str,
    decisions: &Decisions,
    changed_file_ids: &[String],
) -> (DerivedModel, RecomputeReport) {
    let (dirty, reasons) = propagate(previous, changed_file_ids);
    let rebuilt = build(files, branch, decisions);

    let mut report = RecomputeReport {
        reasons,
        ..Default::default()
    };

    let old: HashMap<&str, &DerivedNode> = previous
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n))
        .collect();
    for n in &rebuilt.nodes {
        match old.get(n.id.as_str()) {
            Some(prev) => {
                if dirty.contains(&n.id) || prev.fingerprint != n.fingerprint {
                    report.rebuilt_nodes.push(n.id.clone());
                } else {
                    report.reused_nodes.push(n.id.clone());
                }
            }
            None => report.new_nodes.push(n.id.clone()),
        }
    }
    for prev in &previous.nodes {
        if !rebuilt.nodes.iter().any(|n| n.id == prev.id) {
            report.removed_nodes.push(prev.id.clone());
        }
    }

    (rebuilt, report)
}
