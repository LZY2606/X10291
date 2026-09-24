use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XmlNode {
    pub kind: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attrs: Vec<XmlAttr>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<XmlNode>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XmlAttr {
    pub name: String,
    pub value: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceDecl {
    pub index: u16,
    pub uri: String,
    pub raw: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AliasDef {
    pub name: String,
    pub target: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawReference {
    pub text: String,
    pub ref_type: String,
    pub is_forward: bool,
    pub target: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub key: String,
    pub raw_value: String,
    pub raw_unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_value: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub normalized_unit: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub diagnostic: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnknownExtension {
    pub path: String,
    pub reason: String,
    pub xml: XmlNode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeDef {
    pub kind: String,
    pub raw_node_id: String,
    pub namespace_index: u16,
    pub identifier: String,
    pub browse_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub references: Vec<RawReference>,
    pub quantities: Vec<Quantity>,
    pub unknown_extensions: Vec<UnknownExtension>,
    pub xml: XmlNode,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelDependency {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<String>,
    pub source_id: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<String>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSource {
    pub id: String,
    pub path: String,
    pub label: String,
    pub content: String,
    pub namespaces: Vec<NamespaceDecl>,
    pub aliases: Vec<AliasDef>,
    pub models: Vec<ModelInfo>,
    pub required_models: Vec<ModelDependency>,
    pub nodes: Vec<NodeDef>,
    pub document_extensions: Vec<UnknownExtension>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceEntry {
    pub merged_index: u16,
    pub uri: String,
    pub source_id: String,
    pub local_index: u16,
    pub raw: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entity {
    pub id: String,
    pub global_node_id: String,
    pub uri: String,
    pub identifier: String,
    pub browse_name: String,
    pub display_name: String,
    pub kind: String,
    pub status: String,
    pub source_ids: Vec<String>,
    pub raw_node_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quantities: Vec<Quantity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edge {
    pub id: String,
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub reference_type_id: String,
    pub reference_type_label: String,
    pub normalized_direction: String,
    pub status: String,
    pub source_ids: Vec<String>,
    pub raw_reference_ids: Vec<String>,
    pub source_range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnresolvedRecord {
    pub id: String,
    pub kind: String,
    pub raw: String,
    pub reason: String,
    pub source_id: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyStatus {
    pub uri: String,
    pub required_version: String,
    pub available_version: String,
    pub status: String,
    pub source_id: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecomputeTrace {
    pub object_id: String,
    pub status: String,
    pub reason: String,
    pub changed_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffEntry {
    pub id: String,
    pub kind: String,
    pub object_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
    pub source_ranges: Vec<SourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRef {
    pub source_id: String,
    pub path: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaseEvent {
    pub id: String,
    pub seq: i64,
    pub version_id: String,
    pub branch_id: String,
    pub kind: String,
    pub level: String,
    pub summary: String,
    pub basis: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    pub payload: serde_json::Value,
    pub source_ranges: Vec<SourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Derivation {
    pub version_id: String,
    pub branch_id: String,
    pub parent_version_id: Option<String>,
    pub label: String,
    pub changed_sources: Vec<String>,
    pub namespaces: Vec<NamespaceEntry>,
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
    pub unresolved: Vec<UnresolvedRecord>,
    pub dependencies: Vec<DependencyStatus>,
    pub diffs: Vec<DiffEntry>,
    pub traces: Vec<RecomputeTrace>,
    pub events: Vec<CaseEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInput {
    pub id: String,
    pub path: String,
    pub label: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInput {
    pub id: String,
    pub branch_id: String,
    pub label: String,
    pub parent_id: Option<String>,
    pub changed_sources: Vec<String>,
    pub sources: Vec<SourceInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionInput {
    pub base_version_id: String,
    pub branch_id: String,
    pub summary: String,
    pub accepted_candidate_id: String,
    pub rejected_candidate_id: String,
    pub rationale: String,
}
