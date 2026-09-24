use crate::db::{Database, DatabaseExport, VersionDetail};
use crate::model::*;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub database: Arc<Database>,
}

#[derive(serde::Deserialize)]
pub struct StateQuery {
    branch: Option<String>,
    version: Option<i64>,
}

#[derive(serde::Deserialize)]
pub struct SourceQuery {
    source_id: String,
    start: usize,
    end: usize,
}

#[derive(serde::Serialize)]
pub struct SourceResponse {
    pub source_id: String,
    pub file_name: String,
    pub start: usize,
    pub end: usize,
    pub excerpt: String,
}

#[derive(serde::Serialize)]
pub struct ApiError {
    pub error: String,
    pub conflict: Option<ConflictReport>,
}

pub async fn serve(addr: SocketAddr, database_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let database = Arc::new(Database::open(Path::new(database_path))?);
    crate::scenario::seed_demo(&database)?;
    let app = Router::new()
        .route("/", get(index_page))
        .route("/api/health", get(health))
        .route("/api/versions", get(versions))
        .route("/api/state", get(state))
        .route("/api/source-range", get(source_range))
        .route("/api/import", post(import_snapshot))
        .route("/api/decide", post(decide))
        .route("/api/export", get(export_database))
        .route("/api/import-database", post(import_database))
        .with_state(AppState { database });
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index_page() -> Response {
    let html = include_str!("../static/index.html");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok", "product": "namespace-compass"}))
}

async fn versions(State(state): State<AppState>) -> Result<Json<Vec<VersionRecord>>, ApiErrorResponse> {
    state.database.versions().map(Json).map_err(ApiErrorResponse::from)
}

async fn state(
    State(state): State<AppState>,
    Query(query): Query<StateQuery>,
) -> Result<Json<VersionDetail>, ApiErrorResponse> {
    state
        .database
        .state(query.branch.as_deref(), query.version)
        .map(Json)
        .map_err(ApiErrorResponse::from)
}

async fn source_range(
    State(state): State<AppState>,
    Query(query): Query<SourceQuery>,
) -> Result<Json<SourceResponse>, ApiErrorResponse> {
    let (excerpt, source) = state
        .database
        .source_text(&query.source_id, query.start, query.end)
        .map_err(ApiErrorResponse::from)?;
    Ok(Json(SourceResponse {
        source_id: source.source_id,
        file_name: source.file_name,
        start: query.start,
        end: query.end,
        excerpt,
    }))
}

async fn import_snapshot(
    State(state): State<AppState>,
    Json(request): Json<ImportRequest>,
) -> Result<(StatusCode, Json<VersionDetail>), ApiErrorResponse> {
    state.database.import_snapshot(&request).map(|detail| (StatusCode::CREATED, Json(detail))).map_err(ApiErrorResponse::from)
}

async fn decide(
    State(state): State<AppState>,
    Json(request): Json<DecisionRequest>,
) -> Result<(StatusCode, Json<VersionDetail>), ApiErrorResponse> {
    state.database.decide(&request).map(|detail| (StatusCode::CREATED, Json(detail))).map_err(ApiErrorResponse::from)
}

async fn export_database(State(state): State<AppState>) -> Result<Json<DatabaseExport>, ApiErrorResponse> {
    state.database.export().map(Json).map_err(ApiErrorResponse::from)
}

async fn import_database(
    State(state): State<AppState>,
    Json(export): Json<DatabaseExport>,
) -> Result<StatusCode, ApiErrorResponse> {
    state.database.import_database(export).map(|_| StatusCode::NO_CONTENT).map_err(ApiErrorResponse::from)
}

struct ApiErrorResponse {
    status: StatusCode,
    body: ApiError,
}

impl From<crate::db::StoreError> for ApiErrorResponse {
    fn from(error: crate::db::StoreError) -> Self {
        match error {
            crate::db::StoreError::Conflict(conflict) => Self {
                status: StatusCode::CONFLICT,
                body: ApiError { error: conflict.message.clone(), conflict: Some(conflict) },
            },
            other => Self {
                status: StatusCode::BAD_REQUEST,
                body: ApiError { error: other.to_string(), conflict: None },
            },
        }
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
