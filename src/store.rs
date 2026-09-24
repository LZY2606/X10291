//! SQLite storage with atomic, versioned derived snapshots.
//!
//! Every derivation creates an independent version row. The branch "current"
//! pointer only moves after a *complete* successful transaction, so a crash
//! mid-write leaves either the old version or the complete new one.

use crate::model::DerivedModel;
use crate::raw::SourceFile;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json;
use std::sync::Mutex;

pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Conflict(String),
    NotFound(String),
    Other(String),
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "sqlite: {}", e),
            StoreError::Conflict(m) => write!(f, "conflict: {}", m),
            StoreError::NotFound(m) => write!(f, "not found: {}", m),
            StoreError::Other(m) => write!(f, "{}", m),
        }
    }
}

pub struct Store {
    pub conn: Mutex<Connection>,
}

pub struct VersionMeta {
    pub id: String,
    pub branch: String,
    pub seq: i64,
    pub label: String,
    pub rule_set_version: String,
    pub base_version: Option<String>,
    pub edit_token: String,
    pub created_at: String,
    pub derived: DerivedModel,
}

fn schema_sql() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

    CREATE TABLE IF NOT EXISTS files (
        id TEXT PRIMARY KEY,
        path TEXT NOT NULL,
        content TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        ordinal INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS branches (
        name TEXT PRIMARY KEY,
        current_version TEXT,
        edit_token TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS versions (
        id TEXT PRIMARY KEY,
        branch TEXT NOT NULL,
        seq INTEGER NOT NULL,
        label TEXT NOT NULL,
        rule_set_version TEXT NOT NULL,
        base_version TEXT,
        derived_json TEXT NOT NULL,
        decisions_json TEXT NOT NULL,
        edit_token TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS events (
        id TEXT PRIMARY KEY,
        version_id TEXT NOT NULL,
        branch TEXT NOT NULL,
        seq INTEGER NOT NULL,
        kind TEXT NOT NULL,
        payload_json TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS decisions (
        branch TEXT NOT NULL,
        subject TEXT NOT NULL,
        choice TEXT NOT NULL,
        candidate_ids_json TEXT NOT NULL,
        applied_version TEXT,
        status TEXT NOT NULL,
        created_version TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        PRIMARY KEY (branch, subject)
    );
    "#
}

impl Store {
    pub fn open(path: &str) -> Result<Store, StoreError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "on")?;
        let store = Store {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Store, StoreError> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "journal_mode", "MEMORY")?;
        let store = Store {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().expect("conn lock");
        let existing: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='schema_meta'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if existing.is_none() {
            conn.execute_batch(schema_sql())?;
            conn.execute(
                "INSERT INTO schema_meta(key, value) VALUES ('schema_version', ?1)",
                params![SCHEMA_VERSION.to_string()],
            )?;
            return Ok(());
        }
        let v: String = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "1".to_string());
        let v: u32 = v.parse().unwrap_or(1);
        if v < 2 {
            // v1 -> v2 migration: add token columns when upgrading an old db.
            let cols: Vec<String> = {
                let mut stmt = conn.prepare("PRAGMA table_info(versions)")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            if !cols.contains(&"edit_token".to_string()) {
                conn.execute_batch(
                    "ALTER TABLE versions ADD COLUMN edit_token TEXT NOT NULL DEFAULT 'legacy';
                     ALTER TABLE branches ADD COLUMN edit_token TEXT NOT NULL DEFAULT 'legacy';",
                )?;
            }
            conn.execute(
                "UPDATE schema_meta SET value='2' WHERE key='schema_version'",
                [],
            )?;
        }
        Ok(())
    }
}

impl Store {
    /// Replace the file set inside a transaction and derive a new version.
    /// `crash_after_write` simulates a process killed mid-write: the derived
    /// version row is inserted but the pointer deliberately not moved, then
    /// the transaction is rolled back to prove atomic visibility.
    pub fn save_version(
        &self,
        branch: &str,
        label: &str,
        base_version: Option<&str>,
        files: &[SourceFile],
        derived: &DerivedModel,
        decisions_json: &str,
        expected_edit_token: Option<&str>,
        simulate_crash: bool,
    ) -> Result<String, StoreError> {
        let mut conn = self.conn.lock().expect("conn");
        let tx = conn.transaction()?;

        // optimistic concurrency: two editors from the same base version
        let token_now: String = tx
            .query_row(
                "SELECT edit_token FROM branches WHERE name=?1",
                params![branch],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "none".to_string());
        if let Some(expected) = expected_edit_token {
            if expected != token_now {
                return Err(StoreError::Conflict(format!(
                    "branch {} was modified concurrently (current token {}, expected {})",
                    branch, token_now, expected
                )));
            }
        }

        let seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM versions WHERE branch=?1",
                params![branch],
                |r| r.get(0),
            )
            .unwrap_or(1);
        let id = format!("ver-{}-{}-{}", branch, seq, &crate::identity::short_hash(&[branch, &seq.to_string(), label])[..8]);
        let new_token = crate::identity::short_hash(&[&id, "token"]);
        let derived_json = serde_json::to_string(derived).map_err(|e| StoreError::Other(e.to_string()))?;

        // overwrite files deterministically
        tx.execute("DELETE FROM files", [])?;
        for (i, f) in files.iter().enumerate() {
            tx.execute(
                "INSERT INTO files(id, path, content, content_hash, ordinal)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET path=excluded.path, content=excluded.content,
                   content_hash=excluded.content_hash, ordinal=excluded.ordinal",
                params![
                    f.id,
                    f.path,
                    f.content,
                    crate::identity::short_hash(&[&f.content]),
                    i as i64
                ],
            )?;
        }

        tx.execute(
            "INSERT INTO versions(id, branch, seq, label, rule_set_version, base_version,
                derived_json, decisions_json, edit_token)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                id,
                branch,
                seq,
                label,
                derived.rule_set_version,
                base_version,
                derived_json,
                decisions_json,
                new_token
            ],
        )?;

        for e in &derived.events {
            let payload = serde_json::to_string(e).map_err(|e| StoreError::Other(e.to_string()))?;
            tx.execute(
                "INSERT INTO events(id, version_id, branch, seq, kind, payload_json)
                 VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(id) DO NOTHING",
                params![e.id, id, branch, e.seq, e.kind, payload],
            )?;
        }

        if simulate_crash {
            // process dies before COMMIT: all of the above disappears.
            tx.rollback()?;
            return Err(StoreError::Other(
                "simulated crash: transaction rolled back, current pointer unchanged".into(),
            ));
        }

        tx.execute(
            "INSERT INTO branches(name, current_version, edit_token)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET current_version=excluded.current_version,
               edit_token=excluded.edit_token",
            params![branch, id, new_token],
        )?;

        tx.commit()?;
        Ok(id)
    }

    pub fn edit_token(&self, branch: &str) -> String {
        let conn = self.conn.lock().expect("conn");
        conn.query_row(
            "SELECT edit_token FROM branches WHERE name=?1",
            params![branch],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "none".to_string())
    }

    pub fn current_version_id(&self, branch: &str) -> Option<String> {
        let conn = self.conn.lock().expect("conn");
        conn.query_row(
            "SELECT current_version FROM branches WHERE name=?1",
            params![branch],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn current(&self, branch: &str) -> Result<Option<VersionMeta>, StoreError> {
        match self.current_version_id(branch) {
            Some(id) => Ok(Some(self.version(&id)?)),
            None => Ok(None),
        }
    }

    pub fn version(&self, id: &str) -> Result<VersionMeta, StoreError> {
        let conn = self.conn.lock().expect("conn");
        conn.query_row(
            "SELECT id, branch, seq, label, rule_set_version, base_version, derived_json, edit_token, created_at
             FROM versions WHERE id=?1",
            params![id],
            |r| {
                let derived_json: String = r.get(6)?;
                let derived: DerivedModel = serde_json::from_str(&derived_json).unwrap_or_default();
                Ok(VersionMeta {
                    id: r.get(0)?,
                    branch: r.get(1)?,
                    seq: r.get(2)?,
                    label: r.get(3)?,
                    rule_set_version: r.get(4)?,
                    base_version: r.get(5)?,
                    edit_token: r.get(7)?,
                    created_at: r.get(8)?,
                    derived,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("version {}", id)))
    }

    pub fn history(&self, branch: &str) -> Result<Vec<(String, i64, String, String)>, StoreError> {
        let conn = self.conn.lock().expect("conn");
        let mut stmt = conn.prepare(
            "SELECT id, seq, label, created_at FROM versions WHERE branch=?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![branch], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn decisions(&self, branch: &str) -> Result<crate::merge::Decisions, StoreError> {
        let conn = self.conn.lock().expect("conn");
        let mut stmt = conn.prepare(
            "SELECT subject, choice FROM decisions WHERE branch=?1 AND status='applied'",
        )?;
        let rows = stmt.query_map(params![branch], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut decisions = crate::merge::Decisions::default();
        for row in rows {
            let (subject, choice) = row?;
            classify_decision(&mut decisions, &subject, &choice);
        }
        Ok(decisions)
    }

    /// Record a manual adjudication. If a second, incompatible decision is
    /// made for the same subject both candidates are retained as a conflict
    /// instead of silently overwriting the first.
    pub fn record_decision(
        &self,
        branch: &str,
        subject: &str,
        choice: &str,
        candidate_ids: &[String],
        created_version: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().expect("conn");
        let existing: Option<(String, String)> = conn
            .query_row(
                "SELECT choice, status FROM decisions WHERE branch=?1 AND subject=?2",
                params![branch, subject],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old_choice, status)) = existing {
            if old_choice != choice {
                conn.execute(
                    "UPDATE decisions SET status='conflict', applied_version=NULL
                     WHERE branch=?1 AND subject=?2",
                    params![branch, subject],
                )?;
                return Err(StoreError::Conflict(format!(
                    "subject {} already decided '{}' ({}); new choice '{}' kept as a candidate, nothing overwritten",
                    subject, old_choice, status, choice
                )));
            }
        }
        let cj = serde_json::to_string(candidate_ids).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "INSERT INTO decisions(branch, subject, choice, candidate_ids_json, status, created_version)
             VALUES (?1,?2,?3,?4,'pending',?5)
             ON CONFLICT(branch, subject) DO UPDATE SET choice=excluded.choice,
               candidate_ids_json=excluded.candidate_ids_json, status='pending'",
            params![branch, subject, choice, cj, created_version],
        )?;
        Ok(())
    }

    pub fn mark_decisions_applied(&self, branch: &str, version_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().expect("conn");
        conn.execute(
            "UPDATE decisions SET status='applied', applied_version=?1
             WHERE branch=?2 AND status='pending'",
            params![version_id, branch],
        )?;
        Ok(())
    }

    pub fn pending_decisions(&self, branch: &str) -> Result<Vec<(String, String, String)>, StoreError> {
        let conn = self.conn.lock().expect("conn");
        let mut stmt = conn.prepare(
            "SELECT subject, choice, candidate_ids_json FROM decisions WHERE branch=?1 AND status='pending'",
        )?;
        let rows = stmt.query_map(params![branch], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn files(&self) -> Result<Vec<SourceFile>, StoreError> {
        let conn = self.conn.lock().expect("conn");
        let mut stmt = conn.prepare("SELECT path, content FROM files ORDER BY ordinal")?;
        let rows = stmt.query_map([], |r| {
            let path: String = r.get(0)?;
            let content: String = r.get(1)?;
            Ok((path, content))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (path, content) = row?;
            let parsed = crate::raw::parse_file(&path, &content)
                .map_err(|e| StoreError::Other(e))?;
            out.push(parsed);
        }
        Ok(out)
    }

    pub fn snippet(&self, file_id: &str, start: usize, end: usize) -> Option<(String, String)> {
        let conn = self.conn.lock().expect("conn");
        conn.query_row(
            "SELECT path, content FROM files WHERE id=?1",
            params![file_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|(p, c)| c.get(start..end).map(|s| (p, s.to_string())))
    }
}

fn classify_decision(d: &mut crate::merge::Decisions, subject: &str, choice: &str) {
    if subject.starts_with("g-") || subject.starts_with("ver-") {
        d.duplicates.insert(subject.to_string(), choice.to_string());
    } else if subject.starts_with("edge-") {
        d.edges.insert(subject.to_string(), choice.to_string());
    } else if subject.contains('@') {
        d.requirements.insert(subject.to_string(), choice.to_string());
    } else {
        d.duplicates.insert(subject.to_string(), choice.to_string());
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExportBundle {
    pub bundle: String,
    pub schema_version: u32,
    pub exported_at: String,
    pub files: Vec<ExportedFile>,
    pub versions: Vec<ExportedVersion>,
    pub branches: Vec<ExportedBranch>,
    pub decisions: Vec<ExportedDecision>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExportedFile {
    pub id: String,
    pub path: String,
    pub content: String,
    pub content_hash: String,
    pub ordinal: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExportedVersion {
    pub id: String,
    pub branch: String,
    pub seq: i64,
    pub label: String,
    pub rule_set_version: String,
    pub base_version: Option<String>,
    pub derived_json: String,
    pub edit_token: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExportedBranch {
    pub name: String,
    pub current_version: Option<String>,
    pub edit_token: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExportedDecision {
    pub branch: String,
    pub subject: String,
    pub choice: String,
    pub candidate_ids_json: String,
    pub status: String,
    pub applied_version: Option<String>,
    pub created_version: String,
}

impl Store {
    pub fn export_json(&self) -> Result<String, StoreError> {
        let conn = self.conn.lock().expect("conn");
        let files = {
            let mut stmt = stmt(&conn, "SELECT id, path, content, content_hash, ordinal FROM files ORDER BY ordinal")?;
            collect_exported_files(&mut stmt)?
        };
        let versions = {
            let mut stmt = stmt(
                &conn,
                "SELECT id, branch, seq, label, rule_set_version, base_version, derived_json, edit_token FROM versions ORDER BY branch, seq",
            )?;
            collect_exported_versions(&mut stmt)?
        };
        let branches = {
            let mut stmt = stmt(&conn, "SELECT name, current_version, edit_token FROM branches")?;
            let rows = stmt.query_map([], |r| {
                Ok(ExportedBranch {
                    name: r.get(0)?,
                    current_version: r.get(1)?,
                    edit_token: r.get(2)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let decisions = {
            let mut stmt = stmt(
                &conn,
                "SELECT branch, subject, choice, candidate_ids_json, status, applied_version, created_version FROM decisions",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(ExportedDecision {
                    branch: r.get(0)?,
                    subject: r.get(1)?,
                    choice: r.get(2)?,
                    candidate_ids_json: r.get(3)?,
                    status: r.get(4)?,
                    applied_version: r.get(5)?,
                    created_version: r.get(6)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let bundle = ExportBundle {
            bundle: "nscompass-export".into(),
            schema_version: SCHEMA_VERSION,
            exported_at: chrono_free_now(),
            files,
            versions,
            branches,
            decisions,
        };
        serde_json::to_string_pretty(&bundle).map_err(|e| StoreError::Other(e.to_string()))
    }

    pub fn import_json(&self, json: &str) -> Result<usize, StoreError> {
        let bundle: ExportBundle =
            serde_json::from_str(json).map_err(|e| StoreError::Other(e.to_string()))?;
        let mut conn = self.conn.lock().expect("conn");
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM files", [])?;
        tx.execute("DELETE FROM versions", [])?;
        tx.execute("DELETE FROM branches", [])?;
        tx.execute("DELETE FROM events", [])?;
        tx.execute("DELETE FROM decisions", [])?;
        for f in &bundle.files {
            tx.execute(
                "INSERT INTO files(id, path, content, content_hash, ordinal) VALUES (?1,?2,?3,?4,?5)",
                params![f.id, f.path, f.content, f.content_hash, f.ordinal],
            )?;
        }
        for v in &bundle.versions {
            tx.execute(
                "INSERT INTO versions(id, branch, seq, label, rule_set_version, base_version, derived_json, decisions_json, edit_token)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,'{}',?8)",
                params![v.id, v.branch, v.seq, v.label, v.rule_set_version, v.base_version, v.derived_json, v.edit_token],
            )?;
            if let Ok(derived) = serde_json::from_str::<DerivedModel>(&v.derived_json) {
                for e in &derived.events {
                    let payload = serde_json::to_string(e).unwrap_or_default();
                    tx.execute(
                        "INSERT OR IGNORE INTO events(id, version_id, branch, seq, kind, payload_json)
                         VALUES (?1,?2,?3,?4,?5,?6)",
                        params![e.id, v.id, v.branch, e.seq, e.kind, payload],
                    )?;
                }
            }
        }
        for b in &bundle.branches {
            tx.execute(
                "INSERT INTO branches(name, current_version, edit_token) VALUES (?1,?2,?3)",
                params![b.name, b.current_version, b.edit_token],
            )?;
        }
        for d in &bundle.decisions {
            tx.execute(
                "INSERT INTO decisions(branch, subject, choice, candidate_ids_json, status, applied_version, created_version)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![d.branch, d.subject, d.choice, d.candidate_ids_json, d.status, d.applied_version, d.created_version],
            )?;
        }
        tx.commit()?;
        Ok(bundle.versions.len())
    }
}

fn chrono_free_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{}", secs)
}

fn stmt<'a>(conn: &'a Connection, sql: &str) -> rusqlite::Result<rusqlite::Statement<'a>> {
    conn.prepare(sql)
}

fn collect_exported_files(s: &mut rusqlite::Statement) -> Result<Vec<ExportedFile>, StoreError> {
    let rows = s.query_map([], |r| {
        Ok(ExportedFile {
            id: r.get(0)?,
            path: r.get(1)?,
            content: r.get(2)?,
            content_hash: r.get(3)?,
            ordinal: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn collect_exported_versions(s: &mut rusqlite::Statement) -> Result<Vec<ExportedVersion>, StoreError> {
    let rows = s.query_map([], |r| {
        Ok(ExportedVersion {
            id: r.get(0)?,
            branch: r.get(1)?,
            seq: r.get(2)?,
            label: r.get(3)?,
            rule_set_version: r.get(4)?,
            base_version: r.get(5)?,
            derived_json: r.get(6)?,
            edit_token: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
