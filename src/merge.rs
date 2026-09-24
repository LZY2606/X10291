//! Merge parsed NodeSet files into one model.
//!
//! Enforced semantics:
//! * numeric ns index is per-file; merging remaps by URI;
//! * forward/inverse references are normalized to a canonical direction but
//!   edges that were never declared are never fabricated;
//! * a missing model dependency keeps an `unresolved` placeholder node instead
//!   of dropping the reference;
//! * ambiguous situations (duplicate NodeId, alias cycle, version conflict)
//!   keep every legal candidate and emit machine-readable events.

use crate::model::{parse_node_id, stable_id, IdKind, XNodeId};
use crate::xml::{NodeClass, NodeSetFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const CORE_NS: &str = "http://opcfoundation.org/UA/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: usize,
    pub kind: &'static str,
    pub message: String,
    /// Stable identities of every affected object.
    pub subjects: Vec<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMapping {
    pub file: String,
    pub local_to_global: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRange {
    pub file: String,
    pub line_start: usize,
    pub line_end: usize,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedNode {
    pub stable_id: String,
    pub global_ns: u16,
    pub id: IdKind,
    pub class: NodeClass,
    pub browse_name_raw: String,
    pub browse_name_uri: String,
    pub browse_name_text: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub extensions_raw: Vec<String>,
    pub extra_attrs: Vec<(String, String)>,
    pub raw_xml: String,
    pub source: Option<SourceRange>,
    /// Files contributing a node with this stable id (duplicates).
    pub candidates: Vec<String>,
    pub unresolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedRef {
    pub from: String,
    pub to: String,
    pub ref_type: String,
    pub ref_type_raw: String,
    /// The file in which the reference was declared.
    pub declared_in: String,
    /// `true` when declared IsForward="true" (or omitted), `false` for inverse.
    pub declared_forward: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedRef {
    pub from: String,
    pub to: String,
    pub ref_type_raw: String,
    pub declared_in: String,
    pub declared_forward: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedDependency {
    pub required_by: String,
    pub model_uri: String,
    pub required_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionConflict {
    pub model_uri: String,
    pub provided: Option<String>,
    pub required: Option<String>,
    pub required_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasCycle {
    pub file: String,
    pub cycle: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedModel {
    pub namespaces: Vec<String>,
    pub files: Vec<FileMapping>,
    pub nodes: Vec<MergedNode>,
    pub refs: Vec<MergedRef>,
    pub unresolved_refs: Vec<UnresolvedRef>,
    pub unresolved_deps: Vec<UnresolvedDependency>,
    pub version_conflicts: Vec<VersionConflict>,
    pub alias_cycles: Vec<AliasCycle>,
    pub events: Vec<Event>,
}

struct Builder {
    uris: Vec<String>,
    files: Vec<FileMapping>,
    events: Vec<Event>,
}

impl Builder {
    fn event(
        &mut self,
        kind: &'static str,
        message: String,
        subjects: Vec<String>,
        data: serde_json::Value,
    ) {
        self.events.push(Event {
            seq: self.events.len(),
            kind,
            message,
            subjects,
            data,
        });
    }

    fn uri_index(&mut self, uri: &str) -> u16 {
        if let Some(i) = self.uris.iter().position(|u| u == uri) {
            return i as u16;
        }
        self.uris.push(uri.to_string());
        (self.uris.len() - 1) as u16
    }

    fn remap(&self, local_ns: u16, mapping: &FileMapping) -> Option<&str> {
        let g = *mapping.local_to_global.get(local_ns as usize)? as usize;
        self.uris.get(g).map(|s| s.as_str())
    }
}

/// Resolve a token that may be a NodeId or an alias name.
fn resolve_token(
    token: &str,
    aliases: &HashMap<String, String>,
    mapping: &FileMapping,
    builder: &Builder,
) -> Result<XNodeId, String> {
    if let Ok((local_ns, id)) = parse_node_id(token) {
        let uri = builder
            .remap(local_ns, mapping)
            .ok_or_else(|| format!("namespace index {local_ns} not declared"))?;
        return Ok(XNodeId { ns_uri: uri.to_string(), id });
    }
    let mut chain: Vec<String> = Vec::new();
    let mut current = token.to_string();
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(current.clone()) {
            chain.push(current.clone());
            return Err(format!("cycle:{}", chain.join(" -> ")));
        }
        chain.push(current.clone());
        match aliases.get(&current) {
            Some(value) => {
                if let Ok((local_ns, id)) = parse_node_id(value) {
                    let uri = builder
                        .remap(local_ns, mapping)
                        .ok_or_else(|| format!("namespace index {local_ns} not declared"))?;
                    return Ok(XNodeId { ns_uri: uri.to_string(), id });
                }
                current = value.clone();
            }
            None => return Err(format!("unresolved token: {current}")),
        }
    }
}

fn first_extension_tag(raw: &str) -> Option<String> {
    let inner = raw.trim();
    if !inner.starts_with('<') {
        return None;
    }
    let rest = &inner[1..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(rest.len());
    Some(rest[..end].split(':').next_back().unwrap_or(&rest[..end]).to_string())
}

/// Merge several parsed NodeSet files. Namespace indices are remapped by URI.
pub fn merge_files(files: &[NodeSetFile]) -> MergedModel {
    let mut b = Builder {
        uris: vec![CORE_NS.to_string()],
        files: Vec::new(),
        events: Vec::new(),
    };

    // --- namespace tables -------------------------------------------------
    for f in files {
        let mut local_to_global = vec![0u16]; // local index 0 -> global core
        for uri in &f.namespace_uris {
            let g = b.uri_index(uri);
            local_to_global.push(g);
        }
        b.files.push(FileMapping { file: f.name.clone(), local_to_global });
        b.event(
            "NamespaceTable",
            format!("{} declares namespace table [{}]", f.name, f.namespace_uris.join(", ")),
            f.namespace_uris.iter().map(|u| format!("nsu={u}")).collect(),
            serde_json::json!({"file": f.name, "uris": f.namespace_uris}),
        );
    }
    for idx in 0..b.files.len() {
        let mapping = b.files[idx].clone();
        for (local, &global) in mapping.local_to_global.iter().enumerate().skip(1) {
            let uri = b.uris[global as usize].clone();
            if global as usize != local {
                b.event(
                    "NamespaceRemap",
                    format!(
                        "{} local ns={local} remapped to global ns={global} ({uri})",
                        mapping.file
                    ),
                    vec![format!("nsu={uri}")],
                    serde_json::json!({
                        "file": mapping.file, "local_index": local,
                        "global_index": global, "uri": uri,
                    }),
                );
            }
        }
    }

    // --- model dependency table ------------------------------------------
    let mut providers: BTreeMap<String, Vec<(String, Option<String>)>> = BTreeMap::new();
    for f in files {
        for m in &f.models {
            providers
                .entry(m.model_uri.clone())
                .or_default()
                .push((f.name.clone(), m.version.clone()));
        }
    }
    let mut unresolved_deps = Vec::new();
    let mut version_conflicts = Vec::new();
    for f in files {
        for m in &f.models {
            for req in &m.required {
                let subject = format!("nsu={};s=model", req.model_uri);
                match providers.get(&req.model_uri) {
                    None => {
                        unresolved_deps.push(UnresolvedDependency {
                            required_by: m.model_uri.clone(),
                            model_uri: req.model_uri.clone(),
                            required_version: req.version.clone(),
                        });
                        b.event(
                            "UnresolvedDependency",
                            format!(
                                "{} requires model {} which is not imported; kept as unresolved",
                                m.model_uri, req.model_uri
                            ),
                            vec![subject],
                            serde_json::json!({
                                "required_by": m.model_uri,
                                "model_uri": req.model_uri,
                                "required_version": req.version,
                            }),
                        );
                    }
                    Some(list) => {
                        let matching =
                            list.iter().any(|(_, v)| v.as_ref() == req.version.as_ref());
                        if !matching {
                            let provided = list.first().and_then(|(_, v)| v.clone());
                            version_conflicts.push(VersionConflict {
                                model_uri: req.model_uri.clone(),
                                provided: provided.clone(),
                                required: req.version.clone(),
                                required_by: m.model_uri.clone(),
                            });
                            b.event(
                                "DependencyVersionConflict",
                                format!(
                                    "{} requires {} v{} but v{} is provided; both claims kept",
                                    m.model_uri,
                                    req.model_uri,
                                    req.version.as_deref().unwrap_or("?"),
                                    provided.as_deref().unwrap_or("?")
                                ),
                                vec![subject],
                                serde_json::json!({
                                    "model_uri": req.model_uri,
                                    "provided": provided,
                                    "required": req.version,
                                    "required_by": m.model_uri,
                                }),
                            );
                        }
                    }
                }
            }
        }
    }

    // --- alias cycle detection -------------------------------------------
    let mut alias_cycles = Vec::new();
    for f in files {
        let aliases: HashMap<String, String> = f.aliases.iter().cloned().collect();
        let mut reported: HashSet<Vec<String>> = HashSet::new();
        for (name, _) in &f.aliases {
            let mut chain = vec![name.clone()];
            let mut cur = name.clone();
            loop {
                let value = match aliases.get(&cur) {
                    Some(v) => v.clone(),
                    None => break,
                };
                if parse_node_id(&value).is_ok() {
                    break;
                }
                chain.push(value.clone());
                if chain[..chain.len() - 1].contains(&value) {
                    let mut cyc = chain.clone();
                    cyc.sort();
                    if reported.insert(cyc) {
                        b.event(
                            "AliasCycle",
                            format!("alias cycle in {}: {}", f.name, chain.join(" -> ")),
                            vec![format!("alias:{}:{}", f.name, name)],
                            serde_json::json!({"file": f.name, "cycle": chain}),
                        );
                        alias_cycles.push(AliasCycle {
                            file: f.name.clone(),
                            cycle: chain.clone(),
                        });
                    }
                    break;
                }
                cur = value;
                if chain.len() > 64 {
                    break;
                }
            }
        }
    }
    merge_nodes(b, files, unresolved_deps, version_conflicts, alias_cycles)
}

fn merge_nodes(
    mut b: Builder,
    files: &[NodeSetFile],
    unresolved_deps: Vec<UnresolvedDependency>,
    version_conflicts: Vec<VersionConflict>,
    alias_cycles: Vec<AliasCycle>,
) -> MergedModel {
    // --- nodes ------------------------------------------------------------
    let mut nodes: Vec<MergedNode> = Vec::new();
    let mut index_of: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, f) in files.iter().enumerate() {
        let mapping = b.files[idx].clone();
        for n in &f.nodes {
            let (local_ns, id) = match parse_node_id(&n.node_id) {
                Ok(v) => v,
                Err(e) => {
                    b.event(
                        "InvalidNodeId",
                        format!(
                            "{}:{} has unparsable NodeId '{}': {e}",
                            f.name, n.browse_name, n.node_id
                        ),
                        vec![format!("raw:{}:{}", f.name, n.node_id)],
                        serde_json::json!({"file": f.name, "node_id": n.node_id}),
                    );
                    continue;
                }
            };
            let uri = match b.remap(local_ns, &mapping) {
                Some(u) => u.to_string(),
                None => {
                    b.event(
                        "UndeclaredNamespaceIndex",
                        format!("{} uses ns={local_ns} which is not in its NamespaceUris", f.name),
                        vec![format!("raw:{}:{}", f.name, n.node_id)],
                        serde_json::json!({"file": f.name, "local_index": local_ns}),
                    );
                    CORE_NS.to_string()
                }
            };
            let (bn_ns, bn_text) = crate::model::parse_browse_name(&n.browse_name);
            let bn_uri = b.remap(bn_ns, &mapping).unwrap_or(CORE_NS).to_string();
            let sid = stable_id(&uri, &id);
            let global_ns = b.uris.iter().position(|u| *u == uri).unwrap_or(0) as u16;
            let entry = MergedNode {
                stable_id: sid.clone(),
                global_ns,
                id,
                class: n.class.clone(),
                browse_name_raw: n.browse_name.clone(),
                browse_name_uri: bn_uri,
                browse_name_text: bn_text,
                display_name: n.display_name.clone(),
                description: n.description.clone(),
                extensions_raw: n.extensions_raw.clone(),
                extra_attrs: n.extra_attrs.clone(),
                raw_xml: n.raw_xml.clone(),
                source: Some(SourceRange {
                    file: f.name.clone(),
                    line_start: n.line_start,
                    line_end: n.line_end,
                    byte_start: n.byte_start,
                    byte_end: n.byte_end,
                }),
                candidates: vec![f.name.clone()],
                unresolved: false,
            };
            for ext in &entry.extensions_raw {
                if let Some(tag) = first_extension_tag(ext) {
                    b.event(
                        "UnknownExtension",
                        format!(
                            "{}:{} carries unrecognized element <{tag}>; preserved in place",
                            f.name, n.browse_name
                        ),
                        vec![sid.clone()],
                        serde_json::json!({"file": f.name, "tag": tag, "raw": ext}),
                    );
                }
            }
            index_of.entry(sid).or_default().push(nodes.len());
            nodes.push(entry);
        }
    }

    // Duplicate NodeId after URI remap: keep every candidate, pick none.
    for (sid, idxs) in index_of.iter() {
        if idxs.len() > 1 {
            let files_involved: Vec<String> = idxs
                .iter()
                .filter_map(|&i| nodes[i].source.as_ref().map(|s| s.file.clone()))
                .collect();
            for &i in idxs {
                nodes[i].candidates = files_involved.clone();
            }
            b.event(
                "DuplicateNodeId",
                format!(
                    "NodeId {sid} declared {} times ({}); all candidates kept, none preferred",
                    idxs.len(),
                    files_involved.join(", ")
                ),
                vec![sid.clone()],
                serde_json::json!({"stable_id": sid, "candidates": files_involved}),
            );
        }
    }

    // --- references -------------------------------------------------------
    let known: HashSet<String> = nodes.iter().map(|n| n.stable_id.clone()).collect();
    let mut refs: Vec<MergedRef> = Vec::new();
    let mut unresolved_refs: Vec<UnresolvedRef> = Vec::new();
    let mut placeholder_ids: Vec<(String, String)> = Vec::new(); // (stable_id, file)
    for (idx, f) in files.iter().enumerate() {
        let mapping = b.files[idx].clone();
        let aliases: HashMap<String, String> = f.aliases.iter().cloned().collect();
        for n in &f.nodes {
            let Ok((local_ns, id)) = parse_node_id(&n.node_id) else { continue };
            let Some(uri) = b.remap(local_ns, &mapping) else { continue };
            let from_sid = stable_id(uri, &id);
            for r in &n.references {
                let ref_type = match resolve_token(&r.ref_type, &aliases, &mapping, &b) {
                    Ok(x) => stable_id(&x.ns_uri, &x.id),
                    Err(e) => {
                        b.event(
                            "UnresolvedReferenceType",
                            format!(
                                "{}:{} reference type '{}' cannot be resolved ({e}); kept raw",
                                f.name, n.browse_name, r.ref_type
                            ),
                            vec![from_sid.clone()],
                            serde_json::json!({
                                "file": f.name, "token": r.ref_type, "reason": e,
                            }),
                        );
                        format!("raw:{}", r.ref_type)
                    }
                };
                let target = match resolve_token(&r.target, &aliases, &mapping, &b) {
                    Ok(x) => stable_id(&x.ns_uri, &x.id),
                    Err(e) => {
                        b.event(
                            "UnresolvedReferenceTarget",
                            format!(
                                "{}:{} target '{}' cannot be parsed ({e}); kept raw",
                                f.name, n.browse_name, r.target
                            ),
                            vec![from_sid.clone()],
                            serde_json::json!({
                                "file": f.name, "token": r.target, "reason": e,
                            }),
                        );
                        format!("raw:{}", r.target)
                    }
                };
                // Normalize direction: canonical edge is always forward.
                let (from, to) = if r.is_forward {
                    (from_sid.clone(), target.clone())
                } else {
                    (target.clone(), from_sid.clone())
                };
                if !r.is_forward {
                    b.event(
                        "InverseNormalized",
                        format!(
                            "{}:{} declares inverse {}; normalized to forward edge {} -> {} (no edge fabricated)",
                            f.name, n.browse_name, r.ref_type, from, to
                        ),
                        vec![from.clone(), to.clone()],
                        serde_json::json!({
                            "file": f.name, "declared_forward": false,
                            "from": from, "to": to, "ref_type": ref_type,
                        }),
                    );
                }
                let dangling = !target.starts_with("raw:") && !known.contains(&target);
                if dangling {
                    unresolved_refs.push(UnresolvedRef {
                        from: from.clone(),
                        to: to.clone(),
                        ref_type_raw: r.ref_type.clone(),
                        declared_in: f.name.clone(),
                        declared_forward: r.is_forward,
                    });
                    placeholder_ids.push((target.clone(), f.name.clone()));
                    b.event(
                        "DanglingReference",
                        format!(
                            "{}:{} references {} which has no node; unresolved placeholder kept",
                            f.name, n.browse_name, target
                        ),
                        vec![from.clone(), target.clone()],
                        serde_json::json!({
                            "file": f.name, "from": from, "to": to,
                            "ref_type": r.ref_type, "declared_forward": r.is_forward,
                        }),
                    );
                }
                refs.push(MergedRef {
                    from,
                    to,
                    ref_type,
                    ref_type_raw: r.ref_type.clone(),
                    declared_in: f.name.clone(),
                    declared_forward: r.is_forward,
                });
            }
        }
    }

    // Unresolved placeholder nodes for dangling targets and missing models.
    let mut known_now: HashSet<String> = known;
    for (sid, file) in placeholder_ids {
        if known_now.insert(sid.clone()) {
            nodes.push(MergedNode {
                stable_id: sid.clone(),
                global_ns: 0,
                id: IdKind::String(sid.clone()),
                class: NodeClass::Object,
                browse_name_raw: String::new(),
                browse_name_uri: String::new(),
                browse_name_text: "(unresolved)".to_string(),
                display_name: Some(format!("unresolved: {sid}")),
                description: None,
                extensions_raw: Vec::new(),
                extra_attrs: Vec::new(),
                raw_xml: String::new(),
                source: None,
                candidates: vec![file],
                unresolved: true,
            });
        }
    }
    for dep in &unresolved_deps {
        let sid = format!("nsu={};s=model", dep.model_uri);
        if known_now.insert(sid.clone()) {
            nodes.push(MergedNode {
                stable_id: sid.clone(),
                global_ns: 0,
                id: IdKind::String("model".to_string()),
                class: NodeClass::Object,
                browse_name_raw: String::new(),
                browse_name_uri: String::new(),
                browse_name_text: "(unresolved model)".to_string(),
                display_name: Some(format!("unresolved model: {}", dep.model_uri)),
                description: None,
                extensions_raw: Vec::new(),
                extra_attrs: Vec::new(),
                raw_xml: String::new(),
                source: None,
                candidates: vec![dep.required_by.clone()],
                unresolved: true,
            });
        }
    }

    MergedModel {
        namespaces: b.uris,
        files: b.files,
        nodes,
        refs,
        unresolved_refs,
        unresolved_deps,
        version_conflicts,
        alias_cycles,
        events: b.events,
    }
}
