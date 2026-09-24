//! 核心领域模型：原始解析事实、规范化身份、事件与判定。
//!
//! 设计原则：
//! - 导入层只做"保真"：原 XML 节点、namespace table、NodeId、BrowseName、
//!   reference、alias、model dependency、未识别扩展全部按原始表示保留。
//! - 派生层才做规范化；数字 namespace index 只在单文件内有意义，合并按 URI 重映射。
//! - 所有身份均为确定性内容标识，导出后删除数据库再导入仍保持稳定。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 源 XML 中的字节区间（左闭右开），用于回溯到具体片段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl ByteRange {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// 原始 NodeId 的四类 OPC UA 标识，保留原始命名空间语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdKind {
    Numeric,
    String,
    Guid,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeId {
    /// 原始文本，如 `ns=1;i=1001`、`nsu=urn:x;s=Foo`、`i=58`，或 alias 名 `CoreMotor`。
    pub text: String,
    pub ns_index: Option<u16>,
    pub ns_uri: Option<String>,
    pub id_kind: IdKind,
    pub id_value: String,
    /// 当该 NodeId 文本实际是 alias 引用时记录 alias 名。
    pub alias_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Object,
    Variable,
    ObjectType,
    VariableType,
    ReferenceType,
    DataType,
    Method,
    View,
}

impl NodeKind {
    pub fn from_tag(tag: &str) -> Option<NodeKind> {
        Some(match tag {
            "UAObject" => NodeKind::Object,
            "UAVariable" => NodeKind::Variable,
            "UAObjectType" => NodeKind::ObjectType,
            "UAVariableType" => NodeKind::VariableType,
            "UAReferenceType" => NodeKind::ReferenceType,
            "UADataType" => NodeKind::DataType,
            "UAMethod" => NodeKind::Method,
            "UAView" => NodeKind::View,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Object => "UAObject",
            NodeKind::Variable => "UAVariable",
            NodeKind::ObjectType => "UAObjectType",
            NodeKind::VariableType => "UAVariableType",
            NodeKind::ReferenceType => "UAReferenceType",
            NodeKind::DataType => "UADataType",
            NodeKind::Method => "UAMethod",
            NodeKind::View => "UAView",
        }
    }
}

/// 命名空间表中的一行（文件局部 index → URI）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceRow {
    pub file_id: String,
    pub local_index: u16,
    pub uri: String,
    pub range: ByteRange,
}

/// Alias 表中的一行。target 为原始 NodeId 文本（可能再指向别的 alias）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasRow {
    pub file_id: String,
    pub name: String,
    pub target_raw: String,
    pub range: ByteRange,
}

/// UAX 文件头部的 Model 声明与必需模型依赖。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDecl {
    pub file_id: String,
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub range: ByteRange,
    pub required: Vec<RequiredModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub range: ByteRange,
}

/// 未识别扩展（含带单位/坐标基准的领域量），按原始表示保留。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extension {
    pub file_id: String,
    /// 所属原始节点稳定 id；顶层扩展为 None。
    pub node_raw_id: Option<String>,
    pub local_name: String,
    pub ns_uri: String,
    pub attrs: BTreeMap<String, String>,
    pub text: Option<String>,
    pub range: ByteRange,
}

/// 原始 reference（方向不做翻转，IsForward 按原文保留）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub file_id: String,
    pub node_raw_id: String,
    pub reference_type_raw: String,
    pub target_raw: String,
    pub is_forward: bool,
    pub range: ByteRange,
}

/// 原始节点：NodeId/BrowseName 等全部保留原文本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub raw_id: String,
    pub file_id: String,
    pub kind: NodeKind,
    pub node_id_raw: String,
    pub browse_name_raw: String,
    pub parent_raw: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// 节点标签上的全部属性（DataType/SymbolicName 等），原样保留。
    pub attrs: BTreeMap<String, String>,
    /// 文档序（该文件内的元素出现顺序），保证顺序可重放。
    pub ordinal: usize,
    pub range: ByteRange,
}

/// 一份 NodeSet XML 的保真解析结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ParsedDocument {
    pub file_id: String,
    pub path: String,
    pub sha256: String,
    pub namespaces: Vec<NamespaceRow>,
    pub aliases: Vec<AliasRow>,
    pub models: Vec<ModelDecl>,
    pub nodes: Vec<RawNode>,
    pub references: Vec<RawReference>,
    pub extensions: Vec<Extension>,
}

