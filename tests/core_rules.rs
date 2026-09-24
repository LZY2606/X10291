use namespace_compass_opcua::engine::build_snapshot;
use namespace_compass_opcua::model::*;
use namespace_compass_opcua::scenario::read_input;
use std::collections::BTreeSet;

fn snapshot(version: i64, dir: &str, files: &[&str], parameters: RuleParameters) -> ModelSnapshot {
    build_snapshot(version, &read_input(dir, files), &parameters).expect("fixture parses")
}

fn codes(snapshot: &ModelSnapshot) -> BTreeSet<String> {
    snapshot.events.iter().map(|event| event.code.clone()).collect()
}

#[test]
fn baseline_preserves_raw_xml_namespaces_aliases_nodes_and_extensions() {
    let model = snapshot(1, "v1", &["shared.xml", "core.xml", "line.xml"], RuleParameters::default());

    assert!(model.raw_nodes.iter().any(|node| node.raw_xml.contains("<UANodeSet")));
    assert!(model.raw_nodes.iter().any(|node| node.local_name == "Pressure" && node.xml_namespace == "urn:factory:domain:2026"));
    assert!(model.namespaces.iter().any(|item| item.uri == "urn:factory:core:2026" && item.file_index == 1));
    assert!(model.namespaces.iter().any(|item| item.uri == "urn:factory:core:2026" && item.file_index == 2));
    assert!(model.aliases.iter().any(|alias| alias.name == "Pump1001" && matches!(alias.state, AliasState::Resolved)));
    assert!(model.nodes.iter().any(|node| node.expanded_node_id == "urn:factory:core:2026#i=1001"));
    assert!(model.nodes.iter().any(|node| node.browse_name_original == "1:OutletPressure" && node.browse_name.namespace_uri == "urn:factory:core:2026"));
    assert!(model.requirements.iter().all(|requirement| matches!(requirement.state, DependencyState::Satisfied)));
}

#[test]
fn numeric_namespace_indexes_are_remapped_by_uri_across_files() {
    let model = snapshot(2, "v2", &["shared.xml", "core.xml", "line.xml"], RuleParameters::default());
    let pump_targets = model
        .references
        .iter()
        .filter(|reference| reference.target_node.contains("Pump1001") || reference.target_node.ends_with("#i=1001"))
        .map(|reference| reference.target_node.clone())
        .collect::<BTreeSet<_>>();
    assert!(pump_targets.contains("urn:factory:core:2026#i=1001"));
    assert!(model.nodes.iter().any(|node| node.expanded_node_id == "urn:factory:core:2026#i=1001"));
}

#[test]
fn inverse_references_are_forward_normalized_without_dropping_edge() {
    let model = snapshot(3, "v1", &["shared.xml", "core.xml", "line.xml"], RuleParameters::default());
    let edge = model
        .references
        .iter()
        .find(|edge| edge.source_node.ends_with("#i=3001") && edge.target_node == "urn:factory:core:2026#i=1001")
        .expect("inverse edge is normalized from child sensor to parent pump");
    assert!(!edge.direction_original);
    assert_eq!(edge.direction, "inverse-normalized");
    assert!(matches!(edge.state, ReferenceState::Resolved));
}

#[test]
fn missing_targets_remain_unresolved_instead_of_becoming_invented_nodes() {
    let model = snapshot(4, "v3", &["edge-core.xml", "edge-line.xml"], RuleParameters::default());
    assert!(model
        .unresolved_nodes
        .iter()
        .any(|node| node.expanded_node_id.contains("#i=8801") || node.expanded_node_id.contains("#i=404")));
    assert!(model.nodes.iter().all(|node| !node.expanded_node_id.ends_with("#i=8801")));
    assert!(model.references.iter().any(|reference| matches!(reference.state, ReferenceState::Dangling { .. })));
}

#[test]
fn edge_case_pack_replays_all_required_boundary_conditions() {
    let model = snapshot(5, "v3", &["edge-core.xml", "edge-line.xml"], RuleParameters::default());
    let event_codes = codes(&model);
    for required in [
        "namespace.remap",
        "alias.cycle",
        "nodeid.duplicate",
        "reference.inverse_normalized",
        "dependency.version_conflict",
        "dependency.missing",
        "extension.unknown",
        "reference.dangling",
    ] {
        assert!(event_codes.contains(required), "missing event {required}; got {event_codes:?}");
    }
    assert!(model.requirements.iter().any(|requirement| matches!(requirement.state, DependencyState::VersionConflict { .. })));
    assert!(model.requirements.iter().any(|requirement| matches!(requirement.state, DependencyState::Missing)));
    assert!(model.nodes.iter().filter(|node| node.expanded_node_id == "urn:factory:core:2026#i=1001").count() == 2);
}

#[test]
fn original_quantities_keep_units_coordinate_basis_and_unconvertible_diagnostics() {
    let baseline = snapshot(6, "v1", &["shared.xml", "core.xml", "line.xml"], RuleParameters::default());
    let sensor = baseline
        .nodes
        .iter()
        .find(|node| node.expanded_node_id == "urn:factory:line:2026#i=3001")
        .expect("sensor exists");
    let pressure = sensor.quantities.first().expect("pressure quantity retained");
    assert_eq!(pressure.value_original, "610");
    assert_eq!(pressure.unit_original, "kPa");
    assert_eq!(pressure.coordinate_basis.as_deref(), Some("sensor-port:x-forward"));
    assert_eq!(pressure.value_normalized, Some(6.1));

    let edge = snapshot(7, "v3", &["edge-core.xml", "edge-line.xml"], RuleParameters::default());
    let pump = edge
        .nodes
        .iter()
        .find(|node| node.expanded_node_id == "urn:factory:core:2026#i=1001")
        .expect("duplicate candidates still have first occurrence");
    let non_numeric = pump.quantities.first().expect("unconvertible quantity remains in place");
    assert_eq!(non_numeric.value_original, "tbd");
    assert!(non_numeric.value_normalized.is_none());
    assert!(matches!(non_numeric.state, QuantityState::OriginalOnly { .. }));
}

#[test]
fn every_displayed_semantic_value_has_stable_identity_and_source() {
    let model = snapshot(8, "v1", &["shared.xml", "core.xml", "line.xml"], RuleParameters::default());
    assert!(model.namespaces.iter().all(|item| !item.stable_id.is_empty() && item.range.end >= item.range.start));
    assert!(model.nodes.iter().all(|item| !item.stable_id.is_empty() && !item.range.source_id.is_empty()));
    assert!(model.references.iter().all(|item| !item.stable_id.is_empty() && !item.range.source_id.is_empty()));
    assert!(model.raw_nodes.iter().all(|item| !item.stable_id.is_empty() && item.end >= item.start));
}
