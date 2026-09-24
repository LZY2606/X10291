use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceFile {
    pub id: String,
    pub filename: String,
    pub stage: String,
    pub model_uri: String,
    pub model_version: String,
    pub ord: i64,
    pub xml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XmlSpan {
    pub start_offset: usize,
    pub end_offset: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawAttribute {
    pub namespace: String,
    pub prefix: String,
    pub local_name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawElement {
    pub id: String,
    pub source_id: String,
    pub parent_id: Option<String>,
    pub ord: i64,
    pub namespace: String,
    pub prefix: String,
    pub local_name: String,
    pub attributes: Vec<RawAttribute>,
    pub text: String,
    pub span: XmlSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawNode {
    pub id: String,
    pub source_id: String,
    pub element_id: String,
    pub node_class: String,
    pub node_id_raw: String,
    pub browse_name_raw: String,
    pub display_name: String,
    pub ord: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawReference {
    pub id: String,
    pub source_id: String,
    pub element_id: String,
    pub source_node_raw: String,
    pub target_raw: String,
    pub reference_type_raw: String,
    pub is_inverse: bool,
    pub text: String,
    pub ord: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawAlias {
    pub id: String,
    pub source_id: String,
    pub element_id: String,
    pub name: String,
    pub target_raw: String,
    pub ord: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawDependency {
    pub id: String,
    pub source_id: String,
    pub element_id: String,
    pub model_uri: String,
    pub version: String,
    pub kind: String,
    pub ord: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawQuantity {
    pub id: String,
    pub source_id: String,
    pub element_id: String,
    pub node_raw: String,
    pub kind: String,
    pub original_repr: String,
    pub unit: String,
    pub coordinate_frame: Option<String>,
    pub ord: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParsedSource {
    pub source: SourceFile,
    pub elements: Vec<RawElement>,
    pub nodes: Vec<RawNode>,
    pub references: Vec<RawReference>,
    pub aliases: Vec<RawAlias>,
    pub dependencies: Vec<RawDependency>,
    pub namespace_uris: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedCase {
    pub sources: Vec<ParsedSource>,
    pub quantities: Vec<RawQuantity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct GlobalNodeId {
    pub namespace_uri: String,
    pub id_type: String,
    pub identifier: String,
}

impl GlobalNodeId {
    pub fn stable_key(&self) -> String {
        format!("{}|{}|{}", self.namespace_uri, self.id_type, self.identifier)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRange {
    pub source_id: String,
    pub filename: String,
    pub element_id: String,
    pub span: XmlSpan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    pub node_id: GlobalNodeId,
    pub raw_node_id: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventRecord {
    pub id: String,
    pub seq: i64,
    pub kind: String,
    pub severity: String,
    pub status: String,
    pub rule: String,
    pub basis_code: String,
    pub message: String,
    pub subject_id: String,
    pub source_ranges: Vec<SourceRange>,
    pub candidates: Vec<Candidate>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRecord {
    pub id: String,
    pub event_id: String,
    pub selected_candidate_id: Option<String>,
    pub rejected_candidate_ids: Vec<String>,
    pub rationale: String,
    pub rule_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedNode {
    pub id: String,
    pub global_id: GlobalNodeId,
    pub node_class: String,
    pub browse_name: String,
    pub display_name: String,
    pub raw_node_ids: Vec<String>,
    pub source_ranges: Vec<SourceRange>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedEdge {
    pub id: String,
    pub from_node: String,
    pub to_node: String,
    pub reference_type: GlobalNodeId,
    pub raw_reference_id: String,
    pub source_ranges: Vec<SourceRange>,
    pub normalized_from_inverse: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnresolvedRecord {
    pub id: String,
    pub raw_value: String,
    pub reason: String,
    pub subject_kind: String,
    pub source_ranges: Vec<SourceRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuantityDiagnostic {
    pub id: String,
    pub raw_quantity_id: String,
    pub original_repr: String,
    pub normalized_repr: Option<String>,
    pub normalized_unit: Option<String>,
    pub coordinate_frame: Option<String>,
    pub status: String,
    pub diagnostic: Option<String>,
    pub source_ranges: Vec<SourceRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceMapping {
    pub source_id: String,
    pub filename: String,
    pub local_index: usize,
    pub namespace_uri: String,
    pub global_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuildResult {
    pub version_id: String,
    pub parent_version_id: Option<String>,
    pub label: String,
    pub kind: String,
    pub rules_version: String,
    pub parsed: ParsedCase,
    pub namespaces: Vec<NamespaceMapping>,
    pub nodes: Vec<DerivedNode>,
    pub edges: Vec<DerivedEdge>,
    pub unresolved: Vec<UnresolvedRecord>,
    pub quantities: Vec<QuantityDiagnostic>,
    pub events: Vec<EventRecord>,
    pub decisions: Vec<DecisionRecord>,
    pub fingerprints: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticDiff {
    pub id: String,
    pub kind: String,
    pub stable_id: String,
    pub label: String,
    pub left: Option<serde_json::Value>,
    pub right: Option<serde_json::Value>,
    pub source_range: Option<SourceRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaseSnapshot {
    pub scenario_id: String,
    pub versions: Vec<BuildResult>,
    pub current_version_id: String,
}
