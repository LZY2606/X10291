//! Core domain types for Namespace Compass.
//!
//! All derived identities are stable strings ("stable IDs") so they survive
//! export -> delete database -> import unchanged.

use serde::{Deserialize, Serialize};

/// Byte range inside an original source file, 0-based half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Where a piece of information came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub file_id: String,
    pub span: Span,
    pub fragment: String,
}

impl Provenance {
    pub fn new(file_id: impl Into<String>, span: Span, fragment: impl Into<String>) -> Self {
        Provenance {
            file_id: file_id.into(),
            span,
            fragment: fragment.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Object,
    Variable,
    ObjectType,
    VariableType,
    ReferenceType,
    DataType,
    Method,
    View,
    Unknown,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Object => "Object",
            NodeKind::Variable => "Variable",
            NodeKind::VariableType => "VariableType",
            NodeKind::ObjectType => "ObjectType",
            NodeKind::ReferenceType => "ReferenceType",
            NodeKind::DataType => "DataType",
            NodeKind::Method => "Method",
            NodeKind::View => "View",
            NodeKind::Unknown => "Unknown",
        }
    }
    pub fn parse(tag: &str) -> NodeKind {
        match tag {
            "UAObject" => NodeKind::Object,
            "UAVariable" => NodeKind::Variable,
            "UAObjectType" => NodeKind::ObjectType,
            "UAVariableType" => NodeKind::VariableType,
            "UAReferenceType" => NodeKind::ReferenceType,
            "UADataType" => NodeKind::DataType,
            "UAMethod" => NodeKind::Method,
            "UAView" => NodeKind::View,
            _ => NodeKind::Unknown,
        }
    }
}

/// One parsed source file: the complete NodeSet XML is preserved structurally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedFile {
    pub file_id: String,
    pub path: String,
    pub sha256: String,
    /// `NamespaceUris/Uri` entries, indexed by local ns index (0 implicit OPC UA).
    pub namespace_uris: Vec<String>,
    /// `Models/Model` entries.
    pub models: Vec<RawModel>,
    /// `Models/Model/RequiredModel` entries, linked by model order.
    pub required_models: Vec<RawRequiredModel>,
    pub aliases: Vec<RawAlias>,
    pub nodes: Vec<RawNode>,
    /// Extensions found outside nodes, with their raw XML retained.
    pub unknown_extensions: Vec<RawExtension>,
    /// Raw bytes are kept so any source range can be read back.
    #[serde(with = "serde_bytes_compat")]
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawModel {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub prov: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRequiredModel {
    pub owner_model_uri: String,
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub prov: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAlias {
    pub name: String,
    pub node_id: String,
    pub prov: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub ord: usize,
    pub kind: NodeKind,
    pub node_id: String,
    pub browse_name: String,
    pub symbolic_name: Option<String>,
    pub data_type: Option<String>,
    pub value_rank: Option<String>,
    pub is_abstract: Option<String>,
    pub references: Vec<RawReference>,
    /// Attributes on the node element that are not modelled explicitly.
    pub extra_attributes: Vec<(String, String)>,
    pub extensions: Vec<RawExtension>,
    /// Raw `Value` element text, retained with its original representation.
    pub value_text: Option<String>,
    pub value_attributes: Vec<(String, String)>,
    pub prov: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub reference_type: String,
    pub target: String,
    pub is_forward: bool,
    pub prov: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawExtension {
    pub ord: usize,
    pub tag: String,
    pub raw_xml: String,
    pub attributes: Vec<(String, String)>,
    pub prov: Provenance,
}

/// Severity of a diagnostic event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// Machine-readable judgment events. Codes are part of the contract and tests
/// assert on them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub code: String,
    pub severity: Severity,
    pub message: String,
    /// Stable IDs of objects affected by this event.
    pub subjects: Vec<String>,
    /// Provenance chain (oldest first).
    pub sources: Vec<Provenance>,
    /// Rule that produced the event (with version).
    pub rule: String,
}

/// Resolved global node identity. Numeric local indices never appear here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GlobalNodeId {
    pub namespace_uri: String,
    pub identifier: String,
}

impl GlobalNodeId {
    pub fn stable(&self) -> String {
        format!("gni::{}::{}", self.namespace_uri, self.identifier)
    }
    pub fn display(&self) -> String {
        format!("{}#{}", self.namespace_uri, self.identifier)
    }
    pub fn parse(stable: &str) -> Option<Self> {
        let rest = stable.strip_prefix("gni::")?;
        let at = rest.find("::")?;
        Some(GlobalNodeId {
            namespace_uri: rest[..at].to_string(),
            identifier: rest[at + 2..].to_string(),
        })
    }
}

/// One legal candidate when a NodeId cannot be resolved uniquely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub candidate_id: String,
    pub label: String,
    pub chosen: bool,
    pub reason: String,
}

