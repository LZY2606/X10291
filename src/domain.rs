//! 领域模型：原始 NodeSet XML（原样保留）+ 派生层（合并/规范化结果）。
//!
//! 设计原则：
//! * 原始层 (`Raw*`) 不做任何语义改写，每个显示值都携带可回溯源 XML 的 [`SrcRange`]。
//! * 派生层 (`D*`) 只由原始层 + 已接受决定推出；不确定时保留全部候选，绝不吞解。

use serde::{Deserialize, Serialize};

/// 源 XML 中的字节区间（半开区间 [start,end)），稳定身份可追回到具体片段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrcRange {
    pub file: String,
    pub start: usize,
    pub end: usize,
}

impl SrcRange {
    pub fn new(file: impl Into<String>, start: usize, end: usize) -> Self {
        Self { file: file.into(), start, end }
    }
}

/// 属性值及其在 XML 中的字节范围。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttrRange {
    pub value: String,
    pub range: SrcRange,
}

/// 顶层简单文本子元素（NodeIdLastModified 等非关键但必须保留的内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiscText {
    pub name: String,
    pub text: String,
    pub range: SrcRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceUri {
    /// 在本文件 NamespaceArray 中的序号（0 基，不含 OPC UA 内建 ns=0）。
    pub decl_index: usize,
    pub uri: AttrRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequirement {
    pub model_uri: AttrRange,
    pub version: Option<AttrRange>,
    pub publication_date: Option<AttrRange>,
    pub range: SrcRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasEntry {
    pub name: AttrRange,
    pub target: AttrRange,
    pub range: SrcRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Object => "Object",
            NodeKind::Variable => "Variable",
            NodeKind::Method => "Method",
            NodeKind::ObjectType => "ObjectType",
            NodeKind::VariableType => "VariableType",
            NodeKind::ReferenceType => "ReferenceType",
            NodeKind::DataType => "DataType",
            NodeKind::View => "View",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub text: AttrRange,
    pub reftype: AttrRange,
    pub forward: bool,
    pub range: SrcRange,
}

/// 未识别扩展：原始 XML 逐字节保留，不做解释即不丢信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawExtension {
    /// 原始限定名（prefix:Local），逐字节保真。
    pub name: String,
    /// 元素所在 XML 命名空间 URI（按 xmlns 声明解析）。
    pub ns_uri: Option<String>,
    /// 属于 NodeSet 标准或文件已声明命名空间的扩展为 true；否则为未识别扩展。
    pub recognized: bool,
    pub xml: String,
    pub range: SrcRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub kind: NodeKind,
    pub node_id: AttrRange,
    pub browse_name: AttrRange,
    pub display_name: Option<AttrRange>,
    pub data_type: Option<AttrRange>,
    pub references: Vec<RawReference>,
    pub extensions: Vec<RawExtension>,
    pub range: SrcRange,
}

/// 单个 NodeSet XML 文件解析后的原始档案（基线材料，永不被派生层改写）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeSet {
    pub file: String,
    pub content: String,
    pub namespaces: Vec<NamespaceUri>,
    /// 本文件自身提供的 <Model>（ModelUri/Version/PublicationDate）。
    pub self_models: Vec<ModelRequirement>,
    /// 声明的 <RequiredModel> 依赖。
    pub required_models: Vec<ModelRequirement>,
    pub aliases: Vec<AliasEntry>,
    pub nodes: Vec<RawNode>,
    /// 文件级未识别/厂商扩展（顶层）。
    pub file_extensions: Vec<RawExtension>,
    pub misc: Vec<MiscText>,
}

// ----------------------------------------------------------------------------
// 派生层
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdType {
    Numeric,
    String,
    Guid,
    Opaque,
}

/// 规范化 NodeId。ns=0 表示 OPC UA 内建；其余为全局 namespace 序号（按 URI 重映射）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub ns: u32,
    pub id: String,
    pub id_type: IdType,
}

