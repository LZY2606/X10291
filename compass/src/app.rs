//! Application orchestration: run the bundled replay into a store and expose
//! version lookup used by the HTTP layer.

use crate::derive::EventRecord;
use crate::engine::{derive, DeriveRequest};
use crate::merge::MergedModel;
use crate::model::RawModel;
use crate::replay::{self, StepSpec};
use crate::store::Store;
use anyhow::Result;

pub struct Orchestrator {
    pub store: Store,
}

pub struct ReplaySummary {
    pub steps: usize,
    pub current: String,
}

fn run_steps(
    store: &Store,
    steps: Vec<StepSpec>,
    start_seq: u64,
    prior_map: &mut std::collections::BTreeMap<String, MergedModel>,
    id_map: &mut std::collections::BTreeMap<(String, i64), String>,
) -> Result<usize> {
    let mut count = 0;
    for (i, spec) in steps.iter().enumerate() {
        let seq = start_seq + i as u64;
        let raw = replay::build_raw(spec.files);
        let parent_chain = parent_for(spec, id_map, seq);
        let prior = parent_chain
            .as_ref()
            .and_then(|pid| prior_map.get(pid).cloned());
        let changed_sources = spec.changed_sources.iter().map(|s| s.to_string()).collect();
        let changed_parameters = spec.changed_parameters.iter().map(|s| s.to_string()).collect();

        let mut events: Vec<EventRecord> = vec![replay::record_for(
            seq,
            spec.kind,
            Some(spec.kind),
            &[
                ("label", spec.label),
                ("branch", spec.branch),
                ("note", spec.note),
            ],
        )];
        // Source material records for provenance.
        for (sid, path, _) in spec.files {
            events.push(replay::record_for(
                seq,
                "source_snapshot",
                Some("SOURCE_SNAPSHOT"),
                &[("source_id", sid), ("path", path)],
            ));
        }
        for p in spec.changed_parameters {
            events.push(replay::record_for(
                seq,
                "parameter_change",
                Some("PARAMETER_CHANGE"),
                &[("parameter", p), ("value", &spec.params.pressure_unit)],
            ));
        }

        // Decisions are made relative to the parent version.
        let decisions = match &parent_chain {
            Some(pid) => spec
                .decisions
                .iter()
                .map(|d| replay::decision_for(seq, pid, d))
                .collect(),
            None => Vec::new(),
        };

        let (version, merged) = derive(DeriveRequest {
            seq,
            branch: spec.branch.to_string(),
            parent: parent_chain.clone(),
            base_parent: Some("main#0".to_string()).filter(|_| spec.branch == "boundary"),
            label: spec.label.to_string(),
            kind: spec.kind.to_string(),
            replay_step: spec.replay_step,
            parameters: spec.params.clone(),
            raw: &raw,
            prior: prior.as_ref(),
            changed_sources,
            changed_parameters,
            decisions,
            events,
        });

        store.commit_version(&version, &raw, &merged, None)?;
        if let Some(step) = spec.replay_step {
            id_map.insert((spec.branch.to_string(), step), version.version_id.clone());
        }
        prior_map.insert(version.version_id.clone(), merged);
        count += 1;
    }
    Ok(count)
}

fn parent_for(
    spec: &StepSpec,
    id_map: &std::collections::BTreeMap<(String, i64), String>,
    seq: u64,
) -> Option<String> {
    if spec.replay_step == Some(0) {
        return None;
    }
    if spec.branch == "boundary" && spec.replay_step == Some(10) {
        // Fork from main baseline.
        return id_map.get(&("main".to_string(), 0)).cloned();
    }
    let step = spec.replay_step.unwrap_or(seq as i64);
    let prev = step - 1;
    id_map.get(&(spec.branch.to_string(), prev)).cloned()
}

impl Orchestrator {
    pub fn with_store(store: Store) -> Self {
        Orchestrator { store }
    }

    pub fn bootstrap_if_empty(&self) -> Result<Option<ReplaySummary>> {
        if self.store.current_version_id()?.is_some() {
            return Ok(None);
        }
        Some(self.run_replay()).transpose()
    }

    pub fn run_replay(&self) -> Result<ReplaySummary> {
        let mut prior_map = std::collections::BTreeMap::new();
        let mut id_map = std::collections::BTreeMap::new();
        let main = replay::main_branch_steps();
        let n1 = run_steps(&self.store, main, 0, &mut prior_map, &mut id_map)?;
        let boundary = replay::branch_steps();
        let n2 = run_steps(&self.store, boundary, n1 as u64, &mut prior_map, &mut id_map)?;
        let current = id_map
            .get(&("main".to_string(), 2))
            .cloned()
            .expect("main v3 exists");
        Ok(ReplaySummary { steps: n1 + n2, current })
    }
}

// Silence unused warnings for RawModel re-export used by the web layer.
#[allow(dead_code)]
fn _types(_: RawModel) {}
