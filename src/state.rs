//! Project state serialized into versions.

use crate::model::{MergedModel, NodeSetFile};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    /// What is being decided, e.g. "dependency version for http://.../UA/".
    pub subject: String,
    /// All legal candidates; never collapsed silently.
    pub candidates: Vec<String>,
    /// Index into candidates once a human decided.
    pub chosen: Option<usize>,
    pub rationale: String,
    /// accepted | rejected | pending
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub files: Vec<NodeSetFile>,
    pub merged: MergedModel,
    pub decisions: Vec<Decision>,
}

impl ProjectState {
    pub fn empty() -> Self {
        ProjectState { files: vec![], merged: MergedModel::default(), decisions: vec![] }
    }
}
