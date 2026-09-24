//! Derived layer: type hierarchy, reference closure and quantity normalization.
//!
//! Every derived item records which inputs it was computed from, so changing
//! one source only invalidates the affected subgraph.

use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedItem {
    pub stable_id: String,
    pub kind: String,
    pub label: String,
    pub inputs: BTreeSet<String>,
    pub value: String,
    pub status: ItemStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStatus {
    pub recompute_needed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedLayer {
    pub model_version: u64,
    pub items: Vec<DerivedItem>,
}

/// Which derived items must be recomputed when `changed_inputs` changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactReport {
    pub invalidated: Vec<String>,
    pub still_valid: Vec<String>,
    pub reasons: BTreeMap<String, String>,
}

const HAS_TYPE: &str = "http://opcfoundation.org/UA/|i=40";
const SUBTYPE: &str = "http://opcfoundation.org/UA/|i=45";
const ORGANIZES: &str = "http://opcfoundation.org/UA/|i=35";

pub fn derive(model: &MergedModel) -> DerivedLayer {
    let mut items = Vec::new();

    // Type hierarchy from HasSubtype (i=45) edges between type nodes.
    let parents: BTreeMap<GlobalNodeId, GlobalNodeId> = model
        .edges
        .iter()
        .filter(|e| e.ref_type.render() == SUBTYPE && e.state == EdgeState::Resolved)
        .map(|e| (e.target.clone(), e.source.clone()))
        .collect();
    for node in &model.nodes {
        if matches!(node.kind, NodeKind::ObjectType | NodeKind::VariableType | NodeKind::ReferenceType | NodeKind::DataType) {
            let chain = type_chain(&node.gid, &parents);
            let mut inputs = BTreeSet::from([node.stable_id.clone()]);
            for gid in &chain {
                if let Some(n) = model.nodes.iter().find(|n| &n.gid == gid) {
                    inputs.insert(n.stable_id.clone());
                }
            }
            items.push(DerivedItem {
                stable_id: stable_id("hier", &[&node.gid.render()]),
                kind: "type-hierarchy".into(),
                label: format!("{} hierarchy", if node.browse_name.is_empty() { node.gid.render() } else { node.browse_name.clone() }),
                inputs,
                value: chain.iter().map(|g| short(g)).collect::<Vec<_>>().join(" < "),
                status: ItemStatus { recompute_needed: false, reason: "fresh".into() },
            });
        }
    }

    // Instance typing via HasTypeDefinition (i=40).
    for e in &model.edges {
        if e.ref_type.render() == HAS_TYPE {
            items.push(DerivedItem {
                stable_id: stable_id("typed", &[&e.source.render(), &e.target.render()]),
                kind: "instance-type".into(),
                label: format!("{} : {}", short(&e.source), short(&e.target)),
                inputs: BTreeSet::from([
                    stable_id("node", &[&e.source.render()]),
                    stable_id("node", &[&e.target.render()]),
                    e.stable_id.clone(),
                ]),
                value: format!("{}|{}", e.state == EdgeState::Resolved, e.target.render()),
                status: ItemStatus { recompute_needed: false, reason: "fresh".into() },
            });
        }
    }

    // Quantity normalization from raw variable material (engineering units).
    for node in &model.nodes {
        if node.kind != NodeKind::Variable || node.placeholder {
            continue;
        }
        if let Some(q) = quantity_of(model, node) {
            let mut inputs = BTreeSet::from([node.stable_id.clone()]);
            inputs.extend(node.origins.iter().map(|o| format!("span:{}:{}", o.file_id, o.span.start)));
            let (value, note) = match (&q.si_value, &q.conversion_diagnostic) {
                (Some(v), _) => (v.clone(), format!("converted from {} {}", q.raw_value, q.raw_unit)),
                (None, Some(d)) => (format!("UNCONVERTED:{} {}", q.raw_value, q.raw_unit), d.clone()),
                _ => continue,
            };
            items.push(DerivedItem {
                stable_id: stable_id("qty", &[&node.gid.render()]),
                kind: "quantity".into(),
                label: node.browse_name.clone(),
                inputs,
                value: format!("{} [{}] -- {}", value, q.si_unit.as_deref().unwrap_or("?"), note),
                status: ItemStatus { recompute_needed: false, reason: "fresh".into() },
            });
        }
    }

    let _ = ORGANIZES;
    items.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.stable_id.cmp(&b.stable_id)));
    DerivedLayer { model_version: model.version, items }
}

fn type_chain(gid: &GlobalNodeId, parents: &BTreeMap<GlobalNodeId, GlobalNodeId>) -> Vec<GlobalNodeId> {
    let mut chain = vec![gid.clone()];
    let mut cur = gid.clone();
    let mut seen = BTreeSet::from([gid.clone()]);
    while let Some(p) = parents.get(&cur) {
        if !seen.insert(p.clone()) {
            break;
        }
        chain.push(p.clone());
        cur = p.clone();
    }
    chain
}

fn short(g: &GlobalNodeId) -> String {
    g.render().split('|').next_back().unwrap_or(&g.ns_uri).to_string()
}