/// 规范化后的全局 NodeId（按 URI，而不是文件局部数字 index）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CanonicalNodeId {
    pub ns_uri: String,
    pub id_text: String,
}

impl CanonicalNodeId {
    pub fn stable(&self) -> String {
        format!("{}#{}", self.ns_uri, self.id_text)
    }
    pub fn display(&self, global_index: &BTreeMap<String, u16>) -> String {
        let idx = global_index.get(&self.ns_uri).copied().unwrap_or(u16::MAX);
        format!("ns={};{}", idx, self.id_text)
    }
}

/// 规范化引用方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CanonicalDir {
    Forward,
    /// 原文 IsForward=false，规范化时翻转端点，但不新增/删除边。
    FlippedFromInverse,
}

/// 派生层的边：端点缺失时 target/source 可能指向 unresolved 身份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEdge {
    pub edge_id: String,
    pub source: CanonicalNodeId,
    pub target: CanonicalNodeId,
    pub ref_type: CanonicalNodeId,
    pub dir: CanonicalDir,
    pub origins: Vec<Origin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalNode {
    pub identity: CanonicalNodeId,
    pub browse_namespace: String,
    pub browse_name: String,
    pub kind: NodeKind,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub origins: Vec<Origin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedNode {
    pub identity: CanonicalNodeId,
    pub reason: UnresolvedReason,
    pub origins: Vec<Origin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnresolvedReason {
    /// 引用指向了不存在的节点（删除后悬空 / 依赖缺失）。
    MissingTarget,
    /// 必需模型命名空间完全未提供。
    MissingRequiredModel,
    /// alias 解析成环，无法确定 NodeId。
    AliasCycle,
    /// NodeId 文本无法解析。
    MalformedNodeId,
}

impl UnresolvedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            UnresolvedReason::MissingTarget => "missing_target",
            UnresolvedReason::MissingRequiredModel => "missing_required_model",
            UnresolvedReason::AliasCycle => "alias_cycle",
            UnresolvedReason::MalformedNodeId => "malformed_node_id",
        }
    }
}

/// 来源指针：可一直追到文件内的字节区间。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Origin {
    pub file_id: String,
    pub path: String,
    pub start: usize,
    pub end: usize,
}

/// 领域量的原始表示（单位/坐标基准随值保留）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawMeasure {
    pub measure_id: String,
    pub node_identity: CanonicalNodeId,
    pub value_raw: String,
    pub unit_raw: String,
    pub frame: Option<String>,
    pub origin: Origin,
}

/// 派生层的规范化量；不可转换时 None 并保留诊断。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedMeasure {
    pub measure_id: String,
    pub si_value: Option<String>,
    pub si_unit: Option<String>,
    pub convertible: bool,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// 机器可读事件：判定结论 + 依据（材料、规则版本、决定）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub branch: String,
    pub kind: String,
    pub severity: Severity,
    pub subject: Option<String>,
    pub message: String,
    pub rule_version: String,
    /// 触发该事件的来源材料。
    pub origins: Vec<Origin>,
    /// 结构化判定依据（机器可读）。
    pub basis: serde_json::Value,
    /// 该事件之后分支指向的派生版本。
    pub after_derivation_id: Option<String>,
    /// 关联的人工决定 id。
    pub decision_id: Option<String>,
}

/// 人工决定（接受/否定/参数调整/候选选择），未选候选保留。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub decision_id: String,
    pub branch: String,
    pub base_derivation_id: String,
    pub after_derivation_id: String,
    pub kind: String,
    pub subject: String,
    pub choice: String,
    pub rejected_candidates: Vec<String>,
    pub params: serde_json::Value,
    pub rationale: String,
}

/// 派生行在增量重算中的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecomputeState {
    Unchanged,
    Recomputed,
    New,
    Removed,
}

impl RecomputeState {
    pub fn as_str(self) -> &'static str {
        match self {
            RecomputeState::Unchanged => "unchanged",
            RecomputeState::Recomputed => "recomputed",
            RecomputeState::New => "new",
            RecomputeState::Removed => "removed",
        }
    }
}
