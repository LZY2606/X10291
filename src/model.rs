//! Domain model: parsed NodeSet files (raw-preserving) and the merged global view.

use crate::ids::{content_hash, stable_id};
use crate::xmlmodel::RawXml;
use serde::{Deserialize, Serialize};

pub const UA_CORE_URI: &str = "http://opcfoundation.org/UA/";

// ---------------------------------------------------------------- source files

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFile {
    pub path: String,
    pub content: String,
    pub hash: String,
}

impl SourceFile {
    pub fn new(path: &str, content: String) -> Self {
        SourceFile { hash: content_hash(&content), path: path.to_string(), content }
    }
}

// ---------------------------------------------------------------- parsed nodeset

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDep {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub required: Vec<ModelDep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alias {
    pub name: String,
    /// Raw target text: usually a NodeId, but tools sometimes chain aliases.
    pub target_text: String,
    pub span: (usize, usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub ref_type_text: String,
    pub is_forward: bool,
    pub target_text: String,
    pub span: (usize, usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedNode {
    pub node_class: String,
    pub node_id_text: String,
    pub browse_name: String,
    pub display_name: Option<String>,
    pub references: Vec<Reference>,
    pub value_text: Option<String>,
    /// Everything under <Extensions>, kept raw (recognized or not).
    pub extensions_raw: Vec<RawXml>,
    /// Any other attributes on the node element, preserved.
    pub extra_attrs: Vec<(String, String)>,
    /// Any unrecognized child elements of the node, preserved raw.
    pub extra_children: Vec<RawXml>,
    pub span: (usize, usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSetFile {
    pub source: SourceFile,
    /// NamespaceUris as listed; local index = position + 1 (index 0 is the UA core).
    pub namespace_uris: Vec<String>,
    pub models: Vec<ModelInfo>,
    pub aliases: Vec<Alias>,
    pub nodes: Vec<ParsedNode>,
    /// Unrecognized root-level children, preserved raw.
    pub extra_root: Vec<RawXml>,
}

// ---------------------------------------------------------------- merged view

/// Global node identity: namespace URI + id part. Numeric namespace indexes are
/// only meaningful inside one file; the merged model keys everything by URI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalNodeId {
    pub ns_uri: String,
    pub id: String, // e.g. "i=5001", "s=Foo"
}

impl GlobalNodeId {
    pub fn canonical(&self) -> String {
        format!("{}#{}", self.ns_uri, self.id)
    }
    pub fn stable(&self) -> String {
        stable_id("node", &self.canonical())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    Active,
    /// One of several retained candidates for a duplicated NodeId.
    Candidate,
    /// Candidate not selected by a decision; kept, never overwritten.
    RejectedCandidate,
    /// Placeholder for a reference target that does not exist (dangling).
    UnresolvedPlaceholder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedNode {
    pub stable_id: String,
    pub gid: GlobalNodeId,
    pub node_class: String,
    /// BrowseName with the *global* namespace index (URI-remapped).
    pub browse_name: String,
    /// Raw BrowseName exactly as written in the source file.
    pub browse_name_raw: String,
    pub display_name: Option<String>,
    pub source_file: String,
    pub span: (usize, usize),
    pub status: NodeStatus,
    pub value: Option<QuantityView>,
    /// Raw XML of unrecognized extensions, serialized back to XML text.
    pub unknown_extensions: Vec<String>,
    pub extra_attrs: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantityView {
    pub stable_id: String,
    /// Raw value text exactly as imported.
    pub raw_text: String,
    pub unit: Option<EUInfo>,
    pub coordinate_basis: Option<String>,
    /// Derived normalization; None when unconvertible (value stays in place).
    pub normalized: Option<NormValue>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EUInfo {
    pub namespace_uri: String,
    pub unit_id: i64,
    pub display_name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormValue {
    pub value: f64,
    pub unit: String, // SI symbol, e.g. "Pa", "K", "m", "rad/s"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub stable_id: String,
    pub from: GlobalNodeId,
    pub to: GlobalNodeId,
    pub ref_type: String,
    /// Every (file, span, is_forward) declaration that produced this edge.
    pub sources: Vec<EdgeSource>,
    /// True when the target is an unresolved placeholder (dangling reference).
    pub dangling: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeSource {
    pub file: String,
    pub span: (usize, usize),
    pub declared_inverse: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedDep {
    pub stable_id: String,
    pub required_by_file: String,
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepConflict {
    pub stable_id: String,
    pub model_uri: String,
    /// All (file, version) requirements — every candidate retained.
    pub requirements: Vec<(String, String)>,
    pub resolved: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub stable_id: String,
    pub gid: GlobalNodeId,
    /// All candidate nodes; exactly one is Active, the rest are retained.
    pub candidates: Vec<String>, // stable_ids
    pub resolved: Option<String>, // chosen stable_id
}
