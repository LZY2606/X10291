//! Stable content-addressed identifiers.
//!
//! Every raw object, derived node, edge and event gets an identifier that
//! is derived purely from its content. Re-importing the same fixture after
//! deleting the local database therefore yields identical ids, identical
//! ordering and identical provenance links.

use sha2::{Digest, Sha256};

pub fn short_hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update([0x1f]);
        }
        h.update(p.as_bytes());
    }
    let d = h.finalize();
    hex12(&d)
}

pub fn hex12(d: &[u8]) -> String {
    let mut s = String::with_capacity(12);
    for b in d.iter().take(6) {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// File id is stable across exports/imports: based on declared logical path.
pub fn file_id(path: &str) -> String {
    format!("file-{}", short_hash(&[path]))
}

pub fn raw_node_id(file_path: &str, node_id_attr: &str) -> String {
    format!("rawnode-{}", short_hash(&[file_path, node_id_attr]))
}

pub fn raw_ref_id(file_path: &str, node_id_attr: &str, ordinal: usize, ref_xml: &str) -> String {
    format!(
        "rawref-{}",
        short_hash(&[file_path, node_id_attr, &ordinal.to_string(), ref_xml])
    )
}

pub fn alias_id(file_path: &str, name: &str) -> String {
    format!("alias-{}", short_hash(&[file_path, name]))
}

pub fn model_id(uri: &str, version: &str) -> String {
    format!("model-{}", short_hash(&[uri, version]))
}

/// Canonical global node id: merged namespace uri + literal id.
pub fn global_node_id(ns_uri: &str, literal: &str) -> String {
    format!("g-{}", short_hash(&[ns_uri, literal]))
}

pub fn edge_id(source: &str, rel: &str, target: &str, role: &str) -> String {
    format!("edge-{}", short_hash(&[source, rel, target, role]))
}

pub fn event_id(version_id: &str, seq: u64, kind: &str, subject: &str) -> String {
    format!(
        "evt-{}",
        short_hash(&[version_id, &seq.to_string(), kind, subject])
    )
}

pub fn fingerprint(parts: &[String]) -> String {
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    short_hash(&refs)
}
