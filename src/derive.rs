//! Derived layer. Raw imports are never mutated; every derivation is a
//! separate fact with explicit inputs. Recomputation follows the dependency
//! graph: only facts whose inputs changed are recomputed, and every fact
//! records why it is still valid or why it had to be recomputed.

use crate::merge::{MergedModel, CORE_NS};
use crate::quantity::{normalize, parse_quantity_extension, NormalizedQuantity};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedFact {
    pub key: String,
    pub subject: String,
    pub kind: String,
    pub value: serde_json::Value,
    /// Stable ids of raw inputs this fact depends on.
    pub inputs: Vec<String>,
    pub explanation: String,
    pub recomputed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Derivation {
    pub facts: BTreeMap<String, DerivedFact>,
}

fn has_subtype() -> String {
    format!("nsu={};i=45", CORE_NS)
}

/// Parent of a node: target of a declared forward HasSubtype/HasComponent edge
/// starting at this node, or source of a declared inverse edge (already
/// normalized to forward at merge time).
fn parent_of(model: &MergedModel, subject: &str) -> Option<String> {
    model
        .refs
        .iter()
        .find(|r| r.from == subject && (r.ref_type == has_subtype() || r.ref_type_raw == "HasSubtype"))
        .map(|r| r.to.clone())
        .or_else(|| {
            model
                .refs
                .iter()
                .find(|r| r.from == subject && r.ref_type_raw == "HasComponent")
                .map(|r| r.to.clone())
        })
}

/// Which subjects changed between two models (raw XML or reference set).
pub fn changed_subjects(old: &MergedModel, new: &MergedModel) -> HashSet<String> {
    let mut changed = HashSet::new();
    let old_nodes: HashMap<_, _> = old.nodes.iter().map(|n| (&n.stable_id, n)).collect();
    let new_nodes: HashMap<_, _> = new.nodes.iter().map(|n| (&n.stable_id, n)).collect();
    for (sid, n) in &new_nodes {
        match old_nodes.get(sid) {
            None => {
                changed.insert((*sid).clone());
            }
            Some(o) if o.raw_xml != n.raw_xml => {
                changed.insert((*sid).clone());
            }
            _ => {}
        }
    }
    for sid in old_nodes.keys() {
        if !new_nodes.contains_key(sid) {
            changed.insert((*sid).clone());
        }
    }
    let old_refs: HashSet<_> = old
        .refs
        .iter()
        .map(|r| (&r.from, &r.to, &r.ref_type, r.declared_forward))
        .collect();
    let new_refs: HashSet<_> = new
        .refs
        .iter()
        .map(|r| (&r.from, &r.to, &r.ref_type, r.declared_forward))
        .collect();
    for r in old_refs.symmetric_difference(&new_refs) {
        changed.insert(r.0.clone());
        changed.insert(r.1.clone());
    }
    changed
}

fn fact_defs(model: &MergedModel) -> Vec<(String, String, String, Vec<String>)> {
    // (key, subject, kind, inputs)
    let mut defs = Vec::new();
    for n in &model.nodes {
        if n.unresolved {
            continue;
        }
        for ext in &n.extensions_raw {
            if parse_quantity_extension(ext).is_some() {
                defs.push((
                    format!("quantity:{}", n.stable_id),
                    n.stable_id.clone(),
                    "normalized_quantity".to_string(),
                    vec![n.stable_id.clone()],
                ));
            }
        }
        let parent = parent_of(model, &n.stable_id);
        let mut inputs = vec![n.stable_id.clone()];
        if let Some(p) = &parent {
            inputs.push(p.clone());
        }
        defs.push((
            format!("browse_path:{}", n.stable_id),
            n.stable_id.clone(),
            "browse_path".to_string(),
            inputs,
        ));
    }
    defs
}

fn compute_fact(model: &MergedModel, key: &str, kind: &str, subject: &str) -> serde_json::Value {
    match kind {
        "normalized_quantity" => {
            let node = model.nodes.iter().find(|n| n.stable_id == subject);
            let q = node.and_then(|n| {
                n.extensions_raw.iter().find_map(|e| parse_quantity_extension(e))
            });
            match q {
                Some(q) => {
                    let nq: NormalizedQuantity = normalize(&q);
                    serde_json::to_value(nq).unwrap()
                }
                None => serde_json::json!({"diagnostic": "quantity extension removed"}),
            }
        }
        "browse_path" => {
            let mut parts = Vec::new();
            let mut cur = Some(subject.to_string());
            let mut guard = 0;
            while let Some(sid) = cur {
                if guard > 64 {
                    parts.push("<cycle>".to_string());
                    break;
                }
                guard += 1;
                let node = model.nodes.iter().find(|n| n.stable_id == sid);
                match node {
                    Some(n) => {
                        parts.push(format!("{}:{}", n.browse_name_uri, n.browse_name_text));
                        cur = parent_of(model, &sid);
                    }
                    None => {
                        parts.push(format!("unresolved:{sid}"));
                        cur = None;
                    }
                }
            }
            parts.reverse();
            serde_json::json!({"path": parts.join("/")})
        }
        _ => serde_json::Value::Null,
    }
}

/// Recompute the derivation for `model`, reusing `prev` facts whose inputs
/// are unchanged. Returns the new derivation.
pub fn recompute(
    model: &MergedModel,
    prev: Option<&Derivation>,
    prev_model: Option<&MergedModel>,
) -> Derivation {
    let changed: HashSet<String> = match prev_model {
        None => model.nodes.iter().map(|n| n.stable_id.clone()).collect(),
        Some(old) => changed_subjects(old, model),
    };
    let defs = fact_defs(model);
    // input -> dependent keys, from current definitions.
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    for (key, _, _, inputs) in &defs {
        for i in inputs {
            dependents.entry(i.clone()).or_default().push(key.clone());
        }
    }
    // Dirty set: changed subjects, propagated along the dependency graph.
    let mut dirty_keys: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = changed.iter().cloned().collect();
    let mut seen_subjects: HashSet<String> = changed.clone();
    while let Some(s) = queue.pop_front() {
        if let Some(keys) = dependents.get(&s) {
            for k in keys {
                if dirty_keys.insert(k.clone()) {
                    // A dirty browse_path invalidates facts listing it as input.
                    if let Some((_, subj)) = k.split_once(':') {
                        if seen_subjects.insert(subj.to_string()) {
                            queue.push_back(subj.to_string());
                        }
                    }
                }
            }
        }
    }

    let mut facts = BTreeMap::new();
    for (key, subject, kind, inputs) in defs {
        let dirty = dirty_keys.contains(&key) || prev.is_none();
        if !dirty {
            if let Some(prev_fact) = prev.and_then(|p| p.facts.get(&key)) {
                let mut f = prev_fact.clone();
                f.recomputed = false;
                f.explanation = format!(
                    "still valid: inputs [{}] unchanged",
                    f.inputs.join(", ")
                );
                facts.insert(key, f);
                continue;
            }
        }
        let reason = match prev {
            None => "initial computation".to_string(),
            Some(p) if !p.facts.contains_key(&key) => {
                "new fact: no previous version".to_string()
            }
            Some(_) => format!(
                "recomputed: inputs [{}] changed",
                inputs
                    .iter()
                    .filter(|i| changed.contains(*i))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let value = compute_fact(model, &key, &kind, &subject);
        facts.insert(
            key.clone(),
            DerivedFact {
                key,
                subject,
                kind,
                value,
                inputs,
                explanation: reason,
                recomputed: true,
            },
        );
    }
    Derivation { facts }
}
