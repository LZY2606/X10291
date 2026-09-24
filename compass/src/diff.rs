//! Semantic diff between two merged models.
//!
//! Comparison is identity-based (stable ids), so namespace reordering alone
//! produces no node diffs — only a namespace-mapping event.

use crate::merge::{MergedEdge, MergedModel, MergedNode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeChange {
    pub id: String,
    pub kind: String, // added | removed | changed
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeChange {
    pub id: String,
    pub kind: String, // added | removed
    pub ref_type: String,
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticDiff {
    pub nodes: Vec<NodeChange>,
    pub edges: Vec<EdgeChange>,
    pub namespace_reorder: bool,
}

fn nid(node: &MergedNode) -> String {
    node.id.clone()
}

pub fn diff_models(base: &MergedModel, head: &MergedModel) -> SemanticDiff {
    let mut nodes = Vec::new();
    let base_by_id: std::collections::BTreeMap<_, _> = base.nodes.iter().map(|(id, n)| (id.clone(), n.clone())).collect();
    let head_by_id: std::collections::BTreeMap<_, _> = head.nodes.iter().map(|(id, n)| (id.clone(), n.clone())).collect();

    for (id, hn) in &head_by_id {
        match base_by_id.get(id) {
            None => nodes.push(NodeChange { id: id.clone(), kind: "added".into(), fields: vec![] }),
            Some(bn) => {
                let mut fields = Vec::new();
                if bn.kind != hn.kind {
                    fields.push("kind".into());
                }
                if bn.browse_name != hn.browse_name {
                    fields.push("browseName".into());
                }
                if bn.display_name != hn.display_name {
                    fields.push("displayName".into());
                }
                if bn.candidates.len() != hn.candidates.len() {
                    fields.push("candidates".into());
                }
                if bn.unresolved_reason != hn.unresolved_reason {
                    fields.push("unresolved".into());
                }
                if !fields.is_empty() {
                    nodes.push(NodeChange { id: id.clone(), kind: "changed".into(), fields });
                }
            }
        }
    }
    for (id, _bn) in &base_by_id {
        if !head_by_id.contains_key(id) {
            nodes.push(NodeChange { id: id.clone(), kind: "removed".into(), fields: vec![] });
        }
    }

    let edge_key = |e: &MergedEdge| (e.ref_type.clone(), e.source.display(), e.target.display(), e.status.clone());
    let base_edges: std::collections::BTreeSet<_> = base.edges.iter().map(edge_key).collect();
    let head_edges: std::collections::BTreeSet<_> = head.edges.iter().map(edge_key).collect();
    let mut edges = Vec::new();
    for e in &head.edges {
        if !base_edges.contains(&edge_key(e)) {
            edges.push(EdgeChange {
                id: e.id.clone(),
                kind: "added".into(),
                ref_type: e.ref_type.clone(),
                source: e.source.display(),
                target: e.target.display(),
            });
        }
    }
    for e in &base.edges {
        if !head_edges.contains(&edge_key(e)) {
            edges.push(EdgeChange {
                id: e.id.clone(),
                kind: "removed".into(),
                ref_type: e.ref_type.clone(),
                source: e.source.display(),
                target: e.target.display(),
            });
        }
    }

    // Namespace reorder: same URI set, different global order.
    let base_uris: Vec<&String> = base.namespaces.iter().map(|n| &n.uri).collect();
    let head_uris: Vec<&String> = head.namespaces.iter().map(|n| &n.uri).collect();
    let mut a = base_uris.clone();
    let mut b = head_uris.clone();
    a.sort();
    b.sort();
    let namespace_reorder = a == b && base_uris != head_uris;

    nodes.sort_by(|x, y| x.id.cmp(&y.id));
    SemanticDiff { nodes, edges, namespace_reorder }
}
