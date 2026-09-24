//! Merge layer: file-local material is normalized by namespace URI only here.

use crate::model::*;
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_VERSION: &str = "merge-rules-2026.09.1";

const OPC_UA_URI: &str = "http://opcfoundation.org/UA/";

#[derive(Default)]
pub struct MergeInput {
    pub deleted_file_ids: BTreeSet<String>,
}

struct FileCtx {
    local_ns: BTreeMap<u16, String>,
    aliases: BTreeMap<String, NodeIdValue>,
}

pub fn build(raw_files: &[RawFile], version: u64, parent_version: Option<u64>) -> MergedModel {
    build_with(raw_files, version, parent_version, &MergeInput::default())
}

pub fn build_with(raw_files: &[RawFile], version: u64, parent_version: Option<u64>, input: &MergeInput) -> MergedModel {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut namespace_uris: Vec<String> = vec![OPC_UA_URI.to_string()];
    let mut uri_index: BTreeMap<String, u16> = BTreeMap::from([(OPC_UA_URI.to_string(), 0)]);
    let mut namespace_map = Vec::new();
    let mut contexts = BTreeMap::new();

    // Namespace tables: URIs, not numeric positions, are the identity.
    for file in raw_files {
        if input.deleted_file_ids.contains(&file.file_id) {
            continue;
        }
        let mut local_ns: BTreeMap<u16, String> = BTreeMap::new();
        local_ns.insert(0, OPC_UA_URI.to_string());
        for entry in &file.namespaces {
            local_ns.insert(entry.index, entry.uri.clone());
        }
        let mut local_to_global: BTreeMap<u16, u16> = BTreeMap::new();
        for (local_idx, uri) in &local_ns {
            if let Some(&g) = uri_index.get(uri) {
                local_to_global.insert(*local_idx, g);
            } else {
                let g = namespace_uris.len() as u16;
                namespace_uris.push(uri.clone());
                uri_index.insert(uri.clone(), g);
                local_to_global.insert(*local_idx, g);
            }
        }
        namespace_map.push(FileNsMap { file_id: file.file_id.clone(), local_to_global });

        let mut aliases = BTreeMap::new();
        for a in &file.aliases {
            match NodeIdValue::parse(&a.node_id_text) {
                Some(nid) => { aliases.insert(a.alias.clone(), nid); }
                None => diagnostics.push(diag(
                    "ALIAS_TARGET_UNPARSEABLE", Severity::Error, file,
                    a.element.span, &a.alias,
                    &format!("alias {} points at unparsable NodeId {}", a.alias, a.node_id_text),
                )),
            }
        }
        contexts.insert(file.file_id.clone(), FileCtx { local_ns, aliases });
        for d in &file.parse_diagnostics {
            diagnostics.push(d.clone());
        }
    }

    // Nodes (retain all duplicate NodeId origins).
    let mut node_index: BTreeMap<GlobalNodeId, Node> = BTreeMap::new();
    let mut known_gids: BTreeSet<GlobalNodeId> = BTreeSet::new();
    let mut ordinal = 0usize;
    for file in raw_files {
        if input.deleted_file_ids.contains(&file.file_id) { continue; }
        let ctx = &contexts[&file.file_id];
        for raw in &file.nodes {
            let parsed = match NodeIdValue::parse(&raw.node_id_text) {
                Some(p) => p,
                None => continue,
            };
            let gid = globalize(&parsed, ctx, file, &mut diagnostics, raw.element.span, &mut namespace_uris, &mut uri_index, &mut namespace_map);
            let kind = NodeKind::from_tag(&raw.node_class).unwrap_or(NodeKind::Object);
            let origin = NodeOrigin {
                file_id: file.file_id.clone(),
                node_id_text: raw.node_id_text.clone(),
                span: raw.element.span,
                ordinal,
            };
            ordinal += 1;
            if let Some(node) = node_index.get_mut(&gid) {
                if !node.duplicate {
                    node.duplicate = true;
                    diagnostics.push(diag(
                        "DUPLICATE_NODEID", Severity::Error, file, raw.element.span,
                        &gid.render(),
                        &format!("global node {} is declared more than once; all origins retained", gid.render()),
                    ));
                }
                node.origins.push(origin);
            } else {
                let stable = stable_id("node", &[&gid.render()]);
                node_index.insert(gid.clone(), Node {
                    gid: gid.clone(),
                    stable_id: stable,
                    kind,
                    browse_name: raw.browse_name_text.clone(),
                    origins: vec![origin],
                    duplicate: false,
                    placeholder: false,
                });
                known_gids.insert(gid);
            }
        }
    }

    // Aliases with cycle detection. An alias resolves to either a concrete
    // NodeId or another alias name; the latter is followed until it terminates
    // or revisits a name (cycle).
    let mut alias_rows: Vec<AliasMapEntry> = Vec::new();
    for file in raw_files {
        if input.deleted_file_ids.contains(&file.file_id) { continue; }
        let ctx = &contexts[&file.file_id];
        for a in &file.aliases {
            let mut chain: Vec<String> = vec![a.alias.clone()];
            let mut seen: BTreeSet<String> = BTreeSet::from([a.alias.clone()]);
            let mut cur = a.node_id_text.trim().to_string();
            let mut cycle = Vec::new();
            let mut concrete: Option<NodeIdValue> = None;
            loop {
                if let Some(next) = ctx.aliases.get(&cur) {
                    chain.push(cur.clone());
                    if !seen.insert(cur.clone()) {
                        let dup = chain.iter().position(|c| c == &cur).unwrap();
                        cycle = chain[dup..].to_vec();
                        break;
                    }
                    cur = nid_render(next);
                } else if let Some(nid) = NodeIdValue::parse(&cur) {
                    concrete = Some(nid);
                    break;
                } else {
                    chain.push(cur.clone());
                    break;
                }
            }
            if !cycle.is_empty() {
                diagnostics.push(diag(
                    "ALIAS_CYCLE", Severity::Error, file, a.element.span, &a.alias,
                    &format!("alias chain forms a cycle: {}", cycle.join(" -> ")),
                ));
                alias_rows.push(AliasMapEntry {
                    file_id: file.file_id.clone(),
                    alias: a.alias.clone(),
                    target_text: a.node_id_text.clone(),
                    target_gid: None,
                    cycle,
                    unresolved: true,
                    span: a.element.span,
                });
            } else if let Some(nid) = concrete {
                let g = globalize(&nid, ctx, file, &mut diagnostics, a.element.span, &mut namespace_uris, &mut uri_index, &mut namespace_map);
                alias_rows.push(AliasMapEntry {
                    file_id: file.file_id.clone(),
                    alias: a.alias.clone(),
                    target_text: a.node_id_text.clone(),
                    target_gid: Some(g),
                    cycle: Vec::new(),
                    unresolved: false,
                    span: a.element.span,
                });
            } else {
                diagnostics.push(diag(
                    "ALIAS_UNRESOLVED", Severity::Warning, file, a.element.span, &a.alias,
                    &format!("alias {} does not resolve to a NodeId: {}", a.alias, chain.join(" -> ")),
                ));
                alias_rows.push(AliasMapEntry {
                    file_id: file.file_id.clone(),
                    alias: a.alias.clone(),
                    target_text: a.node_id_text.clone(),
                    target_gid: None,
                    cycle: Vec::new(),
                    unresolved: true,
                    span: a.element.span,
                });
            }
        }
    }

    // Edges: forward/inverse are normalized to source->target. Inverse
    // references keep their flag; normalization never fabricates a reverse edge.
    let mut edges: Vec<Edge> = Vec::new();
    let mut edge_seen: BTreeSet<(GlobalNodeId, GlobalNodeId, GlobalNodeId, bool, String)> = BTreeSet::new();
    let mut edge_ordinal = 0usize;
    for file in raw_files {
        if input.deleted_file_ids.contains(&file.file_id) { continue; }
        let ctx = &contexts[&file.file_id];
        for raw in &file.nodes {
            let source_parsed = match NodeIdValue::parse(&raw.node_id_text) { Some(p) => p, None => continue };
            let source = globalize(&source_parsed, ctx, file, &mut diagnostics, raw.element.span, &mut namespace_uris, &mut uri_index, &mut namespace_map);
            for r in &raw.references {
                let ref_type_parsed = NodeIdValue::parse(&r.element.attrs.get("ReferenceType").cloned().unwrap_or_default());
                let target_text = if ctx.aliases.contains_key(&r.text) {
                    nid_render(&ctx.aliases[&r.text])
                } else { r.text.clone() };
                let target_parsed = NodeIdValue::parse(&target_text);
                let (ref_type, target) = match (ref_type_parsed, target_parsed) {
                    (Some(rt), Some(tg)) => (
                        globalize(&rt, ctx, file, &mut diagnostics, r.element.span, &mut namespace_uris, &mut uri_index, &mut namespace_map),
                        globalize(&tg, ctx, file, &mut diagnostics, r.element.span, &mut namespace_uris, &mut uri_index, &mut namespace_map),
                    ),
                    _ => {
                        diagnostics.push(diag(
                            "REFERENCE_UNPARSEABLE", Severity::Error, file, r.element.span,
                            &r.text,
                            "reference target or type could not be parsed; edge retained as raw material only",
                        ));
                        continue;
                    }
                };
                let (edge_source, edge_target) = if r.is_forward {
                    (source.clone(), target)
                } else {
                    (target, source.clone())
                };
                let key = (edge_source.clone(), edge_target.clone(), ref_type.clone(), r.is_forward, file.file_id.clone());
                if edge_seen.contains(&key) { continue; }
                edge_seen.insert(key.clone());
                let state = if known_gids.contains(&edge_target) && known_gids.contains(&edge_source) {
                    EdgeState::Resolved
                } else {
                    let missing = if !known_gids.contains(&edge_target) { &edge_target } else { &edge_source };
                    diagnostics.push(diag(
                        "DANGLING_REFERENCE", Severity::Warning, file, r.element.span,
                        &missing.render(),
                        &format!("reference endpoint {} has no node declaration; unresolved placeholder kept", missing.render()),
                    ));
                    EdgeState::Dangling
                };
                if state == EdgeState::Dangling {
                    for missing in [&edge_target, &edge_source] {
                        if !known_gids.contains(missing) {
                            known_gids.insert(missing.clone());
                            node_index.insert(missing.clone(), Node {
                                gid: missing.clone(),
                                stable_id: stable_id("node", &[&missing.render()]),
                                kind: NodeKind::Object,
                                browse_name: String::new(),
                                origins: Vec::new(),
                                duplicate: false,
                                placeholder: true,
                            });
                        }
                    }
                }
                edges.push(Edge {
                    stable_id: stable_id("edge", &[&edge_source.render(), &edge_target.render(), &ref_type.render(), &file.file_id, &edge_ordinal.to_string()]),
                    source: edge_source,
                    target: edge_target,
                    ref_type,
                    forward_in_source: r.is_forward,
                    state,
                    file_id: file.file_id.clone(),
                    span: r.element.span,
                    ordinal: edge_ordinal,
                });
                edge_ordinal += 1;
            }
        }
    }

    // Model dependencies: every record retained; conflicting versions reported,
    // missing dependencies stay unresolved (never block graph construction).
    let mut dependencies: Vec<DepRecord> = Vec::new();
    let mut dep_versions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut self_models: BTreeSet<String> = BTreeSet::new();
    for file in raw_files {
        if input.deleted_file_ids.contains(&file.file_id) { continue; }
        for m in &file.models {
            dependencies.push(DepRecord {
                file_id: file.file_id.clone(),
                model_uri: m.model_uri.clone(),
                version: m.version.clone(),
                publication_date: m.publication_date.clone(),
                span: m.element.span,
            });
            if m.element.tag == "RequiredModel" {
                if let Some(v) = &m.version {
                    dep_versions.entry(m.model_uri.clone()).or_default().insert(v.clone());
                }
            } else {
                self_models.insert(m.model_uri.clone());
            }
        }
    }
    for (uri, versions) in &dep_versions {
        if versions.len() > 1 {
            diagnostics.push(Diagnostic {
                code: "DEP_VERSION_CONFLICT".into(),
                severity: Severity::Error,
                message: format!("dependency {} required at conflicting versions: {}", uri, versions.iter().cloned().collect::<Vec<_>>().join(", ")),
                file_id: None,
                span: None,
                subject: Some(uri.clone()),
            });
        }
        if !self_models.contains(uri) {
            diagnostics.push(Diagnostic {
                code: "DEPENDENCY_UNRESOLVED".into(),
                severity: Severity::Warning,
                message: format!("required model {} is not imported in this batch; nodes kept as unresolved where missing", uri),
                file_id: None,
                span: None,
                subject: Some(uri.clone()),
            });
        }
    }

    // Unknown extensions retained verbatim.
    let mut extensions: Vec<UnknownExtension> = Vec::new();
    for file in raw_files {
        if input.deleted_file_ids.contains(&file.file_id) { continue; }
        for ext in &file.top_extensions {
            diagnostics.push(diag("UNKNOWN_EXTENSION", Severity::Info, file, ext.span, &ext.tag,
                "unrecognized top-level element retained verbatim"));
            extensions.push(UnknownExtension {
                stable_id: stable_id("ext", &[&file.file_id, &ext.span.start.to_string(), &ext.span.end.to_string()]),
                file_id: file.file_id.clone(),
                parent_node: None,
                tag: ext.tag.clone(),
                raw_xml: span_text(file, ext.span),
                span: ext.span,
            });
        }
        for raw in &file.nodes {
            let parsed = NodeIdValue::parse(&raw.node_id_text);
            let ctx = &contexts[&file.file_id];
            let parent = parsed.and_then(|p| global_gid(&p, ctx));
            for ext in &raw.extensions {
                diagnostics.push(diag("UNKNOWN_EXTENSION", Severity::Info, file, ext.span, &ext.tag,
                    "unrecognized node child element retained verbatim"));
                extensions.push(UnknownExtension {
                    stable_id: stable_id("ext", &[&file.file_id, &raw.node_id_text, &ext.span.start.to_string()]),
                    file_id: file.file_id.clone(),
                    parent_node: parent.clone(),
                    tag: ext.tag.clone(),
                    raw_xml: span_text(file, ext.span),
                    span: ext.span,
                });
            }
        }
    }

    let mut input_hashes = BTreeMap::new();
    let mut files = Vec::new();
    for file in raw_files {
        if input.deleted_file_ids.contains(&file.file_id) { continue; }
        files.push(file.file_id.clone());
        input_hashes.insert(file.file_id.clone(), stable_id("content", &[&file.content]));
    }

    diagnostics.sort_by(|a, b| (a.code.clone(), a.file_id.clone(), a.span.map(|s| s.start)).cmp(&(b.code.clone(), b.file_id.clone(), b.span.map(|s| s.start))));
    let mut nodes: Vec<Node> = node_index.into_values().collect();
    nodes.sort_by(|a, b| a.gid.cmp(&b.gid).then(a.origins.first().map(|o| o.ordinal).cmp(&b.origins.first().map(|o| o.ordinal))));
    edges.sort_by_key(|e| (e.source.clone(), e.target.clone(), e.stable_id.clone()));
    dependencies.sort_by(|a, b| a.model_uri.cmp(&b.model_uri).then(a.file_id.cmp(&b.file_id)));
    extensions.sort_by(|a, b| a.stable_id.cmp(&b.stable_id));
    alias_rows.sort_by(|a, b| a.file_id.cmp(&b.file_id).then(a.alias.cmp(&b.alias)));

    MergedModel {
        version, parent_version, rule_version: RULE_VERSION.into(),
        files, namespace_uris, namespace_map, nodes, edges,
        aliases: alias_rows, dependencies, extensions, diagnostics, input_hashes,
    }
}


