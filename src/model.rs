//! 核心领域模型。
//!
//! 设计原则：导入时“原样保留”，派生层才做规范化。这里的类型既保存原始表示
//! （`raw_*` 文本、来源范围），也保存解析结果；无法转换的量留在原位置并附诊断。

use serde::{Deserialize, Serialize};

/// 来源 XML 中的字节范围，保证界面可以回读原文片段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRange {
    pub start: usize,
    pub end: usize,
    /// 起始行（1 起），用于界面定位。
    pub start_line: usize,
    pub end_line: usize,
}

impl RawRange {
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// NodeId 的标识符类型。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Identifier {
    Numeric(u64),
    String_(String),
    Guid(String),
    ByteString(String),
    /// 无法解析的原始 NodeId 文本，原样保留并附诊断。
    Opaque(String),
}

impl Identifier {
    pub fn code(&self) -> &'static str {
        match self {
            Identifier::Numeric(_) => "i",
            Identifier::String_(_) => "s",
            Identifier::Guid(_) => "g",
            Identifier::ByteString(_) => "b",
            Identifier::Opaque(_) => "x",
        }
    }
    pub fn payload(&self) -> String {
        match self {
            Identifier::Numeric(n) => n.to_string(),
            Identifier::String_(s) => s.clone(),
            Identifier::Guid(s) => s.clone(),
            Identifier::ByteString(s) => s.clone(),
            Identifier::Opaque(s) => s.clone(),
        }
    }
}

/// `ns=X;i=Y` 的解析结果。`namespace_index` 只在“单文件内”有意义。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId {
    /// 原始文本，原样保留。
    pub raw: String,
    /// 文件内数字 namespace index（`ns=` 缺省为 0）。
    pub namespace_index: u32,
    pub identifier: Identifier,
}

impl NodeId {
    /// 解析一个 NodeId 文本。永远不 panic：无法解析时返回 `Opaque`。
    pub fn parse(text: &str) -> NodeId {
        let raw = text.to_string();
        let trimmed = text.trim();
        let mut ns: u32 = 0;
        let mut rest = trimmed;
        if let Some(rest_part) = trimmed.strip_prefix("ns=") {
            if let Some(semi) = rest_part.find(';') {
                if let Ok(v) = rest_part[..semi].parse::<u32>() {
                    ns = v;
                    rest = &rest_part[semi + 1..];
                }
            }
        }
        let identifier = if let Some(v) = rest.strip_prefix("i=") {
            v.parse::<u64>()
                .map(Identifier::Numeric)
                .unwrap_or_else(|_| Identifier::Opaque(rest.to_string()))
        } else if let Some(v) = rest.strip_prefix("s=") {
            Identifier::String_(v.to_string())
        } else if let Some(v) = rest.strip_prefix("g=") {
            Identifier::Guid(v.to_string())
        } else if let Some(v) = rest.strip_prefix("b=") {
            Identifier::ByteString(v.to_string())
        } else {
            Identifier::Opaque(rest.to_string())
        };
        NodeId {
            raw,
            namespace_index: ns,
            identifier,
        }
    }

    /// 文件内规范文本（用于别名表解析等）。
    pub fn local_text(&self) -> String {
        format!("ns={};{}={}", self.namespace_index, self.identifier.code(), self.identifier.payload())
    }
}

/// BrowseName：`ns=X;Name`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QualifiedName {
    pub raw: String,
    pub namespace_index: u32,
    pub name: String,
}

