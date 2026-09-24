use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RawReference {
    pub ref_type: String,
    pub is_forward: bool,
    pub target: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawNode {
    pub source_file: String,
    pub span_start: usize,
    pub span_end: usize,
    pub raw_xml: String,
    pub node_id: String,
    pub browse_name: String,
    pub node_class: String,
    pub display_name: Option<String>,
    pub references: Vec<RawReference>,
    pub value_raw: Option<String>,
    pub attrs: Vec<(String, String)>,
    pub extensions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequiredModel {
    pub uri: String,
    pub version: String,
    pub publication_date: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelRec {
    pub source_file: String,
    pub uri: String,
    pub version: String,
    pub publication_date: Option<String>,
    pub required: Vec<RequiredModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSetFile {
    pub path: String,
    pub content: String,
    pub namespaces: Vec<String>,
    pub aliases: Vec<(String, String)>,
    pub models: Vec<ModelRec>,
    pub nodes: Vec<RawNode>,
    pub file_extensions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergedNode {
    pub stable_id: String,
    pub canon: String,
    pub global_ns: usize,
    pub raw: RawNode,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Edge {
    pub src: String,
    pub dst: String,
    pub ref_type: String,
    pub forward_origin: bool,
    pub source_file: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AliasRec {
    pub source_file: String,
    pub alias: String,
    pub target_raw: String,
    pub status: String,
    pub resolved_canon: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unresolved {
    pub kind: String,
    pub subject: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: String,
    pub rule: String,
    pub subject: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub subject: String,
    pub adopted: String,
    pub rejected: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NsMapRow {
    pub file: String,
    pub local_index: usize,
    pub uri: String,
    pub global_index: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub kind: String,
    pub rule: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MergedModel {
    pub registry: Vec<String>,
    pub ns_maps: Vec<NsMapRow>,
    pub nodes: Vec<MergedNode>,
    pub edges: Vec<Edge>,
    pub aliases: Vec<AliasRec>,
    pub models: Vec<ModelRec>,
    pub unresolved: Vec<Unresolved>,
    pub diagnostics: Vec<Diagnostic>,
    pub candidates: Vec<Candidate>,
    pub events: Vec<Event>,
    pub files: Vec<(String, String)>,
}

impl MergedModel {
    pub fn find_by_browse(&self, name: &str) -> Option<&MergedNode> {
        self.nodes.iter().find(|n| {
            n.raw.browse_name == name
                || n.raw
                    .browse_name
                    .rsplit(':')
                    .next()
                    .map(|s| s == name)
                    .unwrap_or(false)
        })
    }

    pub fn node_by_id(&self, stable_id: &str) -> Option<&MergedNode> {
        self.nodes.iter().find(|n| n.stable_id == stable_id)
    }
}
