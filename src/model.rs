use crate::xml::{XmlAttr, XmlEl};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const UA_NS0: &str = "http://opcfoundation.org/UA/";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeClass {
    Object,
    Variable,
    ObjectType,
    VariableType,
    DataType,
    ReferenceType,
    Method,
    View,
}

impl NodeClass {
    pub fn from_element(name: &str) -> Option<NodeClass> {
        let local = name.rsplit(':').next().unwrap_or(name);
        Some(match local {
            "UAObject" => NodeClass::Object,
            "UAVariable" => NodeClass::Variable,
            "UAObjectType" => NodeClass::ObjectType,
            "UAVariableType" => NodeClass::VariableType,
            "UADataType" => NodeClass::DataType,
            "UAReferenceType" => NodeClass::ReferenceType,
            "UAMethod" => NodeClass::Method,
            "UAView" => NodeClass::View,
            _ => return None,
        })
    }
    pub fn label(&self) -> &'static str {
        match self {
            NodeClass::Object => "Object",
            NodeClass::Variable => "Variable",
            NodeClass::ObjectType => "ObjectType",
            NodeClass::VariableType => "VariableType",
            NodeClass::DataType => "DataType",
            NodeClass::ReferenceType => "ReferenceType",
            NodeClass::Method => "Method",
            NodeClass::View => "View",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RawNodeId {
    Numeric { ns: u32, id: u64 },
    Str { ns: u32, id: String },
    Guid { ns: u32, id: String },
    Opaque { ns: u32, id: String },
}

impl RawNodeId {
    pub fn parse_raw(s: &str) -> Option<RawNodeId> {
        let s = s.trim();
        let (ns, rest) = if let Some(r) = s.strip_prefix("ns=") {
            let (n, r) = r.split_once(';')?;
            (n.parse::<u32>().ok()?, r)
        } else {
            (0u32, s)
        };
        if let Some(i) = rest.strip_prefix("i=") {
            Some(RawNodeId::Numeric {
                ns,
                id: i.parse().ok()?,
            })
        } else if let Some(t) = rest.strip_prefix("s=") {
            Some(RawNodeId::Str {
                ns,
                id: t.to_string(),
            })
        } else if let Some(t) = rest.strip_prefix("g=") {
            Some(RawNodeId::Guid {
                ns,
                id: t.to_string(),
            })
        } else {
            rest.strip_prefix("b=").map(|t| RawNodeId::Opaque {
                ns,
                id: t.to_string(),
            })
        }
    }
    pub fn ns(&self) -> u32 {
        match self {
            RawNodeId::Numeric { ns, .. }
            | RawNodeId::Str { ns, .. }
            | RawNodeId::Guid { ns, .. }
            | RawNodeId::Opaque { ns, .. } => *ns,
        }
    }
    pub fn id_part(&self) -> String {
        match self {
            RawNodeId::Numeric { id, .. } => format!("i={}", id),
            RawNodeId::Str { id, .. } => format!("s={}", id),
            RawNodeId::Guid { id, .. } => format!("g={}", id),
            RawNodeId::Opaque { id, .. } => format!("b={}", id),
        }
    }
}

pub fn global_node_id(uri: &str, id_part: &str) -> String {
    format!("nsu={};{}", uri, id_part)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawRef {
    pub reftype_raw: String,
    pub is_forward: bool,
    pub target_raw: String,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawNode {
    pub raw_id: String,
    pub class: NodeClass,
    pub browse_name_raw: String,
    pub display_name: Option<String>,
    pub attrs: Vec<XmlAttr>,
    pub references: Vec<RawRef>,
    pub span: (usize, usize),
    pub element: XmlEl,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelDep {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelTable {
    pub model: ModelDep,
    pub required: Vec<ModelDep>,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NsFile {
    pub file_id: String,
    pub name: String,
    pub sha: String,
    pub namespace_uris: Vec<String>,
    pub aliases: Vec<(String, String)>,
    pub models: Vec<ModelTable>,
    pub nodes: Vec<RawNode>,
    pub extensions: Vec<XmlEl>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceRef {
    pub file_id: String,
    pub file_name: String,
    pub start: usize,
    pub end: usize,
    pub excerpt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    pub sources: Vec<SourceRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Anomaly {
    pub id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub detail: serde_json::Value,
    pub sources: Vec<SourceRef>,
    pub candidates: Vec<Candidate>,
    pub status: String,
    pub adopted: Option<String>,
    pub rejected: Vec<String>,
}

impl Anomaly {
    pub fn new(code: &str, severity: &str, id_key: &str, message: String) -> Anomaly {
        Anomaly {
            id: format!("an:{}", crate::util::id_hash(&[code, id_key])),
            code: code.to_string(),
            severity: severity.to_string(),
            message,
            detail: serde_json::Value::Null,
            sources: vec![],
            candidates: vec![],
            status: "open".to_string(),
            adopted: None,
            rejected: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub anomaly_id: String,
    pub choice: String,
    pub rationale: String,
    pub actor: String,
    pub seq: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub map: BTreeMap<String, serde_json::Value>,
    pub rev: i64,
}

impl Default for Profile {
    fn default() -> Self {
        let mut map = BTreeMap::new();
        map.insert(
            "dep_version_policy".to_string(),
            serde_json::Value::String("exact".to_string()),
        );
        map.insert(
            "temperature_unit".to_string(),
            serde_json::Value::String("C".to_string()),
        );
        Profile { map, rev: 0 }
    }
}

impl Profile {
    pub fn get_str(&self, key: &str) -> &str {
        self.map
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }
    pub fn apply_changes(
        &mut self,
        changes: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<(String, serde_json::Value, serde_json::Value)> {
        let mut out = vec![];
        for (k, v) in changes {
            let from = self.map.get(k).cloned().unwrap_or(serde_json::Value::Null);
            self.map.insert(k.clone(), v.clone());
            out.push((k.clone(), from, v.clone()));
        }
        self.rev += 1;
        out
    }
    pub fn fingerprint(&self) -> String {
        crate::util::id_hash(&[serde_json::to_string(&self.map).unwrap_or_default().as_str()])
    }
}
