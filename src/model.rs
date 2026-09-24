//! Derived layer data model.
//!
//! Derived objects are immutable, content-addressed and carry provenance
//! spans back to the raw source XML. Multiple legal candidates are kept
//! side by side; ambiguity is never collapsed into a forced answer.

use serde::{Deserialize, Serialize};

pub const UA_NS_URI: &str = "http://opcfoundation.org/UA/";
pub const NC_NS_URI: &str = "urn:nscompass:extensions:v1";

pub const RULE_SET_VERSION: &str = "rules-2026.09.0";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpanRef {
    pub file_id: String,
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub seq: u64,
    pub kind: String,
    pub severity: String, // info | warn | error
    pub subject: String,
    pub message: String,
    pub rule: String,
    pub evidence: Vec<SpanRef>,
    pub candidates: Vec<String>,
    pub branch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileDigest {
    pub file_id: String,
    pub path: String,
    pub content_hash: String,
    pub order: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NsEntry {
    pub uri: String,
    pub canonical_index: u16,
    /// per-file numeric index bindings, numeric indices stay file-local
    pub bindings: Vec<NsBinding>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NsBinding {
    pub file_id: String,
    pub path: String,
    pub index: u16,
    pub span: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelEntry {
    pub id: String,
    pub uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub provided_by: Vec<String>,
    pub requirements: Vec<ModelRequirement>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelRequirement {
    pub uri: String,
    pub requested_version: Option<String>,
    pub status: String, // satisfied | version_conflict | missing
    pub resolved_by: Option<String>,
    pub span: SpanRef,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AliasEntry {
    pub id: String,
    pub file_id: String,
    pub name: String,
    pub target: String,
    pub status: String, // resolved | cycle | unresolved
    pub chain: Vec<String>,
    pub span: SpanRef,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeProvenance {
    pub file_id: String,
    pub path: String,
    pub raw_node_id: String,
    pub span: (usize, usize),
    pub raw_node_id_attr: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct DerivedNode {
    pub id: String,
    pub ns_uri: String,
    pub id_kind: String,
    pub literal: String,
    pub canonical_node_id: String,
    pub local_node_ids: Vec<String>,
    pub browse_name: Option<String>,
    pub browse_name_uri: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub kind: String,
    pub status: String, // ok | unresolved_stub | duplicate_conflict | bad_nodeid
    pub fingerprint: String,
    pub sources: Vec<NodeProvenance>,
    pub winner: Option<String>,
    pub candidates: Vec<String>,
    pub quantity: Option<String>,
    pub coordinate: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub reference_type: String,
    pub reference_type_resolved: bool,
    /// canonical direction is always forward; true when it came from an
    /// inverse reference and endpoints were normalized (no mirror created)
    pub from_inverse: bool,
    pub status: String, // resolved | dangling | unresolved_alias | unresolved_type
    pub provenance: Vec<RawRefProvenance>,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawRefProvenance {
    pub file_id: String,
    pub path: String,
    pub owner_node: String,
    pub raw_target: String,
    pub raw_type: String,
    pub raw_is_forward: bool,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub id: String,
    pub node: String,
    pub raw_value: String,
    pub raw_unit: String,
    pub canonical_value: Option<f64>,
    pub canonical_unit: Option<String>,
    pub converted: bool,
    pub diagnostic: Option<String>,
    pub rule_version: String,
    pub span: SpanRef,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Coordinate {
    pub id: String,
    pub node: String,
    pub frame: String,
    pub raw_values: Vec<String>,
    pub values: Vec<f64>,
    pub diagnostic: Option<String>,
    pub span: SpanRef,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KeptExtension {
    pub id: String,
    pub container: String,
    pub tag: String,
    pub recognized: bool,
    pub node: Option<String>,
    pub span: SpanRef,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Unresolved {
    pub id: String,
    pub kind: String, // node | alias | model | ref_type | unit
    pub subject: String,
    pub reason: String,
    pub evidence: Vec<SpanRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct DerivedModel {
    pub rule_set_version: String,
    pub branch: String,
    pub files: Vec<FileDigest>,
    pub namespaces: Vec<NsEntry>,
    pub models: Vec<ModelEntry>,
    pub aliases: Vec<AliasEntry>,
    pub nodes: Vec<DerivedNode>,
    pub edges: Vec<Edge>,
    pub quantities: Vec<Quantity>,
    pub coordinates: Vec<Coordinate>,
    pub extensions: Vec<KeptExtension>,
    pub unresolved: Vec<Unresolved>,
    pub events: Vec<Event>,
}
