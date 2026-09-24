//! Merge engine: combines multiple file-local NodeSet views into one model.
//!
//! Guarded semantics (enforced + asserted in tests):
//! - Numeric ns indexes are per-file; identity is remapped via namespace URI.
//! - Inverse/forward refs are canonicalized but no missing edge is fabricated.
//! - Missing dependencies survive as unresolved placeholder nodes.
//! - Multi-candidate/uncertain states are kept, never collapsed to one answer.

use crate::model::*;
use std::collections::{BTreeMap, BTreeSet};

pub const BASE_NS_URI: &str = "http://opcfoundation.org/UA/";

pub fn uri_for_index(doc: &NodeSetDoc, idx: u16) -> Option<String> {
    if idx == 0 {
        Some(BASE_NS_URI.to_string())
    } else if (idx as usize) <= doc.namespace_uris.len() {
        Some(doc.namespace_uris[(idx - 1) as usize].clone())
    } else {
        None
    }
}

/// File-local raw NodeId -> ExpandedNodeId via the namespace URI table.
/// Returns None when the numeric index cannot be mapped.
pub fn expand_node_id(doc: &NodeSetDoc, raw: &str) -> Option<ExpandedNodeId> {
    let id = NodeId::parse(raw)?;
    let uri = uri_for_index(doc, id.ns)?;
    Some(ExpandedNodeId {
        ns_uri: uri,
        kind: id.kind,
        ident: id.ident,
    })
}

/// Resolve an alias reference. Returns Some(final target) when resolution
/// terminates; None when it hits a cycle or unknown alias. The visited path
/// is returned so the caller can emit precise diagnostics.
pub fn resolve_alias(doc: &NodeSetDoc, start: &str) -> (Option<String>, Vec<String>) {
    let mut path: Vec<String> = vec![];
    let mut current = start.to_string();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for _ in 0..=doc.aliases.len() + 1 {
        if !seen.insert(current.clone()) {
            path.push(current);
            return (None, path);
        }
        path.push(current.clone());
        let target = doc
            .aliases
            .iter()
            .find(|a| a.name == current)
            .map(|a| a.target_raw.trim().to_string());
        match target {
            None => return if path.len() == 1 { (None, path) } else { (None, path) },
            Some(t) if looks_like_nodeid(&t) => return (Some(t), path),
            Some(t) => current = t,
        }
    }
    (None, path)
}

pub fn looks_like_nodeid(s: &str) -> bool {
    let t = s.trim();
    t.contains('=') && (t.contains(';') || t.starts_with("i=") || t.starts_with("s=") || t.starts_with("g=") || t.starts_with("b="))
}

/// Resolve any raw reference (NodeId or alias chain) within a file.
pub fn resolve_ref_token(doc: &NodeSetDoc, raw: &str) -> (Option<ExpandedNodeId>, Option<String>, Vec<String>) {
    let raw = raw.trim();
    if looks_like_nodeid(raw) {
        (expand_node_id(doc, raw), None, vec![raw.to_string()])
    } else {
        let (resolved, path) = resolve_alias(doc, raw);
        match resolved {
            Some(nodeid) => (
                expand_node_id(doc, &nodeid),
                Some(nodeid),
                path,
            ),
            None => (None, None, path),
        }
    }
}
