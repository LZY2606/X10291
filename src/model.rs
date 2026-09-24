//! Core domain model.
//!
//! Everything imported from a NodeSet XML file is first retained verbatim in
//! [`RawFile`]; normalization happens only in the derived merge layer.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A NodeId as written in a single file: numeric namespace index is file local.
/// `ns_uri` is filled after merge (`None` while inside a single file).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeIdValue {
    pub ns: u16,
    pub id: String,
    pub id_type: IdType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IdType {
    Numeric,
    String,
    Guid,
    ByteString,
}

impl NodeIdValue {
    pub fn parse(text: &str) -> Option<NodeIdValue> {
        let (ns, rest) = if let Some(rest) = text.strip_prefix("ns=") {
            let semi = rest.find(';')?;
            (rest[..semi].parse().ok()?, &rest[semi + 1..])
        } else {
            (0u16, text)
        };
        if let Some(v) = rest.strip_prefix("i=") {
            Some(NodeIdValue { ns, id: v.to_string(), id_type: IdType::Numeric })
        } else if let Some(v) = rest.strip_prefix("s=") {
            Some(NodeIdValue { ns, id: v.to_string(), id_type: IdType::String })
        } else if let Some(v) = rest.strip_prefix("g=") {
            Some(NodeIdValue { ns, id: v.to_string(), id_type: IdType::Guid })
        } else if let Some(v) = rest.strip_prefix("b=") {
            Some(NodeIdValue { ns, id: v.to_string(), id_type: IdType::ByteString })
        } else {
            None
        }
    }

    pub fn canonical(&self) -> String {
        let p = match self.id_type {
            IdType::Numeric => "i",
            IdType::String => "s",
            IdType::Guid => "g",
            IdType::ByteString => "b",
        };
        format!("ns={};{}={}", self.ns, p, self.id)
    }
}

/// Qualified BrowseName: `ns:Name`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct QualifiedName {
    pub ns: u16,
    pub name: String,
}

impl QualifiedName {
    pub fn parse(text: &str) -> QualifiedName {
        if let Some((ns, name)) = text.split_once(':') {
            QualifiedName { ns: ns.parse().unwrap_or(0), name: name.to_string() }
        } else {
            QualifiedName { ns: 0, name: text.to_string() }
        }
    }
    pub fn render(&self) -> String {
        format!("{}:{}", self.ns, self.name)
    }
}

/// Byte span of a source element inside the original XML file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub start_line: usize,
    pub end_line: usize,
}

/// A retained XML element (node element or any structured child).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawElement {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub text: Option<String>,
    pub children: Vec<RawElement>,
    pub span: Span,
    pub unknown: bool,
}

/// One UANode entry exactly as found in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub node_class: String,
    pub node_id_text: String,
    pub browse_name_text: String,
    pub attrs: BTreeMap<String, String>,
    pub references: Vec<RawRef>,
    pub aliases_used: Vec<String>,
    pub extensions: Vec<RawElement>,
    pub element: RawElement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRef {
    pub text: String,
    pub is_forward: bool,
    pub element: RawElement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDependency {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub element: RawElement,
}

/// Baseline of one imported NodeSet file: nothing is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFile {
    pub file_id: String,
    pub path: String,
    pub content: String,
    pub namespaces: Vec<NamespaceEntry>,
    pub aliases: Vec<AliasEntry>,
    pub models: Vec<ModelDependency>,
    pub nodes: Vec<RawNode>,
    pub top_extensions: Vec<RawElement>,
    pub parse_diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceEntry {
    pub index: u16,
    pub uri: String,
    pub element: RawElement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasEntry {
    pub alias: String,
    pub node_id_text: String,
    pub element: RawElement,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Structured diagnostic with source provenance and machine code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub file_id: Option<String>,
    pub span: Option<Span>,
    pub subject: Option<String>,
}

/// A quantity value: original representation always retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity {
    pub raw_value: String,
    pub raw_unit: String,
    pub unit_basis: String,
    pub si_value: Option<String>,
    pub si_unit: Option<String>,
    pub conversion_diagnostic: Option<String>,
}

// ---------------- Merged model ----------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GlobalNodeId {
    pub ns_uri: String,
    pub id: String,
    pub id_type: IdType,
}

impl GlobalNodeId {
    pub fn render(&self) -> String {
        let p = match self.id_type {
            IdType::Numeric => "i",
            IdType::String => "s",
            IdType::Guid => "g",
            IdType::ByteString => "b",
        };
        format!("{}|{}={}", self.ns_uri, p, self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    pub fn from_tag(tag: &str) -> Option<NodeKind> {
        Some(match tag {
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeOrigin {
    pub file_id: String,
    pub node_id_text: String,
    pub span: Span,
    pub ordinal: usize,
}

/// Canonical node in a merged model. Multiple origins => duplicate NodeId.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub gid: GlobalNodeId,
    pub stable_id: String,
    pub kind: NodeKind,
    pub browse_name: String,
    pub origins: Vec<NodeOrigin>,
    pub duplicate: bool,
    pub placeholder: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EdgeState {
    Resolved,
    Dangling,
}

/// Reference normalized to a forward-directed edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub stable_id: String,
    pub source: GlobalNodeId,
    pub target: GlobalNodeId,
    pub ref_type: GlobalNodeId,
    pub forward_in_source: bool,
    pub state: EdgeState,
    pub file_id: String,
    pub span: Span,
    pub ordinal: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasMapEntry {
    pub file_id: String,
    pub alias: String,
    pub target_text: String,
    pub target_gid: Option<GlobalNodeId>,
    pub cycle: Vec<String>,
    pub unresolved: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepRecord {
    pub file_id: String,
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub stable_id: String,
    pub file_id: String,
    pub parent_node: Option<GlobalNodeId>,
    pub tag: String,
    pub raw_xml: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedModel {
    pub version: u64,
    pub parent_version: Option<u64>,
    pub rule_version: String,
    pub files: Vec<String>,
    pub namespace_uris: Vec<String>,
    pub namespace_map: Vec<FileNsMap>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub aliases: Vec<AliasMapEntry>,
    pub dependencies: Vec<DepRecord>,
    pub extensions: Vec<UnknownExtension>,
    pub diagnostics: Vec<Diagnostic>,
    pub input_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileNsMap {
    pub file_id: String,
    pub local_to_global: BTreeMap<u16, u16>,
}

// ---------------- Diff ----------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    pub subject: String,
    pub detail: String,
    pub stable_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDiff {
    pub from_version: Option<u64>,
    pub to_version: u64,
    pub changes: Vec<Change>,
}

/// Deterministic stable id from arbitrary parts.
pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(prefix.as_bytes());
    for p in parts {
        h.update([0]);
        h.update(p.as_bytes());
    }
    format!("{}_{}", prefix, &hex12(&h.finalize()))
}

fn hex12(bytes: &[u8]) -> String {
    bytes.iter().take(12).map(|b| format!("{:02x}", b)).collect()
}
