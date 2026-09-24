use crate::case;
use crate::diff;
use crate::merge;
use crate::model::*;
use crate::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/state", get(api_state))
        .route("/api/branches", get(api_branches))
        .route("/api/case", get(api_case))
        .route("/api/case/step/:id", post(api_case_step))
        .route("/api/branch/:branch/history", get(api_history))
        .route("/api/branch/:branch/version/:id", get(api_version))
        .route("/api/branch/:branch/recompute", post(api_recompute))
        .route("/api/branch/:branch/decision", post(api_decision))
        .route("/api/diff", get(api_diff))
        .route("/api/snippet", get(api_snippet))
        .route("/api/export", get(api_export))
        .route("/api/import", post(api_import))
        .route("/assets/app.js", get(app_js))
        .route("/assets/app.css", get(app_css))
        .with_state(state)
}

async fn index() -> Response {
    html(include_str!("assets/index.html"))
}
async fn app_js() -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/javascript; charset=utf-8")
        .body(Body::from(include_str!("assets/app.js")))
        .unwrap()
}
async fn app_css() -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
        .body(Body::from(include_str!("assets/app.css")))
        .unwrap()
}

fn html(s: &str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(s.to_string()))
        .unwrap()
}

fn json_ok<T: Serialize>(v: T) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&v).unwrap_or_default(),
    )
        .into_response()
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (
        code,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&json!({ "error": msg.into() })).unwrap_or_default(),
    )
        .into_response()
}

#[derive(Serialize)]
struct BranchView {
    name: String,
    current_version: Option<String>,
    edit_token: String,
}

async fn api_branches(State(st): State<AppState>) -> Response {
    let mut branches = Vec::new();
    for b in ["main", "boundary"] {
        branches.push(BranchView {
            name: b.into(),
            current_version: st.store.current_version_id(b),
            edit_token: st.store.edit_token(b),
        });
    }
    json_ok(branches)
}

#[derive(Deserialize)]
struct BranchQuery {
    branch: Option<String>,
}

async fn api_state(State(st): State<AppState>, q: Query<BranchQuery>) -> Response {
    let branch = q.branch.clone().unwrap_or_else(|| "main".into());
    let cur = match st.store.current(&branch) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let history = st.store.history(&branch).unwrap_or_default();
    json_ok(json!({
        "branch": branch,
        "current": cur.map(|v| json!({
            "id": v.id,
            "seq": v.seq,
            "label": v.label,
            "rule_set_version": v.rule_set_version,
            "base_version": v.base_version,
            "created_at": v.created_at,
            "model": v.derived,
        })),
        "history": history.iter().map(|(id, seq, label, at)| json!({
            "id": id, "seq": seq, "label": label, "created_at": at
        })).collect::<Vec<_>>(),
        "pending_decisions": st.store.pending_decisions(&branch).unwrap_or_default(),
    }))
}

async fn api_case(State(st): State<AppState>) -> Response {
    match st.load_case() {
        Some(c) => json_ok(json!({
            "root": c.root,
            "manifest": c.manifest,
        })),
        None => err(StatusCode::NOT_FOUND, "offline case pack not found"),
    }
}

