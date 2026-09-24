//! Application services: import, dependency-graph propagation, versioning,
//! branching/decisions, diff and export/import.

use crate::domain::*;
use crate::engine::{self, RULE_VERSION};
use crate::parser::parse_nodeset;
use crate::store::{CommitPayload, NewEventInput, SourceFileRow, Store};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub const DEFAULT_BRANCH: &str = "main";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRequest {
    pub file_id: String,
    pub path_hint: String,
    pub content: String,
    pub ordinal: i64,
    pub label: String,
    pub clock_tag: String,
    #[serde(default)]
    pub base_seq: Option<i64>,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRequest {
    pub file_id: String,
    pub label: String,
    pub clock_tag: String,
    #[serde(default)]
    pub base_seq: Option<i64>,
    #[serde(default)]
    pub branch: Option<String>,
}

/// Simple stable digest (not cryptographic): enough to detect change.
pub fn digest_of(graph: &DerivedGraph, file_order: &[String]) -> String {
    let mut h: u64 = 1469598103934665603;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        h ^= 0xff;
        h = h.wrapping_mul(1099511628211);
    };
    feed(RULE_VERSION.as_bytes());
    for f in file_order {
        feed(f.as_bytes());
    }
    for n in &graph.nodes {
        feed(n.gid.to_string().as_bytes());
        feed(n.kind.as_bytes());
        feed(n.browse_name.as_bytes());
    }
    for e in &graph.edges {
        feed(format!("{}->{}:{}", e.key.source, e.key.target, e.key.ref_type).as_bytes());
    }
    for u in &graph.unresolved {
        feed(u.endpoint_raw.as_bytes());
        feed(u.reason.as_bytes());
    }
    format!("{:016x}", h)
}

fn parse_rows(rows: &[SourceFileRow]) -> Result<Vec<ParsedFile>, String> {
    let mut files = Vec::new();
    for r in rows {
        if r.ordinal < 0 || r.content.is_empty() {
            continue; // deletion tombstone
        }
        let outcome = parse_nodeset(&r.file_id, &r.path_hint, r.content.as_bytes())?;
        files.push(outcome.file);
    }
    files.sort_by(|a, b| {
        let oa = ordinal_of(&a.file_id, rows);
        let ob = ordinal_of(&b.file_id, rows);
        oa.cmp(&ob).then(a.file_id.cmp(&b.file_id))
    });
    Ok(files)
}

fn ordinal_of(file_id: &str, rows: &[SourceFileRow]) -> i64 {
    rows.iter()
        .find(|r| r.file_id == file_id)
        .map(|r| r.ordinal)
        .unwrap_or(0)
}

fn node_map(g: &DerivedGraph) -> BTreeMap<GlobalNodeId, &ResolvedNode> {
    g.nodes.iter().map(|n| (n.gid.clone(), n)).collect()
}

fn edge_set(g: &DerivedGraph) -> BTreeSet<EdgeKey> {
    g.edges.iter().map(|e| e.key.clone()).collect()
}

/// Compute impact of a graph change using the reference dependency graph.
pub fn impact(before: &DerivedGraph, after: &DerivedGraph) -> ImpactReport {
    let before_nodes = node_map(before);
    let after_nodes = node_map(after);
    let mut changed_nodes = Vec::new();
    for (gid, n) in &after_nodes {
        match before_nodes.get(gid) {
            Some(old) if node_fingerprint(old) != node_fingerprint(n) => {
                changed_nodes.push(gid.clone())
            }
            None => changed_nodes.push(gid.clone()),
            _ => {}
        }
    }
    for gid in before_nodes.keys() {
        if !after_nodes.contains_key(gid) {
            changed_nodes.push(gid.clone());
        }
    }
    changed_nodes.sort();
    changed_nodes.dedup();

    let be = edge_set(before);
    let ae = edge_set(after);
    let added: BTreeSet<_> = ae.difference(&be).cloned().collect();
    let removed: BTreeSet<_> = be.difference(&ae).cloned().collect();

    // Reverse adjacency over *after* graph plus removed edges to propagate.
    let mut dependents: BTreeMap<GlobalNodeId, BTreeSet<GlobalNodeId>> = BTreeMap::new();
    for e in &after.edges {
        dependents
            .entry(e.key.target.clone())
            .or_default()
            .insert(e.key.source.clone());
    }
    let changed: BTreeSet<GlobalNodeId> = changed_nodes.iter().cloned().collect();
    let mut affected: BTreeSet<GlobalNodeId> = BTreeSet::new();
    for g in &changed {
        propagate(g, &dependents, &mut affected);
    }
    let still_valid_nodes: Vec<GlobalNodeId> = affected
        .difference(&changed)
        .cloned()
        .collect();

    let mut explanation = Vec::new();
    for g in &changed_nodes {
        explanation.push(format!("recompute:{g} changed inputs or definition"));
    }
    for g in &still_valid_nodes {
        explanation.push(format!("still-valid:{g} depends on a changed node but its fingerprint is unchanged"));
    }

    ImpactReport {
        changed_nodes,
        still_valid_nodes,
        removed_edges: removed.into_iter().collect(),
        added_edges: added.into_iter().collect(),
        explanation,
    }
}

