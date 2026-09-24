use crate::diff::compare_snapshots;
use crate::engine::build_snapshot;
use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug)]
pub enum StoreError {
    Conflict(ConflictReport),
    Other(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Conflict(report) => write!(formatter, "{}", report.message),
            StoreError::Other(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for StoreError {}

pub struct Database {
    connection: Mutex<Connection>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionDetail {
    pub record: VersionRecord,
    pub sources: Vec<SourceDocument>,
    pub snapshot: ModelSnapshot,
    pub changes: Vec<SemanticChange>,
    pub parent_label: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DatabaseExport {
    pub schema_version: i64,
    pub current_branch: String,
    pub versions: Vec<VersionExportRow>,
    pub branch_pointers: Vec<(String, i64)>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionExportRow {
    pub record: VersionRecord,
    pub sources: Vec<SourceDocument>,
    pub snapshot: ModelSnapshot,
    pub changes_json: String,
    pub complete: bool,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let connection = Connection::open(path).map_err(other)?;
        connection.busy_timeout(std::time::Duration::from_secs(10)).map_err(other)?;
        connection.pragma_update(None, "foreign_keys", true).map_err(other)?;
        connection.pragma_update(None, "journal_mode", "WAL").map_err(other)?;
        connection.pragma_update(None, "synchronous", "FULL").map_err(other)?;
        let mut database = Self { connection: Mutex::new(connection) };
        database.migrate()?;
        Ok(database)
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory().map_err(other)?;
        connection.pragma_update(None, "foreign_keys", true).map_err(other)?;
        let mut database = Self { connection: Mutex::new(connection) };
        database.migrate()?;
        Ok(database)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        let connection = self.connection.get_mut().map_err(lock)?;
        let existing: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(other)?;
        if existing == 1 {
            migrate_from_v1(connection)?;
            return Ok(());
        }
        if existing != 0 && existing != SCHEMA_VERSION {
            return Err(StoreError::Other(format!("unsupported schema version {existing}")));
        }
        create_schema(connection)?;
        Ok(())
    }

    pub fn import_snapshot(&self, request: &ImportRequest) -> Result<VersionDetail, StoreError> {
        let parameters = request.parameters.clone().unwrap_or_default();
        let base_snapshot = build_snapshot(-1, &request.sources, &parameters).map_err(StoreError::Other)?;
        let connection = self.connection.lock().map_err(lock)?;
        let branch_id = request.branch_id.clone().unwrap_or_else(|| "main".to_string());
        let parent_id = current_version(&connection, &branch_id)?;
        if let Some(expected) = request.expected_version {
            if Some(expected) != parent_id {
            return Err(conflict(&connection, expected, parent_id, &branch_id)?);
            }
        }
        let parent_snapshot = if let Some(parent_id) = parent_id {
            Some(load_snapshot(&connection, parent_id)?)
        } else {
            None
        };
        let (snapshot, changes) = compare_snapshots(parent_snapshot.as_ref(), base_snapshot, &parameters);
        let record = VersionRecord {
            version_id: 0,
            parent_id,
            label: request.label.clone(),
            kind: "import".to_string(),
            status: "complete".to_string(),
            parameters,
            branch_id,
        };
        commit_version(&connection, record, request.sources.clone(), snapshot, changes)
    }

    pub fn decide(&self, request: &DecisionRequest) -> Result<VersionDetail, StoreError> {
        let connection = self.connection.lock().map_err(lock)?;
        let parent = load_version_record(&connection, request.expected_version)?;
        if current_version(&connection, &parent.branch_id)? != Some(request.expected_version) {
            return Err(conflict(&connection, request.expected_version, current_version(&connection, &parent.branch_id)?, &parent.branch_id)?);
        }
        let parameters = request.parameters.clone().unwrap_or(parent.parameters.clone());
        let mut previous = load_snapshot(&connection, parent.version_id)?;
        let sources = load_sources(&connection, parent.version_id)?;
        let mut base = if request.decision == "parameter_adjusted" {
            let inputs: Vec<SourceInput> = sources
                .iter()
                .map(|source| SourceInput { file_name: source.file_name.clone(), content: source.content.clone() })
                .collect();
            build_snapshot(-1, &inputs, &parameters).map_err(StoreError::Other)?
        } else {
            previous.clone()
        };
        let adopted = request.decision != "rejected";
        base.events.push(CompassEvent {
            event_id: format!("decision-{}-{}", request.candidate_fact_id, crate::util::stable_id(&[&request.decision, &request.expected_version.to_string()])),
            version_id: -1,
            code: format!("decision.{}", request.decision),
            severity: if adopted { "info" } else { "warning" }.to_string(),
            adopted,
            subject_id: request.candidate_fact_id.clone(),
            message: format!("reviewer {} candidate {}", request.decision, request.candidate_fact_id),
            basis: "human decisions create a new immutable version and retain unselected candidates".to_string(),
            source_range: None,
            evidence: BTreeMap::from([("decision".to_string(), request.decision.clone())]),
            rule_version: parameters.rule_version.clone(),
        });
        let (snapshot, changes) = compare_snapshots(Some(&previous), base, &parameters);
        let _ = &mut previous;
        let label = request.label.clone().unwrap_or_else(|| format!("decision:{}", request.decision));
        let record = VersionRecord {
            version_id: 0,
            parent_id: Some(parent.version_id),
            label,
            kind: "decision".to_string(),
            status: "complete".to_string(),
            parameters,
            branch_id: parent.branch_id,
        };
        commit_version(&connection, record, source_inputs(&sources), snapshot, changes)
    }

    pub fn state(&self, branch_id: Option<&str>, version_id: Option<i64>) -> Result<VersionDetail, StoreError> {
        let connection = self.connection.lock().map_err(lock)?;
        let branch_id = branch_id.unwrap_or("main");
        let version_id = match version_id {
            Some(version_id) => version_id,
            None => current_version(&connection, branch_id)?.ok_or_else(|| StoreError::Other("no version exists".into()))?,
        };
        load_detail(&connection, version_id)
    }

    pub fn versions(&self) -> Result<Vec<VersionRecord>, StoreError> {
        let connection = self.connection.lock().map_err(lock)?;
        let mut statement = connection
            .prepare("SELECT id, parent_id, label, kind, status, parameters_json, branch_id FROM versions WHERE complete = 1 ORDER BY id")
            .map_err(other)?;
        let rows = statement.query_map([], map_record).map_err(other)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(other)
    }

    pub fn source_text(&self, source_id: &str, start: usize, end: usize) -> Result<(String, SourceDocument), StoreError> {
        let connection = self.connection.lock().map_err(lock)?;
        let source = connection
            .query_row(
                "SELECT source_id, file_name, content, content_sha, ordinal FROM sources WHERE source_id = ?1 ORDER BY version_id DESC LIMIT 1",
                params![source_id],
                |row| Ok(SourceDocument {
                    source_id: row.get(0)?,
                    file_name: row.get(1)?,
                    content: row.get(2)?,
                    content_sha: row.get(3)?,
                    ordinal: row.get(4)?,
                }),
            )
            .optional()
            .map_err(other)?
            .ok_or_else(|| StoreError::Other("source not found".into()))?;
        let excerpt = source.content.get(start..end).unwrap_or_default().to_string();
        Ok((excerpt, source))
    }

    pub fn insert_pending_version(&self) -> Result<i64, StoreError> {
        let connection = self.connection.lock().map_err(lock)?;
        connection
            .execute(
                "INSERT INTO versions (parent_id, label, kind, status, parameters_json, branch_id, snapshot_json, complete) VALUES (NULL, 'pending', 'import', 'pending', ?, 'main', '{}', 0)",
                params![serde_json::to_string(&RuleParameters::default()).unwrap_or_default()],
            )
            .map_err(other)?;
        Ok(connection.last_insert_rowid())
    }

    pub fn export(&self) -> Result<DatabaseExport, StoreError> {
        let connection = self.connection.lock().map_err(lock)?;
        let current_branch: String = connection
            .query_row("SELECT value FROM meta WHERE key = 'current_branch'", [], |row| row.get(0))
            .map_err(other)?;
        let mut versions = Vec::new();
        let mut statement = connection
            .prepare("SELECT id, parent_id, label, kind, status, parameters_json, branch_id, snapshot_json, complete FROM versions ORDER BY id")
            .map_err(other)?;
        let rows = statement
            .query_map([], |row| {
                let parameters: RuleParameters = serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default();
                let record = VersionRecord {
                    version_id: row.get(0)?,
                    parent_id: row.get(1)?,
                    label: row.get(2)?,
                    kind: row.get(3)?,
                    status: row.get(4)?,
                    parameters,
                    branch_id: row.get(6)?,
                };
                let snapshot: ModelSnapshot = serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_else(|_| ModelSnapshot::default());
                Ok((record, row.get::<_, bool>(8)?, snapshot))
            })
            .map_err(other)?;
        for row in rows {
            let (record, complete, snapshot) = row.map_err(other)?;
            let sources = if complete { load_sources(&connection, record.version_id)? } else { Vec::new() };
            let changes_json = connection
                .query_row("SELECT changes_json FROM changes WHERE version_id = ?1", params![record.version_id], |row| row.get(0))
                .optional()
            .map_err(other)?
            .unwrap_or_else(|| "[]".to_string());
            versions.push(VersionExportRow { record, sources, snapshot, changes_json, complete });
        }
        let mut statement = connection.prepare("SELECT branch_id, version_id FROM branch_pointers ORDER BY branch_id").map_err(other)?;
        let branch_pointers = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(other)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(other)?;
        Ok(DatabaseExport { schema_version: SCHEMA_VERSION, current_branch, versions, branch_pointers })
    }

    pub fn import_database(&self, export: DatabaseExport) -> Result<(), StoreError> {
        let connection = self.connection.lock().map_err(lock)?;
        connection.execute_batch("DELETE FROM branch_pointers; DELETE FROM changes; DELETE FROM sources; DELETE FROM versions; DELETE FROM meta;").map_err(other)?;
        create_schema_rows(&connection)?;
        for row in export.versions {
            let snapshot_json = serde_json::to_string(&row.snapshot).map_err(serde)?;
            connection
                .execute(
                    "INSERT INTO versions (id, parent_id, label, kind, status, parameters_json, branch_id, snapshot_json, complete) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        row.record.version_id,
                        row.record.parent_id,
                        row.record.label,
                        row.record.kind,
                        row.record.status,
                        serde_json::to_string(&row.record.parameters).unwrap_or_default(),
                        row.record.branch_id,
                        snapshot_json,
                        row.complete
                    ],
                )
                .map_err(other)?;
            if row.complete {
                connection
                    .execute("INSERT INTO changes (version_id, changes_json) VALUES (?1, ?2)", params![row.record.version_id, row.changes_json])
                    .map_err(other)?;
            }
            for source in row.sources {
                connection
                    .execute(
                        "INSERT INTO sources (version_id, source_id, ordinal, file_name, content, content_sha) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![row.record.version_id, source.source_id, source.ordinal, source.file_name, source.content, source.content_sha],
                    )
                    .map_err(other)?;
            }
        }
        for (branch_id, version_id) in export.branch_pointers {
            connection
                .execute("INSERT INTO branch_pointers (branch_id, version_id) VALUES (?1, ?2)", params![branch_id, version_id])
                .map_err(other)?;
        }
        connection
            .execute("INSERT INTO meta (key, value) VALUES ('current_branch', ?1)", params![export.current_branch])
            .map_err(other)?;
        Ok(())
    }
}

fn other(error: rusqlite::Error) -> StoreError {
    StoreError::Other(error.to_string())
}

fn serde(error: serde_json::Error) -> StoreError {
    StoreError::Other(error.to_string())
}

fn lock<T>(error: T) -> StoreError
where
    T: std::fmt::Display,
{
    StoreError::Other(format!("database lock error: {error}"))
}

fn create_schema(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(include_str!("../sql/schema.sql")).map_err(other)?;
    connection.pragma_update(None, "user_version", SCHEMA_VERSION).map_err(other)
}

fn create_schema_rows(connection: &Connection) -> Result<(), StoreError> {
    connection.execute("INSERT INTO meta (key, value) VALUES ('current_branch', 'main')", []).map_err(other)?;
    Ok(())
}

fn migrate_from_v1(connection: &mut Connection) -> Result<(), StoreError> {
    let tx = connection.transaction().map_err(other)?;
    tx.execute_batch(include_str!("../sql/migrate_v1_to_v2.sql")).map_err(other)?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION).map_err(other)?;
    tx.commit().map_err(other)
}

fn current_version(connection: &Connection, branch_id: &str) -> Result<Option<i64>, StoreError> {
    connection
        .query_row(
            "SELECT version_id FROM branch_pointers WHERE branch_id = ?1",
            params![branch_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(other)
}

fn map_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<VersionRecord> {
    let parameters_json: String = row.get(5)?;
    Ok(VersionRecord {
        version_id: row.get(0)?,
        parent_id: row.get(1)?,
        label: row.get(2)?,
        kind: row.get(3)?,
        status: row.get(4)?,
        parameters: serde_json::from_str(&parameters_json).unwrap_or_default(),
        branch_id: row.get(6)?,
    })
}

fn load_version_record(connection: &Connection, version_id: i64) -> Result<VersionRecord, StoreError> {
    connection
        .query_row(
            "SELECT id, parent_id, label, kind, status, parameters_json, branch_id FROM versions WHERE id = ?1 AND complete = 1",
            params![version_id],
            map_record,
        )
        .map_err(other)
}

fn load_snapshot(connection: &Connection, version_id: i64) -> Result<ModelSnapshot, StoreError> {
    let json: String = connection
        .query_row("SELECT snapshot_json FROM versions WHERE id = ?1 AND complete = 1", params![version_id], |row| row.get(0))
        .map_err(other)?;
    serde_json::from_str(&json).map_err(serde)
}

fn load_sources(connection: &Connection, version_id: i64) -> Result<Vec<SourceDocument>, StoreError> {
    let mut statement = connection
        .prepare("SELECT source_id, file_name, content, content_sha, ordinal FROM sources WHERE version_id = ?1 ORDER BY ordinal")
        .map_err(other)?;
    let rows = statement
        .query_map(params![version_id], |row| {
            Ok(SourceDocument {
                source_id: row.get(0)?,
                file_name: row.get(1)?,
                content: row.get(2)?,
                content_sha: row.get(3)?,
                ordinal: row.get(4)?,
            })
        })
        .map_err(other)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(other)
}

fn source_inputs(sources: &[SourceDocument]) -> Vec<SourceInput> {
    sources
        .iter()
        .map(|source| SourceInput { file_name: source.file_name.clone(), content: source.content.clone() })
        .collect()
}

fn commit_version(
    connection: &Connection,
    mut record: VersionRecord,
    inputs: Vec<SourceInput>,
    mut snapshot: ModelSnapshot,
    changes: Vec<SemanticChange>,
) -> Result<VersionDetail, StoreError> {
    let tx = connection.unchecked_transaction().map_err(other)?;
    let parameters_json = serde_json::to_string(&record.parameters).map_err(serde)?;
    tx.execute(
        "INSERT INTO versions (parent_id, label, kind, status, parameters_json, branch_id, snapshot_json, complete) VALUES (?1, ?2, ?3, 'pending', ?4, ?5, '{}', 0)",
        params![record.parent_id, record.label, record.kind, parameters_json, record.branch_id],
    )
    .map_err(other)?;
    let version_id = tx.last_insert_rowid();
    record.version_id = version_id;
    for event in snapshot.events.iter_mut() {
        event.version_id = version_id;
    }
    let snapshot_json = serde_json::to_string(&snapshot).map_err(serde)?;
    let changes_json = serde_json::to_string(&changes).map_err(serde)?;
    for (ordinal, input) in inputs.iter().enumerate() {
        let source_id = format!("src-{ordinal:02}-{}", crate::util::stable_id(&[&input.file_name, &input.content]));
        tx.execute(
            "INSERT INTO sources (version_id, source_id, ordinal, file_name, content, content_sha) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![version_id, source_id, ordinal as i64, input.file_name, input.content, crate::util::sha_like(&input.content)],
        )
        .map_err(other)?;
    }
    tx.execute("UPDATE versions SET snapshot_json = ?2, status = 'complete', complete = 1 WHERE id = ?1", params![version_id, snapshot_json])
        .map_err(other)?;
    tx.execute("INSERT INTO changes (version_id, changes_json) VALUES (?1, ?2)", params![version_id, changes_json])
        .map_err(other)?;
    tx.execute(
        "INSERT INTO branch_pointers (branch_id, version_id) VALUES (?1, ?2) ON CONFLICT(branch_id) DO UPDATE SET version_id = excluded.version_id",
        params![record.branch_id, version_id],
    )
    .map_err(other)?;
    tx.commit().map_err(other)?;
    let sources = load_sources(connection, version_id)?;
    Ok(VersionDetail {
        record,
        sources,
        snapshot,
        changes,
        parent_label: None,
    })
}

fn load_detail(connection: &Connection, version_id: i64) -> Result<VersionDetail, StoreError> {
    let record = load_version_record(connection, version_id)?;
    let snapshot = load_snapshot(connection, version_id)?;
    let sources = load_sources(connection, version_id)?;
    let changes_json: String = connection
        .query_row("SELECT changes_json FROM changes WHERE version_id = ?1", params![version_id], |row| row.get(0))
        .map_err(other)?;
    let changes = serde_json::from_str(&changes_json).map_err(serde)?;
    let parent_label = record.parent_id.map(|id| load_version_record(connection, id).map(|parent| parent.label).unwrap_or_default());
    Ok(VersionDetail { record, sources, snapshot, changes, parent_label })
}

fn conflict(
    connection: &Connection,
    expected_version: i64,
    parent_id: Option<i64>,
    branch_id: &str,
) -> Result<StoreError, StoreError> {
    let mut candidates = vec![expected_version];
    if let Some(current) = parent_id {
        candidates.push(current);
    }
    if let Some(records) = connection
        .prepare("SELECT id FROM versions WHERE branch_id = ?1 AND complete = 1 ORDER BY id")
        .ok()
        .and_then(|mut statement| {
            statement
                .query_map(params![branch_id], |row| row.get::<_, i64>(0))
                .ok()
                .map(|rows| rows.flatten().collect::<Vec<_>>())
        })
    {
        candidates.extend(records);
    }
    candidates.sort_unstable();
    candidates.dedup();
    Ok(StoreError::Conflict(ConflictReport {
        code: "version_conflict".to_string(),
        message: "two users edited the same base version; choose a branch/candidate to continue".to_string(),
        expected_version,
        current_version: parent_id.unwrap_or(0),
        resolvable_candidates: candidates,
    }))
}
