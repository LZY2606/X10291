//! Semantic diff between two merged models.

use crate::model::{stable_id, MergedModel};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SemanticDiff {
    pub nodes_added: Vec<String>,
    pub nodes_removed: Vec<String>,
    pub nodes_changed: Vec<NodeChange>,
    pub edges_added: Vec<String>,
    pub edges_removed: Vec<String>,
    pub namespaces_added: Vec<String>,
    pub namespaces_removed: Vec<String>,
    pub diagnostics_only_in_a: Vec<String>,
    pub diagnostics_only_in_b: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeChange {
    pub id: String,
    pub changed_attrs: Vec<String>,
    pub display_name_changed: bool,
    pub extensions_changed: bool,
}

fn edge_key(e: &crate::model::Edge) -> String {
    format!(
        "{} -[{}]-> {}",
        stable_id(&e.source),
        stable_id(&e.reference_type),
        stable_id(&e.target)
    )
}

pub fn diff(a: &MergedModel, b: &MergedModel) -> SemanticDiff {
    let mut d = SemanticDiff::default();
    let ids_a: BTreeSet<_> = a.nodes.keys().collect();
    let ids_b: BTreeSet<_> = b.nodes.keys().collect();
    d.nodes_added = ids_b.difference(&ids_a).map(|s| s.to_string()).collect();
    d.nodes_removed = ids_a.difference(&ids_b).map(|s| s.to_string()).collect();
    for id in ids_a.intersection(&ids_b) {
        let (na, nb) = (&a.nodes[*id], &b.nodes[*id]);
        let changed_attrs: Vec<String> = na
            .attrs
            .iter()
            .chain(nb.attrs.iter())
            .filter(|(k, _)| na.attrs.get(*k) != nb.attrs.get(*k))
            .map(|(k, _)| k.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let dn = na.display_name != nb.display_name;
        let ex = na.extensions != nb.extensions;
        if !changed_attrs.is_empty() || dn || ex {
            d.nodes_changed.push(NodeChange {
                id: id.to_string(),
                changed_attrs,
                display_name_changed: dn,
                extensions_changed: ex,
            });
        }
    }
    let ea: BTreeSet<String> = a.edges.iter().map(edge_key).collect();
    let eb: BTreeSet<String> = b.edges.iter().map(edge_key).collect();
    d.edges_added = eb.difference(&ea).cloned().collect();
    d.edges_removed = ea.difference(&eb).cloned().collect();
    let na: BTreeSet<_> = a.namespace_uris.iter().cloned().collect();
    let nb: BTreeSet<_> = b.namespace_uris.iter().cloned().collect();
    d.namespaces_added = nb.difference(&na).cloned().collect();
    d.namespaces_removed = na.difference(&nb).cloned().collect();
    let da: BTreeSet<String> = a.diagnostics.iter().map(|x| format!("{:?}", x)).collect();
    let db: BTreeSet<String> = b.diagnostics.iter().map(|x| format!("{:?}", x)).collect();
    d.diagnostics_only_in_a = da.difference(&db).cloned().collect();
    d.diagnostics_only_in_b = db.difference(&da).cloned().collect();
    d
}
