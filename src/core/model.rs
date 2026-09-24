//! Immutable domain values. Nothing here is "normalised" beyond what parsing
//! requires: raw representations are kept verbatim and normalisation happens
//! only in the derived layer (`derive`).

use serde::{Deserialize, Serialize};

/// Byte range inside an imported source file, 1-based line/col for humans.
/// Every displayed value carries a stable identity and can be traced back to
/// one of these ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub file_id: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
}

impl SourceRange {
    pub fn label(&self) -> String {
        format!("{}:{}:{}", self.file_id, self.start_line, self.start_col)
    }
}

/// One entry of a file's `<NamespaceTable>` (numeric index -> URI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceUri {
    pub index: u16,
    pub uri: String,
    pub range: SourceRange,
}

/// `<Aliases><Alias Name="...">value</Alias>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alias {
    pub name: String,
    pub node_id_text: String,
    pub range: SourceRange,
}

/// `<RequiredModel ModelUri=... Version=... PublicationDate=.../>` — the model
/// dependency is preserved even when the referenced model is absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDependency {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub range: SourceRange,
}

/// A UANode child element that the tool does not model semantically
/// (extensions, vendor elements, ...). Kept verbatim, never dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub ext_id: String,
    pub local_name: String,
    pub namespace: String,
    pub raw_xml: String,
    pub range: SourceRange,
}

/// Literal `<Reference IsForward="...">NodeId</Reference>` exactly as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub reference_type_text: String,
    pub target_node_id_text: String,
    pub is_forward: bool,
    pub range: SourceRange,
}

/// Scalar attribute carrying a raw representation plus an optional engineering
/// unit / coordinate basis declared by an (unknown) extension on the node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAttribute {
    pub name: String,
    pub value: String,
    pub range: SourceRange,
}

/// The node class discriminator present in NodeSet XML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum NodeClass {
    Object,
    ObjectType,
    Variable,
    VariableType,
    ReferenceType,
    DataType,
    Method,
    View,
}

impl NodeClass {
    pub fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "UAObject" => NodeClass::Object,
            "UAObjectType" => NodeClass::ObjectType,
            "UAVariable" => NodeClass::Variable,
            "UAVariableType" => NodeClass::VariableType,
            "UAReferenceType" => NodeClass::ReferenceType,
            "UADataType" => NodeClass::DataType,
            "UAMethod" => NodeClass::Method,
            "UAView" => NodeClass::View,
            _ => return None,
        })
    }
}

/// One `<UA*>` element with everything preserved, including raw XML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    /// File-local stable id, e.g. `f3#n7` — survives re-import and re-order.
    pub raw_id: String,
    pub node_class: NodeClass,
    pub node_id_text: String,
    pub browse_name_text: String,
    pub display_name: Option<String>,
    pub attributes: Vec<RawAttribute>,
    pub references: Vec<RawReference>,
    pub extensions: Vec<UnknownExtension>,
    /// Engineering unit / coordinate basis attached via a `<uax:Unit>` or
    /// `<uax:Basis>` extension: raw value kept, normalised in derived layer.
    pub unit: Option<MeasureBasis>,
    pub raw_xml: String,
    pub range: SourceRange,
}

/// Raw measurement/coordinate annotation. `raw` is the original lexical value;
/// `system` is the unit/coordinate system URI. Conversion (if any) happens in
/// the derived layer and unconvertible values stay here with a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasureBasis {
    pub kind: MeasureKind,
    pub system: String,
    pub raw: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeasureKind {
    Unit,
    CoordinateBasis,
}

/// Everything extracted from one NodeSet XML file, in document order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub file_id: String,
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    /// Default namespace URI for the file (`xmlns` of `<UANodeSet>` may also
    /// appear; we capture all declarations in `xml_namespaces`).
    pub default_namespace_uri: Option<String>,
    pub namespaces: Vec<NamespaceUri>,
    pub aliases: Vec<Alias>,
    pub models: Vec<ModelDependency>,
    pub nodes: Vec<RawNode>,
    /// Parser diagnostics are attached to inputs, never silently fatal.
    pub parse_diagnostics: Vec<ParseDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseDiagnostic {
    pub code: String,
    pub message: String,
    pub range: SourceRange,
}

/// A parsed NodeId in its *file-local* numeric namespace form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeIdValue {
    Numeric { ns: u16, id: u64 },
    String { ns: u16, id: String },
    Guid { ns: u16, id: String },
    Opaque { ns: u16, id: String },
}

impl NodeIdValue {
    pub fn ns(&self) -> u16 {
        match self {
            NodeIdValue::Numeric { ns, .. }
            | NodeIdValue::String { ns, .. }
            | NodeIdValue::Guid { ns, .. }
            | NodeIdValue::Opaque { ns, .. } => *ns,
        }
    }
    pub fn id_part(&self) -> String {
        match self {
            NodeIdValue::Numeric { id, .. } => id.to_string(),
            NodeIdValue::String { id, .. }
            | NodeIdValue::Guid { id, .. }
            | NodeIdValue::Opaque { id, .. } => id.clone(),
        }
    }
}

/// QualifiedName `<namespaceIndex>:<name>` kept raw and split.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualifiedName {
    pub ns: u16,
    pub name: String,
    pub raw: String,
}
