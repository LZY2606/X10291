//! Axum web layer: review UI + JSON API.

use crate::diff::diff_models;
use crate::store::{Edit, Store};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub store: Mutex<Store>,
}

pub type Shared = Arc<AppState>;

fn err(e: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, format!("{e:#}"))
}

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/state", get(api_state))
        .route("/api/versions/{id}", get(api_version))
        .route("/api/diff", get(api_diff))
        .route("/api/source/{version}/{file}", get(api_source))
        .route("/api/edit", post(api_edit))
        .route("/api/resolve", post(api_resolve))
        .route("/api/replay", post(api_replay))
        .route("/api/demo", post(api_demo))
        .route("/api/export", get(api_export))
        .route("/api/import", post(api_import))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("ui.html"))
}

async fn api_state(State(s): State<Shared>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    let versions = store.list_versions().map_err(err)?;
    let pointer = store.current_pointer().map_err(err)?;
    let mut decisions = Vec::new();
    for v in &versions {
        for (kind, payload) in store.events_of(Some(v.id)).map_err(err)? {
            decisions.push(serde_json::json!({
                "version_id": v.id, "kind": kind, "payload": payload,
            }));
        }
    }
    Ok(Json(serde_json::json!({
        "pointer": pointer,
        "versions": versions,
        "store_events": decisions,
    })))
}

async fn api_version(
    State(s): State<Shared>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    let p = store.load_payload(id).map_err(err)?;
    let store_events = store.events_of(Some(id)).map_err(err)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "label": p.label,
        "edit": p.edit,
        "model": p.model,
        "derivation": p.derivation,
        "events": p.events,
        "store_events": store_events,
        "files": p.files.iter().map(|f| {
            serde_json::json!({
                "name": f.name,
                "namespace_uris": f.namespace_uris,
                "aliases": f.aliases,
                "models": f.models,
                "node_count": f.nodes.len(),
                "unknown_top_level": f.unknown_top_level,
            })
        }).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct DiffQuery {
    a: i64,
    b: i64,
}

async fn api_diff(
    State(s): State<Shared>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    let a = store.load_payload(q.a).map_err(err)?;
    let b = store.load_payload(q.b).map_err(err)?;
    Ok(Json(serde_json::json!({
        "a": q.a, "b": q.b,
        "entries": diff_models(&a.model, &b.model),
    })))
}

async fn api_source(
    State(s): State<Shared>,
    Path((version, file)): Path<(i64, String)>,
) -> Result<String, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    let p = store.load_payload(version).map_err(err)?;
    p.files
        .iter()
        .find(|f| f.name == file)
        .map(|f| f.raw_xml.clone())
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("file {file} not in version {version}")))
}

#[derive(Deserialize)]
struct EditRequest {
    base: i64,
    label: String,
    edit: EditBody,
}

#[derive(Deserialize, Clone)]
#[serde(tag = "kind")]
enum EditBody {
    ImportFile { name: String, xml: String },
    ReplaceFile { name: String, xml: String },
    RemoveFile { name: String },
    Decision { action: String, subject: String, note: String },
}

async fn api_edit(
    State(s): State<Shared>,
    Json(req): Json<EditRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let edit = match req.edit {
        EditBody::ImportFile { name, xml } => Edit::ImportFile { name, xml },
        EditBody::ReplaceFile { name, xml } => Edit::ReplaceFile { name, xml },
        EditBody::RemoveFile { name } => Edit::RemoveFile { name },
        EditBody::Decision { action, subject, note } => Edit::Decision { action, subject, note },
    };
    let store = s.store.lock().unwrap();
    let outcome = store.apply_edit(req.base, &req.label, edit).map_err(err)?;
    Ok(Json(serde_json::to_value(outcome).unwrap()))
}

#[derive(Deserialize)]
struct ResolveRequest {
    version_id: i64,
    note: Option<String>,
}

async fn api_resolve(
    State(s): State<Shared>,
    Json(req): Json<ResolveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    store
        .resolve_branch(req.version_id, req.note.as_deref().unwrap_or("manual resolve"))
        .map_err(err)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn api_replay(
    State(s): State<Shared>,
    Json(req): Json<ResolveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    store.replay_to(req.version_id).map_err(err)?;
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn api_demo(State(s): State<Shared>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    store
        .conn
        .execute_batch("DELETE FROM decisions; DELETE FROM pointer; DELETE FROM versions;")
        .map_err(err)?;
    let result = crate::scenario::run_case_pack(&store).map_err(err)?;
    Ok(Json(serde_json::json!({
        "log": result.log,
        "baseline": result.baseline,
        "main_tip": result.main_tip,
        "rejected_candidate": result.rejected_candidate,
        "archive_variants": result.archive_variants,
    })))
}

async fn api_export(State(s): State<Shared>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    Ok(Json(store.export().map_err(err)?))
}

async fn api_import(
    State(s): State<Shared>,
    Json(dump): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = s.store.lock().unwrap();
    store.import(&dump).map_err(err)?;
    Ok(Json(serde_json::json!({"ok": true})))
}