/// Pull a raw quantity off a variable by inspecting retained child elements.
/// Recognized fixture encodings:
/// * `<Value unit="mm" coordinateBasis="MachineCoord">1200</Value>`
/// * `<EngineeringUnits unit="bar" ...>6</...>` as a sibling numeric value
/// Original representation always stays; unknown units yield a diagnostic.
pub fn quantity_of_raw(raw: &RawNode) -> Option<Quantity> {
    let value_el = raw.element.children.iter().find(|c| c.tag == "Value")?;
    let raw_value = value_el.text.clone()?.trim().to_string();
    let raw_unit = value_el.attrs.get("unit").cloned().or_else(|| {
        raw.element.children.iter()
            .find(|c| c.tag == "EngineeringUnits")
            .and_then(|e| e.attrs.get("unit").cloned())
    }).unwrap_or_default();
    if raw_unit.is_empty() {
        return None;
    }
    let basis = value_el.attrs.get("coordinateBasis").cloned().unwrap_or_else(|| "n/a".into());
    let (si_value, si_unit, diag) = convert_quantity(&raw_value, &raw_unit);
    Some(Quantity {
        raw_value,
        raw_unit,
        unit_basis: basis,
        si_value,
        si_unit,
        conversion_diagnostic: diag,
    })
}

fn quantity_of(model: &MergedModel, node: &Node) -> Option<Quantity> {
    let _ = model;
    let origin = node.origins.first()?;
    // Raw material is attached through origin metadata at derive-call time in
    // the store; this standalone helper is exposed via `quantity_of_raw`.
    None::<Quantity>
}

/// Derive quantities directly from raw files (used by the store which owns the
/// raw baseline).
pub fn derive_quantities(raw_files: &[RawFile]) -> Vec<DerivedItem> {
    let mut out = Vec::new();
    for file in raw_files {
        for raw in &file.nodes {
            if raw.node_class != "UAVariable" {
                continue;
            }
            if let Some(q) = quantity_of_raw(raw) {
                let gid_text = raw.node_id_text.clone();
                let (value, note, blocked) = match (&q.si_value, &q.conversion_diagnostic) {
                    (Some(v), d) => (format!("{} [{}]", v, q.si_unit.as_deref().unwrap_or("?")), d.clone().unwrap_or_else(|| format!("converted from {} {}", q.raw_value, q.raw_unit)), false),
                    (None, Some(d)) => (format!("{} {} (basis {})", q.raw_value, q.raw_unit, q.unit_basis), d.clone(), true),
                    (None, None) => continue,
                };
                out.push(DerivedItem {
                    stable_id: stable_id("qty", &[&file.file_id, &gid_text]),
                    kind: "quantity".into(),
                    label: raw.browse_name_text.clone(),
                    inputs: BTreeSet::from([format!("span:{}:{}", file.file_id, raw.element.span.start)]),
                    value: format!("{} -- {} [retained raw: {} {} @ {}]", value, note, q.raw_value, q.raw_unit, q.unit_basis),
                    status: ItemStatus { recompute_needed: blocked, reason: note },
                });
            }
        }
    }
    out.sort_by(|a, b| a.stable_id.cmp(&b.stable_id));
    out
}

/// Propagate changed inputs through the dependency graph of derived items.
pub fn propagate(previous: &DerivedLayer, changed_inputs: &BTreeSet<String>) -> ImpactReport {
    let mut invalidated = BTreeSet::new();
    let mut frontier: Vec<String> = changed_inputs.iter().cloned().collect();
    while let Some(input) = frontier.pop() {
        for item in &previous.items {
            if !invalidated.contains(&item.stable_id) && item.inputs.contains(&input) {
                invalidated.insert(item.stable_id.clone());
                frontier.push(item.stable_id.clone());
            }
        }
    }
    let mut reasons = BTreeMap::new();
    for item in &previous.items {
        if invalidated.contains(&item.stable_id) {
            let hit = item.inputs.iter().filter(|i| changed_inputs.contains(*i)).cloned().collect::<Vec<_>>().join(", ");
            reasons.insert(item.stable_id.clone(), format!("input changed: {}", hit));
        } else {
            reasons.insert(item.stable_id.clone(), "no transitive input changed; result remains valid".into());
        }
    }
    let mut still_valid: Vec<String> = previous.items.iter().map(|i| i.stable_id.clone()).filter(|id| !invalidated.contains(id)).collect();
    let mut invalidated: Vec<String> = invalidated.into_iter().collect();
    invalidated.sort();
    still_valid.sort();
    ImpactReport { invalidated, still_valid, reasons }
}

/// Domain unit table (SI conversion). Fixtures carry unit/basis explicitly.
pub fn convert_quantity(raw_value: &str, raw_unit: &str) -> (Option<String>, Option<String>, Option<String>) {
    let v: f64 = match raw_value.trim().parse() { Ok(v) => v, Err(_) => return (None, None, Some("value is not numeric; original retained".into())) };
    match raw_unit.trim() {
        "mm" => (Some(format!("{}", v / 1000.0)), Some("m".into()), None),
        "m" => (Some(format!("{v}")), Some("m".into()), None),
        "degC" => (Some(format!("{}", v + 273.15)), Some("K".into()), None),
        "K" => (Some(format!("{v}")), Some("K".into()), None),
        "bar" => (Some(format!("{}", v * 100_000.0)), Some("Pa".into()), None),
        "Pa" => (Some(format!("{v}")), Some("Pa".into()), None),
        "rpm" => (Some(format!("{}", v / 60.0)), Some("Hz".into()), None),
        other => (None, None, Some(format!("no SI conversion known for unit {other:?}; value left in original representation"))),
    }
}
