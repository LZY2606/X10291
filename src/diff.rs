//! Semantic diff between two merged model versions.

use crate::model::*;
use std::collections::BTreeMap;

pub fn diff_models(from: Option<&MergedModel>, to: &MergedModel) -> ModelDiff {
    let mut changes = Vec::new();
    let old_nodes: BTreeMap<_, _> = from.map(|m| m.nodes.iter().map(|n| (n.gid.clone(), n)).collect()).unwrap_or_default();
    let new_nodes: BTreeMap<_, _> = to.nodes.iter().map(|n| (n.gid.clone(), n)).collect();

    for (gid, new) in &new_nodes {
        match old_nodes.get(gid) {
            None => changes.push(Change {
                kind: ChangeKind::Added,
                subject: gid.render(),
                detail: format!("{} `{}` added", kind_label(new.kind), nz(&new.browse_name)),
                stable_id: stable_id("change", &["node-add", &gid.render()]),
            }),
            Some(old) => {
                if old.browse_name != new.browse_name || old.kind != new.kind || old.duplicate != new.duplicate || old.placeholder != new.placeholder {
                    changes.push(Change {
                        kind: ChangeKind::Changed,
                        subject: gid.render(),
                        detail: format!(
                            "browse {} -> {}, kind {} -> {}, duplicate {} -> {}, placeholder {} -> {}",
                            nz(&old.browse_name), nz(&new.browse_name),
                            kind_label(old.kind), kind_label(new.kind),
                            old.duplicate, new.duplicate, old.placeholder, new.placeholder
                        ),
                        stable_id: stable_id("change", &["node-mod", &gid.render()]),
                    });
                }
            }
        }
    }
    for (gid, old) in old_nodes {
        if !new_nodes.contains_key(&gid) {
            changes.push(Change {
                kind: ChangeKind::Removed,
                subject: gid.render(),
                detail: format!("{} `{}` removed", kind_label(old.kind), nz(&old.browse_name)),
                stable_id: stable_id("change", &["node-del", &gid.render()]),
            });
        }
    }

    let edge_key = |e: &Edge| (e.source.render(), e.target.render(), e.ref_type.render(), e.forward_in_source, e.state, e.file_id.clone());
    let old_edges: BTreeMap<_, _> = from.map(|m| m.edges.iter().map(|e| (edge_key(e), e)).collect()).unwrap_or_default();
    let new_edges: BTreeMap<_, _> = to.edges.iter().map(|e| (edge_key(e), e)).collect();
    for (k, e) in &new_edges {
        if !old_edges.contains_key(k) {
            changes.push(Change { kind: ChangeKind::Added, subject: e.stable_id.clone(), detail: format!("reference {} -> {}", e.source.render(), e.target.render()), stable_id: stable_id("change", &["edge-add", &e.stable_id]) });
        }
    }
    for (k, e) in &old_edges {
        if !new_edges.contains_key(k) {
            changes.push(Change { kind: ChangeKind::Removed, subject: e.stable_id.clone(), detail: format!("reference {} -> {} removed", e.source.render(), e.target.render()), stable_id: stable_id("change", &["edge-del", &e.stable_id]) });
        }
    }

    let sig_diag = |d: &Diagnostic| (d.code.clone(), d.subject.clone(), d.file_id.clone());
    let old_d: BTreeMap<_, _> = from.map(|m| m.diagnostics.iter().map(|d| (sig_diag(d), d)).collect()).unwrap_or_default();
    let new_d: BTreeMap<_, _> = to.diagnostics.iter().map(|d| (sig_diag(d), d)).collect();
    for (k, d) in &new_d {
        if !old_d.contains_key(k) {
            changes.push(Change { kind: ChangeKind::Added, subject: d.subject.clone().unwrap_or_default(), detail: format!("new diagnostic [{}] {}", d.code, d.message), stable_id: stable_id("change", &["diag-add", &d.code, d.subject.as_deref().unwrap_or("")]) });
        }
    }
    for (k, d) in &old_d {
        if !new_d.contains_key(k) {
            changes.push(Change { kind: ChangeKind::Removed, subject: d.subject.clone().unwrap_or_default(), detail: format!("diagnostic cleared [{}] {}", d.code, d.message), stable_id: stable_id("change", &["diag-del", &d.code, d.subject.as_deref().unwrap_or("")]) });
        }
    }

    changes.sort_by(|a, b| a.subject.cmp(&b.subject).then(a.kind.cmp(&b.kind)));
    ModelDiff { from_version: from.map(|m| m.version), to_version: to.version, changes }
}

fn nz(s: &str) -> &str { if s.is_empty() { "(unresolved)" } else { s } }
fn kind_label(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Object => "Object",
        NodeKind::Variable => "Variable",
        NodeKind::Method => "Method",
        NodeKind::ObjectType => "ObjectType",
        NodeKind::VariableType => "VariableType",
        NodeKind::ReferenceType => "ReferenceType",
        NodeKind::DataType => "DataType",
        NodeKind::View => "View",
    }
}
