//! Derived versions: an independent derivation, its inputs, parameters and
//! decisions. Versions are immutable once committed; decisions produce new
//! versions rather than overwriting history.

use crate::merge::{Event, MergedModel};
use crate::model::RawModel;
use crate::propagate::PropagationReport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A human or system decision. The rejected candidate is retained, so a
/// confirmation is never an irreversible overwrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub decision_id: String,
    pub subject: String,
    pub adopted: String,
    pub rejected: Vec<String>,
    pub reason: String,
    /// Version this decision was made against.
    pub based_on: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub event_id: String,
    pub seq: u64,
    pub kind: String, // import | parameter_change | source_change | decision | note
    pub code: Option<String>,
    pub payload: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionParams {
    /// Example normalization parameter: canonical pressure unit.
    pub pressure_unit: String,
    /// Example: whether to surface anomalies as blocking.
    pub strict_dependencies: bool,
}

impl Default for VersionParams {
    fn default() -> Self {
        VersionParams { pressure_unit: "Pa".into(), strict_dependencies: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub version_id: String,
    pub branch: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub kind: String,
    pub seq: u64,
    pub base_parent_id: Option<String>,
    pub replay_step: Option<i64>,
    pub parameters: VersionParams,
    pub events: Vec<EventRecord>,
    pub decisions: Vec<Decision>,
    pub propagation: Option<PropagationReport>,
    /// Merge events snapshot (anomalies/judgments as seen at this version).
    pub merge_events: Vec<Event>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<RawModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged: Option<MergedModel>,
}
