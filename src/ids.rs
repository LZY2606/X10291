//! NodeId / QualifiedName parsing and stable global identities.
//!
//! Numeric namespace indices are only meaningful inside one file. A raw
//! [`NodeIdRef`] is therefore always resolved against that file's namespace
//! table to produce a [`Gid`] keyed by namespace URI; merges compare only Gids.

pub const UA_NS: &str = "http://opcfoundation.org/UA/";

/// Identifier type inside a NodeId.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IdValue {
    Numeric(u64),
    String(String),
    Guid(String),
    Opaque(String),
}

/// A NodeId exactly as authored inside one file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeIdRef {
    pub ns: u32,
    pub value: IdValue,
    pub raw: String,
}

impl std::fmt::Display for NodeIdRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Parse `i=...`, `s=...`, `g=...`, `b=...` with optional `ns=<n>;`.
pub fn parse_node_id(s: &str) -> Option<NodeIdRef> {
    let s = s.trim();
    let mut rest = s;
    let mut ns = 0u32;
    if let Some(idx) = s.find("ns=") {
        // ns= must appear before the first ';', otherwise it is part of s=...
        if !s[..idx].contains(';') {
            let after = &s[idx + 3..];
            let semi = after.find(';')?;
            ns = after[..semi].parse().ok()?;
            rest = &after[semi + 1..];
        }
    }
    let (tag, val) = rest.split_once('=')?;
    let value = match tag.trim() {
        "i" => IdValue::Numeric(val.trim().parse().ok()?),
        "s" => IdValue::String(val.trim().to_string()),
        "g" => IdValue::Guid(val.trim().to_string()),
        "b" => IdValue::Opaque(val.trim().to_string()),
        _ => return None,
    };
    Some(NodeIdRef { ns, value, raw: s.to_string() })
}

/// Global, URI-keyed node identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Gid {
    pub ns_uri: String,
    pub value: String,
    pub kind: String,
}

impl Gid {
    pub fn new(ns_uri: impl Into<String>, value: IdValue) -> Self {
        let (kind, value) = match value {
            IdValue::Numeric(n) => ("i", n.to_string()),
            IdValue::String(s) => ("s", s),
            IdValue::Guid(s) => ("g", s),
            IdValue::Opaque(s) => ("b", s),
        };
        Gid { ns_uri: ns_uri.into(), value, kind: kind.to_string() }
    }
    pub fn as_key(&self) -> String {
        format!("{}|{}={}", self.ns_uri, self.kind, self.value)
    }
    /// Human label used in the UI when space is limited.
    pub fn label(&self) -> String {
        if self.ns_uri == UA_NS {
            format!("i={}", self.value)
        } else {
            format!("{}={}", self.kind, self.value)
        }
    }
}

/// Resolve an authored NodeId against one file's namespace table.
/// `ns_table[0]` is always the OPC UA namespace.
pub fn resolve_gid(nid: &NodeIdRef, ns_table: &[String]) -> Option<Gid> {
    ns_table.get(nid.ns as usize).map(|uri| Gid::new(uri.clone(), nid.value.clone()))
}

/// Qualified name as authored in `BrowseName` (`0:Name` or plain `Name`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Qn {
    pub ns: u32,
    pub name: String,
}

pub fn parse_qname(s: &str) -> Qn {
    let s = s.trim();
    if let Some((idx, name)) = s.split_once(':') {
        if let Ok(n) = idx.parse::<u32>() {
            return Qn { ns: n, name: name.to_string() };
        }
    }
    Qn { ns: 0, name: s.to_string() }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Bnid {
    pub ns_uri: String,
    pub name: String,
}

pub fn resolve_browse(qn: &Qn, ns_table: &[String]) -> Bnid {
    let uri = ns_table
        .get(qn.ns as usize)
        .cloned()
        .unwrap_or_else(|| format!("__unresolved_ns#{}", qn.ns));
    Bnid { ns_uri: uri, name: qn.name.clone() }
}

/// 64-bit FNV-1a for compact stable identity suffixes.
pub fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Short stable id derived only from a key string; survives export/import.
pub fn stable_id(prefix: &str, key: &str) -> String {
    format!("{}:{:016x}", prefix, fnv1a64(key))
}
