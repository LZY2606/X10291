//! Derived layer: merges per-file raw NodeSets into one global model.
//!
//! Invariants enforced here (see case pack `semantics`):
//! * numeric namespace indices are file-local; merging remaps by URI;
//! * inverse and forward references are normalized to one directed edge
//!   per triple - normalization never invents an edge or its symmetric twin;
//! * missing dependencies and missing targets stay as unresolved nodes,
//!   nothing is dropped to make the picture "complete";
//! * ambiguous situations keep every legal candidate instead of collapsing
//!   to a single answer.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::model::{self, parse_node_id, split_browse_name, Located, RawNodeSet, Range};

pub const RULE_VERSION: &str = "rules/opcua-nodeset/1.0.0";

pub fn short_hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0]);
    }
    hex::encode(h.finalize())[..16].to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DocRef {
    pub doc_id: String,
    pub filename: String,
    pub start: usize,
    pub end: usize,
}

impl DocRef {
    pub fn range(&self) -> Range {
        Range::new(self.start, self.end)
    }
}

fn dref(doc: &RawNodeSet, range: Range) -> DocRef {
    DocRef { doc_id: doc.doc_id.clone(), filename: doc.filename.clone(), start: range.start, end: range.end }
}

/// Global, namespace-independent node identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GNodeId {
    pub uri: String,
    pub id_type: String,
    pub id: String,
}

impl GNodeId {
    pub fn stable(&self) -> String {
        format!("gid:{}#{}={}", self.uri, self.id_type, self.id)
    }
    pub fn display(&self) -> String {
        format!("{}={}", self.id_type, self.id)
    }
}

/// Key for targets we could not globalize (malformed/alias-cycle/dangling).
fn unresolved_key(uri_fallback: &str, id_type: &str, id: &str) -> String {
    format!("gid:{}#{}={}", uri_fallback, id_type, id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NsEntry {
    pub global_ns: u16,
    pub uri: String,
    /// Every (file-local index) that binds to this URI.
    pub origins: Vec<DocRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitInfo {
    pub raw: String,
    pub si_value: Option<f64>,
    pub si_unit: Option<String>,
    pub diagnostic: Option<String>,
    pub origin: DocRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GNode {
    pub gid: GNodeId,
    pub stable_id: String,
    pub kind: String,
    pub browse_name: String,
    pub browse_uri: String,
    pub display_name: Option<String>,
    pub value_raw: Option<String>,
    pub unit: Option<UnitInfo>,
    /// Primary source; the adopted occurrence of a duplicated NodeId.
    pub origin: DocRef,
    /// Every occurrence, including un-adopted duplicates.
    pub occurrences: Vec<DocRef>,
    pub extensions: Vec<DocRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GEdge {
    pub edge_id: String,
    pub source: String,
    /// None while the target does not resolve to an imported node.
    pub target: Option<String>,
    pub reference_type: String,
    /// How each raw occurrence was written before normalization.
    pub raw_directions: Vec<String>,
    pub origins: Vec<DocRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unresolved {
    pub stable_id: String,
    pub kind: String, // target | model | nodeid | browse_name
    pub label: String,
    pub origin: DocRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub label: String,
    pub origin: Option<DocRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub seq: u32,
    pub kind: String,
    pub severity: String, // info | warning | error
    pub rule_version: String,
    pub title: String,
    pub detail: String,
    pub origin: Option<DocRef>,
    pub affected: Vec<String>,
    pub adopted: Vec<Candidate>,
    pub rejected: Vec<Candidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtypeLink {
    pub child: String,
    pub parent: String,
    pub origin: DocRef,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub rule_version: String,
    pub params: UnitParams,
    pub ns_table: Vec<NsEntry>,
    pub nodes: Vec<GNode>,
    pub edges: Vec<GEdge>,
    pub unresolved: Vec<Unresolved>,
    pub subtypes: Vec<SubtypeLink>,
    pub type_roots: Vec<String>,
    pub type_cycles: Vec<Vec<String>>,
    /// node stable id -> reachable stable ids (reference closure).
    pub closure: BTreeMap<String, Vec<String>>,
    pub closure_truncated: Vec<String>,
    pub events: Vec<Event>,
    pub signatures: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnitParams {
    /// True => try engineering-unit conversion to SI on the derived layer.
    pub convert_units: bool,
    /// Closure search cap before a "truncated" diagnostic is recorded.
    pub closure_cap: usize,
}

#[derive(Debug, Clone)]
pub struct BuildInput<'a> {
    pub docs: &'a [RawNodeSet],
    pub params: UnitParams,
    pub base: Option<&'a Snapshot>,
}