async fn api_case_step(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let case = match st.load_case() {
        Some(c) => c,
        None => return err(StatusCode::NOT_FOUND, "case pack missing"),
    };
    match case::replay_step(&st.store, &case, &id) {
        Ok(r) => json_ok(json!({
            "version_id": r.version_id,
            "event_kinds": r.event_kinds,
            "conflict": r.conflict,
        })),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

async fn api_history(State(st): State<AppState>, Path(branch): Path<String>) -> Response {
    match st.store.history(&branch) {
        Ok(h) => json_ok(h),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn api_version(
    State(st): State<AppState>,
    Path((branch, id)): Path<(String, String)>,
) -> Response {
    match st.store.version(&id) {
        Ok(v) if v.branch == branch => json_ok(json!({
            "id": v.id, "branch": v.branch, "seq": v.seq, "label": v.label,
            "rule_set_version": v.rule_set_version, "created_at": v.created_at,
            "model": v.derived,
        })),
        Ok(_) => err(StatusCode::NOT_FOUND, "version not on this branch"),
        Err(crate::store::StoreError::NotFound(m)) => err(StatusCode::NOT_FOUND, m),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct RecomputeReq {
    branch: Option<String>,
    files: Option<Vec<FileIn>>,
}
#[derive(Deserialize)]
struct FileIn {
    path: String,
    content: String,
}

#[derive(Serialize)]
struct RecomputeOut {
    version_id: String,
    changed_files: Vec<String>,
    reused_nodes: Vec<String>,
    rebuilt_nodes: Vec<String>,
    new_nodes: Vec<String>,
    removed_nodes: Vec<String>,
    reasons: Vec<String>,
}

async fn api_recompute(
    State(st): State<AppState>,
    Path(branch): Path<String>,
    Json(req): Json<RecomputeReq>,
) -> Response {
    let inputs = req.files.unwrap_or_default();
    let mut files = Vec::new();
    for f in &inputs {
        match crate::raw::parse_file(&f.path, &f.content) {
            Ok(s) => files.push(s),
            Err(e) => return err(StatusCode::BAD_REQUEST, e),
        }
    }
    let decisions = match st.store.decisions(&branch) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let prev = st.store.current(&branch).ok().flatten();
    let changed: Vec<String> = files
        .iter()
        .filter(|f| {
            prev.as_ref()
                .map(|p| {
                    !p.derived
                        .files
                        .iter()
                        .any(|fd| fd.file_id == f.id && fd.content_hash == crate::identity::short_hash(&[&f.content]))
                })
                .unwrap_or(true)
        })
        .map(|f| f.id.clone())
        .collect();

    let (derived, report) = match &prev {
        Some(p) => merge::recompute(&p.derived, files.clone(), &branch, &decisions, &changed),
        None => (merge::build(files.clone(), &branch, &decisions), merge::RecomputeReport::default()),
    };
    let token = st.store.edit_token(&branch);
    let expected = if token == "none" { None } else { Some(token.as_str()) };
    let base = prev.as_ref().map(|v| v.id.as_str());
    match st.store.save_version(
        &branch,
        "manual-recompute",
        base,
        &files,
        &derived,
        "{}",
        expected,
        false,
    ) {
        Ok(version_id) => {
            st.store.mark_decisions_applied(&branch, &version_id).ok();
            json_ok(RecomputeOut {
                version_id,
                changed_files: changed,
                reused_nodes: report.reused_nodes,
                rebuilt_nodes: report.rebuilt_nodes,
                new_nodes: report.new_nodes,
                removed_nodes: report.removed_nodes,
                reasons: report.reasons,
            })
        }
        Err(crate::store::StoreError::Conflict(m)) => err(StatusCode::CONFLICT, m),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct DecisionReq {
    subject_kind: String,
    subject: String,
    choice: String,
    candidate_ids: Vec<String>,
}

async fn api_decision(
    State(st): State<AppState>,
    Path(branch): Path<String>,
    Json(req): Json<DecisionReq>,
) -> Response {
    let current = match st.store.current_version_id(&branch) {
        Some(id) => id,
        None => return err(StatusCode::CONFLICT, "branch has no version to decide on"),
    };
    match st.store.record_decision(
        &branch,
        &req.subject,
        &req.choice,
        &req.candidate_ids,
        &current,
    ) {
        Ok(()) => {
            // rebuild a new version applying the decision
            let files = match st.store.files() {
                Ok(f) => f,
                Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            };
            let decisions = st.store.decisions(&branch).unwrap_or_default();
            let derived = merge::build(files.clone(), &branch, &decisions);
            let token = st.store.edit_token(&branch);
            match st.store.save_version(
                &branch,
                "decision",
                Some(&current),
                &files,
                &derived,
                "{}",
                Some(&token),
                false,
            ) {
                Ok(vid) => {
                    st.store.mark_decisions_applied(&branch, &vid).ok();
                    json_ok(json!({ "version_id": vid, "status": "applied" }))
                }
                Err(crate::store::StoreError::Conflict(m)) => err(StatusCode::CONFLICT, m),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            }
        }
        Err(crate::store::StoreError::Conflict(m)) => err(StatusCode::CONFLICT, m),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct DiffQuery {
    left_branch: Option<String>,
    right_branch: Option<String>,
    left_version: Option<String>,
    right_version: Option<String>,
}

async fn api_diff(State(st): State<AppState>, q: Query<DiffQuery>) -> Response {
    let load = |branch: &str, version: &Option<String>| -> Result<DerivedModel, String> {
        match version {
            Some(v) => st.store.version(v).map(|vm| vm.derived).map_err(|e| e.to_string()),
            None => st
                .store
                .current(branch)
                .map_err(|e| e.to_string())
                .and_then(|o| o.map(|v| v.derived).ok_or_else(|| format!("no version on {}", branch))),
        }
    };
    let lb = q.left_branch.clone().unwrap_or_else(|| "main".into());
    let rb = q.right_branch.clone().unwrap_or_else(|| "boundary".into());
    let left = match load(&lb, &q.left_version) {
        Ok(m) => m,
        Err(e) => return err(StatusCode::NOT_FOUND, e),
    };
    let right = match load(&rb, &q.right_version) {
        Ok(m) => m,
        Err(e) => return err(StatusCode::NOT_FOUND, e),
    };
    json_ok(diff::diff(&left, &right))
}

#[derive(Deserialize)]
struct SnippetQuery {
    file_id: String,
    start: usize,
    end: usize,
}

async fn api_snippet(State(st): State<AppState>, q: Query<SnippetQuery>) -> Response {
    match st.store.snippet(&q.file_id, q.start, q.end) {
        Some((path, text)) => json_ok(json!({ "path": path, "text": text, "start": q.start, "end": q.end })),
        None => err(StatusCode::NOT_FOUND, "snippet out of range"),
    }
}

async fn api_export(State(st): State<AppState>) -> Response {
    match st.store.export_json() {
        Ok(j) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/json"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=nscompass-export.json",
                ),
            ],
            j,
        )
            .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn api_import(State(st): State<AppState>, body: String) -> Response {
    match st.store.import_json(&body) {
        Ok(n) => json_ok(json!({ "imported_versions": n })),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

/// Convenience used by the UI: node lookup maps for graph layout.
pub fn node_maps(model: &DerivedModel) -> BTreeMap<String, serde_json::Value> {
    let _ = model;
    BTreeMap::new()
}

pub async fn serve(state: AppState, addr: &str) {
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    tracing_free(&format!("nscompass listening on http://{}", addr));
    axum::serve(listener, router(state)).await.expect("server");
}

fn tracing_free(msg: &str) {
    println!("{}", msg);
}
