//! Domain model.
//!
//! The model is split in two layers:
//!
//! * the **baseline / source layer** ([`RawNodeSet`], [`RawNode`] ...) keeps the
//!   imported XML byte-for-byte in spirit: raw XML ranges, the namespace table,
//!   NodeIds, BrowseNames, references, aliases, model dependencies and
//!   unrecognized extensions are all retained;
//! * the **derived layer** ([`DerivedModel`]) contains normalized, merged views
//!   (global NodeIds, forward edges, units, type hierarchy, diffs). Derived data
//!   is recomputed, never mutates the source layer.

use serde::{Deserialize, Serialize};

/// A byte range into the original source XML (1-based line/column and 0-based
/// byte offsets). Every displayed value carries one of these provenance ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct XmlRange {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub start_byte: u64,
    pub end_byte: u64,
}

impl XmlRange {
    pub fn point(line: u32, col: u32, byte: u64) -> Self {
        Self {
            start_line: line,
            start_col: col,
            end_line: line,
            end_col: col,
            start_byte: byte,
            end_byte: byte,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

/// A reference as written in one XML file. `direction` records whether the
/// file used `<Reference IsForward="false">` (inverse). Both the original
/// direction and the normalized forward form are retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub reference_type: String,
    pub target: String,
    pub is_forward: bool,
    pub range: XmlRange,
}

/// An unrecognized (or extension) element kept verbatim, including its
/// namespace-qualified tag and raw inner text. Nothing is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub tag: String,
    pub attributes: Vec<Attribute>,
    pub raw_xml: String,
    pub recognized_kind: Option<String>,
    pub range: XmlRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub node_kind: String,
    pub node_id: String,
    pub browse_name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub references: Vec<RawReference>,
    pub attributes: Vec<Attribute>,
    pub extensions: Vec<UnknownExtension>,
    /// Outer raw XML of the node, so the source can always be read back.
    pub raw_xml: String,
    pub range: XmlRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    pub name: String,
    pub node_id: String,
    pub range: XmlRange,
}

/// `<RequiredModel ModelUri=... Version=... PublicationDate=...>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub range: XmlRange,
}

/// <NamespaceUris><Uri>...</Uri></NamespaceUris> and <Models>.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTable {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required: Vec<RequiredModel>,
    pub range: XmlRange,
}

/// One parsed NodeSet XML file. File-local namespace index 0 is always the
/// implicit OPC UA namespace (`http://opcfoundation.org/UA/`); indices 1..=n
/// come from `<NamespaceUris>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeSet {
    pub file_index: usize,
    pub last_modified: Option<String>,
    pub namespaces: Vec<String>,
    pub aliases: Vec<Alias>,
    pub nodes: Vec<RawNode>,
    pub models: Vec<ModelTable>,
    pub file_extensions: Vec<UnknownExtension>,
}

/// An imported source file (the baseline unit of provenance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    /// Stable content identity (`sf_` + sha256 of the XML bytes).
    pub id: String,
    pub filename: String,
    pub xml: String,
    pub parsed: RawNodeSet,
    pub parse_diagnostics: Vec<Diagnostic>,
}

/// Severity for machine-readable diagnostics/events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A machine-readable finding. `code` is a stable machine code and `range`
/// anchors the finding back to the source XML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub severity: Severity,
    pub source_file_id: Option<String>,
    pub node_id: Option<String>,
    pub range: Option<XmlRange>,
}

/// A node in the merged global identity space. Duplicate NodeIds surface as
/// `duplicate_of` candidates: none of the source copies is silently merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalNode {
    pub id: String,
    pub node_kind: String,
    pub uri: String,
    pub local_id: String,
    pub global_node_id: String,
    pub browse_name: Option<String>,
    pub display_name: Option<String>,
    pub source_file_id: String,
    pub source_filename: String,
    pub range: XmlRange,
    pub candidate_ids: Vec<String>,
    pub duplicate_of: Vec<String>,
}

