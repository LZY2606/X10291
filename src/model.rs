//! 领域模型：原 XML 节点、namespace table、NodeId、BrowseName、reference、
//! alias、model dependency 与未识别扩展均保留。
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const UA_NS: &str = "http://opcfoundation.org/UA/";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NodeIdKind {
    N(u64),
    S(String),
    G(String),
    B(String),
}

/// 文件内 NodeId：数字 namespace index 只在单文件内有意义。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeId {
    pub ns: u16,
    pub kind: NodeIdKind,
}

impl NodeId {
    pub fn parse(s: &str) -> Option<NodeId> {
        let mut ns: u16 = 0;
        let mut kind: Option<NodeIdKind> = None;
        for part in s.trim().split(';') {
            if let Some(v) = part.strip_prefix("ns=") {
                ns = v.parse().ok()?;
            } else if let Some(v) = part.strip_prefix("i=") {
                kind = Some(NodeIdKind::N(v.parse().ok()?));
            } else if let Some(v) = part.strip_prefix("s=") {
                kind = Some(NodeIdKind::S(v.to_string()));
            } else if let Some(v) = part.strip_prefix("g=") {
                kind = Some(NodeIdKind::G(v.to_string()));
            } else if let Some(v) = part.strip_prefix("b=") {
                kind = Some(NodeIdKind::B(v.to_string()));
            } else {
                return None;
            }
        }
        kind.map(|k| NodeId { ns, kind: k })
    }

    pub fn kind_str(&self) -> String {
        match &self.kind {
            NodeIdKind::N(v) => format!("i={v}"),
            NodeIdKind::S(v) => format!("s={v}"),
            NodeIdKind::G(v) => format!("g={v}"),
            NodeIdKind::B(v) => format!("b={v}"),
        }
    }

    /// 合并后的稳定身份：按 URI 重映射，不再依赖数字 index。
    pub fn to_uri_form(&self, uri: &str) -> String {
        format!("nsu={uri};{}", self.kind_str())
    }
}

/// 未识别扩展：保留原始 XML 片段。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Extension {
    pub element: String,
    pub xml: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AliasDef {
    pub alias: String,
    pub target: String,
    pub range: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequiredModel {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelDecl {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub required: Vec<RequiredModel>,
    pub range: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawRef {
    pub ref_type: String,
    pub is_forward: bool,
    pub target: String,
    pub range: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawNode {
    pub class: String,
    pub node_id: String,
    pub browse_name: String,
    pub display_name: Option<String>,
    pub value: Option<String>,
    pub unit: Option<String>,
    pub attrs: BTreeMap<String, String>,
    pub references: Vec<RawRef>,
    pub extensions: Vec<Extension>,
    pub range: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diag {
    pub code: String,
    pub message: String,
    pub file: String,
    pub range: Option<(usize, usize)>,
    pub node: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NodeSetFile {
    pub name: String,
    pub source: String,
    pub namespace_uris: Vec<String>,
    pub aliases: Vec<AliasDef>,
    pub models: Vec<ModelDecl>,
    pub nodes: Vec<RawNode>,
    pub extensions: Vec<Extension>,
    pub diagnostics: Vec<Diag>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceRef {
    pub file: String,
    pub range: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergedNode {
    pub id: String, // nsu=... 稳定身份
    pub class: String,
    pub browse_name: String, // 全局 index 形式 "N:Name"
    pub browse_uri: String,
    pub display_name: Option<String>,
    pub value: Option<String>,           // 原始表示
    pub effective_value: Option<String>, // 派生层（人工调整）后的值
    pub unit: Option<String>,            // 原始单位
    pub normalized: Option<String>,      // 派生层 SI 规范化；无法转换则 None
    pub status: String,                  // ok | unresolved
    pub attrs: BTreeMap<String, String>,
    pub extensions: Vec<Extension>,
    pub source: Option<SourceRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub ref_type: String,
    pub declared_forward: bool,
    pub declared_inverse: bool,
    pub dangling: bool,
    pub sources: Vec<SourceRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unresolved {
    pub kind: String,
    pub detail: String,
    pub node: Option<String>,
    pub source: Option<SourceRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub label: String,
    pub source: Option<SourceRef>,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictItem {
    pub kind: String,
    pub key: String,
    pub kept: Candidate,
    pub alternatives: Vec<Candidate>,
    pub resolved: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MergedModel {
    pub namespaces: Vec<String>,
    pub nodes: Vec<MergedNode>,
    pub edges: Vec<Edge>,
    pub unresolved: Vec<Unresolved>,
    pub conflicts: Vec<ConflictItem>,
    pub diagnostics: Vec<Diag>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub kind: String, // delete-node | param-adjust | resolve-duplicate | note
    pub detail: String,
    pub choice: String,
    pub note: String,
}

/// 版本快照的输入侧：文件 + 决定。派生结果可确定性重算。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AppState {
    pub files: Vec<NodeSetFile>,
    pub decisions: Vec<Decision>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffEntry {
    pub kind: String, // ns-added | ns-removed | node-added | node-removed | node-changed | edge-added | edge-removed | unresolved-added | unresolved-removed
    pub id: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ModelDiff {
    pub entries: Vec<DiffEntry>,
}
