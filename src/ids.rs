//! Parsers for the OPC UA NodeId / ExpandedNodeId / QualifiedName textual forms.
//! Numeric namespace indices stay file-local here; URI resolution happens later.

use crate::domain::{NodeIdValue, QualifiedName, RawNodeId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdParseError {
    pub raw: String,
    pub detail: String,
}

pub fn parse_node_id(raw: &str) -> Result<RawNodeId, IdParseError> {
    let text = raw.trim();
    let mut ns: u16 = 0;
    let mut nsu: Option<String> = None;
    let mut sv: Option<String> = None;
    let mut id: Option<(&str, &str)> = None;
    for part in split_semicolons(text) {
        if let Some(rest) = part.strip_prefix("ns=") {
            ns = rest.parse::<u16>().map_err(|_| IdParseError {
                raw: raw.to_string(),
                detail: format!("ns index not u16: {rest}"),
            })?;
        } else if let Some(rest) = part.strip_prefix("nsu=") {
            nsu = Some(rest.to_string());
        } else if let Some(rest) = part.strip_prefix("sv=") {
            sv = Some(rest.to_string());
        } else if let Some(rest) = part.strip_prefix("i=") {
            id = Some(("i", rest));
        } else if let Some(rest) = part.strip_prefix("s=") {
            id = Some(("s", rest));
        } else if let Some(rest) = part.strip_prefix("g=") {
            id = Some(("g", rest));
        } else if let Some(rest) = part.strip_prefix("b=") {
            id = Some(("b", rest));
        } else if part.is_empty() {
        } else {
            return Err(IdParseError {
                raw: raw.to_string(),
                detail: format!("unrecognized token: {part}"),
            });
        }
    }
    let (kind, value) = id.ok_or_else(|| IdParseError {
        raw: raw.to_string(),
        detail: "missing identifier (i=/s=/g=/b=)".to_string(),
    })?;
    let value = match kind {
        "i" => NodeIdValue::Numeric(value.parse::<i64>().map_err(|_| IdParseError {
            raw: raw.to_string(),
            detail: format!("numeric id not i64: {value}"),
        })?),
        "s" => NodeIdValue::String(value.to_string()),
        "g" => NodeIdValue::Guid(value.to_string()),
        _ => NodeIdValue::ByteString(value.to_string()),
    };
    if nsu.is_some() || sv.is_some() {
        // ExpandedNodeId in a reference slot is not remappable as a plain NodeId.
        return Err(IdParseError {
            raw: raw.to_string(),
            detail: "expanded node id (nsu=/sv=) cannot be used as local NodeId".to_string(),
        });
    }
    Ok(RawNodeId {
        ns,
        id: value,
        raw: raw.to_string(),
    })
}

/// Split on ';' but keep the common GUID semicolon forms working (GUIDs use
/// hyphens, so a plain split is sufficient for NodeId syntax).
fn split_semicolons(text: &str) -> Vec<&str> {
    text.split(';').collect()
}

pub fn parse_qualified_name(raw: &str) -> Result<QualifiedName, IdParseError> {
    let text = raw.trim();
    let (ns, name) = match text.split_once(':') {
        Some((left, right)) => {
            let n = left.parse::<u16>().map_err(|_| IdParseError {
                raw: raw.to_string(),
                detail: format!("browse name prefix not u16: {left}"),
            })?;
            (n, right)
        }
        None => (0u16, text),
    };
    Ok(QualifiedName {
        ns,
        name: name.to_string(),
        raw: raw.to_string(),
    })
}

pub fn value_as_string(v: &NodeIdValue) -> String {
    match v {
        NodeIdValue::Numeric(n) => n.to_string(),
        NodeIdValue::String(s) => s.clone(),
        NodeIdValue::Guid(g) => g.clone(),
        NodeIdValue::ByteString(b) => b.clone(),
    }
}

pub fn value_kind(v: &NodeIdValue) -> &'static str {
    match v {
        NodeIdValue::Numeric(_) => "i",
        NodeIdValue::String(_) => "s",
        NodeIdValue::Guid(_) => "g",
        NodeIdValue::ByteString(_) => "b",
    }
}
