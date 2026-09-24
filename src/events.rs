//! Append-only event log. Every import, decision or parameter change is one
//! event; state is rebuilt by deterministic replay, so any historical event
//! restores that moment's dependencies and conclusions.

use crate::model::RawFile;
use crate::merge::{self, MergeInput};
use crate::model::MergedModel;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    ImportFile,
    ParameterChange,
    LocalSourceChange,
    ManualDecision,
    BranchCheckpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub branch: String,
    pub kind: EventKind,
    pub title: String,
    pub detail: String,
    pub rule_version: String,
    /// Inline material so replay needs nothing outside the repo/export.
    pub file_path: Option<String>,
    pub file_content: Option<String>,
    /// Arbitrary decision/parameter payload.
    pub payload: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ReplayFrame {
    pub seq: u64,
    pub model: MergedModel,
    pub raw: Vec<RawFile>,
}

/// Replay events on one branch up to and including `through_seq`.
///
/// Non-main branches inherit the main timeline up to their checkpoint: main
/// events with `seq <= base_seq` are applied first, so a branch comparison sees
/// the full material.
pub fn replay_with_base(
    events: &[Event],
    branch: &str,
    base_seq: u64,
    through_seq: Option<u64>,
) -> Result<ReplayFrame, String> {
    replay_inner(events, branch, base_seq, through_seq)
}

pub fn replay(events: &[Event], branch: &str, through_seq: Option<u64>) -> Result<ReplayFrame, String> {
    let base_seq = if branch == "main" { 0 } else { u64::MAX };
    replay_inner(events, branch, base_seq, through_seq)
}

fn replay_inner(
    events: &[Event],
    branch: &str,
    base_seq: u64,
    through_seq: Option<u64>,
) -> Result<ReplayFrame, String> {
    let mut raw: BTreeMap<String, RawFile> = BTreeMap::new();
    let mut deleted = std::collections::BTreeSet::new();
    let mut frames: Vec<ReplayFrame> = Vec::new();
    let mut version = 0u64;
    let mut parent: Option<u64> = None;

    for ev in events {
        let is_base = ev.branch == "main" && branch != "main" && ev.seq <= base_seq;
        let is_self = ev.branch == branch;
        if !is_base && !is_self {
            continue;
        }
        if let Some(t) = through_seq {
            if ev.seq > t {
                break;
            }
        }
        if let Some(t) = through_seq {
            if ev.seq > t {
                break;
            }
        }
        match ev.kind {
            EventKind::ImportFile | EventKind::LocalSourceChange => {
                if let (Some(path), Some(content)) = (&ev.file_path, &ev.file_content) {
                    let parsed = crate::parser::parse_file(path, content);
                    deleted.remove(&parsed.file_id);
                    raw.insert(parsed.file_id.clone(), parsed);
                }
            }
            EventKind::ParameterChange | EventKind::ManualDecision | EventKind::BranchCheckpoint => {}
        }
        if ev.payload.get("delete_file_id").is_some() {
            if let Some(id) = ev.payload.get("delete_file_id") {
                deleted.insert(id.clone());
            }
        }

        version += 1;
        let raw_slice: Vec<RawFile> = raw.values().cloned().collect();
        let input = MergeInput { deleted_file_ids: deleted.clone() };
        let model = merge::build_with(&raw_slice, version, parent, &input);
        parent = Some(version);
        frames.push(ReplayFrame { seq: ev.seq, model, raw: raw_slice });
    }

    frames.into_iter().last().ok_or_else(|| format!("no events on branch {branch:?}"))
}

/// All frames (one per event) for the timeline view and historical restore.
pub fn replay_all(events: &[Event], branch: &str) -> Vec<ReplayFrame> {
    let mut out = Vec::new();
    let seqs: Vec<u64> = events.iter().filter(|e| e.branch == branch).map(|e| e.seq).collect();
    for s in seqs {
        if let Ok(frame) = replay(events, branch, Some(s)) {
            out.push(frame);
        }
    }
    out
}

pub fn raw_file_id(path: &str, content: &str) -> String {
    crate::model::stable_id("file", &[path, content])
}

/// Machine-readable rationale records attached to a merge (for export).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RationaleEvent {
    pub code: &'static str,
    pub rule_version: &'static str,
    pub statement: String,
}

pub fn rationale_catalog() -> Vec<RationaleEvent> {
    vec![
        RationaleEvent { code: "NS_INDEX_FILE_LOCAL", rule_version: merge::RULE_VERSION,
            statement: "numeric namespace indices are meaningful only within one file; merges remap by namespace URI".into() },
        RationaleEvent { code: "INVERSE_NORMALIZE_NO_EDGE", rule_version: merge::RULE_VERSION,
            statement: "inverse and forward references are normalized to one directed edge; normalization never fabricates the missing edge".into() },
        RationaleEvent { code: "MISSING_DEP_UNRESOLVED", rule_version: merge::RULE_VERSION,
            statement: "missing dependencies keep nodes as unresolved placeholders instead of dropping them".into() },
        RationaleEvent { code: "MULTIPLE_CANDIDATES_KEPT", rule_version: merge::RULE_VERSION,
            statement: "when no unique resolution exists, all legal candidates remain visible and none is selected implicitly".into() },
    ]
}
