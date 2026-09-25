//! 原始层与派生层的数据模型。
//! 原则：导入时保留原始表示（原 XML 片段、文件内 namespace 序号、原始单位），
//! 派生层才做 URI 重映射与单位规范化；无法转换的值留在原位置并附诊断。

use serde::{Deserialize, Serialize};

// ---------- 原始层（导入即保留，不做规范化） ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawNodeSet {
    pub source_name: String,
    pub source_xml: String,
    /// 文件内 namespace table：下标即文件内数字序号（仅在本文件内有意义）。
    pub namespace_uris: Vec<String>,
    pub models: Vec<RawModel>,
    /// (alias 名, 目标文本) —— 目标可能是 NodeId 文本，也可能是另一个 alias。
    pub aliases: Vec<(String, String)>,
    pub nodes: Vec<RawNode>,
    /// UANodeSet 级未识别扩展，原始 XML 片段。
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawModel {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub required: Vec<RawModelDep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawModelDep {
    pub model_uri: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawNode {
    pub class: String,
    /// 文件内 NodeId 原文，如 "ns=2;i=3001"。
    pub node_id: String,
    /// 文件内 BrowseName 原文，如 "2:Temperature"。
    pub browse_name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub value: Option<RawValue>,
    pub references: Vec<RawReference>,
    /// 节点级未识别扩展，原始 XML 片段。
    pub extensions: Vec<String>,
    /// 源 XML 范围（字节偏移 + 行号），供复核界面回溯。
    pub xml_start: usize,
    pub xml_end: usize,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawValue {
    /// 原始文本表示，导入层不做换算。
    pub text: String,
    /// 原始单位表示（如 "°C"、"psi"），可能无法转换。
    pub unit_raw: Option<String>,
    /// 坐标基准 / 量纲基准说明，原样保留。
    pub coordinate_basis: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawReference {
    /// ReferenceType 原文（可能是 alias 名）。
    pub ref_type: String,
    pub is_forward: bool,
    /// 目标原文（可能是 alias 名或 NodeId 文本）。
    pub target: String,
}

// ---------- 派生层（合并后，按 URI 重映射） ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MergedModel {
    /// 全局 namespace 注册表：全局序号 -> URI（按首次出现顺序分配，稳定）。
    pub namespaces: Vec<String>,
    pub ns_mappings: Vec<NsMapping>,
    pub nodes: Vec<MergedNode>,
    pub edges: Vec<Edge>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NsMapping {
    pub file: String,
    pub local_index: u32,
    pub uri: String,
    pub global_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeStatus {
    Ok,
    /// 重复 NodeId：保留全部合法候选，不吞掉多解。
    Duplicate,
    /// 依赖缺失：保留 unresolved 占位节点。
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeCandidate {
    pub file: String,
    pub line_start: usize,
    pub line_end: usize,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MergedNode {
    /// 稳定身份：nsu=<uri>;<ident>，与文件内数字序号无关。
    pub global_id: String,
    pub class: String,
    pub browse_name: String,
    pub browse_uri: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// 原始值表示（含原始单位与坐标基准）。
    pub value_raw: Option<RawValue>,
    /// 派生规范化值（SI）；无法转换时为 None，原值留在 value_raw。
    pub value_si: Option<f64>,
    pub value_si_unit: Option<String>,
    pub status: NodeStatus,
    pub candidates: Vec<NodeCandidate>,
    pub extensions: Vec<String>,
    pub source_file: String,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub src: String,
    pub dst: String,
    pub ref_type: String,
    /// Forward = 正向声明；InverseNormalized = 由逆向声明规范化而来。
    pub origin: String,
    pub source_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    /// 稳定身份：由 kind+subjects 哈希派生，跨版本/跨导出导入保持稳定。
    pub id: String,
    pub kind: String,
    pub message: String,
    pub subjects: Vec<String>,
}

// ---------- 语义差异 ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffEntry {
    pub id: String,
    pub kind: String,
    pub entity: String,
    pub field: String,
    pub old: String,
    pub new: String,
}

/// 简单稳定的 FNV-1a 哈希，用于派生稳定身份。
pub fn stable_id(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        for b in p.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= 0x1f;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}
