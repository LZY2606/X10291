//! Core domain model for OPC UA NodeSet merging.
//!
//! Semantics held by this layer:
//! - A numeric namespace index is only meaningful inside a single file.
//! - Raw representations (XML text, attribute strings, units) are preserved;
//!   normalization happens only in the derived layer.
//! - Multi-solution / uncertain states are kept as candidate lists, never
//!   collapsed into a single silent answer.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// A NodeId as written inside one source file (local namespace index).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocalNodeId {
    pub ns: u16,
    pub id: NodeIdKind,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum NodeIdKind {
    Numeric(u64),
    Text(String),
    Guid(String),
    Opaque(String),
}

/// A NodeId after URI-based remapping into the merged model.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GlobalNodeId {
    /// Namespace URI (identity), never a numeric index.
    pub ns_uri: String,
    pub id: NodeIdKind,
}

impl fmt::Display for GlobalNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = match &self.id {
            NodeIdKind::Numeric(v) => format!("i={}", v),
            NodeIdKind::Text(v) => format!("s={}", v),
            NodeIdKind::Guid(v) => format!("g={}", v),
            NodeIdKind::Opaque(v) => format!("b={}", v),
        };
        write!(f, "nsu={};{}", self.ns_uri, id)
    }
}

/// Stable identity of a node across export/import: URI-based, index-free.
pub type StableId = String;

pub fn stable_id(g: &GlobalNodeId) -> StableId {
    g.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct QualifiedName {
    pub ns: u16,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalQName {
    pub ns_uri: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeClass {
    Object,
    Variable,
    Method,
    ObjectType,
    VariableType,
    ReferenceType,
    DataType,
    View,
}

impl NodeClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeClass::Object => "UAObject",
            NodeClass::Variable => "UAVariable",
            NodeClass::Method => "UAMethod",
            NodeClass::ObjectType => "UAObjectType",
            NodeClass::VariableType => "UAVariableType",
            NodeClass::ReferenceType => "UAReferenceType",
            NodeClass::DataType => "UADataType",
            NodeClass::View => "UAView",
        }
    }
}

/// A reference exactly as declared in the source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    /// Reference type, possibly an alias name (resolved later).
    pub reference_type: String,
    pub is_forward: bool,
    /// Target as written: alias name or NodeId text with local ns index.
    pub target: String,
}

/// Byte range into the source file, for "trace back to source XML".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub file_id: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawNode {
    pub node_class: NodeClass,
    /// NodeId text as written (may be an alias).
    pub node_id_text: String,
    pub browse_name: QualifiedName,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// All other attributes preserved verbatim (units, ranges, AccessLevel...).
    pub attrs: BTreeMap<String, String>,
    pub references: Vec<RawReference>,
    /// Unrecognized child elements, raw XML preserved.
    pub extensions: Vec<RawExtension>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawExtension {
    pub element_name: String,
    pub raw_xml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required_models: Vec<RequiredModel>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasEntry {
    pub alias: String,
    /// NodeId text the alias maps to (may itself be another alias -> cycle).
    pub target_text: String,
    pub span: SourceSpan,
}

/// One imported NodeSet file, raw layer fully preserved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSetFile {
    pub file_id: String,
    /// Namespace table: index 0 is always http://opcfoundation.org/UA/.
    pub namespaces: Vec<String>,
    pub aliases: Vec<AliasEntry>,
    pub models: Vec<ModelEntry>,
    pub nodes: Vec<RawNode>,
    /// Complete original XML text.
    pub raw_xml: String,
}

/// Normalized reference edge (derived layer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub source: GlobalNodeId,
    pub reference_type: GlobalNodeId,
    pub target: GlobalNodeId,
    /// True if the source declared this as a forward reference.
    pub forward: bool,
    /// File that contributed this edge.
    pub origin_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedNode {
    pub id: GlobalNodeId,
    pub node_class: NodeClass,
    pub browse_name: GlobalQName,
    pub display_name: Option<String>,
    /// Raw attribute strings, units/coordinate basis untouched.
    pub attrs: BTreeMap<String, String>,
    pub extensions: Vec<RawExtension>,
    pub origin_file: String,
    pub span: SourceSpan,
}

/// A diagnostic: machine-readable judgment record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Diagnostic {
    AliasCycle { file_id: String, chain: Vec<String> },
    UnresolvableAlias { file_id: String, alias: String },
    DuplicateNodeId { id: GlobalNodeId, files: Vec<String> },
    DanglingReference { source: GlobalNodeId, reference_type: GlobalNodeId, target: GlobalNodeId },
    UnresolvedDependency { requiring_model: String, missing_model_uri: String },
    DependencyVersionConflict { model_uri: String, candidates: Vec<VersionCandidate> },
    UnknownNamespaceIndex { file_id: String, ns: u16, context: String },
    UnconvertibleValue { node: GlobalNodeId, attr: String, raw: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCandidate {
    pub version: Option<String>,
    pub required_by: String,
}

/// The merged model (derived from a set of files).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergedModel {
    /// Global namespace table: URI -> global index (deterministic order).
    pub namespace_uris: Vec<String>,
    pub nodes: BTreeMap<StableId, MergedNode>,
    /// Duplicate NodeIds: stable id -> all candidate nodes (first is primary).
    pub duplicates: BTreeMap<StableId, Vec<MergedNode>>,
    /// Forward-normalized edges; inverse-only edges kept as-is, no fabricated
    /// counterpart edges are invented.
    pub edges: Vec<Edge>,
    pub models: Vec<ModelEntry>,
    pub diagnostics: Vec<Diagnostic>,
    /// Dependency requirements kept as candidate lists when conflicting.
    pub dependency_candidates: BTreeMap<String, Vec<VersionCandidate>>,
}
