//! Event-sourced history. Every state change is an event; judgments produced
//! while applying it are stored inside the payload so the archive replays
//! exactly.

use crate::judgment::Judgment;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Baseline import of NodeSet files (content embedded).
    ImportFiles { files: Vec<ImportFile> },
    /// Parameter revision: change one node's value in one file.
    ParamRevision { file: String, node_id_text: String, new_value: String, note: String },
    /// Source-data change: replace one file's content (local dependency only).
    SourceChange { file: String, new_content: String, note: String },
    /// A human decision (select candidate, resolve dep version, accept branch...).
    Decision { decision: Decision, note: String },
    /// Free-form annotation.
    Note { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    /// Pick one candidate of a duplicated NodeId; others stay retained.
    SelectCandidate { node_canonical: String, source_file: String },
    /// Resolve a model-dependency version conflict.
    ResolveDepVersion { model_uri: String, chosen_version: String },
    /// Accept one branch as the continuation; the other branch is kept.
    AcceptBranch { branch: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub seq: u64,
    pub branch: String,
    pub parent: Option<String>,
    pub op: Operation,
    pub judgments: Vec<Judgment>,
    pub created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub id: String,
    pub seq: u64,
    pub event_id: String,
    pub branch: String,
    /// Full projection snapshot (JSON of state::Projection).
    pub state: String,
}
