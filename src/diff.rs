//! 两个合并模型版本之间的语义差异。

use crate::model::*;
use std::collections::BTreeMap;

pub fn diff(old: &MergedModel, new: &MergedModel) -> Vec<DiffEntry> {
    let mut out = Vec::new();
    let old_nodes: BTreeMap<&str, &MergedNode> =
        old.nodes.iter().map(|n| (n.global_id.as_str(), n)).collect();
    let new_nodes: BTreeMap<&str, &MergedNode> =
        new.nodes.iter().map(|n| (n.global_id.as_str(), n)).collect();

    for (id, n) in &new_nodes {
        match old_nodes.get(id) {
            None => out.push(DiffEntry {
                id: stable_id(&["NodeAdded", id]),
                kind: "NodeAdded".into(),
                entity: id.to_string(),
                field: String::new(),
                old: String::new(),
                new: n.display_name.clone().unwrap_or_default(),
            }),
            Some(o) => {
                let cmp = |field: &str, a: &str, b: &str, out: &mut Vec<DiffEntry>| {
                    if a != b {
                        out.push(DiffEntry {
                            id: stable_id(&["AttrChanged", id, field]),
                            kind: "AttrChanged".into(),
                            entity: id.to_string(),
                            field: field.to_string(),
                            old: a.to_string(),
                            new: b.to_string(),
                        });
                    }
                };
                cmp("DisplayName", o.display_name.as_deref().unwrap_or(""), n.display_name.as_deref().unwrap_or(""), &mut out);
                cmp("Value", &o.value_raw.as_ref().map(|v| v.text.clone()).unwrap_or_default(), &n.value_raw.as_ref().map(|v| v.text.clone()).unwrap_or_default(), &mut out);
                cmp("ValueSI", &o.value_si.map(|v| v.to_string()).unwrap_or_default(), &n.value_si.map(|v| v.to_string()).unwrap_or_default(), &mut out);
                cmp("Status", &format!("{:?}", o.status), &format!("{:?}", n.status), &mut out);
            }
        }
    }
    for id in old_nodes.keys() {
        if !new_nodes.contains_key(id) {
            out.push(DiffEntry {
                id: stable_id(&["NodeRemoved", id]),
                kind: "NodeRemoved".into(),
                entity: id.to_string(),
                field: String::new(),
                old: old_nodes[id].display_name.clone().unwrap_or_default(),
                new: String::new(),
            });
        }
    }

    let old_edges: BTreeMap<(&str, &str, &str), &Edge> =
        old.edges.iter().map(|e| ((e.src.as_str(), e.dst.as_str(), e.ref_type.as_str()), e)).collect();
    let new_edges: BTreeMap<(&str, &str, &str), &Edge> =
        new.edges.iter().map(|e| ((e.src.as_str(), e.dst.as_str(), e.ref_type.as_str()), e)).collect();
    for (k, _) in &new_edges {
        if !old_edges.contains_key(k) {
            out.push(DiffEntry {
                id: stable_id(&["EdgeAdded", k.0, k.1, k.2]),
                kind: "EdgeAdded".into(),
                entity: format!("{} -> {}", k.0, k.1),
                field: k.2.to_string(),
                old: String::new(),
                new: "present".into(),
            });
        }
    }
    for (k, _) in &old_edges {
        if !new_edges.contains_key(k) {
            out.push(DiffEntry {
                id: stable_id(&["EdgeRemoved", k.0, k.1, k.2]),
                kind: "EdgeRemoved".into(),
                entity: format!("{} -> {}", k.0, k.1),
                field: k.2.to_string(),
                old: "present".into(),
                new: String::new(),
            });
        }
    }

    if old.namespaces != new.namespaces {
        out.push(DiffEntry {
            id: stable_id(&["NamespaceRemapped", &old.namespaces.join("|"), &new.namespaces.join("|")]),
            kind: "NamespaceRemapped".into(),
            entity: "namespaces".into(),
            field: "uri-set".into(),
            old: old.namespaces.join(" | "),
            new: new.namespaces.join(" | "),
        });
    }
    out
}
