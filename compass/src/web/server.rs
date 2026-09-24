use crate::app::Orchestrator;
use crate::diff::diff_models;
use crate::store::Store;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub orch: Arc<Orchestrator>,
}

pub fn router(store: Store) -> Router {
    let state = AppState { orch: Arc::new(Orchestrator::with_store(store)) };
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/api/health", get(health))
        .route("/api/replay", get(replay))
        .route("/api/versions", get(list_versions))
        .route("/api/current", get(current))
        .route("/api/versions/{id}", get(get_version))
        .route("/api/versions/{id}/source", get(source_bytes))
        .route("/api/diff/{a}/{b}", get(diff))
        .route("/api/archive", get(export))
        .route("/api/archive/import", get(import_archive))
        .with_state(state)
}

async fn index() -> Response {
    html(include_str!("index.html"))
}
async fn app_js() -> Response {
    let mut resp = (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        include_str!("app.js"),
    )
        .into_response();
    resp.headers_mut().insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    resp
}

fn html(body: &'static str) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "rule_version": crate::rules::RULESET_VERSION }))
}

async fn replay(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let summary = s.orch.run_replay()?;
    Ok(Json(json!({ "steps": summary.steps, "current": summary.current })))
}

async fn current(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let id = s.orch.store.current_version_id()?;
    match id {
        Some(id) => Ok(Json(json!({ "version_id": id }))),
        None => Ok(Json(json!({ "version_id": null }))),
    }
}

async fn list_versions(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let rows = s.orch.store.list_versions()?;
    let current = s.orch.store.current_version_id()?;
    Ok(Json(json!({
        "current": current,
        "versions": rows.iter().map(|(id, branch, parent, label, seq)| json!({
            "version_id": id, "branch": branch, "parent_id": parent, "label": label, "seq": seq
        })).collect::<Vec<_>>()
    })))
}

async fn get_version(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let version = s.orch.store.load_version(&id)?;
    Ok(Json(serde_json::to_value(&version)?))
}

async fn source_bytes(
    State(s): State<AppState>,
    Path((id, source_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let version = s.orch.store.load_version(&id)?;
    let raw = version.raw.ok_or_else(|| ApiError::not_found("raw model"))?;
    let src = raw
        .sources
        .iter()
        .find(|x| x.source_id == source_id)
        .ok_or_else(|| ApiError::not_found("source"))?;
    Ok(Json(json!({
        "source_id": src.source_id,
        "path": src.path,
        "content": src.bytes,
        "namespaces": src.namespaces,
        "aliases": src.aliases,
        "dependencies": src.dependencies,
        "node_count": src.nodes.len(),
    })))
}

async fn diff(
    State(s): State<AppState>,
    Path((a, b)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let va = s.orch.store.load_version(&a)?;
    let vb = s.orch.store.load_version(&b)?;
    let ma = va.merged.ok_or_else(|| ApiError::not_found("merged a"))?;
    let mb = vb.merged.ok_or_else(|| ApiError::not_found("merged b"))?;
    let d = diff_models(&ma, &mb);
    Ok(Json(serde_json::to_value(d)?))
}

async fn export(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(s.orch.store.export_archive()?))
}

async fn import_archive(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let archive = s.orch.store.export_archive()?;
    s.orch.store.import_archive(&archive)?;
    Ok(Json(json!({ "imported": true })))
}

struct ApiError {
    status: StatusCode,
    message: String,
}
impl ApiError {
    fn not_found(what: &str) -> Self {
        ApiError { status: StatusCode::NOT_FOUND, message: format!("{what} not found") }
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: e.to_string() }
    }
}
impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: e.to_string() }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
