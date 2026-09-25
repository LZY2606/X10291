//! Engine: applies operations, emits machine-readable judgment events,
//! commits versions atomically.

use crate::merge::{merge, CORE_URI};
use crate::model::{stable_id, Diagnostic, NodeSetFile};
use crate::recompute::plan_recompute;
use crate::state::{Decision, ProjectState};
use crate::store::{SaveOutcome, Store};
use crate::xml_import::parse_nodeset;

pub struct Engine {
    pub store: Store,
}

#[derive(Debug)]
pub enum EngineError {
    Store(rusqlite::Error),
    Import(crate::xml_import::ImportError),
    NotFound(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Store(e) => write!(f, "store: {}", e),
            EngineError::Import(e) => write!(f, "import: {}", e),
            EngineError::NotFound(s) => write!(f, "not found: {}", s),
        }
    }
}
impl std::error::Error for EngineError {}
impl From<rusqlite::Error> for EngineError {
    fn from(e: rusqlite::Error) -> Self {
        EngineError::Store(e)
    }
}
impl From<crate::xml_import::ImportError> for EngineError {
    fn from(e: crate::xml_import::ImportError) -> Self {
        EngineError::Import(e)
    }
}

pub type Res<T> = Result<T, EngineError>;

impl Engine {
    pub fn new(store: Store) -> Engine {
        Engine { store }
    }

    pub fn current(&self, branch: &str) -> Res<(Option<String>, ProjectState)> {
        match self.store.head(branch)? {
            Some(id) => {
                let json = self
                    .store
                    .version_state(&id)?
                    .ok_or_else(|| EngineError::NotFound(id.clone()))?;
                let state: ProjectState = serde_json::from_str(&json)
                    .map_err(|e| EngineError::NotFound(format!("bad state: {}", e)))?;
                Ok((Some(id), state))
            }
            None => Ok((None, ProjectState::empty())),
        }
    }

    fn commit(
        &self,
        branch: &str,
        expected: Option<&str>,
        label: &str,
        state: &ProjectState,
        events: Vec<(String, serde_json::Value)>,
    ) -> Res<SaveOutcome> {
        let json = serde_json::to_string(state).unwrap();
        let outcome = self.store.save_version(branch, expected, label, &json)?;
        match &outcome {
            SaveOutcome::Committed { version_id } => {
                for (kind, payload) in events {
                    self.store.append_event(branch, Some(version_id), &kind, &payload)?;
                }
            }
            SaveOutcome::Conflict { kept_version_id, kept_branch, .. } => {
                for (kind, payload) in events {
                    self.store.append_event(kept_branch, Some(kept_version_id), &kind, &payload)?;
                }
            }
        }
        Ok(outcome)
    }

    /// Events translating merged-model diagnostics into machine-readable
    /// judgment records.
    fn judgment_events(state: &ProjectState, file_id: &str) -> Vec<(String, serde_json::Value)> {
        let mut ev = vec![];
        let file = state.files.iter().find(|f| f.file_id == file_id);
        if let Some(f) = file {
            for (i, uri) in f.namespaces.iter().enumerate() {
                let global_index =
                    state.merged.namespace_uris.iter().position(|u| u == uri).unwrap_or(0);
                ev.push((
                    "NamespaceRemapped".into(),
                    serde_json::json!({
                        "rule": "numeric namespace index is only meaningful inside one file; merge remaps by URI",
                        "file_id": f.file_id,
                        "local_ns": i,
                        "uri": uri,
                        "global_index": global_index,
                    }),
                ));
            }
            for n in &f.nodes {
                for ext in &n.extensions {
                    ev.push((
                        "UnknownExtensionPreserved".into(),
                        serde_json::json!({
                            "file_id": f.file_id,
                            "node": n.node_id_text,
                            "element": ext.element_name,
                        }),
                    ));
                }
            }
        }
        for e in &state.merged.edges {
            if !e.forward {
                ev.push((
                    "InverseNormalized".into(),
                    serde_json::json!({
                        "rule": "inverse and forward references are normalized but no missing edge is fabricated",
                        "source": stable_id(&e.source),
                        "reference_type": stable_id(&e.reference_type),
                        "target": stable_id(&e.target),
                        "declared_inverse": true,
                        "origin_file": e.origin_file,
                    }),
                ));
            }
        }
        for d in &state.merged.diagnostics {
            let (kind, payload) = match d {
                Diagnostic::AliasCycle { file_id: f, chain } => (
                    "AliasCycleDetected",
                    serde_json::json!({"file_id": f, "chain": chain}),
                ),
                Diagnostic::UnresolvableAlias { file_id: f, alias } => (
                    "UnresolvableAlias",
                    serde_json::json!({"file_id": f, "alias": alias}),
                ),
                Diagnostic::DuplicateNodeId { id, files } => (
                    "DuplicateNodeId",
                    serde_json::json!({"id": stable_id(id), "files": files, "candidates_kept": true}),
                ),
                Diagnostic::DanglingReference { source, reference_type, target } => (
                    "DanglingReference",
                    serde_json::json!({
                        "source": stable_id(source),
                        "reference_type": stable_id(reference_type),
                        "target": stable_id(target),
                        "edge_kept": true,
                    }),
                ),
                Diagnostic::UnresolvedDependency { requiring_model, missing_model_uri } => (
                    "UnresolvedDependency",
                    serde_json::json!({
                        "rule": "missing dependency keeps an unresolved node",
                        "requiring_model": requiring_model,
                        "missing_model_uri": missing_model_uri,
                    }),
                ),
                Diagnostic::DependencyVersionConflict { model_uri, candidates } => (
                    "DependencyVersionConflict",
                    serde_json::json!({"model_uri": model_uri, "candidates": candidates}),
                ),
                Diagnostic::UnknownNamespaceIndex { file_id: f, ns, context } => (
                    "UnknownNamespaceIndex",
                    serde_json::json!({"file_id": f, "ns": ns, "context": context}),
                ),
                Diagnostic::UnconvertibleValue { node, attr, raw, reason } => (
                    "UnconvertibleValue",
                    serde_json::json!({
                        "node": stable_id(node), "attr": attr, "raw": raw, "reason": reason,
                        "kept_in_place": true,
                    }),
                ),
            };
            ev.push((kind.to_string(), payload));
        }
        ev
    }

