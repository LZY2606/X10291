//! Raw and derived domain model.
//!
//! The raw layer preserves the original XML representation: namespaces,
//! NodeIds, BrowseNames, references, aliases, model dependencies and
//! unrecognized extensions are all retained with source byte spans.
//! Normalization only happens in the derived layer.

use serde::{Deserialize, Serialize};

/// Inclusive byte range inside one source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub source_id: String,
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(source_id: impl Into<String>, start: usize, end: usize) -> Self {
        Self { source_id: source_id.into(), start, end }
    }
}

/// One entry of a file-local `<NamespaceUris><Uri>` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNamespace {
    /// File-local index as written (1-based; 0 is always OPC UA).
    pub local_index: u16,
    pub uri: String,
    pub span: Span,
}

/// Alias name -> raw NodeId text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAlias {
    pub name: String,
    pub node_id_text: String,
    pub span: Span,
}

/// Raw reference element as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRef {
    pub reference_type: String,
    pub target_text: String,
    pub is_forward: bool,
    pub span: Span,
}

/// A value carrying its original representation plus unit/coordinate basis.
/// The raw text is never dropped; canonical normalization lives in `canonical`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawQuantity {
    pub value: String,
    /// Unit as written (e.g. `bar`, `hPa`, `deg`).
    pub unit: Option<String>,
    /// Coordinate reference frame as written (e.g. `WGS84`, `PlantSiteFrame`).
    pub coordinate_frame: Option<String>,
    pub span: Span,
    /// Filled only when a safe conversion exists.
    pub canonical_value: Option<String>,
    pub canonical_unit: Option<String>,
    /// Diagnostic id when conversion was impossible; raw value stays in place.
    pub diagnostic: Option<String>,
}

/// Retained unrecognized extension subtree (raw XML text, verbatim).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub tag: String,
    pub xml: String,
    pub span: Span,
}

/// Declared model dependency (`<RequiredModel>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDependency {
    pub model_uri: String,
    pub version: String,
    pub publication_date: Option<String>,
    pub span: Span,
}

/// Parsed-but-raw node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    /// Element tag: UAObject, UAVariable, UAObjectType, UAVariableType, UAReferenceType ...
    pub kind: String,
    pub node_id_text: String,
    pub browse_name_text: Option<String>,
    pub display_name: Option<String>,
    pub references: Vec<RawRef>,
    pub quantities: Vec<RawQuantity>,
    pub extensions: Vec<UnknownExtension>,
    pub is_abstract: Option<bool>,
    pub span: Span,
}

/// One NodeSet file kept exactly as imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeSet {
    pub source_id: String,
    pub path: String,
    pub bytes: String,
    pub last_modified: Option<String>,
    pub namespaces: Vec<RawNamespace>,
    pub aliases: Vec<RawAlias>,
    pub dependencies: Vec<ModelDependency>,
    pub nodes: Vec<RawNode>,
    pub file_extensions: Vec<UnknownExtension>,
}

/// Parse result of an imported batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RawModel {
    pub sources: Vec<RawNodeSet>,
    /// Parse-level diagnostics (malformed XML etc.), never blocking retention.
    pub parse_diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub span: Option<Span>,
}

/// Normalized BrowseName: (namespace URI, name text) — the numeric index is
/// resolved at merge time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BrowseName {
    pub ns_uri: String,
    pub name: String,
}

/// Fully-qualified, file-independent node key keyed by namespace URI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GlobalNodeId {
    pub ns_uri: String,
    /// Identifier as written (`i=44`, `s=...`, `g=...`, `b=...`).
    pub id: String,
}

impl GlobalNodeId {
    pub fn display(&self) -> String {
        format!("{}|{}", self.ns_uri, self.id)
    }
}
