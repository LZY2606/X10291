//! Domain model: preserved raw XML layer and derived (normalized) layer.
//!
//! Every value that can appear in the UI carries a stable identity derived
//! from content + source, so identity, ordering and provenance survive
//! export / delete-database / re-import.

use serde::{Deserialize, Serialize};

pub const RULE_VERSION: &str = "rules/2025.1";

/// Half-open byte range inside a named source file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SrcRange {
    pub file: String,
    pub start: u64,
    pub end: u64,
}

impl SrcRange {
    pub fn new(file: impl Into<String>, start: u64, end: u64) -> Self {
        Self { file: file.into(), start, end }
    }
}

/// Pointer to the source XML that backs a displayed value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Provenance {
    pub file: String,
    pub file_id: String,
    pub range: SrcRange,
}

// ---------------------------------------------------------------------------
// Raw layer: nothing from the XML is dropped.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawNodeSet {
    pub path: String,
    pub file_id: String,
    pub sha256: String,
    pub root_attrs: Vec<(String, String)>,
    pub namespaces: Vec<RawNamespace>,
    pub models: Vec<RawModelDecl>,
    pub aliases: Vec<RawAlias>,
    pub nodes: Vec<RawNode>,
    /// Unknown top-level children, serialized verbatim.
    pub extensions: Vec<RawExtension>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawNamespace {
    pub id: String,
    /// Local numeric index implied by document order (1-based; 0 = OPC UA base).
    pub local_index: u32,
    pub uri: String,
    pub range: SrcRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawModelDecl {
    pub id: String,
    pub required: bool,
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub publisher: Option<String>,
    pub product: Option<String>,
    pub range: SrcRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAlias {
    pub id: String,
    pub name: String,
    pub value: String,
    pub range: SrcRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRef {
    pub id: String,
    pub ordinal: u32,
    pub reference_type: String,
    pub target: String,
    pub is_forward: bool,
    pub forward_explicit: bool,
    pub range: SrcRange,
    pub text_range: SrcRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawNode {
    pub id: String,
    pub ordinal: u32,
    pub tag: String,
    pub node_id_attr: String,
    pub browse_name_attr: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub attributes: Vec<(String, String)>,
    pub references: Vec<RawRef>,
    pub value_xml: Option<String>,
    pub value_range: Option<SrcRange>,
    pub extensions: Vec<RawExtension>,
    pub range: SrcRange,
}

/// Any element the parser does not know about: kept byte-for-byte.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawExtension {
    pub id: String,
    pub tag: String,
    pub raw_xml: String,
    pub text: Option<String>,
    pub attributes: Vec<(String, String)>,
    pub range: SrcRange,
    pub parent_raw_node: Option<String>,
}

// ---------------------------------------------------------------------------
// NodeId / BrowseName
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Identifier {
    Numeric(u64),
    String_(String),
    Guid(String),
    ByteString(String),
}

impl Identifier {
    pub fn canonical(&self) -> String {
        match self {
            Identifier::Numeric(n) => format!("i={n}"),
            Identifier::String_(s) => format!("s={s}"),
            Identifier::Guid(g) => format!("g={g}"),
            Identifier::ByteString(b) => format!("b={b}"),
        }
    }
    pub fn label(&self) -> String {
        match self {
            Identifier::Numeric(n) => n.to_string(),
            Identifier::String_(s) => s.clone(),
            Identifier::Guid(g) => g.clone(),
            Identifier::ByteString(b) => b.clone(),
        }
    }
}

/// A NodeId resolved to a namespace URI. Numeric indices never escape the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct GlobalNodeId {
    pub ns_uri: String,
    pub identifier: Identifier,
}

impl GlobalNodeId {
    pub fn canonical(&self) -> String {
        format!("su={};{}", self.ns_uri, self.identifier.canonical())
    }
    pub fn stable_id(&self) -> String {
        stable_id("n", &[self.ns_uri.as_str(), &self.identifier.canonical()])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QualifiedBrowseName {
    pub raw: String,
    pub ns_uri: String,
    pub name: String,
}

// ---------------------------------------------------------------------------
// Derived layer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeriveParams {
    pub rule_version: String,
    pub unit_table_version: u32,
    /// "document" keeps file-introduction order; numeric indices are assigned
    /// by URI either way, this only controls presentation order.
    pub namespace_order: String,
}

impl Default for DeriveParams {
    fn default() -> Self {
        Self {
            rule_version: RULE_VERSION.to_string(),
            unit_table_version: 1,
            namespace_order: "document".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInput {
    pub path: String,
    pub file_id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NsView {
    pub id: String,
    pub uri: String,
    pub global_index: u32,
    pub sources: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateView {
    pub raw_node: String,
    pub file: String,
    pub range: SrcRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeView {
    pub id: String,
    pub node_id: String,
    pub ns_uri: String,
    pub identifier: String,
    pub tag: String,
    pub browse_name: Option<QualifiedBrowseName>,
    pub display_name: Option<String>,
    pub raw_node_id: String,
    pub raw_node_id_attr: String,
    pub defined: bool,
    pub ambiguous: bool,
    pub candidates: Vec<CandidateView>,
    pub value_xml: Option<String>,
    pub extension_ids: Vec<String>,
    pub rank: u32,
    pub sources: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeView {
    pub id: String,
    pub source: String,
    pub target: String,
    pub reference_type: String,
    pub reference_type_raw: String,
    /// Always normalized to the semantic forward direction.
    pub forward: bool,
    pub as_written: String,
    pub dangling_source: bool,
    pub dangling_target: bool,
    pub ref_type_unresolved: bool,
    pub evidence: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelView {
    pub id: String,
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub publisher: Option<String>,
    pub required: Vec<Provenance>,
    pub declared_by: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantityView {
    pub id: String,
    pub node: String,
    /// Original textual representation, never rewritten.
    pub raw_value: String,
    pub raw_unit: Option<String>,
    pub normalized_si: Option<f64>,
    pub si_unit: Option<String>,
    pub coordinate_basis: Option<String>,
    pub converted: bool,
    pub sources: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subject {
    pub kind: String,
    pub id: String,
    pub file: Option<String>,
    pub range: Option<SrcRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: String,
    pub severity: String, // error | warning | info | ambiguous | unresolved
    pub code: String,
    pub message: String,
    pub subjects: Vec<Subject>,
}

/// Machine-readable justification tying a result to a semantic rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasisEvent {
    pub id: String,
    pub code: String,
    pub rule_version: String,
    pub detail: String,
    pub adopted: bool,
    pub subjects: Vec<Subject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub params: DeriveParams,
    pub inputs: Vec<SnapshotInput>,
    pub namespaces: Vec<NsView>,
    pub models: Vec<ModelView>,
    pub nodes: Vec<NodeView>,
    pub edges: Vec<EdgeView>,
    pub quantities: Vec<QuantityView>,
    pub diagnostics: Vec<Diagnostic>,
    pub basis: Vec<BasisEvent>,
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffEntry {
    pub kind: String, // node-added | node-removed | node-changed | edge-added | ...
    pub subject: String,
    pub label: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub before_src: Option<Provenance>,
    pub after_src: Option<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub base: String,
    pub target: String,
    pub entries: Vec<DiffEntry>,
}

// ---------------------------------------------------------------------------
// Propagation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationItem {
    pub subject: String,
    pub label: String,
    pub status: String, // still-valid | recomputed | new | removed
    pub reason: String,
    pub depends_on_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagationReport {
    pub base_snapshot: Option<String>,
    pub target_snapshot: String,
    pub changed_files: Vec<String>,
    pub items: Vec<PropagationItem>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(prefix.as_bytes());
    for p in parts {
        h.update([0]);
        h.update(p.as_bytes());
    }
    format!("{prefix}_{}", hex12(&h.finalize()))
}

pub fn content_id(prefix: &str, bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let full: Vec<u8> = h.finalize().to_vec();
    format!("{prefix}_{}", hex12(&full))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex12(bytes: &[u8]) -> String {
    bytes.iter().take(12).map(|b| format!("{b:02x}")).collect()
}
