//! Core domain model for the Namespace Compass toolkit.
//!
//! Semantics preserved by design:
//! - Numeric namespace indexes are only meaningful inside a single file;
//!   merging remaps every NodeId through the namespace URI.
//! - Inverse and forward references are normalized into a canonical edge
//!   form, but no missing edge is ever fabricated.
//! - Missing model dependencies keep `unresolved` placeholder nodes.

use serde::{Deserialize, Serialize};

/// A raw NodeId exactly as written in the source XML, plus its parsed form.
/// The numeric `ns` index is file-local and must never be compared across files.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub raw: String,
    pub ns: u16,
    pub kind: IdentKind,
    pub ident: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdentKind {
    Numeric,
    String,
    Guid,
    Opaque,
}

impl NodeId {
    pub fn parse(raw: &str) -> Option<NodeId> {
        let mut ns: u16 = 0;
        let mut body = raw.trim();
        if let Some(rest) = body.strip_prefix("ns=") {
            let (num, rest2) = rest.split_once(';')?;
            ns = num.parse().ok()?;
            body = rest2;
        }
        let (kind, ident) = if let Some(v) = body.strip_prefix("i=") {
            (IdentKind::Numeric, v)
        } else if let Some(v) = body.strip_prefix("s=") {
            (IdentKind::String, v)
        } else if let Some(v) = body.strip_prefix("g=") {
            (IdentKind::Guid, v)
        } else if let Some(v) = body.strip_prefix("b=") {
            (IdentKind::Opaque, v)
        } else {
            return None;
        };
        Some(NodeId {
            raw: raw.trim().to_string(),
            ns,
            kind,
            ident: ident.to_string(),
        })
    }
}

/// A NodeId after URI-based remapping: the namespace is a URI, not an index.
/// This is the stable identity used across files, versions and exports.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExpandedNodeId {
    pub ns_uri: String,
    pub kind: IdentKind,
    pub ident: String,
}

impl ExpandedNodeId {
    pub fn to_stable_string(&self) -> String {
        let k = match self.kind {
            IdentKind::Numeric => "i",
            IdentKind::String => "s",
            IdentKind::Guid => "g",
            IdentKind::Opaque => "b",
        };
        format!("nsu={};{}={}", self.ns_uri, k, self.ident)
    }
}

/// Source location of an XML fragment, 1-based lines, byte offsets into the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub file: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseName {
    /// Raw text as written, e.g. "2:Pump".
    pub raw: String,
    pub ns: u16,
    pub name: String,
}

impl BrowseName {
    pub fn parse(raw: &str) -> BrowseName {
        if let Some((ns, name)) = raw.split_once(':') {
            if let Ok(ns) = ns.parse::<u16>() {
                return BrowseName {
                    raw: raw.to_string(),
                    ns,
                    name: name.to_string(),
                };
            }
        }
        BrowseName {
            raw: raw.to_string(),
            ns: 0,
            name: raw.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// ReferenceType as written (may be an alias name or a NodeId).
    pub reference_type_raw: String,
    pub is_forward: bool,
    /// Target NodeId as written (may contain an alias).
    pub target_raw: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    pub name: String,
    pub target_raw: String,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDependency {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required_models: Vec<ModelDependency>,
}

/// A domain quantity keeps its raw representation; normalization happens only
/// in the derived layer. Unconvertible values stay in place with a diagnostic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub raw_value: String,
    /// Unit or coordinate basis as declared, e.g. "degC", "kg/h", "IEC61131:INT".
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UaNode {
    pub node_id_raw: String,
    pub browse_name_raw: String,
    pub display_name: String,
    pub node_class: String,
    /// Extra XML attributes we do not interpret, preserved verbatim.
    pub extra_attrs: Vec<(String, String)>,
    pub references: Vec<Reference>,
    /// Unrecognized child elements preserved as raw XML fragments.
    pub extensions: Vec<String>,
    /// Raw engineering value with unit, when present (UAVariable Value).
    pub value: Option<Quantity>,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSetDoc {
    pub file_name: String,
    /// File-local namespace table. Index i in this vec == `ns=i` in this file only.
    pub namespace_uris: Vec<String>,
    pub aliases: Vec<Alias>,
    pub models: Vec<ModelInfo>,
    pub nodes: Vec<UaNode>,
    /// Unrecognized top-level elements preserved as raw XML.
    pub unknown_top_level: Vec<String>,
    pub raw_xml: String,
}

/// A canonical merged edge. `forward` records the *declared* direction:
/// normalization dedupes identical declarations but never invents edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedEdge {
    pub source: ExpandedNodeId,
    pub reference_type: String,
    pub target: ExpandedNodeId,
    pub forward: bool,
    /// Where this edge was declared (file + span) for provenance.
    pub declared_in: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedNode {
    pub id: ExpandedNodeId,
    pub browse_name: String,
    pub display_name: String,
    pub node_class: String,
    pub extra_attrs: Vec<(String, String)>,
    pub extensions: Vec<String>,
    pub value: Option<Quantity>,
    /// Normalized value derived in the merge layer (e.g. SI-converted), if computable.
    pub normalized_value: Option<f64>,
    pub value_diagnostic: Option<String>,
    /// All source occurrences; >1 means a duplicate NodeId across files.
    pub sources: Vec<SourceSpan>,
    /// True when this is an unresolved placeholder for a missing dependency target.
    pub unresolved_placeholder: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub rule_id: String,
    pub severity: String,
    pub message: String,
    pub affected: Vec<String>,
    pub file: Option<String>,
    pub line: Option<usize>,
}

/// A machine-readable judgment event. Every accept/reject/adjust decision and
/// every rule firing is recorded so the archive can be replayed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub kind: String,
    pub rule_id: Option<String>,
    pub summary: String,
    pub detail: serde_json::Value,
    pub affected: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedModel {
    /// Global namespace table: URI list, index == merged ns index.
    pub global_namespaces: Vec<String>,
    /// Per-file mapping: file -> (file-local index -> global index or None if unmapped).
    pub ns_mappings: Vec<FileNsMapping>,
    pub nodes: Vec<MergedNode>,
    pub edges: Vec<MergedEdge>,
    pub unresolved: Vec<ExpandedNodeId>,
    pub diagnostics: Vec<Diagnostic>,
    /// Model dependencies across all files, with satisfaction status.
    pub dependencies: Vec<DependencyStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileNsMapping {
    pub file: String,
    pub entries: Vec<NsMapEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NsMapEntry {
    pub file_index: u16,
    pub uri: String,
    pub global_index: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub required_by_file: String,
    pub model_uri: String,
    pub required_version: Option<String>,
    pub available_version: Option<String>,
    pub status: DepState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DepState {
    Satisfied,
    VersionConflict,
    Missing,
}
