//! Version derivation engine.
//!
//! A derivation consumes the retained raw model, parameters and explicit
//! decisions, runs merge + propagation, and returns an immutable Version.
//! The caller persists it atomically; derivation itself never mutates history.

use crate::derive::{Decision, EventRecord, Version, VersionParams};
use crate::id::stable_id;
use crate::merge::{self, MergedModel};
use crate::model::RawModel;
use crate::propagate;

pub fn event_record(seq: u64, kind: &str, code: Option<&str>, pairs: &[(&str, &str)]) -> EventRecord {
    let payload = pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    EventRecord {
        event_id: stable_id("evt", &[kind, &seq.to_string(), &pairs.len().to_string()]),
        seq,
        kind: kind.to_string(),
        code: code.map(str::to_string),
        payload,
    }
}

pub struct DeriveRequest<'a> {
    pub seq: u64,
    pub branch: String,
    pub parent: Option<String>,
    pub base_parent: Option<String>,
    pub label: String,
    pub kind: String,
    pub replay_step: Option<i64>,
    pub parameters: VersionParams,
    pub raw: &'a RawModel,
    pub prior: Option<&'a MergedModel>,
    pub changed_sources: Vec<String>,
    pub changed_parameters: Vec<String>,
    pub decisions: Vec<Decision>,
    pub events: Vec<EventRecord>,
}

pub fn derive(req: DeriveRequest) -> (Version, MergedModel) {
    let merged = merge::merge(req.raw);
    let (_dirty, report) = match req.prior {
        Some(prior) => propagate::affected_closure(prior, &req.changed_sources, &req.changed_parameters),
        None => (Default::default(), Default::default()),
    };
    let version_id = stable_id(
        "ver",
        &[
            &req.branch,
            &req.seq.to_string(),
            &req.label,
            &serde_json::to_string(&req.raw.sources.iter().map(|s| (&s.source_id, &s.bytes)).collect::<Vec<_>>()).unwrap_or_default(),
            &req.parameters.pressure_unit,
        ],
    );
    let merge_events = merged.events.clone();
    (
        Version {
            version_id,
            branch: req.branch,
            parent_id: req.parent,
            label: req.label,
            kind: req.kind,
            seq: req.seq,
            base_parent_id: req.base_parent,
            replay_step: req.replay_step,
            parameters: req.parameters,
            events: req.events,
            decisions: req.decisions,
            propagation: Some(report),
            merge_events,
            raw: Some(req.raw.clone()),
            merged: Some(merged.clone()),
        },
        merged,
    )
}
