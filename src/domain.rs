//! Domain model: parsed NodeSet files preserved close to source XML.
//!
//! Nothing parsed here is dropped: raw XML nodes (including unrecognized
//! extensions), namespace tables, NodeIds, BrowseNames, references, aliases
//! and model dependencies are all retained with source spans.

use crate::xmlparser::Node;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum IdType {
    Numeric,
    String,
    Guid,
    Opaque,
}

/// An OPC UA NodeId as written in one file. Numeric namespace indices are
/// file-local and stay file-local at this layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct NodeId {
    pub ns: u16,
    pub id_type: IdType,
    pub value: String,
    pub raw: String,
    /// true when the token was an Alias resolved by name (ns/id record the target)
    pub via_alias: Option<String>,
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct QualifiedName {
    pub ns: u16,
    pub name: String,
    pub raw: String,
}

/// A reference declared on a node. `is_forward` is taken straight from XML
/// (default true). Normalization into a directed edge happens in the merge
/// layer; this layer never invents or deletes edges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub reference_type: NodeId,
    pub target: NodeId,
    pub is_forward: bool,
    pub raw_is_forward: String,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawValue {
    pub text: String,
    pub span: (usize, usize),
}

/// A physical quantity / coordinate. Domain values carry a unit or coordinate
/// datum and keep their original representation; canonical SI normalization
/// happens only on the derived layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity {
    pub value: String,
    pub unit_or_datum: String,
    pub kind: String,
    pub raw: String,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub node_class: String,
    pub node_id: NodeId,
    pub browse_name: QualifiedName,
    pub display_name: String,
    pub references: Vec<RawReference>,
    pub value: Option<RawValue>,
    pub quantities: Vec<Quantity>,
    /// every recognized element name (References, DisplayName, Value ...)
    pub recognized_children: Vec<String>,
    /// unrecognized child elements preserved verbatim
    pub unknown_extensions: Vec<UnknownExtension>,
    pub span: (usize, usize),
    pub node_id_span: (usize, usize),
    pub browse_name_span: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub element: String,
    pub raw: String,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTableEntry {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required_by: Option<NodeId>,
    pub access_restrictions: Option<String>,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceUri {
    pub index: u16,
    pub uri: String,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    pub alias: String,
    pub target: String,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedFile {
    pub logical_name: String,
    pub xml: String,
    pub namespaces: Vec<NamespaceUri>,
    pub models: Vec<ModelTableEntry>,
    pub aliases: Vec<Alias>,
    pub nodes: Vec<RawNode>,
    pub parse_diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub span: (usize, usize),
}
