//! Derived (normalized) model layer types.

use crate::nodeset::RawEngineeringValue;
use serde::{Deserialize, Serialize};

/// A file-local NodeId after expanding an alias (if any). Alias expansion is
/// explicit: on a cycle / missing alias the node stays unresolved instead of
/// guessing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalId {
    pub literal: String,
    pub expanded_from_alias: Option<String>,
    pub alias_status: AliasStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AliasStatus {
    Literal,
    Resolved,
    /// alias names participating in a cycle, in traversal order.
    Cycle(Vec<String>),
    Missing(String),
}

/// Global node identity. `global_index` is assigned by URI across the merged
/// set; numeric indices from source files are never reused across files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalId {
    pub uri: String,
    pub local: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NsMapping {
    /// Merged index (stable: sorted by URI).
    pub merged_index: u16,
    pub uri: String,
    /// Where this URI appeared: (file, file-local index).
    pub occurrences: Vec<(String, u16)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeClass {
    Object,
    ObjectType,
    Variable,
    VariableType,
    Method,
    DataType,
    ReferenceType,
    View,
    Unresolved,
}

impl NodeClass {
    pub fn from_tag(t: &str) -> NodeClass {
        match t {
            "UAObject" => NodeClass::Object,
            "UAObjectType" => NodeClass::ObjectType,
            "UAVariable" => NodeClass::Variable,
            "UAVariableType" => NodeClass::VariableType,
            "UAMethod" => NodeClass::Method,
            "UADataType" => NodeClass::DataType,
            "UAReferenceType" => NodeClass::ReferenceType,
            "UAView" => NodeClass::View,
            _ => NodeClass::Unresolved,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            NodeClass::Object => "UAObject",
            NodeClass::ObjectType => "UAObjectType",
            NodeClass::Variable => "UAVariable",
            NodeClass::VariableType => "UAVariableType",
            NodeClass::Method => "UAMethod",
            NodeClass::DataType => "UADataType",
            NodeClass::ReferenceType => "UAReferenceType",
            NodeClass::View => "UAView",
            NodeClass::Unresolved => "Unresolved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedNode {
    pub gid: GlobalId,
    pub node_class: NodeClass,
    pub browse_name: String,
    pub display_name: Option<String>,
    /// Every raw occurrence of this global node (duplicate NodeId merges
    /// keep all candidates).
    pub raw_refs: Vec<RawNodeRef>,
    pub engineering: Vec<EngineeringValue>,
    /// When duplicate raw nodes disagree and no decision has been taken, the
    /// node carries all legal candidates instead of a forced winner.
    pub candidates: Vec<RawNodeRef>,
    pub decided: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeRef {
    pub file: String,
    pub raw_node_id: String,
    pub rawnode_span: (u64, u64),
    pub node_class: String,
    pub browse_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineeringValue {
    pub raw_text: String,
    pub unit: Option<String>,
    pub coordinate_frame: Option<String>,
    /// Normalized SI text when conversion rules apply; else None with a
    /// diagnostic on the node's issue list.
    pub normalized: Option<String>,
    pub source: RawNodeRef,
}

impl From<&RawEngineeringValue> for () {
    fn from(_: &RawEngineeringValue) {}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub src: GlobalId,
    pub dst: GlobalId,
    pub reference_type: GlobalId,
    /// How the edge was observed: forward as authored or the normalized
    /// representation of an inverse reference.
    pub direction: EdgeDirection,
    pub source: RawNodeRef,
    pub ref_span: (u64, u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeDirection {
    Forward,
    InverseNormalized,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingTarget {
    pub from: GlobalId,
    /// Literal target text as authored (may be alias-backed or malformed).
    pub literal: String,
    pub reference_type: Option<GlobalId>,
    pub direction: EdgeDirection,
    pub source: RawNodeRef,
    pub ref_span: (u64, u64),
    pub reason: MissingReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissingReason {
    /// Reference pointed at a node that used to exist and was deleted.
    DeletedTarget { deleted_from: String },
    /// File-local numeric namespace index does not resolve in the merged map.
    NamespaceIndexOutOfRange(u16),
    AliasCycle(Vec<String>),
    AliasMissing(String),
    /// Required model dependency absent: retained as an explicit unresolved
    /// node rather than a fabricated concrete node.
    DependencyMissing { model_uri: String, requested: String },
    MalformedNodeId,
    Unknown,
}

/// Placeholder node retained for a missing model dependency. Semantics:
/// "dependency missing -> unresolved node", never a fabricated real node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedNode {
    pub gid: GlobalId,
    pub reason: String,
    pub detail: String,
    pub raw_refs: Vec<RawNodeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDependency {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required: Vec<RequiredDependency>,
    pub file: String,
    pub span: (u64, u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredDependency {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub span: (u64, u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub file: String,
    pub location: String,
    pub qname: String,
    pub xml: String,
    pub span: (u64, u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasEntry {
    pub file: String,
    pub name: String,
    pub target: String,
    pub span: (u64, u64),
}