/// A normalized reference edge. Inverse references are converted to the
/// forward direction, but the original (inverse) occurrence and the provenance
/// are recorded in `origin_is_forward` / `range`; no edges are invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub source_global: String,
    pub target_global: Option<String>,
    pub target_raw: String,
    pub reference_type_global: Option<String>,
    pub reference_type_raw: String,
    pub direction: EdgeDirection,
    pub source_file_id: String,
    pub range: XmlRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeDirection {
    ForwardAsWritten,
    InvertedFromInverse,
}

/// Alias resolution result for one file-local alias usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasResolution {
    pub file_index: usize,
    pub alias: String,
    pub resolved_node_id: Option<String>,
    pub state: AliasState,
    pub range: XmlRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AliasState {
    Resolved,
    Undefined,
    Cycle,
}

/// A physical/engineering value. The raw representation is always retained;
/// normalization (SI conversion / coordinate frame) happens only in the
/// derived fields. Values that cannot be converted stay at the raw position
/// with a diagnostic instead of being dropped for the sake of a chart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity {
    pub source_file_id: String,
    pub node_global_id: String,
    pub range: XmlRange,
    /// Original text, e.g. "21.5", "300 K".
    pub raw_value: String,
    /// Original unit/coordinate reference text, e.g. "degree_C", "FRAME_LINE".
    pub raw_unit_or_frame: String,
    pub kind: QuantityKind,
    pub normalized_value: Option<String>,
    pub normalized_unit: Option<String>,
    pub conversion_status: ConversionStatus,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuantityKind {
    ScalarWithUnit,
    Coordinate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversionStatus {
    Converted,
    RetainedRaw,
}

/// Unresolved dependencies and edges: kept, never hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unresolved {
    pub kind: UnresolvedKind,
    pub raw_ref: String,
    pub detail: String,
    pub source_file_id: String,
    pub range: XmlRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub enum UnresolvedKind {
    MissingNamespace,
    MissingNode,
    MissingRequiredModel,
    VersionConflict,
    AliasUndefined,
    AliasCycle,
    DuplicateNodeId,
    UnconvertibleValue,
    UnknownExtension,
    DanglingEdge,
}

/// One type-hierarchy relation derived from HasSubtype edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyLink {
    pub parent_global: String,
    pub child_global: String,
    pub edge_id: String,
}

/// A semantic difference between two derived versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticDiff {
    pub kind: DiffKind,
    pub subject: String,
    pub label: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub source_file_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffKind {
    NodeAdded,
    NodeRemoved,
    NodeChanged,
    EdgeAdded,
    EdgeRemoved,
    NamespaceRemapped,
    DependencyChanged,
    Conflict,
    UnresolvedChanged,
    QuantityNormalizationChanged,
}

/// Why a derived item was kept or recomputed during incremental propagation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecomputationTrace {
    pub subject: String,
    pub status: RecomputeStatus,
    pub reason: String,
    pub changed_inputs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecomputeStatus {
    Reused,
    Recomputed,
}

/// Result of a human decision. Rejected candidates are preserved
/// (`adopted == false`) so a confirmation is never an irreversible overwrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub event_id: String,
    pub subject: String,
    pub candidates: Vec<String>,
    pub chosen: Option<String>,
    pub adopted: bool,
    pub rationale: String,
}

/// Everything the derived layer of a version contains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DerivedModel {
    /// URI -> merged global namespace index, assigned deterministically by
    /// first file order.
    pub namespace_map: Vec<(String, u32)>,
    pub nodes: Vec<GlobalNode>,
    pub edges: Vec<Edge>,
    pub alias_resolutions: Vec<AliasResolution>,
    pub quantities: Vec<Quantity>,
    pub unresolved: Vec<Unresolved>,
    pub hierarchy: Vec<HierarchyLink>,
    pub diffs: Vec<SemanticDiff>,
    pub traces: Vec<RecomputationTrace>,
    pub diagnostics: Vec<Diagnostic>,
}