fn propagate(
    start: &GlobalNodeId,
    dependents: &BTreeMap<GlobalNodeId, BTreeSet<GlobalNodeId>>,
    out: &mut BTreeSet<GlobalNodeId>,
) {
    let mut stack = vec![start.clone()];
    let mut seen = HashSet::new();
    while let Some(g) = stack.pop() {
        if !seen.insert(g.clone()) {
            continue;
        }
        if let Some(ds) = dependents.get(&g) {
            for d in ds {
                if out.insert(d.clone()) {
                    stack.push(d.clone());
                }
            }
        }
    }
}

fn node_fingerprint(n: &ResolvedNode) -> String {
    format!("{}|{}|{}|{}", n.kind, n.browse_uri, n.browse_name, n.file_id)
}

pub struct App {
    pub store: Store,
}

impl App {
    pub fn new(store: Store) -> Self {
        App { store }
    }

    pub fn current_branch(&self, branch: &str) -> String {
        if branch.is_empty() {
            DEFAULT_BRANCH.to_string()
        } else {
            branch.to_string()
        }
    }

    fn current_snapshot(&self, branch: &str) -> Option<VersionSnapshot> {
        self.store
            .branch_pointer(branch)
            .ok()
            .flatten()
            .and_then(|bp| self.store.snapshot(&bp.version_id).ok().flatten())
    }

    fn current_files(&self, branch: &str) -> Vec<SourceFileRow> {
        self.store.current_files(branch).unwrap_or_default()
    }

    pub fn import(&self, req: ImportRequest) -> Result<ImportResult, String> {
        let branch = self.current_branch(req.branch.as_deref().unwrap_or(""));
        // Validate parse up front; a bad file never moves the pointer.
        let parsed = parse_nodeset(&req.file_id, &req.path_hint, req.content.as_bytes())?;

        let mut rows = self.current_files(&branch);
        rows.retain(|r| r.file_id != req.file_id);
        rows.push(SourceFileRow {
            file_id: req.file_id.clone(),
            path_hint: req.path_hint.clone(),
            content: req.content.clone(),
            ordinal: req.ordinal,
        });
        let before = self.current_snapshot(&branch);
        let before_graph = before.as_ref().map(|s| s.derived.clone()).unwrap_or_default();
        let files = parse_rows(&rows)?;
        let file_order: Vec<String> = files.iter().map(|f| f.file_id.clone()).collect();
        let after_graph = engine::derive(&files);
        let impact_report = impact(&before_graph, &after_graph);
        let digest = digest_of(&after_graph, &file_order);

        let bp_seq = self
            .store
            .branch_pointer(&branch)
            .ok()
            .flatten()
            .map(|b| b.event_seq);
        let version_id = format!(
            "ver-{}-{}",
            bp_seq.unwrap_or(0) + 1,
            slug(&req.file_id)
        );

        let snap = VersionSnapshot {
            version_id: version_id.clone(),
            branch: branch.clone(),
            event_seq: 0,
            rule_version: RULE_VERSION.to_string(),
            derived: after_graph,
            file_order,
            impact: impact_report.clone(),
            digest: digest.clone(),
        };
        let _ = parsed;

        let mut changed = BTreeMap::new();
        changed.insert(
            req.file_id.clone(),
            (req.path_hint.clone(), req.content.clone(), req.ordinal),
        );
        let outcome = self
            .store
            .commit(CommitPayload {
                event: NewEventInput {
                    branch: branch.clone(),
                    kind: "import".to_string(),
                    label: req.label.clone(),
                    detail_json: serde_json::json!({
                        "file_id": req.file_id,
                        "path_hint": req.path_hint,
                        "ordinal": req.ordinal,
                        "impact_changed": impact_report.changed_nodes.len(),
                        "digest": digest,
                    })
                    .to_string(),
                    source_file_id: Some(req.file_id.clone()),
                    parent_event_id: None,
                    accepted: true,
                    clock_tag: req.clock_tag.clone(),
                    base_seq: req.base_seq,
                },
                changed_files: changed,
                deleted_files: Vec::new(),
                snapshot: Some(snap),
            })
            .map_err(|e| e.to_string())?;

        Ok(ImportResult {
            committed: outcome.conflict.is_none(),
            conflict: outcome.conflict,
            event_seq: outcome.event.seq,
            event_id: outcome.event.id,
            version_id: outcome.version_id,
            impact: impact_report,
        })
    }

