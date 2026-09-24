//! Axum web layer: data, derivation and adjudication stay linked.

use crate::derive;
use crate::diff;
use crate::events::{self, Event, RationaleEvent};
use crate::model::*;
use crate::scenario;
use crate::store::Store;
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
}

pub fn router(store: Arc<Store>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/static/app.js", get(app_js))
        .route("/static/style.css", get(style_css))
        .route("/api/overview", get(overview))
        .route("/api/branches", get(list_branches))
        .route("/api/model/{branch}", get(model_view))
        .route("/api/events/{branch}", get(event_list))
        .route("/api/frame/{branch}/{seq}", get(frame_at))
        .route("/api/source/{file_id}", get(source_view))
        .route("/api/diff/{branch}", get(diff_view))
        .route("/api/quantities/{branch}", get(quantities_view))
        .route("/api/impact/{branch}", get(impact_view))
        .route("/api/decisions/{branch}", get(decisions_view).post(decide))
        .route("/api/rationale", get(rationale))
        .route("/api/tour/play", post(tour_play))
        .route("/api/tour/status", get(tour_status))
        .route("/api/export", get(export))
        .route("/api/import", post(import_bundle))
        .route("/api/upload/{branch}", post(upload))
        .with_state(AppState { store })
}

async fn index() -> Response {
    html(include_str!("../static/index.html"))
}
async fn app_js() -> Response {
    text(include_str!("../static/app.js"), "application/javascript")
}
async fn style_css() -> Response {
    text(include_str!("../static/style.css"), "text/css")
}

fn html(body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}
fn text(body: &'static str, kind: &str) -> Response {
    ([(header::CONTENT_TYPE, kind)], body).into_response()
}

#[derive(Serialize)]
struct Overview {
    schema_version: u32,
    rule_version: &'static str,
    branches: Vec<(String, u64, String, bool)>,
    head: u64,
    tour_steps: Vec<String>,
    rationale: Vec<RationaleEvent>,
}

async fn overview(State(st): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(Overview {
        schema_version: st.store.schema_version().unwrap_or(0),
        rule_version: crate::merge::RULE_VERSION,
        branches: st.store.branches(),
        head: st.store.head_seq("main"),
        tour_steps: scenario::expected_tour().iter().map(|s| s.title.clone()).collect(),
        rationale: events::rationale_catalog(),
    }).unwrap())
}

async fn list_branches(State(st): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "branches": st.store.branches() }))
}

#[derive(Serialize)]
struct ModelView {
    model: MergedModel,
    diagnostics: Vec<Diagnostic>,
    raw_files: Vec<RawFileMeta>,
    quantities: Vec<derive::DerivedItem>,
}

#[derive(Serialize)]
struct RawFileMeta {
    file_id: String,
    path: String,
    namespace_count: usize,
    node_count: usize,
}

async fn model_view(State(st): State<AppState>, Path(branch): Path<String>) -> Response {
    let model = match st.store.current_model(&branch) {
        Ok(m) => m,
        Err(e) => return (axum::http::StatusCode::NOT_FOUND, e).into_response(),
    };
    let raw = st.store.raw_files(&branch);
    let view = ModelView {
        raw_files: raw.iter().map(|f| RawFileMeta {
            file_id: f.file_id.clone(), path: f.path.clone(),
            namespace_count: f.namespaces.len(), node_count: f.nodes.len(),
        }).collect(),
        diagnostics: model.diagnostics.clone(),
        quantities: derive::derive_quantities(&raw),
        model,
    };
    Json(serde_json::to_value(view).unwrap()).into_response()
}

async fn event_list(State(st): State<AppState>, Path(branch): Path<String>) -> Json<serde_json::Value> {
    let events = st.store.all_events().into_iter().filter(|e| branch == "main" || e.branch == "main" && e.seq <= st.store.branch_base(&branch) || e.branch == branch).collect::<Vec<_>>();
    Json(serde_json::json!({ "events": events }))
}

async fn frame_at(State(st): State<AppState>, Path((branch, seq)): Path<(String, u64)>) -> Response {
    match st.store.model_at(&branch, Some(seq)) {
        Ok(model) => Json(serde_json::to_value(model).unwrap()).into_response(),
        Err(e) => (axum::http::StatusCode::NOT_FOUND, e).into_response(),
    }
}

#[derive(Serialize)]
struct SourceView {
    file_id: String,
    path: String,
    content: String,
    namespaces: Vec<NamespaceEntry>,
    aliases: Vec<AliasEntry>,
    models: Vec<ModelDependency>,
    nodes: Vec<RawNode>,
    top_extensions: Vec<RawElement>,
}

async fn source_view(State(st): State<AppState>, Path(file_id): Path<String>) -> Response {
    let raw = st.store.raw_files("main");
    let file = match raw.into_iter().find(|f| f.file_id == file_id) {
        Some(f) => f,
        None => return (axum::http::StatusCode::NOT_FOUND, "source file not found").into_response(),
    };
    Json(serde_json::to_value(SourceView {
        file_id: file.file_id, path: file.path, content: file.content,
        namespaces: file.namespaces, aliases: file.aliases, models: file.models,
        nodes: file.nodes, top_extensions: file.top_extensions,
    }).unwrap()).into_response()
}