/// A derived node in the merged graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedNode {
    pub node_id: GlobalNodeId,
    pub kind: NodeKind,
    pub browse_name: String,
    pub browse_namespace_uri: String,
    pub resolved: bool,
    /// Stable id of the chosen raw node when duplicates exist (`raw::<file>::<ord>`).
    pub chosen_source: Option<String>,
    pub duplicate_sources: Vec<String>,
    pub sources: Vec<Provenance>,
    pub value: Option<RawValue>,
    pub unit: Option<UnitInfo>,
    pub coordinate_basis: Option<CoordinateBasis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawValue {
    pub text: String,
    pub attributes: Vec<(String, String)>,
    pub source: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitInfo {
    pub raw: String,
    pub normalized: Option<String>,
    pub convertible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateBasis {
    pub frame: String,
    pub raw: String,
    pub convertible: bool,
}

/// A normalized edge. Direction is always source -> target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedEdge {
    pub edge_id: String,
    pub source: GlobalNodeId,
    pub target: GlobalNodeId,
    pub reference_type: GlobalNodeId,
    pub direction: EdgeDirection,
    pub resolved_target: bool,
    pub sources: Vec<Provenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeDirection {
    Forward,
    InvertedInverse,
}

impl EdgeDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeDirection::Forward => "forward",
            EdgeDirection::InvertedInverse => "inverted_inverse",
        }
    }
}

/// A placeholder kept when a dependency node is missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedNode {
    pub placeholder_id: String,
    pub node_id: GlobalNodeId,
    pub reason: String,
    pub candidates: Vec<Candidate>,
    pub sources: Vec<Provenance>,
}

/// Model dependency status derived from `Models` / `RequiredModel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDependency {
    pub dep_id: String,
    pub model_uri: String,
    pub required_version: String,
    pub present_version: Option<String>,
    pub state: DependencyState,
    pub sources: Vec<Provenance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyState {
    Satisfied,
    VersionConflict,
    Missing,
}

impl DependencyState {
    pub fn as_str(self) -> &'static str {
        match self {
            DependencyState::Satisfied => "satisfied",
            DependencyState::VersionConflict => "version_conflict",
            DependencyState::Missing => "missing",
        }
    }
}

/// Complete result of one derivation version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derivation {
    pub derivation_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub rule_version: String,
    pub base_event_id: Option<String>,
    pub namespace_map: Vec<NamespaceEntry>,
    pub nodes: Vec<DerivedNode>,
    pub edges: Vec<DerivedEdge>,
    pub unresolved: Vec<UnresolvedNode>,
    pub dependencies: Vec<ModelDependency>,
    pub events: Vec<Event>,
    /// Explanation of why each output was reused or recomputed.
    pub recompute: Vec<RecomputeNote>,
    pub input_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceEntry {
    pub global_index: usize,
    pub namespace_uri: String,
    /// `(file_id, local_index)` pairs proving the remap.
    pub local_bindings: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecomputeNote {
    pub subject: String,
    pub status: RecomputeStatus,
    pub reason: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecomputeStatus {
    Reused,
    Recomputed,
}

impl RecomputeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RecomputeStatus::Reused => "reused",
            RecomputeStatus::Recomputed => "recomputed",
        }
    }
}

/// A snapshot of all imported raw material, content addressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub import_id: String,
    pub label: String,
    pub files: Vec<ParsedFile>,
    pub created_at: String,
}

/// A human decision, recorded as an immutable event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub event_id: String,
    pub base_derivation_id: String,
    pub kind: DecisionKind,
    pub subject: String,
    pub payload: serde_json::Value,
    pub comment: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionKind {
    Accept,
    Reject,
    Parameter,
}

impl DecisionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DecisionKind::Accept => "accept",
            DecisionKind::Reject => "reject",
            DecisionKind::Parameter => "parameter",
        }
    }
    pub fn parse(s: &str) -> Option<DecisionKind> {
        Some(match s {
            "accept" => DecisionKind::Accept,
            "reject" => DecisionKind::Reject,
            "parameter" => DecisionKind::Parameter,
            _ => return None,
        })
    }
}

/// Semantic difference between two derivations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticDiff {
    pub left_id: String,
    pub right_id: String,
    pub nodes_added: Vec<String>,
    pub nodes_removed: Vec<String>,
    pub nodes_changed: Vec<NodeChange>,
    pub edges_added: Vec<String>,
    pub edges_removed: Vec<String>,
    pub dependencies_changed: Vec<String>,
    pub events_introduced: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeChange {
    pub node: String,
    pub fields: Vec<String>,
}

mod serde_bytes_compat {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&String::from_utf8_lossy(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(de)?;
        Ok(s.into_bytes())
    }
}
