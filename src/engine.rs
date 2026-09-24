use crate::model::*;
use crate::parser::{attribute_value, descendant_text, direct_children, is_semantic_node, parse_raw_xml, range, text};
use crate::util::stable_id;
use std::collections::{BTreeMap, BTreeSet};

fn event(
    version_id: i64,
    code: &str,
    severity: &str,
    adopted: bool,
    subject_id: &str,
    message: &str,
    basis: &str,
    source_range: Option<SourceRange>,
    evidence: BTreeMap<String, String>,
    rule_version: &str,
) -> CompassEvent {
    CompassEvent {
        event_id: stable_id(&[
            "event",
            &version_id.to_string(),
            code,
            subject_id,
            &deterministic_json(&evidence),
        ]),
        version_id,
        code: code.to_string(),
        severity: severity.to_string(),
        adopted,
        subject_id: subject_id.to_string(),
        message: message.to_string(),
        basis: basis.to_string(),
        source_range,
        evidence,
        rule_version: rule_version.to_string(),
    }
}

fn deterministic_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

pub fn build_snapshot(
    version_id: i64,
    inputs: &[SourceInput],
    parameters: &RuleParameters,
) -> Result<ModelSnapshot, String> {
    if inputs.is_empty() {
        return Err("at least one NodeSet XML source is required".to_string());
    }
    let mut documents = Vec::new();
    let mut raw_nodes = Vec::new();
    for (ordinal, input) in inputs.iter().enumerate() {
        let source_id = format!("src-{ordinal:02}-{}", stable_id(&[&input.file_name, &input.content]));
        let nodes = parse_raw_xml(&source_id, &input.file_name, &input.content)?;
        documents.push((source_id, input.file_name.clone(), input.content.clone(), nodes.clone(), ordinal));
        raw_nodes.extend(nodes);
    }

    let mut namespace_index_to_uri: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut namespaces = Vec::new();
    let mut model_versions: BTreeMap<String, String> = BTreeMap::new();
    let mut required_models: Vec<(String, String, SourceRange)> = Vec::new();

    for (source_id, file_name, _content, nodes, ordinal) in &documents {
        let mut uris = vec!["http://opcfoundation.org/UA/".to_string()];
        for node in nodes {
            if node.local_name == "UANodeSet" {
                for child in direct_children(nodes, node) {
                    if child.local_name == "NamespaceUris" {
                        for uri_node in direct_children(nodes, child) {
                            if uri_node.local_name == "Uri" {
                                let uri = text(nodes, uri_node);
                                uris.push(uri.clone());
                                namespaces.push(NamespaceEntry {
                                    file_index: (uris.len() - 1) as u32,
                                    uri: uri.clone(),
                                    stable_id: stable_id(&["namespace", source_id, &uri, &uri_node.start.to_string()]),
                                    range: range(uri_node),
                                });
                            }
                        }
                    }
                    if child.local_name == "Models" {
                        for model_node in direct_children(nodes, child) {
                            if model_node.local_name == "Model" {
                                if let Some(uri) = attribute_value(model_node, "ModelUri") {
                                    let version = attribute_value(model_node, "Version").unwrap_or_else(|| "0".into());
                                    model_versions.insert(uri.clone(), version.clone());
                                    for required in direct_children(nodes, model_node) {
                                        if required.local_name == "RequiredModel" {
                                            if let Some(model_uri) = attribute_value(required, "ModelUri") {
                                                required_models.push((
                                                    model_uri,
                                                    attribute_value(required, "Version").unwrap_or_else(|| "0".into()),
                                                    range(required),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        namespace_index_to_uri.insert(source_id.clone(), uris);
        let _ = ordinal;
        let _ = file_name;
    }

    let mut requirement_records = Vec::new();
    for (model_uri, required_version, requirement_range) in required_models {
        let state = match model_versions.get(&model_uri) {
            None => DependencyState::Missing,
            Some(actual) if compare_versions(actual, &required_version).is_lt() => {
                DependencyState::VersionConflict { expected: required_version.clone(), actual: actual.clone() }
            }
            Some(_) => DependencyState::Satisfied,
        };
        requirement_records.push(ModelRequirement {
            model_uri: model_uri.clone(),
            required_version,
            actual_version: model_versions.get(&model_uri).cloned(),
            state,
            range: requirement_range,
        });
    }

    let mut alias_lookup: BTreeMap<String, (String, SourceRange)> = BTreeMap::new();
    for (source_id, _file_name, _content, nodes, _) in &documents {
        for node in nodes {
            if node.local_name == "Aliases" {
                for alias_node in direct_children(nodes, node) {
                    if alias_node.local_name == "Alias" {
                        let name = attribute_value(alias_node, "Alias").unwrap_or_default();
                        alias_lookup.insert(format!("{source_id}::{name}"), (text(nodes, alias_node), range(alias_node)));
                    }
                }
            }
        }
    }

    let mut alias_records = Vec::new();
    for (key, (raw_value, alias_range)) in &alias_lookup {
        let source_id = key.split("::").next().unwrap_or_default();
        let name = key.split("::").nth(1).unwrap_or_default();
        let mut visited = BTreeSet::new();
        let mut current = raw_value.clone();
        loop {
            let alias_key = format!("{source_id}::{current}");
            if !visited.insert(alias_key.clone()) {
                let path = visited.iter().cloned().collect();
                alias_records.push(Alias {
                    name: name.to_string(),
                    raw_value: raw_value.clone(),
                    resolved_node_id: None,
                    state: AliasState::Cycle { path },
                    range: alias_range.clone(),
                });
                break;
            }
            if let Some((next, _)) = alias_lookup.get(&alias_key) {
                current = next.clone();
                continue;
            }
            alias_records.push(Alias {
                name: name.to_string(),
                raw_value: raw_value.clone(),
                resolved_node_id: Some(expand_node_id(&current, source_id, &namespace_index_to_uri)),
                state: AliasState::Resolved,
                range: alias_range.clone(),
            });
            break;
        }
    }

    let mut parsed_nodes = Vec::new();
    let mut semantic_by_expanded = BTreeMap::<String, Vec<String>>::new();
    for (source_id, file_name, _content, nodes, _) in &documents {
        for node in nodes.iter().filter(|node| is_semantic_node(&node.local_name)) {
            let original = attribute_value(node, "NodeId").unwrap_or_default();
            let expanded = expand_with_aliases(&original, source_id, &alias_lookup, &namespace_index_to_uri);
            let browse_original = direct_children(nodes, node)
                .iter()
                .find(|child| child.local_name == "BrowseName")
                .map(|child| text(nodes, child))
                .or_else(|| descendant_text(nodes, node, "BrowseName").map(|(value, _)| value))
                .unwrap_or_default();
            let browse_name = parse_qualified_name(&browse_original, source_id, &namespace_index_to_uri);
            let display_name = direct_children(nodes, node)
                .iter()
                .find(|child| child.local_name == "DisplayName")
                .map(|child| text(nodes, child))
                .or_else(|| descendant_text(nodes, node, "DisplayName").map(|(value, _)| value))
                .unwrap_or_default();
            let stable = stable_id(&["node", &expanded, &node.source_id, &node.start.to_string()]);
            let mut quantities = Vec::new();
            collect_quantities(nodes, node, source_id, file_name, &mut quantities, parameters);
            parsed_nodes.push(ParsedNode {
                stable_id: stable.clone(),
                source_id: source_id.clone(),
                file_name: file_name.clone(),
                raw_node_id: original.clone(),
                kind: node.local_name.clone(),
                original_node_id: original.clone(),
                expanded_node_id: expanded.clone(),
                browse_name_original: browse_original.clone(),
                browse_name,
                display_name,
                range: range(node),
                duplicates: Vec::new(),
                quantities,
            });
            semantic_by_expanded.entry(expanded).or_default().push(stable);
        }
    }

    for node in parsed_nodes.iter_mut() {
        if let Some(duplicates) = semantic_by_expanded.get(&node.expanded_node_id) {
            if duplicates.len() > 1 {
                node.duplicates = duplicates.iter().filter(|id| **id != node.stable_id).cloned().collect();
            }
        }
    }

    let mut references = Vec::new();
    for (source_id, file_name, _content, nodes, _) in &documents {
        for node in &parsed_nodes {
            let raw_node = nodes
                .iter()
                .find(|raw| raw.source_id == node.source_id && raw.start == node.range.start)
                .expect("semantic node has a raw node");
            let source_original = attribute_value(raw_node, "NodeId").unwrap_or_default();
            let source_expanded = expand_with_aliases(&source_original, source_id, &alias_lookup, &namespace_index_to_uri);
            if let Some(reference_container) = direct_children(nodes, raw_node).into_iter().find(|child| child.local_name == "References") {
                for reference_node in direct_children(nodes, reference_container) {
                    if reference_node.local_name != "Reference" {
                        continue;
                    }
                    let raw_type = attribute_value(reference_node, "ReferenceType").unwrap_or_default();
                    let target_original = text(nodes, reference_node);
                    let is_forward = attribute_value(reference_node, "IsForward").map(|value| value != "false").unwrap_or(true);
                    let target_expanded = expand_with_aliases(&target_original, source_id, &alias_lookup, &namespace_index_to_uri);
                    let reference_type = expand_with_aliases(&raw_type, source_id, &alias_lookup, &namespace_index_to_uri);
                    let (forward_source, forward_target) = if is_forward {
                        (source_expanded.clone(), target_expanded.clone())
                    } else {
                        (target_expanded.clone(), source_expanded.clone())
                    };
                    if !is_forward {
                        let _ = ReferenceDirection::Inverse;
                    }
                    let exists = semantic_by_expanded.contains_key(&forward_target);
                    let state = if exists {
                        ReferenceState::Resolved
                    } else if target_original.starts_with("cycle:") || alias_lookup.contains_key(&format!("{source_id}::{target_original}")) {
                        ReferenceState::UnresolvedAlias { alias: target_original }
                    } else {
                        ReferenceState::Dangling { missing_endpoint: forward_target.clone() }
                    };
                    references.push(Reference {
                        stable_id: stable_id(&["reference", &forward_source, &forward_target, &reference_type, &reference_node.start.to_string()]),
                        source_node: forward_source,
                        target_node: forward_target,
                        reference_type_original: raw_type,
                        reference_type,
                        direction_original: is_forward,
                        direction: if is_forward { "forward".into() } else { "inverse-normalized".into() },
                        state,
                        range: range(reference_node),
                    });
                }
            }
        }
        let _ = file_name;
    }

    Ok(finalize(version_id, inputs, parameters, namespaces, alias_records, requirement_records, parsed_nodes, references, raw_nodes, &documents))
}

#[derive(Clone, Copy)]
enum ReferenceDirection {
    Inverse,
}

fn expand_node_id(raw: &str, source_id: &str, namespace_uris: &BTreeMap<String, Vec<String>>) -> String {
    if let Some((namespace_part, suffix)) = raw.split_once(';') {
        if let Some(index_text) = namespace_part.strip_prefix("ns=") {
            if let Ok(index) = index_text.parse::<u32>() {
                let uris = namespace_uris.get(source_id).cloned().unwrap_or_default();
                if let Some(uri) = uris.get(index as usize) {
                    return format!("{uri}#{suffix}");
                }
            }
        }
    }
    if raw.starts_with("nsu=") {
        if let Some((uri, suffix)) = raw[4..].split_once(';') {
            if let Some((_, identifier)) = suffix.split_once('=') {
                return format!("{uri}#{identifier}");
            }
        }
    }
    raw.to_string()
}

fn expand_with_aliases(
    raw: &str,
    source_id: &str,
    aliases: &BTreeMap<String, (String, SourceRange)>,
    namespace_uris: &BTreeMap<String, Vec<String>>,
) -> String {
    let mut visited = BTreeSet::new();
    let mut current = raw.to_string();
    loop {
        let key = format!("{source_id}::{current}");
        if !visited.insert(key.clone()) {
            return format!("alias-cycle:{current}");
        }
        let Some((next, _)) = aliases.get(&key) else {
            break;
        };
        current = next.clone();
    }
    expand_node_id(&current, source_id, namespace_uris)
}

fn parse_qualified_name(raw: &str, source_id: &str, namespace_uris: &BTreeMap<String, Vec<String>>) -> QualifiedName {
    if let Some((index_text, name)) = raw.split_once(':') {
        if let Ok(index) = index_text.parse::<u32>() {
            let uri = namespace_uris
                .get(source_id)
                .and_then(|uris| uris.get(index as usize))
                .cloned()
                .unwrap_or_else(|| "urn:unresolved-namespace".to_string());
            return QualifiedName { namespace_uri: uri, name: name.to_string(), original: raw.to_string() };
        }
    }
    QualifiedName {
        namespace_uri: "http://opcfoundation.org/UA/".to_string(),
        name: raw.to_string(),
        original: raw.to_string(),
    }
}

fn collect_quantities(
    nodes: &[RawNode],
    semantic_node: &RawNode,
    source_id: &str,
    file_name: &str,
    quantities: &mut Vec<Quantity>,
    parameters: &RuleParameters,
) {
    let mut stack = direct_children(nodes, semantic_node);
    while let Some(node) = stack.pop() {
        let local = node.local_name.as_str();
        if matches!(local, "Pressure" | "Temperature" | "Flow" | "Length") {
            let original = attribute_value(node, "Value").unwrap_or_else(|| text(nodes, node));
            let unit = attribute_value(node, "Unit").or_else(|| attribute_value(node, "EngineeringUnit")).unwrap_or_default();
            let coordinate_basis = attribute_value(node, "CoordinateBasis");
            let numeric: Option<f64> = original.parse().ok();
            let normalized_unit = normalized_unit(&unit, &parameters.pressure_unit, local);
            let value_normalized = numeric.and_then(|value| normalize_value(value, &unit, normalized_unit.as_deref()));
            let state = match (numeric, value_normalized) {
                (Some(_), Some(_)) => QuantityState::Normalized,
                _ => QuantityState::OriginalOnly { reason: "value or unit is not convertible".to_string() },
            };
            quantities.push(Quantity {
                stable_id: stable_id(&["quantity", &node.source_id, &node.start.to_string()]),
                value_original: original,
                value_normalized,
                unit_original: unit,
                unit_normalized: normalized_unit,
                coordinate_basis,
                state,
                range: SourceRange { source_id: source_id.to_string(), file_name: file_name.to_string(), start: node.start, end: node.end },
            });
        }
        stack.extend(direct_children(nodes, node));
    }
}

fn normalized_unit(unit: &str, target_pressure: &str, kind: &str) -> Option<String> {
    let lower = unit.to_ascii_lowercase();
    if kind == "Pressure" && matches!(lower.as_str(), "bar" | "kpa" | "pa" | "psi") {
        return Some(target_pressure.to_string());
    }
    if matches!(lower.as_str(), "bar" | "kpa" | "pa" | "psi" | "c" | "k" | "m3/h" | "m" | "mm") {
        return Some(lower);
    }
    None
}

fn normalize_value(value: f64, original_unit: &str, normalized_unit: Option<&str>) -> Option<f64> {
    let original = original_unit.to_ascii_lowercase();
    let target = normalized_unit?.to_ascii_lowercase();
    if original == target {
        return Some(value);
    }
    match (original.as_str(), target.as_str()) {
        ("kpa", "bar") => Some(value / 100.0),
        ("pa", "bar") => Some(value / 100_000.0),
        ("psi", "bar") => Some(value * 0.0689476),
        ("bar", "kpa") => Some(value * 100.0),
        ("k", "c") => Some(value - 273.15),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn finalize(
    version_id: i64,
    inputs: &[SourceInput],
    parameters: &RuleParameters,
    namespaces: Vec<NamespaceEntry>,
    aliases: Vec<Alias>,
    requirement_records: Vec<ModelRequirement>,
    nodes: Vec<ParsedNode>,
    references: Vec<Reference>,
    raw_nodes: Vec<RawNode>,
    documents: &[(String, String, String, Vec<RawNode>, usize)],
) -> ModelSnapshot {
    let _ = inputs;
    let _ = documents;
    let _known_endpoints: BTreeSet<String> = nodes.iter().map(|node| node.expanded_node_id.clone()).collect();
    let mut unresolved = BTreeMap::<String, UnresolvedNode>::new();
    for reference in &references {
        if let ReferenceState::Dangling { missing_endpoint } = &reference.state {
            let entry = unresolved.entry(missing_endpoint.clone()).or_insert_with(|| UnresolvedNode {
                stable_id: stable_id(&["unresolved-node", missing_endpoint]),
                expanded_node_id: missing_endpoint.clone(),
                referenced_by: Vec::new(),
                reason: "referenced endpoint is absent after URI namespace remap".to_string(),
            });
            entry.referenced_by.push(reference.stable_id.clone());
        }
    }
    let mut unresolved_nodes = unresolved.into_values().collect::<Vec<_>>();
    unresolved_nodes.sort_by(|left, right| left.expanded_node_id.cmp(&right.expanded_node_id));
    let facts = build_facts(&namespaces, &nodes, &references, &unresolved_nodes, parameters);
    let mut events = Vec::new();
    append_events(
        version_id,
        &mut events,
        &namespaces,
        &aliases,
        &requirement_records,
        &nodes,
        &references,
        &unresolved_nodes,
        &raw_nodes,
        parameters,
    );
    events.sort_by(|left, right| left.event_id.cmp(&right.event_id));
    ModelSnapshot {
        namespaces,
        aliases,
        requirements: requirement_records,
        nodes,
        references,
        unresolved_nodes,
        raw_nodes,
        facts,
        events,
    }
}

pub(crate) fn build_facts(
    namespaces: &[NamespaceEntry],
    nodes: &[ParsedNode],
    references: &[Reference],
    unresolved: &[UnresolvedNode],
    parameters: &RuleParameters,
) -> Vec<DerivedFact> {
    let mut facts = Vec::new();
    for namespace in namespaces {
        facts.push(DerivedFact {
            stable_id: stable_id(&["fact-namespace", &namespace.stable_id]),
            kind: "namespace-uri-map".into(),
            status: FactStatus::Valid,
            depends_on: vec![stable_id(&["raw", &namespace.range.source_id, &namespace.range.start.to_string()])],
            reason: "numeric namespace indices are file-local; merged identity uses URI".into(),
        });
    }
    for node in nodes {
        facts.push(DerivedFact {
            stable_id: stable_id(&["fact-node", &node.stable_id]),
            kind: "node-identity".into(),
            status: FactStatus::Valid,
            depends_on: vec![stable_id(&["raw", &node.source_id, &node.range.start.to_string()])],
            reason: "NodeId expanded with file-local namespace index through the namespace table".into(),
        });
        for quantity in &node.quantities {
            facts.push(DerivedFact {
                stable_id: stable_id(&["fact-quantity", &quantity.stable_id, &parameters.rule_version, &parameters.pressure_unit]),
                kind: "quantity-normalization".into(),
                status: FactStatus::Valid,
                depends_on: vec![node.stable_id.clone(), parameters.rule_version.clone(), parameters.pressure_unit.clone()],
                reason: "original representation stays in place; derived value is normalized only when convertible".into(),
            });
        }
    }
    for reference in references {
        facts.push(DerivedFact {
            stable_id: stable_id(&["fact-reference", &reference.stable_id]),
            kind: "reference-edge".into(),
            status: FactStatus::Valid,
            depends_on: vec![reference.source_node.clone(), reference.target_node.clone(), reference.reference_type.clone()],
            reason: "inverse references are normalized to forward edges without dropping the source edge".into(),
        });
        if !matches!(reference.state, ReferenceState::Resolved) {
            facts.push(DerivedFact {
                stable_id: stable_id(&["fact-unresolved-reference", &reference.stable_id]),
                kind: "unresolved-reference".into(),
                status: FactStatus::Valid,
                depends_on: vec![reference.stable_id.clone()],
                reason: "missing endpoint remains an unresolved node and does not become an invented edge".into(),
            });
        }
    }
    for node in unresolved {
        facts.push(DerivedFact {
            stable_id: stable_id(&["fact-unresolved", &node.stable_id]),
            kind: "unresolved-dependency".into(),
            status: FactStatus::Valid,
            depends_on: node.referenced_by.clone(),
            reason: "dependency resolution retained all inbound references without fabricating a target".into(),
        });
    }
    facts
}

#[allow(clippy::too_many_arguments)]
fn append_events(
    version_id: i64,
    events: &mut Vec<CompassEvent>,
    namespaces: &[NamespaceEntry],
    aliases: &[Alias],
    requirements: &[ModelRequirement],
    nodes: &[ParsedNode],
    references: &[Reference],
    unresolved: &[UnresolvedNode],
    raw_nodes: &[RawNode],
    parameters: &RuleParameters,
) {
    let mut seen_uri: BTreeSet<String> = BTreeSet::new();
    for namespace in namespaces {
        if !seen_uri.insert(namespace.uri.clone()) {
            events.push(event(
                version_id,
                "namespace.remap",
                "info",
                true,
                &namespace.stable_id,
                "file-local numeric namespace index was remapped by URI",
                "numeric namespace index is meaningful only in one file; merging uses namespace URI",
                Some(namespace.range.clone()),
                crate::util::sorted_map(vec![("uri", &namespace.uri), ("file_index", &namespace.file_index.to_string())]),
                &parameters.rule_version,
            ));
        }
    }
    for alias in aliases {
        if matches!(alias.state, AliasState::Cycle { .. }) {
            events.push(event(
                version_id,
                "alias.cycle",
                "error",
                false,
                &alias.name,
                "alias expansion enters a cycle and remains unresolved",
                "do not guess a cyclic alias; preserve alias and unresolved candidates",
                Some(alias.range.clone()),
                crate::util::sorted_map(vec![("alias", &alias.name), ("raw", &alias.raw_value)]),
                &parameters.rule_version,
            ));
        }
    }
    let mut duplicate_groups: BTreeMap<String, Vec<&ParsedNode>> = BTreeMap::new();
    for node in nodes {
        duplicate_groups.entry(node.expanded_node_id.clone()).or_default().push(node);
    }
    for (identity, group) in duplicate_groups {
        if group.len() > 1 {
            for node in group {
                events.push(event(
                    version_id,
                    "nodeid.duplicate",
                    "warning",
                    false,
                    &node.stable_id,
                    "duplicate expanded NodeId retains all raw occurrences",
                    "identity collision is a candidate set, not silent overwrite",
                    Some(node.range.clone()),
                    crate::util::sorted_map(vec![("expanded_node_id", &identity), ("raw_node_id", &node.raw_node_id)]),
                    &parameters.rule_version,
                ));
            }
        }
    }
    for reference in references {
        if !reference.direction_original {
            events.push(event(
                version_id,
                "reference.inverse_normalized",
                "info",
                true,
                &reference.stable_id,
                "inverse reference was represented as a forward edge",
                "inverse reference and forward reference are normalized without creating a missing edge",
                Some(reference.range.clone()),
                crate::util::sorted_map(vec![("source", &reference.source_node), ("target", &reference.target_node)]),
                &parameters.rule_version,
            ));
        }
        if !matches!(reference.state, ReferenceState::Resolved) {
            events.push(event(
                version_id,
                "reference.dangling",
                "error",
                false,
                &reference.stable_id,
                "reference endpoint is absent after merge",
                "a missing dependency remains an unresolved node and does not generate a synthetic endpoint",
                Some(reference.range.clone()),
                crate::util::sorted_map(vec![("source", &reference.source_node), ("target", &reference.target_node)]),
                &parameters.rule_version,
            ));
        }
    }
    for requirement in requirements {
        let code = match requirement.state {
            DependencyState::Satisfied => continue,
            DependencyState::Missing => "dependency.missing",
            DependencyState::VersionConflict { .. } => "dependency.version_conflict",
        };
        events.push(event(
            version_id,
            code,
            "error",
            false,
            &requirement.model_uri,
            "required model dependency cannot be satisfied",
            "model dependency is version-sensitive; unsatisfied input is retained as unresolved",
            Some(requirement.range.clone()),
            crate::util::sorted_map(vec![("required", &requirement.required_version), ("actual", requirement.actual_version.as_deref().unwrap_or("missing"))]),
            &parameters.rule_version,
        ));
    }
    for node in unresolved {
        let _ = node;
    }
    for raw in raw_nodes {
        let known = matches!(
            raw.local_name.as_str(),
            "UANodeSet" | "NamespaceUris" | "Uri" | "Aliases" | "Alias" | "Models" | "Model" | "RequiredModel"
                | "UAObject" | "UAVariable" | "UAMethod" | "UAObjectType" | "UAVariableType" | "UADataType"
                | "UAReferenceType" | "DisplayName" | "Description" | "Category" | "References" | "Reference"
                | "BrowseName" | "Value" | "Text" | "Pressure" | "Temperature" | "Flow" | "Length"
        );
        let is_foreign_ns = !raw.xml_namespace.is_empty();
        if !known || is_foreign_ns {
            events.push(event(
                version_id,
                "extension.unknown",
                "warning",
                true,
                &raw.stable_id,
                "unrecognized extension was retained without interpretation",
                "unknown extensions stay attached to raw XML and never drive graph inference",
                Some(range(raw)),
                crate::util::sorted_map(vec![("element", &raw.local_name), ("xml_namespace", &raw.xml_namespace)]),
                &parameters.rule_version,
            ));
        }
    }
}

fn compare_versions(actual: &str, required: &str) -> std::cmp::Ordering {
    let parse = |value: &str| {
        value
            .split(['.', '-'])
            .filter_map(|part| part.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<u64>().ok())
            .collect::<Vec<_>>()
    };
    parse(actual).cmp(&parse(required))
}
