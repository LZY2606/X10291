//! 领域模型：NodeId / BrowseName / Reference / Alias / ModelDependency / NodeSet。
//! 原则：导入层保留原始表示（含未识别扩展与源 XML 范围），派生层才做规范化。

use serde::{Deserialize, Serialize};

/// UA 基础 namespace（全局 index 0）。
pub const UA_NS_URI: &str = "http://opcfoundation.org/UA/";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ident {
    Num(u64),
    Str(String),
    Guid(String),
    Opaque(String),
}

/// NodeId，ns 为“文件内局部 index”或“合并后全局 index”，取决于所在阶段。
/// 语义规则：数字 namespace index 只在单文件内有意义。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    pub ns: u16,
    pub ident: Ident,
}

impl NodeId {
    pub fn parse(raw: &str) -> Option<NodeId> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let mut ns: u16 = 0;
        let mut body = raw;
        if let Some(rest) = raw.strip_prefix("ns=") {
            let (num, rest2) = rest.split_once(';')?;
            ns = num.parse().ok()?;
            body = rest2;
        }
        if let Some(v) = body.strip_prefix("i=") {
            Some(NodeId { ns, ident: Ident::Num(v.parse().ok()?) })
        } else if let Some(v) = body.strip_prefix("s=") {
            Some(NodeId { ns, ident: Ident::Str(v.to_string()) })
        } else if let Some(v) = body.strip_prefix("g=") {
            Some(NodeId { ns, ident: Ident::Guid(v.to_string()) })
        } else if let Some(v) = body.strip_prefix("b=") {
            Some(NodeId { ns, ident: Ident::Opaque(v.to_string()) })
        } else {
            None
        }
    }

    /// 单文件内的局部写法（数字 index 仅在此语境有意义）。
    pub fn to_local_string(&self) -> String {
        let body = match &self.ident {
            Ident::Num(v) => format!("i={v}"),
            Ident::Str(v) => format!("s={v}"),
            Ident::Guid(v) => format!("g={v}"),
            Ident::Opaque(v) => format!("b={v}"),
        };
        if self.ns == 0 { body } else { format!("ns={};{}", self.ns, body) }
    }

    /// 稳定身份：按 URI 展开，跨文件、跨版本、导出再导入后不变。
    pub fn to_stable_string(&self, global_uris: &[String]) -> String {
        let body = match &self.ident {
            Ident::Num(v) => format!("i={v}"),
            Ident::Str(v) => format!("s={v}"),
            Ident::Guid(v) => format!("g={v}"),
            Ident::Opaque(v) => format!("b={v}"),
        };
        match global_uris.get(self.ns as usize) {
            Some(uri) if self.ns > 0 => format!("nsu={uri};{body}"),
            _ => body,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    /// 原始写法（可能是 alias 名或局部 NodeId 字符串）。
    pub ref_type_raw: String,
    pub is_forward: bool,
    /// 原始目标写法（可能是 alias 名或局部 NodeId 字符串）。
    pub target_raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelDependency {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
}

/// 未识别扩展：原样保留的 XML 片段。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawExtension {
    pub element: String,
    pub raw_xml: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UaNode {
    pub node_id: NodeId,
    pub browse_name: (u16, String),
    pub node_class: String,
    pub display_name: String,
    pub description: Option<String>,
    /// 未专门解析的属性（如 DataType、ValueRank、EngineeringUnits 等）原样保留。
    pub attrs: Vec<(String, String)>,
    pub references: Vec<Reference>,
    pub extensions: Vec<RawExtension>,
    /// 源文件中的原始 XML 片段与字节范围（可追到源 XML 范围）。
    pub raw_xml: String,
    pub span: (usize, usize),
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub kind: String,
    pub message: String,
    pub node: Option<String>,
    pub source: String,
    pub span: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSet {
    pub source_id: String,
    pub filename: String,
    /// 文件内 namespace table（局部 index 2.. 对应下标 0..）。
    pub namespace_uris: Vec<String>,
    pub models: Vec<ModelDependency>,
    pub aliases: Vec<(String, String)>,
    pub nodes: Vec<UaNode>,
    pub diagnostics: Vec<Diagnostic>,
}

/// 合并后的全局模型。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergedModel {
    pub global_uris: Vec<String>,
    /// 每个来源文件的局部 index -> 全局 index 映射。
    pub ns_maps: Vec<NsMap>,
    pub nodes: Vec<MergedNode>,
    pub aliases: Vec<ResolvedAlias>,
    pub dependencies: Vec<DependencyStatus>,
    pub duplicate_groups: Vec<DuplicateGroup>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NsMap {
    pub source: String,
    /// (局部 index, 全局 index, URI)
    pub entries: Vec<(u16, u16, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedAlias {
    pub source: String,
    pub name: String,
    pub raw_target: String,
    pub status: AliasStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AliasStatus {
    Resolved(String),
    /// alias 循环：保留原始定义，不做唯一化。
    Cyclic(Vec<String>),
    Unresolved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub required_by: String,
    pub dep: ModelDependency,
    pub status: DepStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum DepStatus {
    Satisfied { loaded_version: String },
    /// 依赖缺失：保留 unresolved 节点。
    Missing,
    /// 依赖版本冲突。
    VersionConflict { loaded_version: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub stable_id: String,
    /// 全部合法候选都保留，不做唯一取舍。
    pub candidates: Vec<DuplicateCandidate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateCandidate {
    pub source: String,
    pub span: (usize, usize),
    pub node_class: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    Ok,
    Duplicate,
    /// 悬空/缺失目标的占位节点（依赖缺失时保留）。
    Unresolved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergedRef {
    pub ref_type: NodeId,
    pub ref_type_raw: String,
    pub is_forward: bool,
    pub target: NodeId,
    pub target_raw: String,
    pub target_stable: String,
    pub resolved: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergedNode {
    pub id: NodeId,
    pub stable_id: String,
    pub browse_name: (u16, String),
    pub node_class: String,
    pub display_name: String,
    pub description: Option<String>,
    pub attrs: Vec<(String, String)>,
    pub references: Vec<MergedRef>,
    pub extensions: Vec<RawExtension>,
    pub raw_xml: String,
    pub span: (usize, usize),
    pub source: String,
    pub status: NodeStatus,
}

impl MergedModel {
    pub fn find(&self, stable_id: &str) -> Option<&MergedNode> {
        self.nodes.iter().find(|n| n.stable_id == stable_id)
    }
}