    /// Import (or replace) one NodeSet file and commit a new version.
    pub fn import_file(
        &self,
        branch: &str,
        file_id: &str,
        xml: &str,
        label: &str,
    ) -> Res<SaveOutcome> {
        let parsed: NodeSetFile = parse_nodeset(file_id, xml)?;
        let (head, mut state) = self.current(branch)?;
        let old_files = state.files.clone();
        state.files.retain(|f| f.file_id != file_id);
        state.files.push(parsed);
        state.merged = merge(&state.files);
        let mut events = vec![(
            "Import".to_string(),
            serde_json::json!({
                "file_id": file_id,
                "node_count": state.files.last().map(|f| f.nodes.len()).unwrap_or(0),
                "namespaces": state.files.last().map(|f| f.namespaces.clone()).unwrap_or_default(),
                "aliases": state.files.last().map(|f| f.aliases.len()).unwrap_or(0),
                "models": state.files.last().map(|f| f.models.len()).unwrap_or(0),
            }),
        )];
        events.extend(Self::judgment_events(&state, file_id));
        let report = plan_recompute(&old_files, &state.files, &state.merged);
        events.push((
            "Recompute".to_string(),
            serde_json::json!({
                "changed_files": report.changed_files,
                "recomputed": report.recomputed,
                "still_valid": report.still_valid,
                "reasons": report.reasons,
            }),
        ));
        Ok(self.commit(branch, head.as_deref(), label, &state, events)?)
    }

    /// Delete a node from a file's raw layer (dangling-reference scenario).
    pub fn delete_node(&self, branch: &str, file_id: &str, node_id_text: &str) -> Res<SaveOutcome> {
        let (head, mut state) = self.current(branch)?;
        let old_files = state.files.clone();
        let file = state
            .files
            .iter_mut()
            .find(|f| f.file_id == file_id)
            .ok_or_else(|| EngineError::NotFound(file_id.to_string()))?;
        let before = file.nodes.len();
        file.nodes.retain(|n| n.node_id_text != node_id_text);
        if file.nodes.len() == before {
            return Err(EngineError::NotFound(node_id_text.to_string()));
        }
        state.merged = merge(&state.files);
        let mut events = vec![(
            "NodeDeleted".to_string(),
            serde_json::json!({"file_id": file_id, "node_id": node_id_text}),
        )];
        events.extend(Self::judgment_events(&state, file_id));
        let report = plan_recompute(&old_files, &state.files, &state.merged);
        events.push((
            "Recompute".to_string(),
            serde_json::json!({
                "changed_files": report.changed_files,
                "recomputed": report.recomputed,
                "still_valid": report.still_valid,
            }),
        ));
        Ok(self.commit(
            branch,
            head.as_deref(),
            &format!("delete {} from {}", node_id_text, file_id),
            &state,
            events,
        )?)
    }

