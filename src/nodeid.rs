//! OPC UA NodeId parsing and canonical rendering.
//!
//! A numeric namespace index is only meaningful inside one file. We keep
//! the raw `NodeId` attribute verbatim and translate it into a
//! `(namespace uri, literal id)` pair using the owning file's namespace
//! table; merging across files is always performed on the URI pair.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedNodeId {
    pub ns_index: u16,
    pub id_kind: char, // 'i' | 's' | 'g' | 'b' | '?'
    pub literal: String,
    pub raw: String,
}

#[derive(Debug, Clone)]
pub struct NodeIdError {
    pub raw: String,
    pub reason: String,
}

pub fn parse(raw: &str) -> Result<ParsedNodeId, NodeIdError> {
    let mut ns_index: u16 = 0;
    let mut id_kind = '?';
    let mut literal: Option<String> = None;
    for part in raw.split(';') {
        if part.is_empty() {
            continue;
        }
        let (k, v) = part.split_once('=').ok_or_else(|| NodeIdError {
            raw: raw.to_string(),
            reason: format!("segment '{}' is not key=value", part),
        })?;
        match k.trim() {
            "ns" => {
                ns_index = v.trim().parse().map_err(|_| NodeIdError {
                    raw: raw.to_string(),
                    reason: format!("invalid namespace index '{}'", v),
                })?;
            }
            k @ ("i" | "s" | "g" | "b") => {
                if id_kind != '?' {
                    return Err(NodeIdError {
                        raw: raw.to_string(),
                        reason: "multiple identifier parts".into(),
                    });
                }
                id_kind = k.chars().next().unwrap();
                literal = Some(v.to_string());
            }
            "svr" => {
                return Err(NodeIdError {
                    raw: raw.to_string(),
                    reason: "server-local NodeIds (svr) cannot be merged".into(),
                });
            }
            other => {
                return Err(NodeIdError {
                    raw: raw.to_string(),
                    reason: format!("unknown NodeId field '{}'", other),
                });
            }
        }
    }
    let literal = literal.ok_or_else(|| NodeIdError {
        raw: raw.to_string(),
        reason: "missing i/s/g/b identifier".into(),
    })?;
    Ok(ParsedNodeId {
        ns_index,
        id_kind,
        literal,
        raw: raw.to_string(),
    })
}

/// Render using the file-local numeric index (preserves single-file meaning).
pub fn render_local(p: &ParsedNodeId) -> String {
    format!("ns={};{}={}", p.ns_index, p.id_kind, p.literal)
}

/// Render using a namespace URI, independent of any file-local index.
pub fn render_uri(uri: &str, p: &ParsedNodeId) -> String {
    format!("uri={};{}={}", uri, p.id_kind, p.literal)
}
