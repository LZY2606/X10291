use crate::merge::MergedModel;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffEntry {
    pub kind: String, // Added | Removed | Changed
    pub node: String,
    pub changes: Vec<FieldChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelDiff {
    pub entries: Vec<DiffEntry>,
}

/// 两个合并模型之间的语义差异（节点增删、属性/名称/引用变化）。
pub fn diff_models(a: &MergedModel, b: &MergedModel) -> ModelDiff {
    let mut entries = Vec::new();
    for (id, bn) in &b.nodes {
        match a.nodes.get(id) {
            None => entries.push(DiffEntry { kind: "Added".into(), node: id.clone(), changes: vec![] }),
            Some(an) => {
                let mut changes = Vec::new();
                if an.browse_name != bn.browse_name {
                    changes.push(FieldChange { field: "BrowseName".into(), before: an.browse_name.clone(), after: bn.browse_name.clone() });
                }
                if an.display_name != bn.display_name {
                    changes.push(FieldChange { field: "DisplayName".into(), before: an.display_name.clone(), after: bn.display_name.clone() });
                }
                let am: BTreeMap<&str, &str> = an.attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                let bm: BTreeMap<&str, &str> = bn.attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                for (k, bv) in &bm {
                    match am.get(k) {
                        None => changes.push(FieldChange { field: format!("attr:{k}"), before: "∅".into(), after: bv.to_string() }),
                        Some(av) if av != bv => changes.push(FieldChange { field: format!("attr:{k}"), before: av.to_string(), after: bv.to_string() }),
                        _ => {}
                    }
                }
                for k in am.keys() {
                    if !bm.contains_key(k) {
                        changes.push(FieldChange { field: format!("attr:{k}"), before: am[k].to_string(), after: "∅".into() });
                    }
                }
                let ea: BTreeSet<(&str, &str)> = a.edges.iter().filter(|e| &e.from == id).map(|e| (e.ref_type.as_str(), e.to.as_str())).collect();
                let eb: BTreeSet<(&str, &str)> = b.edges.iter().filter(|e| &e.from == id).map(|e| (e.ref_type.as_str(), e.to.as_str())).collect();
                if ea != eb {
                    let fmt = |s: &BTreeSet<(&str, &str)>| s.iter().map(|(t, to)| format!("{t}->{to}")).collect::<Vec<_>>().join(", ");
                    changes.push(FieldChange { field: "references".into(), before: fmt(&ea), after: fmt(&eb) });
                }
                if !changes.is_empty() {
                    entries.push(DiffEntry { kind: "Changed".into(), node: id.clone(), changes });
                }
            }
        }
    }
    for id in a.nodes.keys() {
        if !b.nodes.contains_key(id) {
            entries.push(DiffEntry { kind: "Removed".into(), node: id.clone(), changes: vec![] });
        }
    }
    ModelDiff { entries }
}
