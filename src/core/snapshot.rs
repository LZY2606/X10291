//! Canonical (URI-remapped) and derived representation.
//!
//! Raw layer keeps file-local numeric indexes. The canonical layer rewrites
//! every NodeId to `<namespaceUri>#<identifier>` so that identity is stable
//! regardless of the numeric slot a file happened to use.

use serde::{Deserialize, Serialize};

use crate::core::events::Event;
use crate::core::model::{ParsedSource, RawNode, RawValue, Span};

/// Canonical, file-independent NodeId.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CanonicalId {
    pub ns_uri: String,
    pub id_type: String,
    pub identifier: String,
}

impl CanonicalId {
    pub fn new(ns_uri: impl Into<String>, id_type: impl Into<String>, id: impl Into<String>) -> Self {
        CanonicalId { ns_uri: ns_uri.into(), id_type: id_type.into(), identifier: id.into() }
    }
    pub fn key(&self) -> String {
        format!("{}#{}:{}", self.ns_uri, self.id_type, self.identifier)
    }
    pub fn parse(key: &str) -> Option<Self> {
        let hash = key.find('#')?;
        let colon = key[hash + 1..].find(':').map(|i| hash + 1 + i)?;
        Some(CanonicalId {
            ns_uri: key[..hash].to_string(),
            id_type: key[hash + 1..colon].to_string(),
            identifier: key[colon + 1..].to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceMapping {
    pub source_sha: String,
    pub file_index: u16,
    pub uri: String,
    pub merged_index: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_sha: String,
    pub filename: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalNode {
    pub id: CanonicalId,
    pub class: String,
    pub browse_name: Option<String>,
    pub display_name: Option<String>,
    pub raw: RawNode,
    pub origins: Vec<Provenance>,
    /// When the same canonical id is declared by multiple sources: ids of the
    /// competing origins. First is adopted; rest retained as candidates.
    pub duplicates: Vec<Provenance>,
}

/// Forward-only, normalized edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub ref_type: String,
    /// `forward` or `inverse`.
    pub direction: String,
    /// True when target node does not exist in any source (dangling).
    pub dangling: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedNode {
    pub id: CanonicalId,
    pub reasons: Vec<String>,
    pub referenced_by: Vec<String>,
    pub origin: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub uri: String,
    pub required_version: String,
    pub provided_version: Option<String>,
    pub state: String, // satisfied | version_conflict | missing
    pub origin: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasCycleState {
    Resolved,
    InCycle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasReport {
    pub source_sha: String,
    pub alias: String,
    pub state: AliasCycleState,
    pub cycle: Vec<String>,
    pub resolved_target: Option<String>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtensionReport {
    pub tag: String,
    pub raw_xml: String,
    pub origin: Provenance,
}

/// A value with a unit / coordinate basis. Raw representation is always kept;
/// derived normalization adds SI when a safe conversion exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantifiedValue {
    pub node: String,
    pub raw: RawValue,
    pub si_value: Option<String>,
    pub si_unit: Option<String>,
    pub converted: bool,
    pub diagnostics: Vec<String>,
    pub origin: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedModel {
    pub sources: Vec<ParsedSource>,
    pub namespace_mappings: Vec<NamespaceMapping>,
    pub merged_namespaces: Vec<String>,
    pub nodes: Vec<CanonicalNode>,
    pub edges: Vec<Edge>,
    pub unresolved: Vec<UnresolvedNode>,
    pub dependencies: Vec<DependencyStatus>,
    pub aliases: Vec<AliasReport>,
    pub unknown_extensions: Vec<UnknownExtensionReport>,
    pub events: Vec<Event>,
}

/// One entry in a type hierarchy (subtype -> supertype), forward normalized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyLink {
    pub child: String,
    pub parent: String,
    pub ref_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecomputeReportItem {
    pub key: String,
    pub state: String, // recomputed | still_valid
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedSnapshot {
    pub hierarchy: Vec<HierarchyLink>,
    pub quantified: Vec<QuantifiedValue>,
    /// Transitive subtype+instance closure materialized per node for the UI.
    pub closure: Vec<ClosureSet>,
    pub recompute: Vec<RecomputeReportItem>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosureSet {
    pub node: String,
    pub supertypes: Vec<String>,
    pub instances: Vec<String>,
}
