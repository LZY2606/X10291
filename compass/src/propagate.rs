//! Dependency-graph propagation.
//!
//! When sources/parameters change, only the affected closure is recomputed.
//! Each derived object records why it is still valid or why it had to be
//! recomputed, using machine-readable propagation events.

use crate::merge::MergedModel;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagationEvent {
    pub object_id: String,
    pub state: String, // valid | recomputed | new
    pub reason_code: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagationReport {
    pub changed_sources: Vec<String>,
    pub changed_parameters: Vec<String>,
    pub events: Vec<PropagationEvent>,
}

/// Compute the closure of nodes that must be recomputed given changed source
/// files and changed parameter keys.
///
/// A node depends on: its defining source files, and (through normalized
/// reference edges) its direct neighbors; type hierarchy propagates through
/// HasSubtype edges. Quantities depend on their node plus normalization params.
pub fn affected_closure(
    model: &MergedModel,
    changed_sources: &[String],
    changed_parameters: &[String],
) -> (BTreeSet<String>, PropagationReport) {
    let changed: BTreeSet<&str> = changed_sources.iter().map(String::as_str).collect();
    let mut dirty: BTreeSet<String> = BTreeSet::new();
    let mut report = PropagationReport {
        changed_sources: changed_sources.to_vec(),
        changed_parameters: changed_parameters.to_vec(),
        events: Vec::new(),
    };

    // Seeds: nodes directly defined (or retained as placeholder) by changed src.
    for node in model.nodes.values() {
        if node.source_ids.iter().any(|s| changed.contains(s.as_str())) {
            dirty.insert(node.id.clone());
        }
    }

    // Parameter changes (normalization) only dirty quantities' nodes.
    let params: BTreeSet<&str> = changed_parameters.iter().map(String::as_str).collect();
    if !params.is_empty() {
        for q in &model.quantities {
            // pressure/temperature normalization parameters affect every
            // quantity node; kept general on purpose.
            dirty.insert(q.node_id.clone());
        }
    }

    // Propagate across normalized edges: adjacency is undirected for closure,
    // HasSubtype additionally walks the hierarchy.
    let key_of = |k: &crate::model::GlobalNodeId| {
        crate::id::stable_id("node", &[&k.ns_uri, &k.id])
    };
    let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &model.edges {
        let s = key_of(&e.source);
        let t = key_of(&e.target);
        adjacency.entry(s.clone()).or_default().push(t.clone());
        adjacency.entry(t).or_default().push(s);
    }

    let mut frontier: Vec<String> = dirty.iter().cloned().collect();
    while let Some(id) = frontier.pop() {
        if let Some(neighbors) = adjacency.get(&id) {
            for n in neighbors {
                if dirty.insert(n.clone()) {
                    frontier.push(n.clone());
                }
            }
        }
    }

    for node in model.nodes.values() {
        if dirty.contains(&node.id) {
            let direct = node.source_ids.iter().any(|s| changed.contains(s.as_str()));
            report.events.push(PropagationEvent {
                object_id: node.id.clone(),
                state: "recomputed".into(),
                reason_code: if direct { "SOURCE_CHANGED" } else { "REFERENCE_CLOSURE" }.into(),
                reason: if direct {
                    "node is defined by a changed source".into()
                } else {
                    "node is in the reference closure of a changed node".into()
                },
            });
        } else {
            report.events.push(PropagationEvent {
                object_id: node.id.clone(),
                state: "valid".into(),
                reason_code: "UNCHANGED_CLOSURE".into(),
                reason: "none of the node's sources or reference neighbors changed".into(),
            });
        }
    }
    (dirty, report)
}
