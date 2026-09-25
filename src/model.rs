//! Raw layer model: everything from a NodeSet XML file is preserved.
//!
//! Nothing here is "normalized" across files: namespace indices are the
//! numbers written in *this* file, aliases are file-local strings, and
//! values keep their original textual representation. The derived layer
//! lives in `merge.rs`.

use std::collections::BTreeMap;

/// Byte range inside the source document, later mapped to 1-based
/// line/column so the UI can jump back to the exact XML span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// A value with a source range inside a named document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located<T> {
    pub value: T,
    pub doc_id: String,
    pub range: Range,
}

impl<T> Located<T> {
    pub fn new(value: T, doc_id: impl Into<String>, range: Range) -> Self {
        Self { value, doc_id: doc_id.into(), range }
    }
}

pub const NODE_KINDS: &[&str] = &[
    "UAObject",
    "UAObjectType",
    "UAVariable",
    "UAVariableType",
    "UAMethod",
    "UADataType",
    "UAReferenceType",
    "UAView",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required_model: Vec<RawRequiredModel>,
    pub attrs: BTreeMap<String, String>,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRequiredModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAlias {
    pub alias: String,
    pub node_id_text: String,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawValue {
    /// Original textual representation, exactly as imported (decoded entities).
    pub text: String,
    /// Raw bytes span of the <Value> element in the source document.
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNode {
    pub id: String,
    pub kind: String,
    pub node_id: String,
    pub browse_name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub write_mask: Option<String>,
    pub user_write_mask: Option<String>,
    pub value: Option<RawValue>,
    pub data_type: Option<String>,
    pub value_rank: Option<String>,
    pub references: Vec<RawRef>,
    /// Fully qualified element names that were not understood, with raw text.
    pub unknown: Vec<RawUnknown>,
    /// All attributes on the element (name -> value), source locations kept.
    pub attrs: BTreeMap<String, String>,
    pub attr_ranges: BTreeMap<String, Range>,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRef {
    pub reference_type: String,
    pub is_forward: bool,
    pub target_text: String,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawUnknown {
    pub path: String,
    pub text: String,
    pub attrs: BTreeMap<String, String>,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNodeSet {
    pub doc_id: String,
    pub filename: String,
    /// Bytes of the file, kept so source spans always resolve.
    pub bytes: Vec<u8>,
    /// Entry i is the namespace bound to local index i. Index 0 is the
    /// implicit OPC UA namespace.
    pub namespace_uris: Vec<Located<String>>,
    pub models: Vec<RawModel>,
    pub aliases: Vec<RawAlias>,
    pub nodes: Vec<RawNode>,
    /// Extensions under UANodeSet that belong to none of the known sections.
    pub unknown: Vec<RawUnknown>,
}

impl RawNodeSet {
    pub fn text(&self, range: Range) -> String {
        let end = range.end.min(self.bytes.len());
        match std::str::from_utf8(&self.bytes[range.start..end]) {
            Ok(s) => s.to_string(),
            Err(_) => String::from_utf8_lossy(&self.bytes[range.start..end]).into_owned(),
        }
    }

    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        let mut line = 1usize;
        let mut col = 1usize;
        for (i, b) in self.bytes.iter().enumerate().take(pos.min(self.bytes.len())) {
            if *b == b'\n' {
                line += 1;
                col = 1;
            } else if i + 1 <= pos {
                col += 1;
            }
        }
        (line, col)
    }
}

/// Parse a textual NodeId `ns=n;i=v | s= | g= | b=` or an alias text that
/// could not be resolved. None means the text is not a well-formed NodeId.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNodeId<'a> {
    pub ns: u16,
    pub id_type: char,
    pub id: &'a str,
}

pub fn parse_node_id(text: &str) -> Option<ParsedNodeId<'_>> {
    let t = text.trim();
    let rest = t.strip_prefix("ns=")?;
    let split = rest.find(';')?;
    let ns: u16 = rest[..split].parse().ok()?;
    let tail = &rest[split + 1..];
    let (marker, id) = if let Some(id) = tail.strip_prefix("i=") {
        ('i', id)
    } else if let Some(id) = tail.strip_prefix("s=") {
        ('s', id)
    } else if let Some(id) = tail.strip_prefix("g=") {
        ('g', id)
    } else if let Some(id) = tail.strip_prefix("b=") {
        ('b', id)
    } else {
        return None;
    };
    if id.is_empty() {
        return None;
    }
    Some(ParsedNodeId { ns, id_type: marker, id })
}

/// BrowseNames are `ns:Name` (prefix omitted when ns=0).
pub fn split_browse_name(text: &str) -> (u16, &str) {
    match text.split_once(':') {
        Some((n, name)) => (n.parse().unwrap_or(0), name),
        None => (0, text),
    }
}
