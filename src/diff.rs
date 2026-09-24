//! Semantic diff between two derived model versions.
//!
//! Each entry is keyed by stable content ids so a reviewer can trace a
//! difference to the raw material, the rule set version and the decision
//! event that produced it.

use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DiffEntry {
    pub kind: String, // node_added | node_removed | node_changed | edge_added | ...
    pub subject: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub before_id: Option<String>,
    pub after_id: Option<String>,
    pub evidence_before: Vec<SpanRef>,
    pub evidence_after: Vec<SpanRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelDiff {
    pub left_branch: String,
    pub right_branch: String,
    pub entries: Vec<DiffEntry>,
}

fn node_label(n: &DerivedNode) -> String {
    format!(
        "{} [{}] browse={} status={}",
        n.canonical_node_id,
        n.kind,
        n.browse_name.as_deref().unwrap_or("-"),
        n.status
    )
}

pub fn diff(left: &DerivedModel, right: &DerivedModel) -> ModelDiff {
    let mut entries = Vec::new();

    let ln: std::collections::BTreeMap<&str, &DerivedNode> =
        left.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let rn: std::collections::BTreeMap<&str, &DerivedNode> =
        right.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    for (id, n) in &rn {
        match ln.get(id) {
            None => entries.push(DiffEntry {
                kind: "node_added".into(),
                subject: n.canonical_node_id.clone(),
                before: None,
                after: Some(node_label(n)),
                before_id: None,
                after_id: Some(n.id.clone()),
                evidence_before: vec![],
                evidence_after: n.sources.iter().map(|s| SpanRef {
                    file_id: s.file_id.clone(),
                    path: s.path.clone(),
                    start: s.span.0,
                    end: s.span.1,
                    label: "node source".into(),
                }).collect(),
            }),
            Some(old) if old.fingerprint != n.fingerprint
                || old.status != n.status
                || old.winner != n.winner
                || old.browse_name != n.browse_name =>
            {
                entries.push(DiffEntry {
                    kind: "node_changed".into(),
                    subject: n.canonical_node_id.clone(),
                    before: Some(node_label(old)),
                    after: Some(node_label(n)),
                    before_id: Some(old.id.clone()),
                    after_id: Some(n.id.clone()),
                    evidence_before: old.sources.iter().map(|s| SpanRef {
                        file_id: s.file_id.clone(),
                        path: s.path.clone(),
                        start: s.span.0,
                        end: s.span.1,
                        label: "before source".into(),
                    }).collect(),
                    evidence_after: n.sources.iter().map(|s| SpanRef {
                        file_id: s.file_id.clone(),
                        path: s.path.clone(),
                        start: s.span.0,
                        end: s.span.1,
                        label: "after source".into(),
                    }).collect(),
                });
            }
            Some(_) => {}
        }
    }
    for (id, n) in &ln {
        if !rn.contains_key(id) {
            entries.push(DiffEntry {
                kind: "node_removed".into(),
                subject: n.canonical_node_id.clone(),
                before: Some(node_label(n)),
                after: None,
                before_id: Some(n.id.clone()),
                after_id: None,
                evidence_before: n.sources.iter().map(|s| SpanRef {
                    file_id: s.file_id.clone(),
                    path: s.path.clone(),
                    start: s.span.0,
                    end: s.span.1,
                    label: "removed node source".into(),
                }).collect(),
                evidence_after: vec![],
            });
        }
    }

    let le: std::collections::BTreeMap<&str, &Edge> =
        left.edges.iter().map(|e| (e.id.as_str(), e)).collect();
    let re: std::collections::BTreeMap<&str, &Edge> =
        right.edges.iter().map(|e| (e.id.as_str(), e)).collect();
    for (id, e) in &re {
        match le.get(id) {
            None => entries.push(DiffEntry {
                kind: "edge_added".into(),
                subject: format!("{} --{}--> {}", e.source, e.reference_type, e.target),
                before: None,
                after: Some(e.status.clone()),
                before_id: None,
                after_id: Some(e.id.clone()),
                evidence_before: vec![],
                evidence_after: e.provenance.iter().map(|p| SpanRef {
                    file_id: p.file_id.clone(),
                    path: p.path.clone(),
                    start: p.span.0,
                    end: p.span.1,
                    label: "edge source".into(),
                }).collect(),
            }),
            Some(old) if old.fingerprint != e.fingerprint || old.status != e.status => {
                entries.push(DiffEntry {
                    kind: "edge_changed".into(),
                    subject: format!("{} --{}--> {}", e.source, e.reference_type, e.target),
                    before: Some(old.status.clone()),
                    after: Some(e.status.clone()),
                    before_id: Some(old.id.clone()),
                    after_id: Some(e.id.clone()),
                    evidence_before: old.provenance.iter().map(|p| SpanRef {
                        file_id: p.file_id.clone(),
                        path: p.path.clone(),
                        start: p.span.0,
                        end: p.span.1,
                        label: "before".into(),
                    }).collect(),
                    evidence_after: e.provenance.iter().map(|p| SpanRef {
                        file_id: p.file_id.clone(),
                        path: p.path.clone(),
                        start: p.span.0,
                        end: p.span.1,
                        label: "after".into(),
                    }).collect(),
                });
            }
            Some(_) => {}
        }
    }
    for (id, e) in &le {
        if !re.contains_key(id) {
            entries.push(DiffEntry {
                kind: "edge_removed".into(),
                subject: format!("{} --{}--> {}", e.source, e.reference_type, e.target),
                before: Some(e.status.clone()),
                after: None,
                before_id: Some(e.id.clone()),
                after_id: None,
                evidence_before: e.provenance.iter().map(|p| SpanRef {
                    file_id: p.file_id.clone(),
                    path: p.path.clone(),
                    start: p.span.0,
                    end: p.span.1,
                    label: "removed edge".into(),
                }).collect(),
                evidence_after: vec![],
            });
        }
    }

    // unresolved delta
    for u in &right.unresolved {
        if !left.unresolved.iter().any(|x| x.id == u.id) {
            entries.push(DiffEntry {
                kind: "unresolved_added".into(),
                subject: format!("{}: {}", u.kind, u.subject),
                before: None,
                after: Some(u.reason.clone()),
                before_id: None,
                after_id: Some(u.id.clone()),
                evidence_before: vec![],
                evidence_after: u.evidence.clone(),
            });
        }
    }
    for u in &left.unresolved {
        if !right.unresolved.iter().any(|x| x.id == u.id) {
            entries.push(DiffEntry {
                kind: "unresolved_resolved".into(),
                subject: format!("{}: {}", u.kind, u.subject),
                before: Some(u.reason.clone()),
                after: Some("resolved".into()),
                before_id: Some(u.id.clone()),
                after_id: None,
                evidence_before: u.evidence.clone(),
                evidence_after: vec![],
            });
        }
    }

    entries.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.subject.cmp(&b.subject)));
    ModelDiff {
        left_branch: left.branch.clone(),
        right_branch: right.branch.clone(),
        entries,
    }
}