    pub fn delete_source(&self, req: DeleteRequest) -> Result<ImportResult, String> {
        let branch = self.current_branch(req.branch.as_deref().unwrap_or(""));
        let mut rows = self.current_files(&branch);
        if !rows.iter().any(|r| r.file_id == req.file_id && r.ordinal >= 0) {
            return Err(format!("no source file {}", req.file_id));
        }
        rows.retain(|r| r.file_id != req.file_id);
        let before = self.current_snapshot(&branch);
        let before_graph = before.as_ref().map(|s| s.derived.clone()).unwrap_or_default();
        let files = parse_rows(&rows)?;
        let file_order: Vec<String> = files.iter().map(|f| f.file_id.clone()).collect();
        let after_graph = engine::derive(&files);
        let impact_report = impact(&before_graph, &after_graph);
        let digest = digest_of(&after_graph, &file_order);

        let bp = self
            .store
            .branch_pointer(&branch)
            .ok()
            .flatten();
        let bp_seq = bp.as_ref().map(|b| b.event_seq).unwrap_or(0);
        let version_id = format!("ver-{}-del-{}", bp_seq + 1, slug(&req.file_id));
        let snap = VersionSnapshot {
            version_id: version_id.clone(),
            branch: branch.clone(),
            event_seq: 0,
            rule_version: RULE_VERSION.to_string(),
            derived: after_graph,
            file_order,
            impact: impact_report.clone(),
            digest: digest.clone(),
        };
        let outcome = self
            .store
            .commit(CommitPayload {
                event: NewEventInput {
                    branch: branch.clone(),
                    kind: "delete_source".to_string(),
                    label: req.label.clone(),
                    detail_json: serde_json::json!({
                        "file_id": req.file_id,
                        "impact_changed": impact_report.changed_nodes.len(),
                        "digest": digest,
                    })
                    .to_string(),
                    source_file_id: Some(req.file_id.clone()),
                    parent_event_id: None,
                    accepted: true,
                    clock_tag: req.clock_tag.clone(),
                    base_seq: req.base_seq,
                },
                changed_files: BTreeMap::new(),
                deleted_files: vec![req.file_id.clone()],
                snapshot: Some(snap),
            })
            .map_err(|e| e.to_string())?;
        Ok(ImportResult {
            committed: outcome.conflict.is_none(),
            conflict: outcome.conflict,
            event_seq: outcome.event.seq,
            event_id: outcome.event.id,
            version_id: outcome.version_id,
            impact: impact_report,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub committed: bool,
    pub conflict: Option<String>,
    pub event_seq: i64,
    pub event_id: String,
    pub version_id: String,
    pub impact: ImpactReport,
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(24)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchRequest {
    pub name: String,
    pub from_branch: String,
    pub label: String,
    pub clock_tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub branch: String,
    pub subject: String,
    pub options: Vec<String>,
    pub chosen: String,
    pub label: String,
    pub clock_tag: String,
    #[serde(default)]
    pub base_seq: Option<i64>,
}

impl App {
    /// Create a branch at an existing branch's current pointer by recording a
    /// `branch` event and copying the pointer to the new branch.
    pub fn create_branch(&self, req: BranchRequest) -> Result<String, String> {
        let base = self.current_branch(&req.from_branch);
        let bp = self
            .store
            .branch_pointer(&base)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("base branch {base} has no version"))?;
        if self
            .store
            .branch_pointer(&req.name)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err(format!("branch {} already exists", req.name));
        }
        // Copy the *entire* source-file history of the base branch up to its
        // pointer so the new branch can recompute and time-travel on its own.
        let rows = self
            .store
            .all_file_rows(&base, bp.event_seq)
            .map_err(|e| e.to_string())?;
        let mut changed = BTreeMap::new();
        for (_branch, _seq, file_id, path_hint, content, ordinal) in rows {
            changed.insert(file_id, (path_hint, content, ordinal));
        }
        let base_snap = self
            .store
            .snapshot(&bp.version_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "base snapshot missing".to_string())?
            .with_branch(req.name.clone());
        let outcome = self
            .store
            .commit(CommitPayload {
                event: NewEventInput {
                    branch: req.name.clone(),
                    kind: "branch".to_string(),
                    label: req.label.clone(),
                    detail_json: serde_json::json!({"from_branch": base, "from_version": bp.version_id})
                        .to_string(),
                    source_file_id: None,
                    parent_event_id: None,
                    accepted: true,
                    clock_tag: req.clock_tag.clone(),
                    base_seq: None,
                },
                changed_files: changed,
                deleted_files: Vec::new(),
                snapshot: Some(base_snap),
            })
            .map_err(|e| e.to_string())?;
        Ok(outcome.version_id)
    }

    /// Record a human decision. All options are retained; the rejected ones are
    /// never deleted, so a confirmation is reversible by revisiting the event.
    pub fn decide(&self, req: DecisionRequest) -> Result<i64, String> {
        let branch = self.current_branch(&req.branch);
        if !req.options.contains(&req.chosen) {
            return Err("chosen value must be one of the legal candidates".to_string());
        }
        let seq_after = self
            .store
            .branch_pointer(&branch)
            .map_err(|e| e.to_string())?
            .map(|b| b.event_seq + 1)
            .unwrap_or(1);
        let decision = Decision {
            id: format!("dec-{seq_after}-{}", slug(&req.subject)),
            branch: branch.clone(),
            subject: req.subject.clone(),
            options: req.options.clone(),
            chosen: Some(req.chosen.clone()),
            rejected: req.options.iter().filter(|o| **o != req.chosen).cloned().collect(),
            created_event_seq: seq_after,
            resolved_event_seq: Some(seq_after),
        };

        let outcome = self
            .store
            .commit(CommitPayload {
                event: NewEventInput {
                    branch: branch.clone(),
                    kind: "decision".to_string(),
                    label: req.label.clone(),
                    detail_json: serde_json::to_string(&decision)
                        .map_err(|e| e.to_string())?,
                    source_file_id: None,
                    parent_event_id: None,
                    accepted: true,
                    clock_tag: req.clock_tag.clone(),
                    base_seq: req.base_seq,
                },
                changed_files: BTreeMap::new(),
                deleted_files: Vec::new(),
                snapshot: None,
            })
            .map_err(|e| e.to_string())?;
        if let Some(conflict) = outcome.conflict {
            return Err(conflict);
        }
        self.store.put_decision(&decision).map_err(|e| e.to_string())?;
        Ok(outcome.event.seq)
    }

    pub fn history_view(&self, branch: &str, seq: i64) -> Result<HistoryView, String> {
        let branch = self.current_branch(branch);
        let snap = self
            .store
            .snapshot_for_event(&branch, seq)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no version at that event".to_string())?;
        let version_id = snap.version_id.clone();
        let rule_version = snap.rule_version.clone();
        let files = self.store.files_at(&branch, seq).map_err(|e| e.to_string())?;
        Ok(HistoryView {
            branch,
            at_seq: seq,
            version_id: version_id.clone(),
            rule_version,
            file_ids: files.iter().map(|f| f.file_id.clone()).collect(),
            snapshot: self
                .store
                .snapshot(&version_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "snapshot vanished".to_string())?,
        })
    }
}

impl VersionSnapshot {
    fn with_branch(mut self, branch: String) -> Self {
        self.branch = branch;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryView {
    pub branch: String,
    pub at_seq: i64,
    pub version_id: String,
    pub rule_version: String,
    pub file_ids: Vec<String>,
    pub snapshot: VersionSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffItem {
    pub kind: String,
    pub identity: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub file_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiffReport {
    pub left_version: String,
    pub right_version: String,
    pub items: Vec<DiffItem>,
}

impl App {
    pub fn diff_versions(&self, left: &str, right: &str) -> Result<DiffReport, String> {
        let l = self
            .store
            .snapshot(left)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("version {left} missing"))?;
        let r = self
            .store
            .snapshot(right)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("version {right} missing"))?;
        Ok(diff_graphs(&l, &r))
    }

    pub fn diff_branches(&self, left: &str, right: &str) -> Result<DiffReport, String> {
        let lv = self
            .store
            .branch_pointer(left)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("branch {left} missing"))?
            .version_id;
        let rv = self
            .store
            .branch_pointer(right)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("branch {right} missing"))?
            .version_id;
        self.diff_versions(&lv, &rv)
    }
}

pub fn diff_graphs(l: &VersionSnapshot, r: &VersionSnapshot) -> DiffReport {
    let mut items = Vec::new();
    let ln: BTreeMap<_, _> = l.derived.nodes.iter().map(|n| (n.gid.clone(), n)).collect();
    let rn: BTreeMap<_, _> = r.derived.nodes.iter().map(|n| (n.gid.clone(), n)).collect();
    for gid in ln.keys().chain(rn.keys()).collect::<BTreeSet<_>>() {
        match (ln.get(gid), rn.get(gid)) {
            (Some(a), Some(b)) => {
                if node_fingerprint(a) != node_fingerprint(b) {
                    items.push(DiffItem {
                        kind: "node-changed".to_string(),
                        identity: gid.to_string(),
                        before: Some(node_fingerprint(a)),
                        after: Some(node_fingerprint(b)),
                        file_id: Some(b.file_id.clone()),
                    });
                }
            }
            (Some(a), None) => items.push(DiffItem {
                kind: "node-removed".to_string(),
                identity: gid.to_string(),
                before: Some(node_fingerprint(a)),
                after: None,
                file_id: Some(a.file_id.clone()),
            }),
            (None, Some(b)) => items.push(DiffItem {
                kind: "node-added".to_string(),
                identity: gid.to_string(),
                before: None,
                after: Some(node_fingerprint(b)),
                file_id: Some(b.file_id.clone()),
            }),
            (None, None) => {}
        }
    }
    let le = edge_set(&l.derived);
    let re = edge_set(&r.derived);
    for k in le.union(&re).collect::<BTreeSet<_>>() {
        match (le.contains(k), re.contains(k)) {
            (true, false) => items.push(DiffItem {
                kind: "edge-removed".to_string(),
                identity: format!("{}->{}:{}", k.source, k.target, k.ref_type),
                before: Some("present".to_string()),
                after: None,
                file_id: None,
            }),
            (false, true) => items.push(DiffItem {
                kind: "edge-added".to_string(),
                identity: format!("{}->{}:{}", k.source, k.target, k.ref_type),
                before: None,
                after: Some("present".to_string()),
                file_id: None,
            }),
            _ => {}
        }
    }
    // Quantities / unresolved diagnostics differences (semantic layer).
    let lq: BTreeMap<_, _> = l
        .derived
        .quantities
        .iter()
        .map(|q| (q.gid.clone(), q))
        .collect();
    let rq: BTreeMap<_, _> = r
        .derived
        .quantities
        .iter()
        .map(|q| (q.gid.clone(), q))
        .collect();
    for gid in lq.keys().chain(rq.keys()).collect::<BTreeSet<_>>() {
        match (lq.get(gid), rq.get(gid)) {
            (Some(a), Some(b)) if a.value_text != b.value_text || a.unit != b.unit => {
                items.push(DiffItem {
                    kind: "quantity-changed".to_string(),
                    identity: gid.to_string(),
                    before: Some(format!("{} {}", a.value_text, a.unit)),
                    after: Some(format!("{} {}", b.value_text, b.unit)),
                    file_id: None,
                });
            }
            _ => {}
        }
    }
    let ld: BTreeSet<String> = l
        .derived
        .diagnostics
        .iter()
        .map(|d| format!("{}:{}:{}", d.code, d.subject, d.message))
        .collect();
    let rd: BTreeSet<String> = r
        .derived
        .diagnostics
        .iter()
        .map(|d| format!("{}:{}:{}", d.code, d.subject, d.message))
        .collect();
    for m in ld.difference(&rd) {
        items.push(DiffItem {
            kind: "diagnostic-cleared".to_string(),
            identity: m.clone(),
            before: Some(m.clone()),
            after: None,
            file_id: None,
        });
    }
    for m in rd.difference(&ld) {
        items.push(DiffItem {
            kind: "diagnostic-raised".to_string(),
            identity: m.clone(),
            before: None,
            after: Some(m.clone()),
            file_id: None,
        });
    }
    DiffReport {
        left_version: l.version_id.clone(),
        right_version: r.version_id.clone(),
        items,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBundle {
    pub format: String,
    pub identity_scheme: String,
    pub schema_version: i64,
    pub exported_clock: String,
    pub events: Vec<ProjectEvent>,
    pub branches: Vec<BranchPoint>,
    pub snapshots: Vec<VersionSnapshot>,
    pub file_rows: Vec<ExportFileRow>,
    pub decisions: Vec<Decision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportFileRow {
    pub branch: String,
    pub event_seq: i64,
    pub file_id: String,
    pub path_hint: String,
    pub content: String,
    pub ordinal: i64,
}

impl App {
    pub fn export_bundle(&self, clock: String) -> Result<ExportBundle, String> {
        let store = &self.store;
        let events = store.all_events().map_err(|e| e.to_string())?;
        let branches = store.list_branches().map_err(|e| e.to_string())?;
        let decisions = {
            let mut all = Vec::new();
            for b in &branches {
                all.extend(store.list_decisions(&b.branch).map_err(|e| e.to_string())?);
            }
            all
        };
        let snapshots = store.all_snapshots().map_err(|e| e.to_string())?;
        // Export full source row history for every branch.
        let mut file_rows = Vec::new();
        for b in &branches {
            let max_seq = b.event_seq;
            // Dump all rows (history) using a direct helper.
            for (branch, event_seq, file_id, path_hint, content, ordinal) in
                self.store.all_file_rows(&b.branch, max_seq).map_err(|e| e.to_string())?
            {
                file_rows.push(ExportFileRow {
                    branch,
                    event_seq,
                    file_id,
                    path_hint,
                    content,
                    ordinal,
                });
            }
        }
        Ok(ExportBundle {
            format: "compass-bundle/1".to_string(),
            identity_scheme: "explicit-stable-ids".to_string(),
            schema_version: crate::store::CURRENT_SCHEMA,
            exported_clock: clock,
            events,
            branches,
            snapshots,
            file_rows,
            decisions,
        })
    }
}

pub fn import_bundle(store: &Store, bundle: &ExportBundle) -> Result<(), String> {
    let mut conn = store.lock_raw();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch(
        "DELETE FROM events; DELETE FROM branches; DELETE FROM snapshots;
         DELETE FROM source_files; DELETE FROM decisions; DELETE FROM meta;",
    )
    .map_err(|e| e.to_string())?;
    for ev in &bundle.events {
        tx.execute(
            "INSERT INTO events(seq,id,branch,kind,rule_version,label,detail_json,source_file_id,
                                parent_event_id,base_version_id,result_version_id,accepted,clock_tag)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            rusqlite::params![
                ev.seq,
                ev.id,
                ev.branch,
                ev.kind,
                ev.rule_version,
                ev.label,
                ev.detail_json,
                ev.source_file_id,
                ev.parent_event_id,
                ev.base_version_id,
                ev.result_version_id,
                ev.accepted as i64,
                ev.clock_tag,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    for b in &bundle.branches {
        tx.execute(
            "INSERT INTO branches(branch,event_seq,version_id) VALUES(?1,?2,?3)",
            rusqlite::params![b.branch, b.event_seq, b.version_id],
        )
        .map_err(|e| e.to_string())?;
    }
    for s in &bundle.snapshots {
        let payload = serde_json::to_string(s).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO snapshots(version_id,branch,event_seq,rule_version,payload,digest)
             VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                s.version_id,
                s.branch,
                s.event_seq,
                s.rule_version,
                payload,
                s.digest
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    for f in &bundle.file_rows {
        tx.execute(
            "INSERT INTO source_files(branch,event_seq,file_id,path_hint,content,ordinal)
             VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![f.branch, f.event_seq, f.file_id, f.path_hint, f.content, f.ordinal],
        )
        .map_err(|e| e.to_string())?;
    }
    for d in &bundle.decisions {
        tx.execute(
            "INSERT INTO decisions(id,branch,subject,options_json,chosen,rejected_json,
                                   created_event_seq,resolved_event_seq)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![
                d.id,
                d.branch,
                d.subject,
                serde_json::to_string(&d.options).unwrap_or_default(),
                d.chosen,
                serde_json::to_string(&d.rejected).unwrap_or_default(),
                d.created_event_seq,
                d.resolved_event_seq
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.pragma_update(None, "user_version", crate::store::CURRENT_SCHEMA)
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}
