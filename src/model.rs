//! 领域模型：NodeId、BrowseName、引用、诊断、量值。
//! 所有身份均以 URI 为基准（数字 namespace index 只在单文件内有意义）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub const UA_CORE_URI: &str = "http://opcfoundation.org/UA/";

/// 稳定身份字符串：uri + "|" + id，跨导出/导入不变。
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId {
    pub ns_uri: String,
    pub id: IdKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IdKind {
    Num(u64),
    Str(String),
    Guid(String),
    Opaque(String),
}

impl NodeId {
    pub fn key(&self) -> String {
        let id = match &self.id {
            IdKind::Num(n) => format!("i={}", n),
            IdKind::Str(s) => format!("s={}", s),
            IdKind::Guid(g) => format!("g={}", g),
            IdKind::Opaque(b) => format!("b={}", b),
        };
        format!("{}|{}", self.ns_uri, id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseName {
    pub ns_uri: String,
    pub name: String,
}

/// 源位置：文件 + 字节范围，可回读源 XML 片段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub file: String,
    pub start: usize,
    pub end: usize,
}

/// 未识别扩展：原样保留 XML 文本与范围。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extension {
    pub name: String,
    pub xml: String,
    pub span: (usize, usize),
}

/// 领域量：导入时保留原始表示，派生层才规范化。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub raw_value: String,
    pub raw_unit: String,
    pub basis: String, // 坐标基准，如 frame:base_link / shaft:spindle-1
    pub source: SourceRef,
    pub id: String, // 稳定身份：node key + 序号
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NormalizedQuantity {
    pub id: String,
    pub raw: Quantity,
    pub value: Option<f64>,
    pub unit: String, // 规范化单位；无法转换时为空
    pub diagnostic: Option<String>,
}

/// 引用目标：解析成功 / 未解析（保留原样）/ 多解（保留全部合法候选）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefTarget {
    Node(NodeId),
    Unresolved { raw: String, reason: String },
    Ambiguous { raw: String, candidates: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub kind_raw: String,
    pub kind_node: Option<NodeId>,
    pub forward: bool, // 声明方向；规范化不制造缺边
    pub target: RefTarget,
    pub source_span: (usize, usize),
}

/// 原始节点：保留全部原始属性、引用与未识别扩展。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawNode {
    pub node_class: String,
    pub node_id_raw: String,
    pub node_id: Option<NodeId>,
    pub browse_name_raw: String,
    pub browse_name: Option<BrowseName>,
    pub display_name: Option<String>,
    pub attrs: Vec<(String, String)>,
    pub references: Vec<Reference>,
    pub extensions: Vec<Extension>,
    pub source: SourceRef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequiredModel {
    pub uri: String,
    pub version: String,
    pub publication_date: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub uri: String,
    pub version: String,
    pub publication_date: String,
    pub required: Vec<RequiredModel>,
    pub source: SourceRef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AliasDef {
    pub alias: String,
    pub target_raw: String,
    pub source: SourceRef,
}

/// 单个 NodeSet 文件的原始导入结果：一切均保留。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawFile {
    pub name: String,
    pub source: String,
    pub fingerprint: String,
    pub namespace_uris: Vec<String>,
    pub aliases: Vec<AliasDef>,
    pub models: Vec<ModelInfo>,
    pub nodes: Vec<RawNode>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagKind {
    NamespaceIndexOutOfRange,
    AliasCycle,
    DuplicateNodeId,
    DanglingReference,
    UnresolvedDependency,
    DependencyVersionConflict,
    UnknownExtensionPreserved,
    UnconvertibleUnit,
    BasisMismatch,
    XmlParse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: String,
    pub kind: DiagKind,
    pub message: String,
    pub subject: Option<String>,
    pub source: Option<SourceRef>,
    pub candidates: Vec<String>,
    pub caused_by: Vec<u64>, // 触发该诊断的事件 seq
}

/// 机器可读取的判定事件（合并规则的判断依据）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleEvent {
    pub rule: String,
    pub basis: String,
    pub subject: String,
    pub detail: BTreeMap<String, String>,
}

// ---------- 合并后的模型 ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeCandidate {
    pub node: RawNode,
    pub file: String,
    pub adopted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeEntry {
    pub key: String, // NodeId::key()
    pub identity: NodeId,
    pub candidates: Vec<NodeCandidate>,
    pub external: bool, // 核心命名空间或缺失依赖的占位节点
    pub placeholder_reason: Option<String>,
}

/// 规范化边：每条声明的引用只存一次，保留声明方向与声明者。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub from: String,           // 声明者节点 key
    pub kind_raw: String,
    pub forward: bool,          // 声明方向
    pub to: RefTarget,
    pub declared_by: String,    // file
    pub derived_inverse: bool,  // 规范化视图标记；声明存储永不制造缺边
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileNsMap {
    pub file: String,
    // 文件内数字 index -> (URI, 合并后 canonical index)
    pub map: Vec<(u32, String, u32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergedModel {
    pub ns_table: Vec<String>, // canonical index -> URI（0 固定为核心 URI）
    pub file_ns_maps: Vec<FileNsMap>,
    pub nodes: BTreeMap<String, NodeEntry>,
    pub edges: Vec<Edge>,
    pub quantities: Vec<Quantity>,
    pub diagnostics: Vec<Diagnostic>,
    pub rule_events: Vec<RuleEvent>,
    pub fingerprint: String,
}

/// 稳定短哈希（FNV-1a 64），用于指纹与诊断 id。
pub fn fnv64(data: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let mut s = String::new();
    write!(s, "{:016x}", h).unwrap();
    s
}