impl NodeId {
    pub fn canonical(&self) -> String {
        match (self.ns, self.id_type) {
            (0, IdType::Numeric) => format!("i={}", self.id),
            (0, IdType::String) => format!("s={}", self.id),
            (0, IdType::Guid) => format!("g={}", self.id),
            (0, IdType::Opaque) => format!("b={}", self.id),
            (n, IdType::Numeric) => format!("ns={n};i={}", self.id),
            (n, IdType::String) => format!("ns={n};s={}", self.id),
            (n, IdType::Guid) => format!("ns={n};g={}", self.id),
            (n, IdType::Opaque) => format!("ns={n};b={}", self.id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QualifiedName {
    pub ns: u32,
    pub name: String,
}

impl QualifiedName {
    pub fn canonical(&self) -> String {
        format!("{}:{}", self.ns, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity {
    pub node_id: String,
    pub property: String,
    pub raw: String,
    pub unit: Option<String>,
    pub datum: Option<String>,
    /// 派生层规范化结果（SI/基准坐标）；无法转换时为 None 且 raw 原地保留。
    pub normalized: Option<String>,
    pub convertible: bool,
    pub diagnostic: Option<String>,
    pub range: SrcRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DNode {
    /// 稳定身份：`node:` + canonical NodeId。
    pub id: String,
    pub node_id: NodeId,
    pub browse_name: QualifiedName,
    pub display_name: Option<String>,
    pub kind: NodeKind,
    pub data_type: Option<String>,
    pub defining_file: String,
    pub defining_range: SrcRange,
    pub all_sources: Vec<String>,
    pub quantities: Vec<Quantity>,
    pub extensions: Vec<RawExtension>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DEdge {
    /// 稳定身份：源|类型|目标|方向 规范化后哈希无关的拼接。
    pub id: String,
    pub source: String,
    pub target: String,
    pub ref_type: String,
    /// true 表示原 XML 是 IsForward=false，规范化时翻转得到，不制造缺边。
    pub from_inverse: bool,
    pub origin_range: SrcRange,
    pub origin_file: String,
    pub unresolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedRef {
    pub edge_id: String,
    pub source: String,
    pub target_text: String,
    pub ref_type: String,
    pub reason: String,
    pub candidates: Vec<String>,
    pub range: SrcRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub required_by: String,
    pub range: SrcRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownExtRecord {
    pub owner: String,
    pub name: String,
    pub xml: String,
    pub range: SrcRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictKind {
    DuplicateNodeId,
    VersionConflict,
    AliasCycle,
}

impl ConflictKind {
    pub fn label(self) -> &'static str {
        match self {
            ConflictKind::DuplicateNodeId => "重复 NodeId",
            ConflictKind::VersionConflict => "依赖版本冲突",
            ConflictKind::AliasCycle => "alias 循环",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    pub file: String,
    pub range: SrcRange,
    pub adopted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub kind: ConflictKind,
    pub key: String,
    pub summary: String,
    pub candidates: Vec<Candidate>,
    /// 未裁定前为 None；裁定后记录被采纳候选，未采纳候选原样保留。
    pub adopted: Option<String>,
    pub rule: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventLevel {
    Info,
    Warning,
    Error,
    Decision,
    Recompute,
}

impl EventLevel {
    pub fn label(self) -> &'static str {
        match self {
            EventLevel::Info => "info",
            EventLevel::Warning => "warning",
            EventLevel::Error => "error",
            EventLevel::Decision => "decision",
            EventLevel::Recompute => "recompute",
        }
    }
}

/// 机器可读的判定事件：rule 是规则码，basis 是结构化判断依据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub level: EventLevel,
    pub rule: String,
    pub message: String,
    pub basis: serde_json::Value,
    pub sources: Vec<SrcRange>,
    pub objects: Vec<String>,
    /// 该事件属于哪个推导版本（回滚旧事件时恢复当时结论）。
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceEntry {
    /// 全局序号（OPC UA 内建固定 0；业务 URI 从 1 开始，按首次出现稳定分配）。
    pub global_ns: u32,
    pub uri: String,
    /// (file, 该文件内 local ns) —— 证明“数字 index 只在单文件内有意义”。
    pub bindings: Vec<(String, u32)>,
    pub declared_ranges: Vec<SrcRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasStatus {
    pub file: String,
    pub name: String,
    pub target_text: String,
    pub resolved: Option<String>,
    pub cycle: Vec<String>,
    pub range: SrcRange,
}

/// 一次完整推导产生的不可变版本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedVersion {
    pub version: u64,
    pub parent: Option<u64>,
    pub label: String,
    pub cause_event: Option<u64>,
    pub namespaces: Vec<NamespaceEntry>,
    pub nodes: Vec<DNode>,
    pub edges: Vec<DEdge>,
    pub unresolved_refs: Vec<UnresolvedRef>,
    pub unresolved_models: Vec<UnresolvedModel>,
    pub unknown_extensions: Vec<UnknownExtRecord>,
    pub conflicts: Vec<Conflict>,
    pub aliases: Vec<AliasStatus>,
    pub events: Vec<Event>,
}