fn nid_render(n: &NodeIdValue) -> String {
    n.canonical()
}

fn span_text(file: &RawFile, span: Span) -> String {
    crate::parser::snippet(&file.content, span)
}

fn diag(
    code: &str, severity: Severity, file: &RawFile, span: Span, subject: &str, message: &str,
) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity,
        message: message.into(),
        file_id: Some(file.file_id.clone()),
        span: Some(span),
        subject: Some(subject.into()),
    }
}

fn global_gid(n: &NodeIdValue, ctx: &FileCtx) -> Option<GlobalNodeId> {
    let uri = ctx.local_ns.get(&n.ns)?;
    Some(GlobalNodeId { ns_uri: uri.clone(), id: n.id.clone(), id_type: n.id_type })
}

/// Map a file-local NodeId to a global, URI-based identity. Numeric indices
/// that the file never declared are reported rather than guessed.
#[allow(clippy::too_many_arguments)]
fn globalize(
    n: &NodeIdValue,
    ctx: &FileCtx,
    file: &RawFile,
    diagnostics: &mut Vec<Diagnostic>,
    span: Span,
    namespace_uris: &mut Vec<String>,
    uri_index: &mut BTreeMap<String, u16>,
    namespace_map: &mut Vec<FileNsMap>,
) -> GlobalNodeId {
    if let Some(uri) = ctx.local_ns.get(&n.ns) {
        return GlobalNodeId { ns_uri: uri.clone(), id: n.id.clone(), id_type: n.id_type };
    }
    diagnostics.push(diag(
        "NAMESPACE_INDEX_UNDECLARED", Severity::Error, file, span, &n.canonical(),
        &format!("NodeId uses local ns={} but this file declares no such namespace table entry; index treated as opaque marker", n.ns),
    ));
    let synthetic = format!("urn:undeclared?file={}&ns={}", file.file_id, n.ns);
    if !uri_index.contains_key(&synthetic) {
        let g = namespace_uris.len() as u16;
        namespace_uris.push(synthetic.clone());
        uri_index.insert(synthetic.clone(), g);
        if let Some(map) = namespace_map.iter_mut().find(|m| m.file_id == file.file_id) {
            map.local_to_global.insert(n.ns, g);
        }
    }
    GlobalNodeId { ns_uri: synthetic, id: n.id.clone(), id_type: n.id_type }
}

/// Transitive reference closure from a seed set, restricted to resolved edges.
pub fn reference_closure(model: &MergedModel, seeds: &[GlobalNodeId]) -> BTreeSet<GlobalNodeId> {
    let mut out: BTreeSet<GlobalNodeId> = seeds.iter().cloned().collect();
    let mut frontier: Vec<GlobalNodeId> = seeds.to_vec();
    while let Some(cur) = frontier.pop() {
        for e in &model.edges {
            if e.state == EdgeState::Resolved && e.source == cur && out.insert(e.target.clone()) {
                frontier.push(e.target.clone());
            }
        }
    }
    out
}
