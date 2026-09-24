//! Raw, lossless domain model. Nothing here is normalized across files:
//! numeric namespace indexes stay exactly as written in the source XML.

use serde::{Deserialize, Serialize};

/// Byte range into the original source file, 0-based half-open [start,end).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// A NodeId exactly as written, before any cross-file remapping.
/// `ns` is the *numeric* index and is only meaningful inside one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeId {
    pub ns: u16,
    pub identifier: String,
    #[serde(rename = "idType")]
    pub id_type: String,
    pub text: String,
    pub span: Option<Span>,
}

/// BrowseName "prefix:LocalText" kept verbatim plus split parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawBrowseName {
    pub text: String,
    pub prefix: Option<String>,
    pub local: String,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceUri {
    pub index: u16,
    pub uri: String,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    pub name: String,
    pub target: RawNodeId,
    pub span: Option<Span>,
}

/// A declared `<Model>` / RequiredModel dependency (model dependency).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDependency {
    pub uri: String,
    pub version: String,
    pub publication_date: String,
    pub required: bool,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAttribute {
    pub name: String,
    pub value: String,
    pub span: Option<Span>,
}

/// Engineering-unit / coordinate-basis information preserved verbatim.
/// Conversion, if any, happens only in the derived layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawValue {
    /// Verbatim XML text of the value.
    pub raw_xml: String,
    /// Best-effort scalar text (e.g. the contents of a simple `<uax:Double>`).
    pub scalar_text: Option<String>,
    /// XML Schema datatype when recognizable, e.g. `Double`.
    pub datatype: Option<String>,
    /// Display name of an EUInformation unit if present, else None.
    pub unit_display: Option<String>,
    /// Unit id (e.g. UNECE/OPC unit id) if present.
    pub unit_id: Option<i64>,
    /// Coordinate / engineering-basis namespace URI if present.
    pub coordinate_basis: Option<String>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    /// Reference type NodeId, alias-expanded *within the file only*.
    pub ref_type: RawNodeId,
    pub target: RawNodeId,
    pub is_forward: bool,
    /// True when the reference type was given as an alias in this file.
    pub via_alias: bool,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub tag: String,
    pub raw_xml: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub class: String,
    pub node_id: RawNodeId,
    pub browse_name: Option<RawBrowseName>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub references: Vec<RawReference>,
    pub attributes: Vec<RawAttribute>,
    pub value: Option<RawValue>,
    pub unknown_extensions: Vec<UnknownExtension>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedSource {
    /// Logical file label (path-like); stable identity uses content hash.
    pub filename: String,
    pub content: String,
    pub sha256: String,
    pub namespaces: Vec<NamespaceUri>,
    pub aliases: Vec<Alias>,
    pub models: Vec<ModelDependency>,
    pub nodes: Vec<RawNode>,
    /// Extensions found outside any node (document level), preserved.
    pub document_extensions: Vec<UnknownExtension>,
    pub parse_errors: Vec<ParseIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseIssue {
    pub code: String,
    pub message: String,
    pub span: Option<Span>,
}
