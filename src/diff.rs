use crate::engine::build_facts;
use crate::model::*;
use crate::util::stable_id;
use std::collections::{BTreeMap, BTreeSet};

pub fn compare_snapshots(
    previous: Option<&ModelSnapshot>,
    mut next: ModelSnapshot,
    parameters: &RuleParameters,
) -> (ModelSnapshot, Vec<SemanticChange>) {
    let Some(previous) = previous else {
        mark_initial(&mut next);
        return (next, Vec::new());
    };

    let mut changes = Vec::new();
    let before_namespace = index(&previous.namespaces, |item| item.stable_id.clone());
    let after_namespace = index(&next.namespaces, |item| item.stable_id.clone());
    compare_maps("namespace", &to_json_map(&before_namespace), &to_json_map(&after_namespace), &mut changes);

    let before_nodes = index(&previous.nodes, |item| item.expanded_node_id.clone());
    let after_nodes = index(&next.nodes, |item| item.expanded_node_id.clone());
    compare_maps("node", &to_json_map(&before_nodes), &to_json_map(&after_nodes), &mut changes);

    let before_references = index(&previous.references, |item| item.stable_id.clone());
    let after_references = index(&next.references, |item| item.stable_id.clone());
    compare_maps("reference", &to_json_map(&before_references), &to_json_map(&after_references), &mut changes);

    let before_unresolved = index(&previous.unresolved_nodes, |item| item.expanded_node_id.clone());
    let after_unresolved = index(&next.unresolved_nodes, |item| item.expanded_node_id.clone());
    compare_maps("unresolved-dependency", &to_json_map(&before_unresolved), &to_json_map(&after_unresolved), &mut changes);

    let before_quantities = quantity_index(previous);
    let after_quantities = quantity_index(&next);
    compare_maps("quantity", &to_json_map(&before_quantities), &to_json_map(&after_quantities), &mut changes);

    let mut changed_dependencies: BTreeSet<String> = BTreeSet::new();
    for change in &changes {
        changed_dependencies.insert(change.subject_id.clone());
    }
    changed_dependencies.insert(parameters.rule_version.clone());
    changed_dependencies.insert(parameters.pressure_unit.clone());
    if parameters.rule_version != previous.facts.first().map(|_| "").unwrap_or("") {
        changed_dependencies.insert(parameters.rule_version.clone());
    }

    next.facts = build_facts(&next.namespaces, &next.nodes, &next.references, &next.unresolved_nodes, parameters);
    let previous_facts = index(&previous.facts, |fact| fact.stable_id.clone());
    let fact_snapshot = next.facts.clone();
    for fact in next.facts.iter_mut() {
        let affected = fact.depends_on.iter().any(|dependency| changed_dependencies.contains(dependency))
            || transitive_dependency_hit(&fact.stable_id, &fact_snapshot, &changed_dependencies);
        fact.status = match previous_facts.get(&fact.stable_id) {
            Some(old) if old == fact && !affected => FactStatus::Valid,
            Some(_) => FactStatus::Recomputed,
            None => FactStatus::Recomputed,
        };
    }

    for old_fact in previous.facts.iter() {
        if !next.facts.iter().any(|fact| fact.stable_id == old_fact.stable_id) {
            changes.push(SemanticChange {
                stable_id: stable_id(&["change", "fact-removed", &old_fact.stable_id]),
                kind: "derived-fact-removed".into(),
                severity: "warning".into(),
                subject_id: old_fact.stable_id.clone(),
                before: serde_json::to_string(old_fact).unwrap_or_default(),
                after: String::new(),
                cause_event_ids: next.events.iter().map(|event| event.event_id.clone()).collect(),
            });
        }
    }

    changes.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    (next, changes)
}

fn mark_initial(snapshot: &mut ModelSnapshot) {
    for fact in snapshot.facts.iter_mut() {
        fact.status = FactStatus::Valid;
    }
}

fn index<T: Clone, F: Fn(&T) -> String>(values: &[T], key: F) -> BTreeMap<String, T> {
    values.iter().map(|value| (key(value), value.clone())).collect()
}

fn to_json_map<T: serde::Serialize>(values: &BTreeMap<String, T>) -> BTreeMap<String, serde_json::Value> {
    values
        .iter()
        .filter_map(|(key, value)| serde_json::to_value(value).ok().map(|json| (key.clone(), json)))
        .collect()
}

fn compare_maps(
    kind: &str,
    before: &BTreeMap<String, serde_json::Value>,
    after: &BTreeMap<String, serde_json::Value>,
    changes: &mut Vec<SemanticChange>,
) {
    for (identity, old) in before {
        match after.get(identity) {
            Some(new) if old != new => changes.push(change(kind, "changed", identity, old, new)),
            None => changes.push(change(kind, "removed", identity, old, &serde_json::Value::Null)),
            _ => {}
        }
    }
    for (identity, new) in after {
        if !before.contains_key(identity) {
            changes.push(change(kind, "added", identity, &serde_json::Value::Null, new));
        }
    }
}

fn change(kind: &str, delta: &str, identity: &str, before: &serde_json::Value, after: &serde_json::Value) -> SemanticChange {
    SemanticChange {
        stable_id: stable_id(&["change", kind, delta, identity]),
        kind: format!("{kind}-{delta}"),
        severity: if delta == "removed" { "warning" } else { "info" }.to_string(),
        subject_id: identity.to_string(),
        before: serde_json::to_string(before).unwrap_or_default(),
        after: serde_json::to_string(after).unwrap_or_default(),
        cause_event_ids: Vec::new(),
    }
}

fn quantity_index(snapshot: &ModelSnapshot) -> BTreeMap<String, Quantity> {
    snapshot
        .nodes
        .iter()
        .flat_map(|node| node.quantities.iter().map(move |quantity| (format!("{}:{}", node.stable_id, quantity.stable_id), quantity.clone())))
        .collect()
}

fn transitive_dependency_hit(
    fact_id: &str,
    facts: &[DerivedFact],
    changed: &BTreeSet<String>,
) -> bool {
    let mut visited = BTreeSet::new();
    let mut stack: Vec<String> = facts
        .iter()
        .find(|fact| fact.stable_id == fact_id)
        .map(|fact| fact.depends_on.clone())
        .unwrap_or_default();
    while let Some(dependency) = stack.pop() {
        if changed.contains(&dependency) {
            return true;
        }
        if !visited.insert(dependency.clone()) {
            continue;
        }
        if let Some(fact) = facts.iter().find(|fact| fact.stable_id == dependency) {
            stack.extend(fact.depends_on.iter().cloned());
        }
    }
    false
}
