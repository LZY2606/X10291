use serde::{Deserialize, Serialize};
use std::fmt;

/// 解析后的 NodeId（ns 为“文件内”数字索引；合并时才按 URI 重映射为全局 ns）。
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId {
    pub ns: u16,
    pub kind: NodeIdKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NodeIdKind {
    Num(u64),
    Str(String),
    Guid(String),
    Opaque(String),
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let body = match &self.kind {
            NodeIdKind::Num(v) => format!("i={}", v),
            NodeIdKind::Str(v) => format!("s={}", v),
            NodeIdKind::Guid(v) => format!("g={}", v),
            NodeIdKind::Opaque(v) => format!("b={}", v),
        };
        if self.ns == 0 {
            write!(f, "{}", body)
        } else {
            write!(f, "ns={};{}", self.ns, body)
        }
    }
}

/// 解析 "ns=2;i=1001" / "i=47" / "ns=1;s=Foo" 形式；非 NodeId 文本（如 alias 名）返回 None。
pub fn parse_node_id(text: &str) -> Option<NodeId> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut ns: u16 = 0;
    let mut body = text;
    if let Some(rest) = text.strip_prefix("ns=") {
        let (num, rest2) = rest.split_once(';')?;
        ns = num.parse().ok()?;
        body = rest2;
    }
    let kind = if let Some(v) = body.strip_prefix("i=") {
        NodeIdKind::Num(v.parse().ok()?)
    } else if let Some(v) = body.strip_prefix("s=") {
        NodeIdKind::Str(v.to_string())
    } else if let Some(v) = body.strip_prefix("g=") {
        NodeIdKind::Guid(v.to_string())
    } else if let Some(v) = body.strip_prefix("b=") {
        NodeIdKind::Opaque(v.to_string())
    } else {
        return None;
    };
    Some(NodeId { ns, kind })
}

/// BrowseName 如 "1:PumpType"，前缀为文件内 namespace index。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualifiedName {
    pub ns: u16,
    pub name: String,
}

pub fn parse_browse_name(text: &str) -> QualifiedName {
    let text = text.trim();
    if let Some((prefix, name)) = text.split_once(':') {
        if let Ok(ns) = prefix.parse::<u16>() {
            return QualifiedName { ns, name: name.to_string() };
        }
    }
    QualifiedName { ns: 0, name: text.to_string() }
}

/// 诊断：机器可读的判定依据。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub diag_id: String,
    pub kind: String,
    pub message: String,
    pub doc_id: Option<String>,
    pub xml_start: Option<usize>,
    pub xml_end: Option<usize>,
    pub object_ref: Option<String>,
}

/// 领域量：原始表示 + 单位/坐标基准；规范化值只在派生层产生。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quantity {
    pub gnodeid: String,
    pub doc_id: String,
    pub raw_value: String,
    pub unit: Option<String>,
    pub basis: Option<String>,
    pub norm_value: Option<f64>,
    pub norm_unit: Option<String>,
    pub status: String, // normalized | unconvertible | no_unit
}

/// 单位规范化：无法转换的值留在原位置并附诊断，绝不为了画图丢掉。
pub fn normalize_quantity(raw: &str, unit: Option<&str>) -> (Option<f64>, Option<String>, String) {
    let value: f64 = match raw.trim().parse() {
        Ok(v) => v,
        Err(_) => return (None, None, "unconvertible".into()),
    };
    match unit {
        None => (None, None, "no_unit".into()),
        Some("m") => (Some(value), Some("m".into()), "normalized".into()),
        Some("mm") => (Some(value / 1000.0), Some("m".into()), "normalized".into()),
        Some("cm") => (Some(value / 100.0), Some("m".into()), "normalized".into()),
        Some("K") => (Some(value), Some("K".into()), "normalized".into()),
        Some("degC") | Some("°C") => (Some(value + 273.15), Some("K".into()), "normalized".into()),
        Some(_) => (None, None, "unconvertible".into()),
    }
}
