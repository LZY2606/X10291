//! Parsed-but-not-yet-derived domain model.
//!
//! Everything here mirrors what is literally in one source XML file:
//! namespace table, models (dependencies), aliases, raw node records and
//! unknown extensions. No cross-file remapping happens at this layer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLoc {
    /// Stable id of the source file (content-addressed sha).
    pub file_id: String,
    pub path: String,
    pub span: Span,
    pub line: usize,
}

/// A declared ``Models/Model`` entry -> a model dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDecl {
    pub model_uri: String,
    pub version: String,
    pub publication_date: Option<String>,
    pub required_models: Vec<RequiredModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredModel {
    pub model_uri: String,
    pub version: String,
    pub publication_date: Option<String>,
}

/// One ``Alias`` entry. Duplicate alias names are preserved as separate
/// records so the derived layer can surface ambiguity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasRecord {
    pub name: String,
    pub target_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attr {
    /// Standard attributes: NodeId, BrowseName, IsAbstract, NodeClass...
    Plain(String),
    /// ``DisplayName`` carries lang metadata; text is kept.
    DisplayName(String),
}

/// A child element the parser recognizes structurally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnownChild {
    /// ``<Reference ReferenceType=".." [IsForward="false"]>target</Reference>``
    Reference {
        reference_type: String,
        is_forward: bool,
        target: String,
    },
    /// ``<Definition FieldName=.. DataType=.. ValueRank=../>``
    Definition {
        field_name: String,
        data_type: String,
        value_rank: Option<String>,
    },
    /// Compass physical-quantity extension: value with engineering unit.
    PhysicalQuantity {
        raw_value: String,
        unit: String,
        /// Whether the element sat in the Compass extension namespace.
        namespaced: bool,
    },
    /// Compass coordinate extension: raw value with a coordinate reference.
    CoordinateReference {
        raw_value: String,
        reference: String,
        namespaced: bool,
    },
}

/// An unrecognized extension element, preserved wholesale (raw XML text and
/// source span). Nothing is dropped "to make the picture draw".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub xml_name: String,
    pub namespace_uri: Option<String>,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRecord {
    /// Stable record id: ``sha(file_id + node-id + ordinal)``.
    pub record_id: String,
    pub loc: SourceLoc,
    pub element: String,
    pub attrs: Vec<(String, String)>,
    pub node_id: Option<String>,
    pub browse_name: Option<String>,
    pub children: Vec<RawChild>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawChild {
    Known(KnownChild),
    Unknown(UnknownExtension),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedFile {
    pub path: String,
    pub content_sha: String,
    pub namespace_uris: Vec<String>,
    pub models: Vec<ModelDecl>,
    pub aliases: Vec<AliasRecord>,
    pub records: Vec<RawRecord>,
    /// Unknown children of the root ``UANodeSet`` element.
    pub root_extensions: Vec<UnknownExtension>,
    /// Parse problems that did not prevent parsing (positioned diagnostics).
    pub parse_diagnostics: Vec<ParseDiag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseDiag {
    pub code: String,
    pub message: String,
    pub line: usize,
    pub span: Option<Span>,
}
