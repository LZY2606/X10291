//! 领域模型：NodeId、BrowseName、引用、模型依赖、事件。
//! 核心语义：
//! - 数字 namespace index 只在单文件内有意义，合并按 URI 重映射；
//! - inverse reference 与 forward reference 规范化但不制造缺边；
//! - 依赖缺失保留 unresolved 节点。

use serde::{Deserialize, Serialize};

/// 全局稳定 NodeId：以 namespace URI 为基准，跨文件、跨版本稳定。
/// 形如 `nsu=http://example/ua/;i=1001` / `nsu=...;s=Foo` / `nsu=...;g=..` / `nsu=...;b=..`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GlobalNodeId(pub String);

impl GlobalNodeId {
    pub fn from_parts(ns_uri: &str, id: &str) -> Self {
        GlobalNodeId(format!("nsu={};{}", ns_uri, id))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 文件内原始 NodeId 文本（如 `ns=2;i=5`、`i=45`、别名 `MyAlias`），原样保留。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeIdText(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawReference {
    pub ref_type_text: String, // 原始 ReferenceType 文本（可能是 i=45 或别名）
    pub is_forward: bool,      // 文件中 IsForward 属性，缺省 true
    pub target_text: String,   // 原始目标文本（可能是别名或 ns=N 形式）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawFragment {
    pub element: String,
    pub raw_xml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawNode {
    pub element: String, // UAObject / UAVariable / UAMethod / UAObjectType ...
    pub node_id_text: String,
    pub browse_name: String, // 原始 BrowseName（含文件内数字前缀，如 1:Valve）
    pub attrs: Vec<(String, String)>,
    pub references: Vec<RawReference>,
    pub extensions: Vec<RawFragment>, // 未识别子元素原样保留
    pub raw_xml: String,              // 原始 XML 片段
    pub start: usize,                 // 源文件字节范围
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDependency {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParsedNodeSet {
    /// 文件内 namespace 表：index 0 固定为 OPC UA 核心 URI，其后为 <NamespaceUris> 顺序。
    pub namespaces: Vec<String>,
    pub aliases: Vec<(String, String)>, // alias -> 原始 NodeId 文本
    pub model: Option<ModelDependency>, // <Models><Model> 自身声明
    pub required_models: Vec<ModelDependency>, // <RequiredModel> 依赖
    pub nodes: Vec<RawNode>,
    pub top_level_extensions: Vec<RawFragment>, // 顶层未识别扩展
}

/// 规范化后的引用边（forward 方向统一）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub src: GlobalNodeId,
    pub dst: GlobalNodeId,
    pub ref_type: GlobalNodeId,
}

/// 机器可读取的判定事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgmentEvent {
    pub kind: String,          // alias_cycle / duplicate_nodeid / dependency_version_conflict / ...
    pub subject: String,       // 受影响对象（全局 NodeId / URI / alias 名）
    pub rule: String,          // 判定依据（规则文本）
    pub rationale: String,     // 说明
    pub inputs: Vec<String>,   // 输入片段标识
    pub candidates: Vec<serde_json::Value>, // 全部合法候选（含未采纳）
    pub chosen: Option<usize>, // 采纳的候选下标；无法唯一决定时为 None
}

impl JudgmentEvent {
    pub fn new(kind: &str, subject: &str, rule: &str, rationale: &str) -> Self {
        JudgmentEvent {
            kind: kind.into(),
            subject: subject.into(),
            rule: rule.into(),
            rationale: rationale.into(),
            inputs: vec![],
            candidates: vec![],
            chosen: None,
        }
    }
}
