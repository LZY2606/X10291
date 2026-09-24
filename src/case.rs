//! Replayable offline case pack driver.
//!
//! The namespace-compass pack is pure repository fixture: initial baseline,
//! one parameter revision, one local dependency edit and one conflicting
//! human decision. Each replay step names the files involved so the
//! dependency-graph recompute only rebuilds affected nodes.

use crate::merge;
use crate::raw;
use crate::store::Store;
use serde::{Deserialize, Serialize};

pub const CASE_DIR: &str = "fixtures/cases/namespace-compass";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub title: String,
    pub narrative: String,
    pub steps: Vec<CaseStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseStep {
    pub id: String,
    pub title: String,
    pub kind: String, // import | revise_parameter | local_dependency_edit | boundary | decision | reset
    pub branch: String,
    pub files: Vec<String>,
    pub base_step: Option<String>,
    pub expected_events: Vec<String>,
    pub description: String,
    pub decision: Option<StepDecision>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepDecision {
    pub subject_kind: String, // duplicate | requirement | edge
    pub subject_match: String,
    pub choice: String,
    pub expect_conflict_with: Option<String>,
}

pub struct LoadedCase {
    pub manifest: Manifest,
    pub root: String,
}

pub fn discover(root_override: Option<&str>) -> Option<LoadedCase> {
    let candidates = match root_override {
        Some(r) => vec![r.to_string()],
        None => vec![
            CASE_DIR.to_string(),
            format!("../{}/manifest.json", "fixtures/cases/namespace-compass"),
        ],
    };
    for dir in candidates {
        let path = format!("{}/manifest.json", dir);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(manifest) = serde_json::from_str::<Manifest>(&text) {
                return Some(LoadedCase {
                    manifest,
                    root: dir,
                });
            }
        }
    }
    None
}

impl LoadedCase {
    pub fn read_file(&self, rel: &str) -> Result<(String, String), String> {
        let path = std::path::Path::new(&self.root).join(rel);
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {}", path.display(), e))?;
        Ok((rel.to_string(), content))
    }

    pub fn load_files(&self, rels: &[String]) -> Result<Vec<raw::SourceFile>, String> {
        let mut out = Vec::new();
        for rel in rels {
            let (path, content) = self.read_file(rel)?;
            out.push(raw::parse_file(&path, &content)?);
        }
        Ok(out)
    }
}

/// Result of replaying a single step: version id + fired event kinds.
pub struct StepResult {
    pub version_id: String,
    pub event_kinds: Vec<String>,
    pub conflict: Option<String>,
}

pub fn replay_step(
    store: &Store,
    case: &LoadedCase,
    step_id: &str,
) -> Result<StepResult, String> {
    let step = case
        .manifest
        .steps
        .iter()
        .find(|s| s.id == step_id)
        .ok_or_else(|| format!("unknown step {}", step_id))?
        .clone();

    let files = case.load_files(&step.files)?;

    // decisions effective on this branch up to this point
    let mut decisions = store.decisions(&step.branch).unwrap_or_default();

    // optionally inject a manual decision (the conflicting-decision story)
    let mut conflict = None;
    if let Some(d) = &step.decision {
        let derived_preview = merge::build(
            files
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            &step.branch,
            &decisions,
        );
        let subject = find_subject(&derived_preview, &d.subject_kind, &d.subject_match)
            .ok_or_else(|| format!("decision subject {} not found", d.subject_match))?;
        let choice = if d.choice == "__pick-skid-v2__" {
            derived_preview
                .nodes
                .iter()
                .find(|n| n.id == subject)
                .and_then(|n| {
                    n.sources
                        .iter()
                        .find(|s| s.path.contains("skid") && s.path.contains("v2"))
                        .map(|s| s.raw_node_id.clone())
                })
                .unwrap_or_else(|| {
                    derived_preview
                        .nodes
                        .iter()
                        .find(|n| n.id == subject)
                        .and_then(|n| n.candidates.first().cloned())
                        .unwrap_or_default()
                })
        } else {
            d.choice.clone()
        };
        let current = store.current_version_id(&step.branch).unwrap_or_else(|| "none".into());
        if let Err(crate::store::StoreError::Conflict(msg)) =
            store.record_decision(&step.branch, &subject, &choice, &[], &current)
        {
            conflict = Some(msg);
        }
        decisions = store.decisions(&step.branch).unwrap_or_default();
    }

    let base_version = match &step.base_step {
        Some(base) => Some(
            store
                .history(&step.branch)
                .unwrap_or_default()
                .iter()
                .map(|(id, _, label, _)| (id.clone(), label.clone()))
                .find(|(_, label)| label == base)
                .map(|(id, _)| id)
                .unwrap_or_else(|| "none".into()),
        ),
        None => store.current_version_id(&step.branch),
    };

    let changed = changed_file_ids(&files, store);
    let (derived, _report) = match store.current(&step.branch).map_err(|e| e.to_string())? {
        Some(prev) if base_version.is_some() => {
            merge::recompute(&prev.derived, files, &step.branch, &decisions, &changed)
        }
        _ => (merge::build(files, &step.branch, &decisions), Default::default()),
    };

    let expected_token = store.edit_token(&step.branch);
    let token = if expected_token == "none" {
        None
    } else {
        Some(expected_token.as_str())
    };

    let version_id = store
        .save_version(
            &step.branch,
            &step.id,
            base_version.as_deref(),
            &reload_files(case, &step.files)?,
            &derived,
            "{}",
            token,
            false,
        )
        .map_err(|e| e.to_string())?;
    store.mark_decisions_applied(&step.branch, &version_id).ok();

    Ok(StepResult {
        version_id,
        event_kinds: derived.events.iter().map(|e| e.kind.clone()).collect(),
        conflict,
    })
}

fn reload_files(case: &LoadedCase, rels: &[String]) -> Result<Vec<raw::SourceFile>, String> {
    case.load_files(rels)
}

fn changed_file_ids(files: &[raw::SourceFile], store: &Store) -> Vec<String> {
    let old = store.files().unwrap_or_default();
    let old_map: std::collections::BTreeMap<String, String> = old
        .iter()
        .map(|f| (f.path.clone(), crate::identity::short_hash(&[&f.content])))
        .collect();
    files
        .iter()
        .filter(|f| old_map.get(&f.path) != Some(&crate::identity::short_hash(&[&f.content])))
        .map(|f| f.id.clone())
        .collect()
}

fn find_subject(
    derived: &crate::model::DerivedModel,
    kind: &str,
    matches: &str,
) -> Option<String> {
    match kind {
        "duplicate" => derived
            .nodes
            .iter()
            .find(|n| n.status == "duplicate_conflict" && n.canonical_node_id.contains(matches))
            .map(|n| n.id.clone()),
        "requirement" => derived
            .events
            .iter()
            .find(|e| e.kind == "model_version_conflict" && e.subject.contains(matches))
            .map(|e| e.subject.clone()),
        "edge" => derived
            .edges
            .iter()
            .find(|e| e.reference_type.contains(matches) || e.status.contains(matches))
            .map(|e| e.id.clone()),
        _ => None,
    }
}
