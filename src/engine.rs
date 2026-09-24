use crate::identity::stable_id;
use crate::model::{
    CaseEvent, DecisionInput, DependencyStatus, Derivation, DiffEntry, Edge, Entity,
    NamespaceEntry, NodeDef, ParsedSource, Quantity, RawReference, RecomputeTrace, SourceInput,
    SourceRef, SourceRange, UnresolvedRecord, VersionInput,
};
use crate::parser::parse_source;
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_VERSION: &str = "opc-compass-rules-1.0.0";

#[derive(Debug, Clone)]
struct AliasIndex {
    map: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct Context {
    version_id: String,
    branch_id: String,
    seq: i64,
    sources: BTreeMap<String, ParsedSource>,
    events: Vec<CaseEvent>,
    unresolved: Vec<UnresolvedRecord>,
}

#[derive(Debug, Clone)]
struct RawNodeRecord {
    source_id: String,
    path: String,
    node: NodeDef,
    global_node_id: String,
    uri: String,
}

#[derive(Debug, Clone)]
struct ResolvedToken {
    global_node_id: Option<String>,
    raw: String,
    status: String,
}

fn source_ref(source: &ParsedSource, range: SourceRange) -> SourceRef {
    SourceRef {
        source_id: source.id.clone(),
        path: source.path.clone(),
        range,
    }
}

fn next_event(
    context: &mut Context,
    kind: &str,
    level: &str,
    summary: &str,
    basis: &str,
    payload: serde_json::Value,
    ranges: Vec<SourceRef>,
) -> CaseEvent {
    context.seq += 1;
    CaseEvent {
        id: stable_id(&[
            &context.version_id,
            &context.branch_id,
            &context.seq.to_string(),
            kind,
            summary,
        ]),
        seq: context.seq,
        version_id: context.version_id.clone(),
        branch_id: context.branch_id.clone(),
        kind: kind.to_string(),
        level: level.to_string(),
        summary: summary.to_string(),
        basis: basis.to_string(),
        accepted: true,
        candidate_id: None,
        payload,
        source_ranges: ranges,
    }
}

fn add_event(
    context: &mut Context,
    kind: &str,
    level: &str,
    summary: &str,
    basis: &str,
    payload: serde_json::Value,
    ranges: Vec<SourceRef>,
) {
    let event = next_event(context, kind, level, summary, basis, payload, ranges);
    context.events.push(event);
}

pub fn parse_inputs(inputs: Vec<SourceInput>) -> Result<Vec<ParsedSource>, String> {
    inputs
        .into_iter()
        .map(|input| parse_source(&input.id, &input.path, &input.label, &input.content))
        .collect()
}

pub fn derive_version(
    input: VersionInput,
    parent: Option<&Derivation>,
    decision: Option<&DecisionInput>,
    start_seq: i64,
) -> Result<Derivation, String> {
    let sources_list = parse_inputs(input.sources)?;
    let mut source_map = BTreeMap::new();
    for source in sources_list {
        source_map.insert(source.id.clone(), source);
    }

    let mut context = Context {
        version_id: input.id.clone(),
        branch_id: input.branch_id.clone(),
        seq: start_seq,
        sources: source_map,
        events: Vec::new(),
        unresolved: Vec::new(),
    };

    let namespaces = build_namespace_map(&mut context);
    let raw_nodes = collect_raw_nodes(&mut context);
    let entities = build_entities(&raw_nodes, &mut context);
    let entity_ids: BTreeSet<String> = entities.iter().map(|entity| entity.id.clone()).collect();
    let edges = build_edges(&raw_nodes, &entity_ids, &mut context);
    let dependencies = build_dependencies(&mut context);
    record_unknown_extensions(&raw_nodes, &mut context);
    record_dangling(&raw_nodes, &entity_ids, &mut context);
    let traces = recompute_traces(parent, &input, &entities, &edges, &context);
    record_rule_events(&mut context, &namespaces);

    if let Some(decision) = decision {
        record_decision(&mut context, decision);
    }

    let mut diffs = match parent {
        Some(parent) => diff_versions(parent, &entities, &edges, &context.unresolved, &dependencies),
        None => initial_diffs(&entities, &edges, &context.unresolved, &dependencies),
    };
    diffs.sort_by(|left, right| {
        left.kind.cmp(&right.kind).then(left.object_id.cmp(&right.object_id))
    });

    add_event(
        &mut context,
        "derivation_committed",
        "info",
        &format!("{} committed as an immutable derivation", input.label),
        "commit-switch-atomic-version-pointer",
        serde_json::json!({
            "version_id": input.id,
            "changed_sources": input.changed_sources,
            "rule_version": RULE_VERSION,
        }),
        Vec::new(),
    );

    Ok(Derivation {
        version_id: input.id,
        branch_id: input.branch_id,
        parent_version_id: input.parent_id,
        label: input.label,
        changed_sources: input.changed_sources,
        namespaces,
        entities,
        edges,
        unresolved: context.unresolved,
        dependencies,
        diffs,
        traces,
        events: context.events,
    })
}

const OPC_UA_NAMESPACE: &str = "http://opcfoundation.org/UA/";

fn build_namespace_map(context: &mut Context) -> Vec<NamespaceEntry> {
    let mut uris = BTreeSet::new();
    for source in context.sources.values() {
        for namespace in &source.namespaces {
            if !namespace.uri.trim().is_empty() {
                uris.insert(namespace.uri.clone());
            }
        }
    }
    let uri_index: BTreeMap<String, u16> = uris
        .iter()
        .enumerate()
        .map(|(index, uri)| (uri.clone(), index as u16 + 1))
        .collect();

    let mut entries = Vec::new();
    for source in context.sources.values() {
        for namespace in &source.namespaces {
            if namespace.uri.trim().is_empty() {
                let range = namespace.range.clone();
                context.unresolved.push(UnresolvedRecord {
                    id: stable_id(&[&source.id, "namespace", &namespace.raw]),
                    kind: "unresolved_namespace".to_string(),
                    raw: namespace.raw.clone(),
                    reason: "local namespace index points to an empty URI".to_string(),
                    source_id: source.id.clone(),
                    range: range.clone(),
                });
                add_event(
                    context,
                    "unresolved_namespace",
                    "error",
                    "A namespace table row has no URI and cannot be merged",
                    "numeric-namespace-index-is-file-local-merge-by-uri",
                    serde_json::json!({"source_id": source.id, "local_index": namespace.index, "raw": namespace.raw}),
                    vec![source_ref(source, range)],
                );
                continue;
            }
            let merged_index = *uri_index.get(&namespace.uri).unwrap_or(&0);
            entries.push(NamespaceEntry {
                merged_index,
                uri: namespace.uri.clone(),
                source_id: source.id.clone(),
                local_index: namespace.index,
                raw: namespace.raw.clone(),
                range: namespace.range.clone(),
            });
            if namespace.index != merged_index {
                add_event(
                    context,
                    "namespace_reordered",
                    "warning",
                    "A local namespace index was rebound by URI for the merged model",
                    "numeric-namespace-index-is-file-local-merge-by-uri",
                    serde_json::json!({
                        "uri": namespace.uri,
                        "source_local_index": namespace.index,
                        "merged_index": merged_index,
                    }),
                    vec![source_ref(source, namespace.range.clone())],
                );
            }
        }
    }
    entries.sort_by(|left, right| {
        left.merged_index
            .cmp(&right.merged_index)
            .then(&left.source_id, &right.source_id)
            .then(left.local_index.cmp(&right.local_index))
    });
    entries
}

fn collect_raw_nodes(context: &mut Context) -> Vec<RawNodeRecord> {
    let mut records = Vec::new();
    for source in context.sources.values() {
        for node in &source.nodes {
            let (uri, global_node_id, status) = resolve_node_identity(source, node.raw_node_id.clone(), context);
            if status == "unresolved_namespace" {
                context.unresolved.push(UnresolvedRecord {
                    id: stable_id(&[&source.id, "node", &node.raw_node_id]),
                    kind: "unresolved_namespace".to_string(),
                    raw: node.raw_node_id.clone(),
                    reason: "node uses a local namespace index absent from this file's namespace table".to_string(),
                    source_id: source.id.clone(),
                    range: node.range.clone(),
                });
            }
            records.push(RawNodeRecord {
                source_id: source.id.clone(),
                path: source.path.clone(),
                node: node.clone(),
                global_node_id,
                uri,
            });
        }
    }
    records
}

fn resolve_node_identity(
    source: &ParsedSource,
    raw_node_id: String,
    context: &mut Context,
) -> (String, String, String) {
    let (local_index, identifier) = crate::parser::split_node_id(&raw_node_id);
    if local_index == 0 {
        return (
            OPC_UA_NAMESPACE.to_string(),
            format!("{}|{}", OPC_UA_NAMESPACE, identifier),
            "active".to_string(),
        );
    }
    match source.namespaces.iter().find(|namespace| namespace.index == local_index) {
        Some(namespace) if !namespace.uri.is_empty() => (
            namespace.uri.clone(),
            format!("{}|{}", namespace.uri, identifier),
            "active".to_string(),
        ),
        _ => {
            let global = format!("unresolved-namespace|{}|{}", source.id, raw_node_id);
            if context.sources.contains_key(&source.id) {
                add_event(
                    context,
                    "unresolved_namespace",
                    "error",
                    "NodeId cannot be merged because its local namespace is unknown",
                    "numeric-namespace-index-is-file-local-merge-by-uri",
                    serde_json::json!({"source_id": source.id, "raw_node_id": raw_node_id}),
                    Vec::new(),
                );
            }
            (String::new(), global, "unresolved_namespace".to_string())
        }
    }
}

fn build_entities(raw_nodes: &[RawNodeRecord], context: &mut Context) -> Vec<Entity> {
    let mut grouped: BTreeMap<String, Vec<RawNodeRecord>> = BTreeMap::new();
    for record in raw_nodes {
        grouped
            .entry(record.global_node_id.clone())
            .or_default()
            .push(record.clone());
    }

    let mut entities = Vec::new();
    for (global_node_id, records) in grouped {
        let first = &records[0];
        let mut source_ids: BTreeSet<String> = BTreeSet::new();
        let mut raw_node_ids = BTreeSet::new();
        let mut quantities: Vec<Quantity> = Vec::new();
        for record in &records {
            source_ids.insert(record.source_id.clone());
            raw_node_ids.insert(record.node.raw_node_id.clone());
            quantities.extend(record.node.quantities.iter().cloned());
        }
        let conflicting_duplicates = records.len() > 1
            && records.iter().any(|record| {
                record.node.kind != first.node.kind
                    || record.node.browse_name != first.node.browse_name
                    || record.node.display_name != first.node.display_name
            });
        let unresolved_namespace = records.iter().any(|record| record.uri.is_empty());
        let status = if unresolved_namespace {
            "unresolved_namespace"
        } else if conflicting_duplicates {
            "duplicate_node_id"
        } else {
            "active"
        };
        if conflicting_duplicates {
            let ranges = records
                .iter()
                .map(|record| source_ref(&context.sources[&record.source_id], record.node.range.clone()))
                .collect::<Vec<_>>();
            add_event(
                context,
                "duplicate_node_id",
                "error",
                "The same global NodeId has conflicting raw definitions; all candidates remain visible",
                "stable-identity-is-uri-plus-identifier-raw-candidates-are-retained",
                serde_json::json!({
                    "global_node_id": global_node_id,
                    "candidate_count": records.len(),
                    "raw_candidates": records.iter().map(|record| serde_json::json!({
                        "source_id": record.source_id,
                        "raw_node_id": record.node.raw_node_id,
                        "browse_name": record.node.browse_name,
                        "kind": record.node.kind
                    })).collect::<Vec<_>>(),
                }),
                ranges,
            );
        }
        entities.push(Entity {
            id: stable_id(&["entity", &global_node_id]),
            global_node_id,
            uri: first.uri.clone(),
            identifier: first.node.identifier.clone(),
            browse_name: first.node.browse_name.clone(),
            display_name: first.node.display_name.clone(),
            kind: first.node.kind.clone(),
            status: status.to_string(),
            source_ids: source_ids.into_iter().collect(),
            raw_node_ids: raw_node_ids.into_iter().collect(),
            quantities,
        });
    }
    entities.sort_by(|left, right| left.global_node_id.cmp(&right.global_node_id));
    entities
}

impl AliasIndex {
    fn from_sources(sources: &BTreeMap<String, ParsedSource>) -> BTreeMap<String, AliasIndex> {
        sources
            .iter()
            .map(|(id, source)| {
                (
                    id.clone(),
                    AliasIndex {
                        map: source
                            .aliases
                            .iter()
                            .map(|alias| (alias.name.clone(), alias.target.clone()))
                            .collect(),
                    },
                )
            })
            .collect()
    }

