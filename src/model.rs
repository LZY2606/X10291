use serde::{Deserialize, Serialize};

pub const RULE_NS_REMAP: &str =
    "数字 namespace index 只在单文件内有意义，合并按 URI 重映射";
pub const RULE_INVERSE: &str =
    "inverse reference 与 forward reference 规范化但不制造缺边";
pub const RULE_UNRESOLVED: &str =
    "依赖缺失保留 unresolved 节点";

pub const UA_NS: &str = "http://opcfoundation.org/UA/";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawRef {
    pub ref_type: String,
    pub is_forward: bool,
    pub target_raw: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawNode {
    pub file_id: String,
    pub ord: i64,
    pub local_id: String,
    pub node_class: String,
    pub browse_name: String,
    pub display_name: String,
    pub value_raw: Option<String>,
    pub xml_fragment: String,
    pub span_start: i64,
    pub span_end: i64,
    pub refs: Vec<RawRef>,
    pub extensions: Vec<String>,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedFile {
    pub file_id: String,
    pub name: String,
    pub xml: String,
    pub sha: String,
    pub namespace_uris: Vec<String>,
    pub aliases: Vec<(String, String)>,
    pub models: Vec<ModelDecl>,
    pub nodes: Vec<RawNode>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Params {
    pub alarm_threshold: f64,
    pub alarm_threshold_unit: String,
}

impl Params {
    pub fn initial() -> Self {
        Params { alarm_threshold: 80.0, alarm_threshold_unit: "degC".into() }
    }
}

#[derive(Clone, Debug)]
pub struct NodeRow {
    pub uid: String,
    pub file_id: String,
    pub local_id: String,
    pub node_class: String,
    pub browse_uri: String,
    pub browse_name: String,
    pub display_name: String,
    pub value_raw: Option<String>,
    pub unit: Option<String>,
    pub basis: Option<String>,
    pub extensions: Vec<String>,
    pub xml_fragment: String,
    pub span_start: i64,
    pub span_end: i64,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct RefRow {
    pub src_uid: String,
    pub ref_type_uri: String,
    pub ref_type_name: String,
    pub tgt_uid: String,
    pub tgt_raw: String,
    pub origin: String,
    pub resolved: bool,
    pub dangling: bool,
}

#[derive(Clone, Debug)]
pub struct DiagRow {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub basis: String,
    pub subject_uid: Option<String>,
    pub file_id: Option<String>,
    pub span_start: Option<i64>,
    pub span_end: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct AltRow {
    pub uid: String,
    pub cand_ord: i64,
    pub file_id: String,
    pub local_id: String,
    pub display_name: String,
    pub value_raw: Option<String>,
    pub reason: String,
    pub chosen: bool,
}

#[derive(Clone, Debug)]
pub struct QuantityRow {
    pub uid: String,
    pub raw_text: String,
    pub unit: String,
    pub basis: String,
    pub norm_value: Option<f64>,
    pub norm_unit: Option<String>,
    pub status: String,
    pub diag: Option<String>,
}

pub fn sha1_hex(data: &str) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(data.as_bytes());
    hex(&h.finalize())
}

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

pub fn expanded_node_id(ns_uri: &str, id: &str) -> String {
    format!("nsu={};{}", ns_uri, id)
}

pub fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}
