//! Domain model: raw preservation layer (single-file view) plus derived
//! merged-graph types. Raw types never discard information.

use serde::{Deserialize, Serialize};

/// A source byte range, half-open `[start,end)`, with the 1-based line where
/// the element begins (so reviewers can jump into the original XML).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
}

impl Span {
    pub const ZERO: Span = Span { start: 0, end: 0, line: 0 };
}

/// NodeId exactly as written in one file. `ns_index` is the *file-local*
/// numeric namespace index (0 when omitted) and is meaningless across files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeId {
    pub raw: String,
    pub ns_index: u32,
    pub kind: String, // "i" | "s" | "g" | "b"
    pub value: String,
    pub valid: bool,
}

/// Target written in a References block: either an alias name or a raw id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawTarget {
    pub raw: String,
    pub looks_like_alias: bool,
    pub parsed: Option<RawNodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAlias {
    pub name: String,
    pub value: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequire {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub requires: Vec<ModelRequire>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRef {
    pub seq: usize,
    pub type_raw: RawTarget,
    pub target_raw: RawTarget,
    pub is_forward: bool,
    pub span: Span,
}

/// Any child element not recognised as a standard NodeSet2 construct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawExt {
    pub seq: usize,
    pub xmlns: String,
    pub name: String,
    pub outer_xml: String,
    pub span: Span,
}

/// A value whose engineering unit / coordinate frame is unknown or
/// non-convertible. The original representation is always retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measure {
    pub raw_value: String,
    pub unit_raw: String,
    pub unit_ns_uri: String,
    pub si_value: Option<String>,
    pub si_unit: Option<String>,
    pub coord_frame: Option<String>,
    pub convert_diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub seq: usize,
    pub node_class: String,
    pub node_id_raw: String,
    pub node_id: Option<RawNodeId>,
    pub browse_name_raw: String,
    pub browse_ns_index: Option<u32>,
    pub browse_name: String,
    pub display_name: String,
    pub description: String,
    pub value_repr: Option<String>,
    pub measures: Vec<Measure>,
    pub type_def_raw: Option<RawTarget>,
    pub references: Vec<RawRef>,
    pub extras: Vec<RawExt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseDiagnostic {
    pub code: String,
    pub message: String,
    pub span: Span,
}

/// Everything preserved from a single imported file. Nothing here is
/// normalised away (including unknown extensions and invalid ids).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub name: String,
    pub content: String,
    pub sha256: String,
    pub namespaces: Vec<String>,
    pub aliases: Vec<RawAlias>,
    pub models: Vec<ModelEntry>,
    pub nodes: Vec<RawNode>,
    pub diagnostics: Vec<ParseDiagnostic>,
}

// ---------------------------------------------------------------------------
// Derived layer
// ---------------------------------------------------------------------------

/// The merged namespace table, URI-keyed. File-local indexes are mapped onto
/// the merged index but identity is always the URI, never the number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NsEntry {
    pub merged_index: u32,
    pub uri: String,
    pub first_seen_source: String,
    pub local_indexes: Vec<LocalIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalIndex {
    pub source_id: String,
    pub source_name: String,
    pub local_index: u32,
}

/// Globally stable node key: content hash over (source sha256, per-file seq).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeKey {
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub seq: usize,
    pub node_class: String,
    pub node_id_raw: String,
    pub ns_uri: Option<String>,
    pub ns_index_local: Option<u32>,
    pub id_kind: String,
    pub id_value: String,
    pub id_valid: bool,
    pub browse_name_raw: String,
    pub browse_ns_uri: Option<String>,
    pub display_name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeKey {
    pub id: String,
    pub owner_node_id: String,
    pub source_source: String,
    pub seq: usize,
    /// `a -> b` always, regardless of the IsForward flag in the file.
    pub from_node: Option<String>,
    pub to_node: Option<String>,
    pub ref_type_uri: Option<String>,
    pub ref_type_raw: String,
    pub target_raw: String,
    pub original_is_forward: bool,
    /// true when the file declared an inverse reference and we flipped it.
    pub normalised_from_inverse: bool,
    pub status: String, // "resolved" | "unresolved"
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasKey {
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub name: String,
    pub raw_value: String,
    pub status: String, // "resolved" | "cycle" | "unresolved"
    pub cycle_path: Vec<String>,
    pub points_to: Option<String>, // resolved node id
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepKey {
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub model_uri: String,
    pub version: String,
    pub requires_uri: String,
    pub requires_version: String,
    pub status: String, // "ok" | "missing" | "conflict"
    pub detail: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtKey {
    pub id: String,
    pub owner_node_id: Option<String>,
    pub source_id: String,
    pub xmlns: String,
    pub name: String,
    pub outer_xml: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: String,
    pub seq: i64,
    pub version_id: String,
    pub at: String,
    pub code: String,
    pub status: String, // "adopted" | "rejected" | "open"
    pub severity: String, // info | warning | error
    pub subject: String,
    pub message: String,
    pub basis: String, // machine-readable JSON: rule + evidence
    pub candidates: String, // JSON array of legal alternatives
    pub source_id: Option<String>,
    pub span: Option<Span>,
    pub triggered_by: Option<String>, // previous event/decision id
    pub decided: bool,
}

/// A full derived snapshot for one version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub namespaces: Vec<NsEntry>,
    pub nodes: Vec<NodeKey>,
    pub edges: Vec<EdgeKey>,
    pub aliases: Vec<AliasKey>,
    pub deps: Vec<DepKey>,
    pub exts: Vec<ExtKey>,
    pub events: Vec<EventRecord>,
}
