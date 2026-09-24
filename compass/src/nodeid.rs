//! File-local NodeId and BrowseName parsing.
//!
//! Numeric namespace indices are only meaningful inside a single file; this
//! module never promotes an index across files. URI remapping lives in merge.

use crate::model::Diagnostic;
use crate::parser::UA_NS;

/// Identifier part of a NodeId, normalized but not remapped.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Identifier {
    Numeric(u64),
    String_(String),
    Guid(String),
    ByteString(String),
}

impl Identifier {
    pub fn key(&self) -> String {
        match self {
            Identifier::Numeric(n) => format!("i={n}"),
            Identifier::String_(s) => format!("s={s}"),
            Identifier::Guid(g) => format!("g={g}"),
            Identifier::ByteString(b) => format!("b={b}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalNodeId {
    pub ns: u16,
    pub id: Identifier,
    /// Original textual representation, preserved verbatim.
    pub raw: String,
}

/// Parse forms like `i=58`, `ns=2;s=Pump`, `s=AliasName`, `g=...`.
/// Alias names are accepted at the raw level; resolution happens in merge.
pub fn parse_node_id(text: &str) -> Result<LocalNodeId, String> {
    let raw = text.to_string();
    let t = text.trim();
    if t.is_empty() {
        return Err("empty NodeId".into());
    }
    // Plain alias token (no '='), e.g. `HasComponent`.
    if !t.contains('=') {
        return Ok(LocalNodeId {
            ns: 0,
            id: Identifier::String_(t.to_string()),
            raw,
        });
    }
    let (ns_part, id_part) = match t.split_once(';') {
        Some((n, r)) => (n, r),
        None => ("", t),
    };
    let mut ns: u16 = 0;
    let body = if let Some(v) = ns_part.strip_prefix("ns=") {
        ns = v.parse().map_err(|_| format!("bad namespace index `{v}`"))?;
        id_part
    } else {
        t
    };
    let id = if let Some(v) = body.strip_prefix("i=") {
        Identifier::Numeric(v.parse().map_err(|_| format!("bad numeric id `{v}`"))?)
    } else if let Some(v) = body.strip_prefix("s=") {
        Identifier::String_(v.to_string())
    } else if let Some(v) = body.strip_prefix("g=") {
        Identifier::Guid(v.to_string())
    } else if let Some(v) = body.strip_prefix("b=") {
        Identifier::ByteString(v.to_string())
    } else {
        return Err(format!("unrecognized NodeId `{text}`"));
    };
    Ok(LocalNodeId { ns, id, raw })
}

/// Parse a BrowseName `ns=N;Name` or plain `Name` (ns=0).
pub fn parse_browse_name(text: &str) -> Result<(u16, String), String> {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("ns=") {
        let (n, name) = rest.split_once(';').ok_or("BrowseName missing name")?;
        Ok((n.parse().map_err(|_| "bad BrowseName ns")?, name.to_string()))
    } else {
        Ok((0, t.to_string()))
    }
}

/// Resolve a file-local namespace index to the URI written in that file's
/// namespace table. Index 0 is always the core OPC UA namespace.
pub fn uri_for_index(index: u16, table: &[String]) -> Result<String, Diagnostic> {
    if index == 0 {
        return Ok(UA_NS.to_string());
    }
    let pos = (index as usize) - 1;
    table.get(pos).cloned().ok_or_else(|| Diagnostic {
        code: "NS_INDEX_UNDECLARED".into(),
        message: format!("file-local namespace index {index} has no Uri entry"),
        span: None,
    })
}
