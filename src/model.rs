use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Ident {
    Numeric(u64),
    Text(String),
    Guid(String),
    Opaque(String),
}

/// 单文件内的 NodeId：数字 namespace index 只在该文件内有意义。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub ns: u16,
    pub ident: Ident,
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.ident {
            Ident::Numeric(v) => write!(f, "ns={};i={}", self.ns, v),
            Ident::Text(v) => write!(f, "ns={};s={}", self.ns, v),
            Ident::Guid(v) => write!(f, "ns={};g={}", self.ns, v),
            Ident::Opaque(v) => write!(f, "ns={};b={}", self.ns, v),
        }
    }
}

pub fn parse_node_id(raw: &str) -> Option<NodeId> {
    let mut ns: u16 = 0;
    let mut ident: Option<Ident> = None;
    for part in raw.trim().split(';') {
        let (k, v) = part.split_once('=')?;
        match k.trim() {
            "ns" => ns = v.trim().parse().ok()?,
            "i" => ident = Some(Ident::Numeric(v.trim().parse().ok()?)),
            "s" => ident = Some(Ident::Text(v.trim().to_string())),
            "g" => ident = Some(Ident::Guid(v.trim().to_string())),
            "b" => ident = Some(Ident::Opaque(v.trim().to_string())),
            _ => return None,
        }
    }
    ident.map(|ident| NodeId { ns, ident })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowseName {
    pub ns: u16,
    pub name: String,
    pub raw: String,
}

pub fn parse_browse_name(raw: &str) -> BrowseName {
    if let Some((ns, name)) = raw.split_once(':') {
        if let Ok(ns) = ns.trim().parse::<u16>() {
            return BrowseName { ns, name: name.to_string(), raw: raw.to_string() };
        }
    }
    BrowseName { ns: 0, name: raw.to_string(), raw: raw.to_string() }
}

/// 源 XML 范围：所有显示值都能追回到这里。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceSpan {
    pub file: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reference {
    pub ref_type_raw: String,
    pub is_forward: bool,
    pub target_raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UANode {
    pub node_class: String,
    pub node_id: NodeId,
    pub browse_name: BrowseName,
    pub display_name: String,
    pub description: Option<String>,
    pub references: Vec<Reference>,
    /// 原始属性（含单位/坐标基准），导入时保留原始表示。
    pub attributes: Vec<(String, String)>,
    /// 未识别扩展的原始 XML 片段，原样保留。
    pub extensions: Vec<String>,
    pub source: SourceSpan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelDep {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMeta {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub required_models: Vec<ModelDep>,
    pub source_file: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSetFile {
    pub file_name: String,
    pub namespace_uris: Vec<String>,
    /// alias 名 -> 原始目标文本（可能是 NodeId，也可能是另一个 alias）。
    pub aliases: Vec<(String, String)>,
    pub models: Vec<ModelMeta>,
    pub nodes: Vec<UANode>,
    /// 文件级未识别扩展，原样保留。
    pub file_extensions: Vec<String>,
    pub content_hash: String,
    pub raw_xml: String,
}

pub fn sha256_hex(data: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
