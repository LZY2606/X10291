use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRange {
    pub source_id: String,
    pub file_name: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawAttribute {
    pub name: String,
    pub xml_namespace: String,
    pub value: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawNode {
    pub stable_id: String,
    pub source_id: String,
    pub file_name: String,
    pub start: usize,
    pub end: usize,
    pub depth: usize,
    pub kind: String,
    pub xml_namespace: String,
    pub local_name: String,
    pub attributes: Vec<RawAttribute>,
    pub text: String,
    pub parent_stable_id: Option<String>,
    pub raw_xml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceDocument {
    pub source_id: String,
    pub file_name: String,
    pub content: String,
    pub content_sha: String,
    pub ordinal: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Alias {
    pub name: String,
    pub raw_value: String,
    pub resolved_node_id: Option<String>,
    pub state: AliasState,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AliasState {
    Resolved,
    Cycle { path: Vec<String> },
    Unresolved { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceEntry {
    pub file_index: u32,
    pub uri: String,
    pub stable_id: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRequirement {
    pub model_uri: String,
    pub required_version: String,
    pub actual_version: Option<String>,
    pub state: DependencyState,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependencyState {
    Satisfied,
    Missing,
    VersionConflict { expected: String, actual: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedNode {
    pub stable_id: String,
    pub source_id: String,
    pub file_name: String,
    pub raw_node_id: String,
    pub kind: String,
    pub original_node_id: String,
    pub expanded_node_id: String,
    pub browse_name_original: String,
    pub browse_name: QualifiedName,
    pub display_name: String,
    pub range: SourceRange,
    pub duplicates: Vec<String>,
    pub quantities: Vec<Quantity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QualifiedName {
    pub namespace_uri: String,
    pub name: String,
    pub original: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Quantity {
    pub stable_id: String,
    pub value_original: String,
    pub value_normalized: Option<f64>,
    pub unit_original: String,
    pub unit_normalized: Option<String>,
    pub coordinate_basis: Option<String>,
    pub state: QuantityState,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuantityState {
    OriginalOnly { reason: String },
    Normalized,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    pub stable_id: String,
    pub source_node: String,
    pub target_node: String,
    pub reference_type_original: String,
    pub reference_type: String,
    pub direction_original: bool,
    pub direction: String,
    pub state: ReferenceState,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReferenceState {
    Resolved,
    Dangling { missing_endpoint: String },
    UnresolvedAlias { alias: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnresolvedNode {
    pub stable_id: String,
    pub expanded_node_id: String,
    pub referenced_by: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedFact {
    pub stable_id: String,
    pub kind: String,
    pub status: FactStatus,
    pub depends_on: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FactStatus {
    Valid,
    Recomputed,
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompassEvent {
    pub event_id: String,
    pub version_id: i64,
    pub code: String,
    pub severity: String,
    pub adopted: bool,
    pub subject_id: String,
    pub message: String,
    pub basis: String,
    pub source_range: Option<SourceRange>,
    pub evidence: BTreeMap<String, String>,
    pub rule_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelSnapshot {
    pub namespaces: Vec<NamespaceEntry>,
    pub aliases: Vec<Alias>,
    pub requirements: Vec<ModelRequirement>,
    pub nodes: Vec<ParsedNode>,
    pub references: Vec<Reference>,
    pub unresolved_nodes: Vec<UnresolvedNode>,
    pub raw_nodes: Vec<RawNode>,
    pub facts: Vec<DerivedFact>,
    pub events: Vec<CompassEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticChange {
    pub stable_id: String,
    pub kind: String,
    pub severity: String,
    pub subject_id: String,
    pub before: String,
    pub after: String,
    pub cause_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleParameters {
    pub rule_version: String,
    pub pressure_unit: String,
}

impl Default for RuleParameters {
    fn default() -> Self {
        Self {
            rule_version: "rules-2026.09.1".to_string(),
            pressure_unit: "bar".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VersionRecord {
    pub version_id: i64,
    pub parent_id: Option<i64>,
    pub label: String,
    pub kind: String,
    pub status: String,
    pub parameters: RuleParameters,
    pub branch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceInput {
    pub file_name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportRequest {
    pub label: String,
    #[serde(default)]
    pub sources: Vec<SourceInput>,
    #[serde(default)]
    pub parameters: Option<RuleParameters>,
    pub expected_version: Option<i64>,
    pub branch_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRequest {
    pub expected_version: i64,
    pub decision: String,
    pub candidate_fact_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub parameters: Option<RuleParameters>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictReport {
    pub code: String,
    pub message: String,
    pub expected_version: i64,
    pub current_version: i64,
    pub resolvable_candidates: Vec<i64>,
}
