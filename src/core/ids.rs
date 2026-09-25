//! Stable identity construction.
//!
//! Semantic rule encoded here:
//!   * the numeric namespace index of a NodeId is only meaningful inside one
//!     file;
//!   * across files a node is keyed by (namespace **URI**, local id), never
//!     by the raw number;
//!   * ids are percent-escaped so they survive JSON and URLs unchanged.

use crate::core::hash::hash_parts;

pub fn pct_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub fn pct_unescape(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16)?;
                let lo = (bytes[i + 2] as char).to_digit(16)?;
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            b'%' => return None,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalNodeId {
    /// Raw namespace index as written in the file (0 if absent).
    pub file_ns: u32,
    /// Identifier type tag as written: ``i``, ``s``, ``g`` or ``b``.
    pub kind: char,
    /// Identifier part exactly as written (trimmed).
    pub local: String,
}

/// Parse ``ns=2;s=Pump101`` / ``i=58`` / ``nsu=...;s=x``.
pub fn parse_node_id(raw: &str) -> Option<LocalNodeId> {
    let mut file_ns: u32 = 0;
    let mut kind: Option<char> = None;
    let mut local: Option<String> = None;
    for part in raw.split(';') {
        let (k, v) = part.split_once('=')?;
        match k.trim() {
            "ns" => file_ns = v.trim().parse().ok()?,
            "nsu" => {
                // nsu= form: we treat the URI as local component; cross-file
                // resolution happens via the namespace table key instead.
                local = Some(v.trim().to_string());
                kind = Some('u');
            }
            idk @ ("i" | "s" | "g" | "b") => {
                kind = Some(idk.chars().next().unwrap());
                local = Some(v.trim().to_string());
            }
            _ => {}
        }
    }
    Some(LocalNodeId {
        file_ns,
        kind: kind?,
        local: local?,
    })
}

/// Global, file-independent key for a node: ``uri :: idkind:id``.
pub fn global_key(namespace_uri: &str, local: &LocalNodeId) -> String {
    format!(
        "{}::{}:{}",
        pct_escape(namespace_uri),
        local.kind,
        pct_escape(&local.local)
    )
}

/// Stable id of a source file record (duplicates still get distinct ids).
pub fn record_id(file_id: &str, node_id: &str, ordinal: usize) -> String {
    format!(
        "rec-{}",
        &hash_parts(&[file_id, node_id, &ordinal.to_string()])[..16]
    )
}

/// Stable id of a normalized edge (independent of which file declared it).
pub fn edge_id(
    src: &str,
    ref_type_global: &str,
    dst: &str,
    direction: &str,
) -> String {
    format!(
        "edge-{}",
        &hash_parts(&[src, ref_type_global, dst, direction])[..16]
    )
}

/// Stable id of an alias in a file.
pub fn alias_id(file_id: &str, name: &str, ordinal: usize) -> String {
    format!(
        "alias-{}",
        &hash_parts(&[file_id, name, &ordinal.to_string()])[..16]
    )
}

/// Human-readable node label derived from a global key.
pub fn display_global(key: &str) -> String {
    match key.split_once("::") {
        Some((_uri, rest)) => rest.to_string(),
        None => key.to_string(),
    }
}

/// Parse a BrowseName of the form ``2:Pump``.
pub fn parse_browse_name(raw: &str) -> Option<(u32, String)> {
    let (ns, name) = raw.split_once(':')?;
    Some((ns.trim().parse().ok()?, name.trim().to_string()))
}