#[derive(Deserialize)]
struct DiffQuery { from: Option<u64>, to: Option<u64> }

async fn diff_view(State(st): State<AppState>, Path(branch): Path<String>, Query(q): Query<DiffQuery>) -> Response {
    let to = match st.store.model_at(&branch, q.to) { Ok(m) => m, Err(e) => return (axum::http::StatusCode::NOT_FOUND, e).into_response() };
    let from = q.from.and_then(|s| st.store.model_at(&branch, Some(s)).ok());
    let d = diff::diff_models(from.as_ref(), &to);
    Json(serde_json::to_value(d).unwrap()).into_response()
}

async fn quantities_view(State(st): State<AppState>, Path(branch): Path<String>) -> Json<serde_json::Value> {
    let raw = st.store.raw_files(&branch);
    Json(serde_json::json!({ "items": derive::derive_quantities(&raw) }))
}

#[derive(Deserialize)]
struct ImpactQuery { changed: String }

async fn impact_view(State(st): State<AppState>, Path(branch): Path<String>, Query(q): Query<ImpactQuery>) -> Json<serde_json::Value> {
    let model = st.store.current_model(&branch).unwrap();
    let layer = derive::derive(&model);
    let changed = q.changed.split(',').map(String::from).collect();
    let report = derive::propagate(&layer, &changed);
    Json(serde_json::json!({ "report": report, "layer_size": layer.items.len() }))
}

async fn decisions_view(State(st): State<AppState>, Path(branch): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "decisions": st.store.decisions(&branch) }))
}

#[derive(Deserialize)]
struct DecideBody { seq: u64, topic: String, chosen: Option<String>, rejected: Vec<String>, reason: String }

async fn decide(State(st): State<AppState>, Path(branch): Path<String>, Json(body): Json<DecideBody>) -> Json<serde_json::Value> {
    let mut payload = BTreeMap::new();
    payload.insert("topic".into(), body.topic.clone());
    payload.insert("chosen".into(), body.chosen.clone().unwrap_or_default());
    payload.insert("rejected".into(), body.rejected.join("|"));
    let head = st.store.head_seq(&branch);
    let (ev, _) = st.store.append_event(
        &branch, "ManualDecision", &format!("裁定：{}", body.topic), &body.reason,
        crate::merge::RULE_VERSION, None, None, payload, Some(head),
    ).unwrap();
    let id = st.store.record_decision(&branch, ev.seq, &body.topic, body.chosen.as_deref(), &body.rejected, &body.reason).unwrap();
    serde_json::json!({ "ok": true, "seq": ev.seq, "decision_id": id }).into()
}

async fn rationale() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "rationale": events::rationale_catalog() }))
}

async fn tour_play(State(st): State<AppState>) -> Json<serde_json::Value> {
    let steps = scenario::play(&st.store);
    serde_json::json!({ "ok": true, "steps": steps }).into()
}

async fn tour_status(State(st): State<AppState>) -> Json<serde_json::Value> {
    let events: Vec<Event> = st.store.all_events();
    serde_json::json!({
        "head_main": st.store.head_seq("main"),
        "codes_main": scenario::codes(&st.store, "main"),
        "codes_draft": scenario::codes(&st.store, scenario::BRANCH_B),
        "events": events.iter().map(|e| (e.seq, e.branch.clone(), e.title.clone())).collect::<Vec<_>>(),
    }).into()
}

async fn export(State(st): State<AppState>) -> Response {
    match st.store.export_json() {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json"), (header::CONTENT_DISPOSITION, "attachment; filename=\"opc-compass-export.json\"")], json).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn import_bundle(State(st): State<AppState>, body: String) -> Response {
    match st.store.import_json(&body) {
        Ok(branches) => Json(serde_json::json!({ "ok": true, "branches": branches })).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct UploadBody { path: String, content: String, kind: Option<String>, expected_head: Option<u64> }

async fn upload(State(st): State<AppState>, Path(branch): Path<String>, Json(body): Json<UploadBody>) -> Response {
    let kind = body.kind.as_deref().unwrap_or("ImportFile");
    let head = body.expected_head.or_else(|| Some(st.store.head_seq(&branch)));
    match st.store.append_event(
        &branch, kind, &format!("导入 {}", body.path), &body.path,
        crate::merge::RULE_VERSION, Some(&body.path), Some(&body.content), BTreeMap::new(), head,
    ) {
        Ok((ev, model)) => Json(serde_json::json!({ "ok": true, "seq": ev.seq, "nodes": model.nodes.len(), "diagnostics": model.diagnostics.len() })).into_response(),
        Err(e) => (axum::http::StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

pub async fn serve(store: Arc<Store>, addr: &str) {
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, router(store)).await.unwrap();
}
