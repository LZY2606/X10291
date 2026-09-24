//! Core domain types for the Namespace Compass tool.
//!
//! Types split into three layers:
//! * raw parsed shapes (what was literally in the XML),
//! * resolved model shapes (URI remapped, reference directions normalized),
//! * events / provenance (machine readable judgments).

use serde::{Deserialize, Serialize};

/// Byte span inside an original source file, plus derived line/column range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// A NodeId as written in a *single* source file. `ns` is the file-local
/// numeric index and is never used for cross-file identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RawNodeId {
    pub ns: u16,
    pub id: NodeIdValue,
    /// Raw text between equals signs, retained for diagnostics.
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeIdValue {
    Numeric(i64),
    String(String),
    Guid(String),
    ByteString(String),
}

/// Fully qualified NodeId: identity is (namespace URI, value).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GlobalNodeId {
    pub uri: String,
    pub value: String,
    pub id_kind: String,
}

impl std::fmt::Display for GlobalNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}|{}|{}", self.uri, self.id_kind, self.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QualifiedName {
    pub ns: u16,
    pub name: String,
    pub raw: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Object,
    Variable,
    Method,
    ObjectType,
    VariableType,
    ReferenceType,
    DataType,
    View,
}

impl NodeKind {
    pub fn from_element(local: &str) -> Option<NodeKind> {
        Some(match local {
            "UAObject" => NodeKind::Object,
            "UAVariable" => NodeKind::Variable,
            "UAMethod" => NodeKind::Method,
            "UAObjectType" => NodeKind::ObjectType,
            "UAVariableType" => NodeKind::VariableType,
            "UAReferenceType" => NodeKind::ReferenceType,
            "UADataType" => NodeKind::DataType,
            "UAView" => NodeKind::View,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Object => "UAObject",
            NodeKind::Variable => "UAVariable",
            NodeKind::Method => "UAMethod",
            NodeKind::ObjectType => "UAObjectType",
            NodeKind::VariableType => "UAVariableType",
            NodeKind::ReferenceType => "UAReferenceType",
            NodeKind::DataType => "UADataType",
            NodeKind::View => "UAView",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub ref_type_raw: String,
    pub is_forward: bool,
    pub target_raw: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAttribute {
    pub name: String,
    pub value: String,
    /// Non-empty for attributes in an unmodeled XML namespace.
    pub ns_uri: String,
}

/// An element the parser did not model. Its raw XML text is retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub id: String,
    pub parent_kind: String,
    /// "top-level" or node global id placeholder resolved later.
    pub parent_ref: String,
    pub element_local: String,
    pub ns_uri: String,
    pub raw_xml: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub local_id: String,
    pub kind: NodeKind,
    pub node_id_raw: String,
    pub browse_name_raw: String,
    pub display_name: Option<String>,
    pub data_type_raw: Option<String>,
    pub parent_node_raw: Option<String>,
    pub references: Vec<RawReference>,
    pub attributes: Vec<RawAttribute>,
    pub scalar_value_text: Option<String>,
    pub quantities: Vec<RawQuantity>,
    pub range: SourceRange,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawQuantity {
    pub value_text: String,
    pub unit: String,
    pub coordinate_basis: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequirement {
    pub model_uri: String,
    pub version: String,
    pub publication_date: Option<String>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedFile {
    pub file_id: String,
    pub path_hint: String,
    pub model_uri: String,
    pub model_version: String,
    pub publication_date: Option<String>,
    /// Index 0.. as written in NamespaceTable.
    pub namespace_table: Vec<String>,
    pub requirements: Vec<ModelRequirement>,
    pub aliases: Vec<RawAlias>,
    pub nodes: Vec<RawNode>,
    pub unknown_extensions: Vec<UnknownExtension>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAlias {
    pub name: String,
    pub target_raw: String,
    pub range: SourceRange,
}

// ---- Derived layer ----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedNode {
    pub gid: GlobalNodeId,
    pub file_id: String,
    pub kind: String,
    pub browse_uri: String,
    pub browse_name: String,
    pub display_name: Option<String>,
    pub ordinal: u32,
    pub raw_range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeKey {
    pub source: GlobalNodeId,
    pub target: GlobalNodeId,
    pub ref_type: GlobalNodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedEdge {
    pub key: EdgeKey,
    pub direction: String,
    pub source_file: String,
    pub raw_is_forward: bool,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedRef {
    pub source: Option<GlobalNodeId>,
    pub endpoint_raw: String,
    pub ref_type_raw: String,
    pub reason: String,
    pub file_id: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub gid: GlobalNodeId,
    pub value_text: String,
    pub unit: String,
    pub coordinate_basis: String,
    pub normalized_value: Option<f64>,
    pub normalized_unit: Option<String>,
    pub convertible: bool,
    pub diagnostic: Option<String>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedNode {
    pub gid: GlobalNodeId,
    pub referenced_from: Vec<GlobalNodeId>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub required_uri: String,
    pub required_version: String,
    pub available_version: Option<String>,
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DerivedGraph {
    pub rule_version: String,
    pub namespace_map: Vec<NamespaceMapping>,
    pub nodes: Vec<ResolvedNode>,
    pub edges: Vec<ResolvedEdge>,
    pub unresolved: Vec<UnresolvedRef>,
    pub unresolved_nodes: Vec<UnresolvedNode>,
    pub quantities: Vec<Quantity>,
    pub dependencies: Vec<DependencyStatus>,
    pub duplicates: Vec<DuplicateRecord>,
    pub alias_cycles: Vec<AliasCycle>,
    pub diagnostics: Vec<DiagnosticEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceMapping {
    pub file_id: String,
    pub file_index: u16,
    pub uri: String,
    pub global_index: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateRecord {
    pub gid: GlobalNodeId,
    pub file_ids: Vec<String>,
    pub resolution: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasCycle {
    pub file_id: String,
    pub cycle: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticEvent {
    pub code: String,
    pub severity: String,
    pub file_id: String,
    pub subject: String,
    pub message: String,
    pub basis: String,
    pub range: Option<SourceRange>,
}

// ---- Event log / versions ---------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEvent {
    pub seq: i64,
    pub id: String,
    pub branch: String,
    pub kind: String,
    pub rule_version: String,
    pub label: String,
    pub detail_json: String,
    pub source_file_id: Option<String>,
    pub parent_event_id: Option<String>,
    pub base_version_id: Option<String>,
    pub result_version_id: Option<String>,
    pub accepted: bool,
    pub clock_tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchPoint {
    pub branch: String,
    pub event_seq: i64,
    pub version_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ImpactReport {
    /// Nodes whose derived result changed fingerprint.
    pub changed_nodes: Vec<GlobalNodeId>,
    /// Nodes depending on changed nodes but whose result stays valid.
    pub still_valid_nodes: Vec<GlobalNodeId>,
    /// Edges removed by the change.
    pub removed_edges: Vec<EdgeKey>,
    /// Edges added by the change.
    pub added_edges: Vec<EdgeKey>,
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionSnapshot {
    pub version_id: String,
    pub branch: String,
    pub event_seq: i64,
    pub rule_version: String,
    pub derived: DerivedGraph,
    /// Stable id -> raw source text (bytes as utf8 lossy retained separately).
    pub file_order: Vec<String>,
    pub impact: ImpactReport,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub branch: String,
    pub subject: String,
    pub options: Vec<String>,
    pub chosen: Option<String>,
    pub rejected: Vec<String>,
    pub created_event_seq: i64,
    pub resolved_event_seq: Option<i64>,
}
