//! Semantic diff between two merged models. Entries reference stable ids so
//! results survive namespace reordering.

use crate::merge::MergedModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffEntry {
    pub kind: String,
    pub subject: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub detail: String,
}

pub fn diff_models(a: &MergedModel, b: &MergedModel) -> Vec<DiffEntry> {
    let mut out = Vec::new();

    // namespace table
    if a.namespaces != b.namespaces {
        out.push(DiffEntry {
            kind: "NamespaceTableChanged".into(),
            subject: "namespaces".into(),
            before: Some(a.namespaces.join(", ")),
            after: Some(b.namespaces.join(", ")),
            detail: "global namespace table differs (indices are per-file; URIs are identity)"
                .into(),
        });
    }

    let an: HashMap<_, _> = a.nodes.iter().map(|n| (&n.stable_id, n)).collect();
    let bn: HashMap<_, _> = b.nodes.iter().map(|n| (&n.stable_id, n)).collect();
    for (sid, nb) in &bn {
        match an.get(sid) {
            None => out.push(DiffEntry {
                kind: "NodeAdded".into(),
                subject: (*sid).clone(),
                before: None,
                after: Some(nb.browse_name_raw.clone()),
                detail: format!("{} ({:?}) added", nb.browse_name_text, nb.class),
            }),
            Some(na) => {
                if na.browse_name_raw != nb.browse_name_raw {
                    out.push(DiffEntry {
                        kind: "BrowseNameChanged".into(),
                        subject: (*sid).clone(),
                        before: Some(na.browse_name_raw.clone()),
                        after: Some(nb.browse_name_raw.clone()),
                        detail: "browse name changed".into(),
                    });
                }
                if na.display_name != nb.display_name {
                    out.push(DiffEntry {
                        kind: "DisplayNameChanged".into(),
                        subject: (*sid).clone(),
                        before: na.display_name.clone(),
                        after: nb.display_name.clone(),
                        detail: "display name changed".into(),
                    });
                }
                if na.extensions_raw != nb.extensions_raw {
                    out.push(DiffEntry {
                        kind: "ExtensionsChanged".into(),
                        subject: (*sid).clone(),
                        before: Some(na.extensions_raw.join(" | ")),
                        after: Some(nb.extensions_raw.join(" | ")),
                        detail: "extension payload changed (raw values preserved)".into(),
                    });
                }
                if na.unresolved != nb.unresolved {
                    out.push(DiffEntry {
                        kind: "ResolutionChanged".into(),
                        subject: (*sid).clone(),
                        before: Some(na.unresolved.to_string()),
                        after: Some(nb.unresolved.to_string()),
                        detail: "unresolved flag changed".into(),
                    });
                }
            }
        }
    }
    for (sid, na) in &an {
        if !bn.contains_key(sid) {
            out.push(DiffEntry {
                kind: "NodeRemoved".into(),
                subject: (*sid).clone(),
                before: Some(na.browse_name_raw.clone()),
                after: None,
                detail: format!("{} removed", na.browse_name_text),
            });
        }
    }

    let edge = |r: &crate::merge::MergedRef| {
        format!("{} -[{}]-> {}", r.from, r.ref_type, r.to)
    };
    let ae: HashMap<String, _> = a.refs.iter().map(|r| (edge(r), r)).collect();
    let be: HashMap<String, _> = b.refs.iter().map(|r| (edge(r), r)).collect();
    for (k, r) in &be {
        if !ae.contains_key(k) {
            out.push(DiffEntry {
                kind: "ReferenceAdded".into(),
                subject: r.from.clone(),
                before: None,
                after: Some(k.clone()),
                detail: format!("declared in {}", r.declared_in),
            });
        }
    }
    for (k, r) in &ae {
        if !be.contains_key(k) {
            out.push(DiffEntry {
                kind: "ReferenceRemoved".into(),
                subject: r.from.clone(),
                before: Some(k.clone()),
                after: None,
                detail: format!("was declared in {}", r.declared_in),
            });
        }
    }

    let aun: std::collections::HashSet<_> =
        a.unresolved_deps.iter().map(|d| (&d.model_uri, &d.required_by)).collect();
    let bun: std::collections::HashSet<_> =
        b.unresolved_deps.iter().map(|d| (&d.model_uri, &d.required_by)).collect();
    for d in bun.difference(&aun) {
        out.push(DiffEntry {
            kind: "UnresolvedDependencyAdded".into(),
            subject: format!("nsu={};s=model", d.0),
            before: None,
            after: Some(format!("required by {}", d.1)),
            detail: "dependency missing; unresolved node kept".into(),
        });
    }
    for d in aun.difference(&bun) {
        out.push(DiffEntry {
            kind: "UnresolvedDependencyResolved".into(),
            subject: format!("nsu={};s=model", d.0),
            before: Some(format!("required by {}", d.1)),
            after: None,
            detail: "dependency now provided".into(),
        });
    }

    out.sort_by(|x, y| (&x.kind, &x.subject).cmp(&(&y.kind, &y.subject)));
    out
}
