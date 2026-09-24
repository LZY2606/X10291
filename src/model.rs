//! 领域模型：NodeSet 导入后保留的原始信息 + 派生层的规范化视图。
//! 原则：导入层保留原始表示（raw XML、原始 NodeId 文本、原始单位），
//! 派生层才做 namespace URI 重映射、单位换算、引用规范化。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// OPC UA 核心命名空间（ns=0 的固定 URI）。
pub const UA_CORE_NS: &str = "http://opcfoundation.org/UA/";

/// 稳定节点身份：namespace URI + 标识符部分。
/// 数字 namespace index 只在单文件内有意义，跨文件一律用 URI 身份。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableNodeId {
    pub ns_uri: String,
    /// 形如 `i=5` / `s=Pump01` / `g=...` / `b=...` 的标识符部分（不含 ns 段）。
    pub ident: String,
}

impl StableNodeId {
    pub fn new(ns_uri: impl Into<String>, ident: impl Into<String>) -> Self {
        StableNodeId { ns_uri: ns_uri.into(), ident: ident.into() }
    }
    /// 稳定字符串身份，导出/导入、事件、diff 全部以此为准。
    pub fn key(&self) -> String {
        format!("nsu={};{}", self.ns_uri, self.ident)
    }
    pub fn parse_key(key: &str) -> Option<Self> {
        let rest = key.strip_prefix("nsu=")?;
        let (uri, ident) = rest.split_once(';')?;
        Some(StableNodeId::new(uri, ident))
    }
}

impl fmt::Display for StableNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.key())
    }
}

/// 源 XML 范围：源文件 + 字节区间，供页面"追到源 XML 范围"。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRange {
    pub source_id: String,
    pub start: usize,
    pub end: usize,
}

/// 未识别扩展：原样保留 raw XML，绝不丢弃。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnknownExtension {
    pub element: String,
    pub raw_xml: String,
}

/// 一条引用（导入层保留原始写法；target 为派生层解析结果）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    pub reference_type_raw: String,
    pub reference_type: Option<StableNodeId>,
    pub is_forward: bool,
    /// 文件中原始目标文本（可能是 alias 或带数字 ns 的 NodeId）。
    pub target_raw: String,
    /// 派生层解析出的稳定身份；无法唯一解析时为 None。
    pub target: Option<StableNodeId>,
    pub unresolved_reason: Option<String>,
}

/// 带单位的领域量：导入保留原始表示，派生层才规范化。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeValue {
    pub raw: String,
    /// 原始单位/坐标基准标签（如 "mm"、"degC"），来自 EngineeringUnits 扩展。
    pub unit_raw: Option<String>,
    /// 规范化到 SI 基准后的值；无法转换时为 None（值留在原位置并附诊断）。
    pub normalized_si: Option<f64>,
    pub si_unit: Option<String>,
    pub diagnostic: Option<String>,
}

/// 一个 UA 节点：原始 XML 片段、NodeId、BrowseName、引用、扩展全部保留。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: StableNodeId,
    /// 文件中原始 NodeId 文本（含数字 ns index）。
    pub node_id_raw: String,
    pub class: String,
    /// 原始 BrowseName（如 "2:Pump"）。
    pub browse_name_raw: String,
    /// 派生层重映射后的 BrowseName（全局 ns index 形式）。
    pub browse_name_global: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub references: Vec<Reference>,
    pub extensions: Vec<UnknownExtension>,
    pub value: Option<NodeValue>,
    pub source: SourceRange,
    pub source_file: String,
    /// 原始 XML 片段，完整保留。
    pub raw_xml: String,
}

/// Model 依赖声明（ModelSet/Model 与 RequiredModel）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequiredModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required: Vec<RequiredModel>,
    pub source_id: String,
}

/// 单文件导入结果：namespace table、alias、model dependency、节点、未知扩展全保留。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedNodeSet {
    pub source_id: String,
    pub file_name: String,
    pub sha256: String,
    /// 文件内 namespace table：下标 i 对应文件内 ns index i+1（ns=0 固定为核心 URI）。
    pub namespace_uris: Vec<String>,
    /// (alias 名, 原始 NodeId 文本)，保持文件内顺序。
    pub aliases: Vec<(String, String)>,
    pub models: Vec<ModelInfo>,
    pub nodes: Vec<Node>,
    /// 根级未识别元素，原样保留。
    pub unknown_root_elements: Vec<UnknownExtension>,
}

/// 规范化引用边的方向来源。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EdgeBasis {
    /// 文件中显式声明的 forward 引用。
    DeclaredForward,
    /// 文件中显式声明的 inverse 引用（规范化为正向边，但标记来源）。
    DeclaredInverse,
    /// 由对向声明推导出的镜像边（派生视图，不制造缺边：没有声明基础的边不存在）。
    DerivedMirror,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub from: StableNodeId,
    pub to: StableNodeId,
    pub reference_type: Option<StableNodeId>,
    pub reference_type_raw: String,
    pub basis: EdgeBasis,
    /// 目标/端点无法解析时保留边并标记，而不是丢边。
    pub dangling: bool,
    pub source_file: String,
}

/// 机器可读取的诊断/判定事件主体。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    /// 受影响对象的稳定身份列表。
    pub subjects: Vec<String>,
    pub details: serde_json::Value,
}

/// 重复 NodeId 的候选（多解不吞掉，全部保留）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DuplicateCandidate {
    pub id: StableNodeId,
    pub candidates: Vec<Node>,
    /// 裁定后被采纳的候选下标；未裁定时为 None（不确定状态保留）。
    pub chosen: Option<usize>,
}

/// 未解析项（缺失依赖、悬空引用、无法转换的单位等）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UnresolvedItem {
    pub kind: String,
    pub subject: String,
    pub reason: String,
    pub source_file: String,
}

/// 合并后的派生模型（一个版本的内容）。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MergedModel {
    /// 全局 namespace 表：index 0 固定为核心 URI，其余按 URI 首次出现顺序分配。
    pub global_namespaces: Vec<String>,
    /// 每个身份被采纳的节点。
    pub nodes: BTreeMap<String, Node>,
    /// 重复 NodeId 的全部候选。
    pub duplicates: Vec<DuplicateCandidate>,
    pub edges: Vec<Edge>,
    pub diagnostics: Vec<Diagnostic>,
    pub unresolved: Vec<UnresolvedItem>,
    pub models: Vec<ModelInfo>,
    /// 增量重算说明：某项结果为何仍有效或为何需要重算。
    pub recompute_report: Vec<RecomputeNote>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecomputeNote {
    pub subject: String,
    /// "still_valid" | "recomputed"
    pub status: String,
    pub reason: String,
}