    fn resolve(&self, token: &str) -> (String, Vec<String>, bool) {
        let mut visited = Vec::new();
        let mut current = token.to_string();
        while let Some(next) = self.map.get(&current) {
            if visited.contains(&current) {
                visited.push(current);
                return (String::new(), visited, true);
            }
            visited.push(current);
            current = next.clone();
        }
        if visited.is_empty() {
            (token.to_string(), visited, false)
        } else {
            (current, visited, false)
        }
    }
}

fn build_edges(
    raw_nodes: &[RawNodeRecord],
    entity_ids: &BTreeSet<String>,
    context: &mut Context,
) -> Vec<Edge> {
    let aliases_by_source = AliasIndex::from_sources(&context.sources);
    let entity_by_source_raw: BTreeMap<(String, String), String> = raw_nodes
        .iter()
        .map(|record| {
            (
                (record.source_id.clone(), record.node.raw_node_id.clone()),
                record.global_node_id.clone(),
            )
        })
        .collect();
    let mut dedup: BTreeMap<String, Edge> = BTreeMap::new();

    for record in raw_nodes {
        let source = &context.sources[&record.source_id];
        let aliases = &aliases_by_source[&record.source_id];
        for reference in &record.node.references {
            let raw_reference_id = stable_id(&[
                &record.source_id,
                &record.node.raw_node_id,
                &reference.ref_type,
                &reference.target,
                &reference.is_forward.to_string(),
                &reference.range.start.to_string(),
            ]);
            let (target_token, _, target_alias_cycle) = aliases.resolve(reference.target.trim());
            let (reference_token, _, reference_alias_cycle) = aliases.resolve(reference.ref_type.trim());
            if target_alias_cycle {
                push_unresolved_and_event(
                    context,
                    source.id.clone(),
                    reference.range.clone(),
                    "alias_cycle",
                    &reference.target,
                    "Alias expansion revisited an alias and has no unique NodeId",
                    "alias-cycles-are-kept-unresolved-not-expanded-guessed",
                    serde_json::json!({"alias": reference.target}),
                );
                continue;
            }
            if reference_alias_cycle {
                push_unresolved_and_event(
                    context,
                    source.id.clone(),
                    reference.range.clone(),
                    "alias_cycle",
                    &reference.ref_type,
                    "Reference type alias expansion is cyclic",
                    "alias-cycles-are-kept-unresolved-not-expanded-guessed",
                    serde_json::json!({"alias": reference.ref_type}),
                );
                continue;
            }
            let target_global = resolve_global_token(source, &target_token);
            let reference_type = normalize_reference_type(&reference_token);
            let source_global = record.global_node_id.clone();
            let target_id = match &target_global {
                Some(global) if entity_ids.contains(global) => stable_id(&["entity", global]),
                _ => {
                    push_unresolved_and_event(
                        context,
                        source.id.clone(),
                        reference.range.clone(),
                        "unresolved_target",
                        &reference.target,
                        "Reference endpoint is absent and remains a dangling unresolved dependency",
                        "missing-dependency-retains-unresolved-node-without-fabricating-edge",
                        serde_json::json!({
                            "source_raw_node_id": record.node.raw_node_id,
                            "target_raw": reference.target,
                            "resolved_token": target_token,
                            "is_forward": reference.is_forward
                        }),
                    );
                    continue;
                }
            };
            let source_id = stable_id(&["entity", &source_global]);
            let (canonical_source, canonical_target, direction) = if reference.is_forward {
                (source_id, target_id, "forward")
            } else {
                (target_id, source_id, "inverse-normalized")
            };
            let edge_id = stable_id(&[
                "edge",
                &canonical_source,
                &canonical_target,
                &reference_type.0,
            ]);
            let entry = dedup.entry(edge_id.clone()).or_insert_with(|| Edge {
                id: edge_id,
                source_entity_id: canonical_source,
                target_entity_id: canonical_target,
                reference_type_id: reference_type.0,
                reference_type_label: reference_type.1,
                normalized_direction: direction.to_string(),
                status: if reference_type.2 { "active" } else { "unresolved_reference_type" }.to_string(),
                source_ids: Vec::new(),
                raw_reference_ids: Vec::new(),
                source_range: reference.range.clone(),
            });
            if !entry.source_ids.contains(&source.id) {
                entry.source_ids.push(source.id.clone());
            }
            entry.raw_reference_ids.push(raw_reference_id);
        }
    }

    let mut edges: Vec<Edge> = dedup.into_values().collect();
    edges.sort_by(|left, right| {
        left.source_entity_id
            .cmp(&right.source_entity_id)
            .then(left.target_entity_id.cmp(&right.target_entity_id))
            .then(left.reference_type_id.cmp(&right.reference_type_id))
    });
    for edge in &edges {
        if edge.status != "active" {
            let source = &context.sources[&edge.source_ids[0]];
            add_event(
                context,
                "unresolved_reference_type",
                "warning",
                "Reference edge is retained with an unrecognized reference type",
                "inverse-and-forward-references-are-normalized-without-inventing-missing-edges",
                serde_json::json!({"edge_id": edge.id, "reference_type_id": edge.reference_type_id}),
                vec![source_ref(source, edge.source_range.clone())],
            );
        }
    }
    edges
}

fn push_unresolved_and_event(
    context: &mut Context,
    source_id: String,
    range: SourceRange,
    kind: &str,
    raw: &str,
    reason: &str,
    basis: &str,
    payload: serde_json::Value,
) {
    let id = stable_id(&[&source_id, kind, raw, &range.start.to_string()]);
    if context.unresolved.iter().any(|record| record.id == id) {
        return;
    }
    context.unresolved.push(UnresolvedRecord {
        id,
        kind: kind.to_string(),
        raw: raw.to_string(),
        reason: reason.to_string(),
        source_id: source_id.clone(),
        range: range.clone(),
    });
    let source = &context.sources[&source_id];
    add_event(
        context,
        kind,
        "error",
        reason,
        basis,
        payload,
        vec![source_ref(source, range)],
    );
}

fn resolve_global_token(source: &ParsedSource, token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Some(rest) = token.strip_prefix("ns=") {
        let mut parts = rest.splitn(2, ';');
        let local_index = parts.next()?.parse::<u16>().ok()?;
        let identifier = parts.next()?;
        if local_index == 0 {
            return Some(format!("{}|{}", OPC_UA_NAMESPACE, identifier));
        }
        let namespace = source
            .namespaces
            .iter()
            .find(|namespace| namespace.index == local_index)?;
        return Some(format!("{}|{}", namespace.uri, identifier));
    }
    if token.starts_with("i=") || token.starts_with("s=") || token.starts_with("g=") || token.starts_with("b=") {
        return Some(format!("{}|{}", OPC_UA_NAMESPACE, token));
    }
    None
}

fn normalize_reference_type(token: &str) -> (String, String, bool) {
    let known = match token {
        "References" | "ns=0;i=31" => ("i=31", "References"),
        "HierarchicalReferences" | "ns=0;i=33" => ("i=33", "HierarchicalReferences"),
        "NonHierarchicalReferences" | "ns=0;i=32" => ("i=32", "NonHierarchicalReferences"),
        "HasChild" | "ns=0;i=34" => ("i=34", "HasChild"),
        "HasSubtype" | "OrganizesSubTypes" | "OrganizesSubtypes" | "ns=0;i=45" => ("i=45", "HasSubtype"),
        "HasTypeDefinition" | "ns=0;i=40" => ("i=40", "HasTypeDefinition"),
        "HasProperty" | "PropertyOf" | "ns=0;i=46" => ("i=46", "HasProperty"),
        "HasComponent" | "ns=0;i=47" => ("i=47", "HasComponent"),
        "Aggregates" | "ns=0;i=44" => ("i=44", "Aggregates"),
        "HasEncoding" | "ns=0;i=38" => ("i=38", "HasEncoding"),
        "HasDescription" | "ns=0;i=37" => ("i=37", "HasDescription"),
        _ => return (format!("unresolved|{}", token), token.to_string(), false),
    };
    (
        format!("{}|{}", OPC_UA_NAMESPACE, known.0),
        known.1.to_string(),
        true,
    )
}

fn build_dependencies(context: &mut Context) -> Vec<DependencyStatus> {
    let mut available: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for source in context.sources.values() {
        for model in &source.models {
            if let Some(version) = &model.version {
                available.entry(model.uri.clone()).or_default().insert(version.clone());
            }
        }
    }

    let mut dependencies = Vec::new();
    for source in context.sources.values() {
        for required in &source.required_models {
            let required_version = required.version.clone().unwrap_or_default();
            let available_versions = available.get(&required.uri).cloned().unwrap_or_default();
            let available_version = available_versions.iter().cloned().collect::<Vec<_>>().join(", ");
            let (status, summary, basis) = if available_versions.is_empty() {
                (
                    "missing_dependency",
                    "Required model is absent and is retained as an unresolved dependency",
                    "missing-dependency-retains-unresolved-node-without-fabricating-edge",
                )
            } else if !required_version.is_empty() && !available_versions.contains(&required_version) {
                (
                    "version_conflict",
                    "Required model exists, but none of its versions matches the requested version",
                    "dependency-version-conflict-is-not-resolved-by-silently-picking-another-version",
                )
            } else {
                ("available", "Required model version is available", "dependency-resolution-by-model-uri-and-version")
            };
            dependencies.push(DependencyStatus {
                uri: required.uri.clone(),
                required_version,
                available_version,
                status: status.to_string(),
                source_id: source.id.clone(),
                range: required.range.clone(),
            });
            if status != "available" {
                context.unresolved.push(UnresolvedRecord {
                    id: stable_id(&[&source.id, status, &required.uri, &required.range.start.to_string()]),
                    kind: status.to_string(),
                    raw: required.uri.clone(),
                    reason: summary.to_string(),
                    source_id: source.id.clone(),
                    range: required.range.clone(),
                });
                add_event(
                    context,
                    status,
                    if status == "version_conflict" { "error" } else { "warning" },
                    summary,
                    basis,
                    serde_json::json!({
                        "required_uri": required.uri,
                        "required_version": required.version,
                        "available_versions": available_versions
                    }),
                    vec![source_ref(source, required.range.clone())],
                );
            }
        }
    }
    dependencies.sort_by(|left, right| left.uri.cmp(&right.uri).then(left.source_id.cmp(&right.source_id)));
    dependencies
}

fn record_unknown_extensions(raw_nodes: &[RawNodeRecord], context: &mut Context) {
    let mut record_extension = |source_id: &str, path: &str, range: SourceRange, xml_tag: &str| {
        let id = stable_id(&[source_id, "unknown-extension", path, &range.start.to_string()]);
        if context.unresolved.iter().any(|record| record.id == id) {
            return;
        }
        context.unresolved.push(UnresolvedRecord {
            id,
            kind: "unknown_extension".to_string(),
            raw: xml_tag.to_string(),
            reason: "unrecognized extension is preserved in its original XML location".to_string(),
            source_id: source_id.to_string(),
            range: range.clone(),
        });
        let source = &context.sources[source_id];
        add_event(
            context,
            "unknown_extension",
            "info",
            "An unrecognized extension was retained rather than dropped for display",
            "unknown-extensions-are-preserved-at-source-scope",
            serde_json::json!({"path": path, "tag": xml_tag}),
            vec![source_ref(source, range)],
        );
    };

    for source in context.sources.values() {
        for extension in &source.document_extensions {
            record_extension(&source.id, &extension.path, extension.xml.range.clone(), &extension.xml.tag);
        }
    }
    for record in raw_nodes {
        for extension in &record.node.unknown_extensions {
            record_extension(&record.source_id, &extension.path, extension.xml.range.clone(), &extension.xml.tag);
        }
    }
}

fn record_dangling(_raw_nodes: &[RawNodeRecord], _entity_ids: &BTreeSet<String>, _context: &mut Context) {
    // Dangling references are recorded in build_edges as soon as the target endpoint is tested.
}

fn record_rule_events(context: &mut Context, namespaces: &[NamespaceEntry]) {
    add_event(
        context,
        "namespace_rule",
        "info",
        "Namespace indexes were treated as file-local and merged using namespace URIs",
        "numeric-namespace-index-is-file-local-merge-by-uri",
        serde_json::json!({"rule_version": RULE_VERSION, "namespace_count": namespaces.len()}),
        Vec::new(),
    );
    add_event(
        context,
        "reference_rule",
        "info",
        "Forward and inverse references were normalized without adding a reverse edge",
        "inverse-and-forward-references-are-normalized-without-inventing-missing-edges",
        serde_json::json!({"rule_version": RULE_VERSION}),
        Vec::new(),
    );
}

fn record_decision(context: &mut Context, decision: &DecisionInput) {
    add_event(
        context,
        "manual_decision",
        "info",
        &decision.summary,
        &decision.rationale,
        serde_json::json!({
            "base_version_id": decision.base_version_id,
            "branch_id": decision.branch_id,
            "accepted_candidate_id": decision.accepted_candidate_id,
            "rejected_candidate_id": decision.rejected_candidate_id,
            "unselected_candidates_are_retained": true
        }),
        Vec::new(),
    );
}
