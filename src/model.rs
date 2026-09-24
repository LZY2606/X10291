//! Core identifiers: NodeId parsing and URI-qualified stable identity.
//!
//! Semantic rule (1): a numeric namespace index is only meaningful inside the
//! single file that declares it. Stable identity therefore uses the namespace
//! URI, never the index.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdKind {
    Numeric(u64),
    String(String),
    Guid(String),
    Opaque(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GNodeId {
    pub ns: u16,
    pub id: IdKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct XNodeId {
    pub ns_uri: String,
    pub id: IdKind,
}

impl IdKind {
    pub fn suffix(&self) -> String {
        match self {
            IdKind::Numeric(n) => format!("i={n}"),
            IdKind::String(s) => format!("s={s}"),
            IdKind::Guid(g) => format!("g={g}"),
            IdKind::Opaque(b) => format!("b={b}"),
        }
    }
}

/// Parse a raw NodeId token such as `ns=2;i=1001`, `i=45`, `ns=1;s=Foo`.
/// ns defaults to 0 (OPC UA core) when absent.
pub fn parse_node_id(raw: &str) -> Result<(u16, IdKind)> {
    let raw = raw.trim();
    let mut ns: u16 = 0;
    let mut id: Option<IdKind> = None;
    for part in raw.split(';').filter(|p| !p.is_empty()) {
        let (k, v) = part
            .split_once('=')
            .ok_or_else(|| anyhow!("bad NodeId part: {part}"))?;
        match k.trim() {
            "ns" => {
                ns = v
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("bad namespace index: {v}"))?
            }
            "i" => id = Some(IdKind::Numeric(v.trim().parse()?)),
            "s" => id = Some(IdKind::String(v.trim().to_string())),
            "g" => id = Some(IdKind::Guid(v.trim().to_string())),
            "b" => id = Some(IdKind::Opaque(v.trim().to_string())),
            other => return Err(anyhow!("unknown NodeId field {other}")),
        }
    }
    Ok((ns, id.ok_or_else(|| anyhow!("NodeId missing identifier: {raw}"))?))
}

/// Stable, file-independent identity: `nsu=<uri>;i=42`.
pub fn stable_id(ns_uri: &str, id: &IdKind) -> String {
    format!("nsu={};{}", ns_uri, id.suffix())
}

/// Parse BrowseName (`1:Foo`) into (local ns index, name text).
pub fn parse_browse_name(raw: &str) -> (u16, String) {
    match raw.split_once(':') {
        Some((ns, name)) => (ns.parse().unwrap_or(0), name.to_string()),
        None => (0, raw.to_string()),
    }
}