    /// Record a human decision (accept/reject a candidate). The unchosen
    /// candidates stay in the state; nothing is overwritten irreversibly.
    pub fn decide(
        &self,
        branch: &str,
        subject: &str,
        candidates: Vec<String>,
        chosen: Option<usize>,
        accept: bool,
        rationale: &str,
    ) -> Res<SaveOutcome> {
        let (head, mut state) = self.current(branch)?;
        let id = format!("d{:03}", state.decisions.len() + 1);
        state.decisions.push(Decision {
            id: id.clone(),
            subject: subject.to_string(),
            candidates: candidates.clone(),
            chosen,
            rationale: rationale.to_string(),
            status: if accept { "accepted" } else { "rejected" }.to_string(),
        });
        let events = vec![(
            if accept { "DecisionAccepted" } else { "DecisionRejected" }.to_string(),
            serde_json::json!({
                "decision_id": id,
                "subject": subject,
                "candidates": candidates,
                "chosen": chosen,
                "unchosen_kept": true,
                "rationale": rationale,
            }),
        )];
        Ok(self.commit(branch, head.as_deref(), &format!("decision: {}", subject), &state, events)?)
    }

    /// Replay: state visible at a given event sequence number.
    pub fn state_at_event(&self, seq: i64) -> Res<Option<(String, ProjectState)>> {
        let events = self.store.events_up_to(seq)?;
        let vid = events.iter().rev().find_map(|e| e.version_id.clone());
        match vid {
            Some(id) => {
                let json = self
                    .store
                    .version_state(&id)?
                    .ok_or_else(|| EngineError::NotFound(id.clone()))?;
                let state: ProjectState = serde_json::from_str(&json)
                    .map_err(|e| EngineError::NotFound(e.to_string()))?;
                Ok(Some((id, state)))
            }
            None => Ok(None),
        }
    }

    pub fn core_uri(&self) -> &'static str {
        CORE_URI
    }
}

impl Engine {
    /// Decide with an explicit expected parent (used to model a second user
    /// editing from the same base version -> resolvable conflict).
    pub fn decide_at(
        &self,
        branch: &str,
        expected_parent: Option<&str>,
        subject: &str,
        candidates: Vec<String>,
        chosen: Option<usize>,
        accept: bool,
        rationale: &str,
    ) -> Res<SaveOutcome> {
        let (head, mut state) = self.current(branch)?;
        let expected = expected_parent.map(|s| s.to_string()).or(head);
        // When simulating a concurrent user, load state at the expected parent.
        if let Some(ep) = &expected {
            if self.store.head(branch)? != Some(ep.clone()) {
                let json = self
                    .store
                    .version_state(ep)?
                    .ok_or_else(|| EngineError::NotFound(ep.clone()))?;
                state = serde_json::from_str(&json)
                    .map_err(|e| EngineError::NotFound(e.to_string()))?;
            }
        }
        let id = format!("d{:03}", state.decisions.len() + 1);
        state.decisions.push(Decision {
            id: id.clone(),
            subject: subject.to_string(),
            candidates: candidates.clone(),
            chosen,
            rationale: rationale.to_string(),
            status: if accept { "accepted" } else { "rejected" }.to_string(),
        });
        let events = vec![(
            if accept { "DecisionAccepted" } else { "DecisionRejected" }.to_string(),
            serde_json::json!({
                "decision_id": id,
                "subject": subject,
                "candidates": candidates,
                "chosen": chosen,
                "unchosen_kept": true,
                "rationale": rationale,
            }),
        )];
        Ok(self.commit(
            branch,
            expected.as_deref(),
            &format!("decision: {}", subject),
            &state,
            events,
        )?)
    }

    /// Promote a version to a branch head after human comparison. The
    /// previously current version stays in the store; nothing is overwritten.
    pub fn promote(&self, branch: &str, version_id: &str) -> Res<()> {
        if self.store.version_state(version_id)?.is_none() {
            return Err(EngineError::NotFound(version_id.to_string()));
        }
        let old = self.store.head(branch)?;
        self.store.conn.execute(
            "INSERT OR REPLACE INTO heads(branch,version_id) VALUES(?1,?2)",
            rusqlite::params![branch, version_id],
        )?;
        self.store.append_event(
            branch,
            Some(version_id),
            "BranchPromoted",
            &serde_json::json!({
                "branch": branch,
                "promoted": version_id,
                "previous_head": old,
                "unchosen_kept": true,
            }),
        )?;
        Ok(())
    }

    pub fn reset(&self) -> Res<()> {
        self.store.conn.execute_batch(
            "DELETE FROM events; DELETE FROM versions; DELETE FROM heads; DELETE FROM artifacts;",
        )?;
        Ok(())
    }
}