impl QualifiedName {
    pub fn parse(text: &str) -> QualifiedName {
        let raw = text.to_string();
        let trimmed = text.trim();
        if let Some(rest) = trimmed.strip_prefix("ns=") {
            if let Some(semi) = rest.find(';') {
                if let Ok(ns) = rest[..semi].parse::<u32>() {
                    return QualifiedName {
                        raw,
                        namespace_index: ns,
                        name: rest[semi + 1..].to_string(),
                    };
                }
            }
        }
        QualifiedName {
            raw,
            namespace_index: 0,
            name: trimmed.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeClass {
    ObjectType,
    VariableType,
    Object,
    Variable,
    ReferenceType,
    DataType,
    Method,
    View,
    /// 未识别的 NodeClass，原样保留。
    Unknown(String),
}

impl NodeClass {
    pub fn from_tag(tag: &str) -> NodeClass {
        match tag {
            "UAObjectType" => NodeClass::ObjectType,
            "UAVariableType" => NodeClass::VariableType,
            "UAObject" => NodeClass::Object,
            "UAVariable" => NodeClass::Variable,
            "UAReferenceType" => NodeClass::ReferenceType,
            "UADataType" => NodeClass::DataType,
            "UAMethod" => NodeClass::Method,
            "UAView" => NodeClass::View,
            other => NodeClass::Unknown(other.to_string()),
        }
    }
    pub fn label(&self) -> String {
        match self {
            NodeClass::ObjectType => "ObjectType".into(),
            NodeClass::VariableType => "VariableType".into(),
            NodeClass::Object => "Object".into(),
            NodeClass::Variable => "Variable".into(),
            NodeClass::ReferenceType => "ReferenceType".into(),
            NodeClass::DataType => "DataType".into(),
            NodeClass::Method => "Method".into(),
            NodeClass::View => "View".into(),
            NodeClass::Unknown(s) => s.clone(),
        }
    }
    pub fn is_type(&self) -> bool {
        matches!(
            self,
            NodeClass::ObjectType | NodeClass::VariableType | NodeClass::ReferenceType | NodeClass::DataType
        )
    }
}

/// 领域量：原始表示 + 可选规范化结果。无法转换时 `normalized` 为 None 并附诊断。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainQuantity {
    pub kind: String,
    pub raw_value: String,
    pub raw_unit: Option<String>,
    pub normalized_value: Option<f64>,
    pub normalized_unit: Option<String>,
    pub coordinate_basis: Option<String>,
    pub diagnostic: Option<String>,
    pub range: RawRange,
}

/// 单个原 XML 节点，完整保留。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawNode {
    /// 稳定身份（内容寻址，见 merge）。
    pub stable_id: String,
    /// 文件内序号，保持来源顺序。
    pub ord: usize,
    pub node_id: NodeId,
    pub browse_name: Option<QualifiedName>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub class: NodeClass,
    /// 额外属性（如 DataType、ValueRank、ArrayDimensions、EventNotifier 等）原值保留。
    #[serde(default)]
    pub attributes: Vec<(String, String)>,
    /// `<Value>` 原文（若存在），原样保留。
    pub value_xml: Option<String>,
    #[serde(default)]
    pub references: Vec<RawReference>,
    #[serde(default)]
    pub quantities: Vec<DomainQuantity>,
    /// 未识别扩展：保留外层 XML 原文。
    #[serde(default)]
    pub unknown_extensions: Vec<UnknownExtension>,
    pub range: RawRange,
}

/// 未识别扩展：不丢弃，整体保留并打标。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtension {
    pub name: String,
    pub outer_xml: String,
    pub range: RawRange,
}

/// 原 XML 中的 Reference（含 IsForward 方向信息）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub target: NodeId,
    pub reference_type: NodeId,
    /// `IsForward` 缺省为 true。
    pub is_forward: bool,
    /// 引用类型是通过别名书写时的别名名（用于 alias 循环诊断）。
    pub via_alias: Option<String>,
    pub range: RawRange,
}

/// NamespaceTable 中的一个 URI（文件内 index 1..n）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceEntry {
    pub file_index: u32,
    pub uri: String,
    pub range: RawRange,
}

/// Models/Model 中的 RequiredModel 依赖。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDependency {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub range: RawRange,
}

/// 当前文件声明的模型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDecl {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub range: RawRange,
    #[serde(default)]
    pub required: Vec<ModelDependency>,
}

/// 单份 NodeSet 文件解析结果（保真基线）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawDocument {
    /// 来源稳定 id，例如 s0001。
    pub source_id: String,
    pub filename: String,
    pub raw_xml: String,
    /// 该文件 `NamespaceTable`：index 0 隐式 OPC UA；这里只列 1..n。
    #[serde(default)]
    pub namespaces: Vec<NamespaceEntry>,
    #[serde(default)]
    pub aliases: Vec<AliasEntry>,
    #[serde(default)]
    pub models: Vec<ModelDecl>,
    #[serde(default)]
    pub nodes: Vec<RawNode>,
}

impl RawDocument {
    /// 按文件内 namespace index 解析 URI。index 0 为 OPC UA 基础命名空间。
    pub fn uri_for_index(&self, index: u32) -> Option<&str> {
        if index == 0 {
            return Some(OPC_UA_NS);
        }
        self.namespaces
            .iter()
            .find(|n| n.file_index == index)
            .map(|n| n.uri.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasEntry {
    pub alias: String,
    pub node_id: NodeId,
    pub range: RawRange,
}

pub const OPC_UA_NS: &str = "http://opcfoundation.org/UA/";

/// 解析期产生的诊断（不阻断导入）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseDiagnostic {
    pub code: String,
    pub message: String,
    pub source_id: Option<String>,
    pub range: Option<RawRange>,
}
