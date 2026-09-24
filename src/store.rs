//! SQLite-backed, crash-safe storage.
//!
//! Writes that produce a new derived version run inside a single transaction:
//! the branch pointer only moves once the full snapshot is inserted, so a
//! process interrupted mid-write leaves either the old version or a complete
//! new one visible.

use crate::domain::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const CURRENT_SCHEMA: i64 = 2;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct SourceFileRow {
    pub file_id: String,
    pub path_hint: String,
    pub content: String,
    pub ordinal: i64,
}

#[derive(Debug, Clone)]
pub struct CommitOutcome {
    pub event: ProjectEvent,
    pub version_id: String,
    pub conflict: Option<String>,
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Store> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 10_000)?;
        let store = Store {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// In-memory store (tests / ephemeral demo).
    pub fn open_memory() -> rusqlite::Result<Store> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Store {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        let tx = conn.transaction()?;
        if version < 1 {
            schema_v1(&tx)?;
        }
        if version < 2 {
            schema_v2(&tx)?;
        }
        tx.pragma_update(None, "user_version", CURRENT_SCHEMA)?;
        tx.commit()?;
        Ok(())
    }

    pub fn lock_raw(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }

    pub fn all_file_rows(
        &self,
        branch: &str,
        up_to_seq: i64,
    ) -> rusqlite::Result<
        Vec<(String, i64, String, String, String, i64)>,
    > {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT branch,event_seq,file_id,path_hint,content,ordinal
             FROM source_files WHERE branch=?1 AND event_seq<=?2
             ORDER BY event_seq, file_id",
        )?;
        let rows = stmt.query_map(params![branch, up_to_seq], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        rows.collect()
    }

    pub fn all_snapshots(&self) -> rusqlite::Result<Vec<VersionSnapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT payload FROM snapshots ORDER BY event_seq")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for p in rows {
            if let Ok(snap) = serde_json::from_str::<VersionSnapshot>(&p?) {
                out.push(snap);
            }
        }
        Ok(out)
    }

    pub fn schema_version(&self) -> rusqlite::Result<i64> {
        self.conn
            .lock()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
    }
}

fn schema_v1(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE source_files (
            file_id TEXT NOT NULL,
            branch TEXT NOT NULL,
            event_seq INTEGER NOT NULL,
            path_hint TEXT NOT NULL,
            content TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            PRIMARY KEY (branch, event_seq, file_id)
        );
        CREATE TABLE events (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            branch TEXT NOT NULL,
            kind TEXT NOT NULL,
            rule_version TEXT NOT NULL,
            label TEXT NOT NULL,
            detail_json TEXT NOT NULL,
            source_file_id TEXT,
            parent_event_id TEXT,
            base_version_id TEXT,
            result_version_id TEXT,
            accepted INTEGER NOT NULL DEFAULT 1,
            clock_tag TEXT NOT NULL
        );
        CREATE TABLE branches (
            branch TEXT PRIMARY KEY,
            event_seq INTEGER NOT NULL,
            version_id TEXT NOT NULL
        );
        CREATE TABLE snapshots (
            version_id TEXT PRIMARY KEY,
            branch TEXT NOT NULL,
            event_seq INTEGER NOT NULL,
            rule_version TEXT NOT NULL,
            payload TEXT NOT NULL,
            digest TEXT NOT NULL
        );
        CREATE TABLE decisions (
            id TEXT PRIMARY KEY,
            branch TEXT NOT NULL,
            subject TEXT NOT NULL,
            options_json TEXT NOT NULL,
            chosen TEXT,
            rejected_json TEXT NOT NULL DEFAULT '[]',
            created_event_seq INTEGER NOT NULL,
            resolved_event_seq INTEGER
        );
        "#,
    )?;
    Ok(())
}

fn schema_v2(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    // v2 adds stable identity/order metadata on snapshots for export round-trip.
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS export_manifest (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            exported_at TEXT NOT NULL,
            schema_version INTEGER NOT NULL,
            identity_scheme TEXT NOT NULL
        );
        ALTER TABLE snapshots ADD COLUMN ordinal_override INTEGER;
        "#,
    )?;
    Ok(())
}

impl Store {
    pub fn set_meta(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO meta(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| {
                r.get(0)
            })
            .optional()
    }

    pub fn branch_pointer(&self, branch: &str) -> rusqlite::Result<Option<BranchPoint>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT branch,event_seq,version_id FROM branches WHERE branch=?1",
                params![branch],
                |r| {
                    Ok(BranchPoint {
                        branch: r.get(0)?,
                        event_seq: r.get(1)?,
                        version_id: r.get(2)?,
                    })
                },
            )
            .optional()
    }

    pub fn list_branches(&self) -> rusqlite::Result<Vec<BranchPoint>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT branch,event_seq,version_id FROM branches ORDER BY branch")?;
        let rows = stmt.query_map([], |r| {
            Ok(BranchPoint {
                branch: r.get(0)?,
                event_seq: r.get(1)?,
                version_id: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn list_events(&self, branch: &str) -> rusqlite::Result<Vec<ProjectEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT seq,id,branch,kind,rule_version,label,detail_json,source_file_id,
                    parent_event_id,base_version_id,result_version_id,accepted,clock_tag
             FROM events WHERE branch=?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![branch], map_event)?;
        rows.collect()
    }

    pub fn all_events(&self) -> rusqlite::Result<Vec<ProjectEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT seq,id,branch,kind,rule_version,label,detail_json,source_file_id,
                    parent_event_id,base_version_id,result_version_id,accepted,clock_tag
             FROM events ORDER BY seq",
        )?;
        let rows = stmt.query_map([], map_event)?;
        rows.collect()
    }

    pub fn event_by_id(&self, id: &str) -> rusqlite::Result<Option<ProjectEvent>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT seq,id,branch,kind,rule_version,label,detail_json,source_file_id,
                        parent_event_id,base_version_id,result_version_id,accepted,clock_tag
                 FROM events WHERE id=?1",
                params![id],
                map_event,
            )
            .optional()
    }

    pub fn snapshot(&self, version_id: &str) -> rusqlite::Result<Option<VersionSnapshot>> {
        let payload: Option<String> = self
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT payload FROM snapshots WHERE version_id=?1",
                params![version_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(payload.and_then(|p| serde_json::from_str(&p).ok()))
    }

    pub fn snapshot_for_event(
        &self,
        branch: &str,
        seq: i64,
    ) -> rusqlite::Result<Option<VersionSnapshot>> {
        // Find latest event at or before seq on branch that produced a version.
        let version_id: Option<String> = self
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT result_version_id FROM events
                 WHERE branch=?1 AND seq<=?2 AND result_version_id IS NOT NULL
                 ORDER BY seq DESC LIMIT 1",
                params![branch, seq],
                |r| r.get(0),
            )
            .optional()?;
        match version_id {
            Some(v) => self.snapshot(&v),
            None => Ok(None),
        }
    }

    pub fn files_at(&self, branch: &str, seq: i64) -> rusqlite::Result<Vec<SourceFileRow>> {
        // Most recent row per file_id at or before seq.
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT file_id, path_hint, content, ordinal FROM source_files
             WHERE branch=?1 AND event_seq<=?2
             GROUP BY file_id HAVING event_seq=MAX(event_seq)
             ORDER BY ordinal, file_id",
        )?;
        let rows = stmt.query_map(params![branch, seq], |r| {
            Ok(SourceFileRow {
                file_id: r.get(0)?,
                path_hint: r.get(1)?,
                content: r.get(2)?,
                ordinal: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn current_files(&self, branch: &str) -> rusqlite::Result<Vec<SourceFileRow>> {
        match self.branch_pointer(branch)? {
            Some(bp) => self.files_at(branch, bp.event_seq),
            None => Ok(Vec::new()),
        }
    }

    pub fn put_decision(&self, d: &Decision) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO decisions(id,branch,subject,options_json,chosen,rejected_json,
                                   created_event_seq,resolved_event_seq)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(id) DO UPDATE SET
               chosen=excluded.chosen,
               rejected_json=excluded.rejected_json,
               resolved_event_seq=excluded.resolved_event_seq",
            params![
                d.id,
                d.branch,
                d.subject,
                serde_json::to_string(&d.options).unwrap_or_default(),
                d.chosen,
                serde_json::to_string(&d.rejected).unwrap_or_default(),
                d.created_event_seq,
                d.resolved_event_seq,
            ],
        )?;
        Ok(())
    }

    pub fn list_decisions(&self, branch: &str) -> rusqlite::Result<Vec<Decision>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id,branch,subject,options_json,chosen,rejected_json,
                    created_event_seq,resolved_event_seq
             FROM decisions WHERE branch=?1 ORDER BY created_event_seq",
        )?;
        let rows = stmt.query_map(params![branch], |r| {
            let options: Vec<String> =
                serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default();
            let rejected: Vec<String> =
                serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default();
            Ok(Decision {
                id: r.get(0)?,
                branch: r.get(1)?,
                subject: r.get(2)?,
                options,
                chosen: r.get(4)?,
                rejected,
                created_event_seq: r.get(6)?,
                resolved_event_seq: r.get(7)?,
            })
        })?;
        rows.collect()
    }
}

fn map_event(r: &rusqlite::Row) -> rusqlite::Result<ProjectEvent> {
    Ok(ProjectEvent {
        seq: r.get(0)?,
        id: r.get(1)?,
        branch: r.get(2)?,
        kind: r.get(3)?,
        rule_version: r.get(4)?,
        label: r.get(5)?,
        detail_json: r.get(6)?,
        source_file_id: r.get(7)?,
        parent_event_id: r.get(8)?,
        base_version_id: r.get(9)?,
        result_version_id: r.get(10)?,
        accepted: r.get::<_, i64>(11)? != 0,
        clock_tag: r.get(12)?,
    })
}

pub struct NewEventInput {
    pub branch: String,
    pub kind: String,
    pub label: String,
    pub detail_json: String,
    pub source_file_id: Option<String>,
    pub parent_event_id: Option<String>,
    pub accepted: bool,
    pub clock_tag: String,
    /// For edits: the event seq the editor started from.
    pub base_seq: Option<i64>,
}

pub struct CommitPayload {
    pub event: NewEventInput,
    /// file_id -> (path_hint, content, ordinal) for changed/new files.
    pub changed_files: BTreeMap<String, (String, String, i64)>,
    /// file_ids deleted by this event.
    pub deleted_files: Vec<String>,
    /// Derived snapshot; None for pure decisions that don't yield a version.
    pub snapshot: Option<VersionSnapshot>,
}

impl Store {
    /// Atomically append an event, file rows and a snapshot, then move the
    /// branch pointer. Optimistic locking: if the caller started from an older
    /// seq while the branch moved, returns a conflict result without writing.
    pub fn commit(&self, payload: CommitPayload) -> rusqlite::Result<CommitOutcome> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let current_seq: Option<i64> = tx
            .query_row(
                "SELECT event_seq FROM branches WHERE branch=?1",
                params![payload.event.branch],
                |r| r.get(0),
            )
            .optional()?;

        if let (Some(base), Some(cur)) = (payload.event.base_seq, current_seq) {
            if base < cur {
                return Ok(CommitOutcome {
                    event: ProjectEvent {
                        seq: -1,
                        id: String::new(),
                        branch: payload.event.branch.clone(),
                        kind: payload.event.kind.clone(),
                        rule_version: crate::engine::RULE_VERSION.to_string(),
                        label: payload.event.label.clone(),
                        detail_json: payload.event.detail_json.clone(),
                        source_file_id: payload.event.source_file_id.clone(),
                        parent_event_id: payload.event.parent_event_id.clone(),
                        base_version_id: None,
                        result_version_id: None,
                        accepted: payload.event.accepted,
                        clock_tag: payload.event.clock_tag.clone(),
                    },
                    version_id: String::new(),
                    conflict: Some(format!(
                        "branch moved to seq {cur} since your base seq {base}; resolve and retry"
                    )),
                });
            }
        }

        let next_seq = tx
            .query_row("SELECT COALESCE(MAX(seq),0)+1 FROM events", [], |r| {
                r.get::<_, i64>(0)
            })?;
        let global_version = tx
            .query_row(
                "SELECT COALESCE(MAX(CAST(SUBSTR(version_id,5) AS INTEGER)),0)+1
                 FROM snapshots WHERE version_id GLOB 'ver-*'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(1);
        let event_id = format!("ev-{next_seq}-{}", short_tag(&payload.event.label));

        let base_version_id: Option<String> = tx
            .query_row(
                "SELECT version_id FROM branches WHERE branch=?1",
                params![payload.event.branch],
                |r| r.get(0),
            )
            .optional()?;

        let (result_version_id, digest) = match &payload.snapshot {
            Some(snap) => (Some(snap.version_id.clone()), snap.digest.clone()),
            None => (None, String::new()),
        };

        tx.execute(
            "INSERT INTO events(seq,id,branch,kind,rule_version,label,detail_json,source_file_id,
                                parent_event_id,base_version_id,result_version_id,accepted,clock_tag)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                next_seq,
                event_id,
                payload.event.branch,
                payload.event.kind,
                crate::engine::RULE_VERSION,
                payload.event.label,
                payload.event.detail_json,
                payload.event.source_file_id,
                payload.event.parent_event_id,
                base_version_id,
                result_version_id,
                payload.event.accepted as i64,
                payload.event.clock_tag,
            ],
        )?;

        for (file_id, (path_hint, content, ordinal)) in &payload.changed_files {
            tx.execute(
                "INSERT INTO source_files(branch,event_seq,file_id,path_hint,content,ordinal)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    payload.event.branch,
                    next_seq,
                    file_id,
                    path_hint,
                    content,
                    ordinal
                ],
            )?;
        }
        for file_id in &payload.deleted_files {
            // Deletion recorded as empty-content row at this seq; files_at uses
            // latest row, so it disappears. Raw history keeps prior rows.
            tx.execute(
                "INSERT INTO source_files(branch,event_seq,file_id,path_hint,content,ordinal)
                 VALUES(?1,?2,?3,'','',-1)",
                params![payload.event.branch, next_seq, file_id],
            )?;
        }

        let mut assigned_version_id: Option<String> = None;
        if let Some(snap) = &payload.snapshot {
            let mut snap = snap.clone();
            snap.version_id = format!("ver-{global_version}");
            snap.event_seq = next_seq;
            assigned_version_id = Some(snap.version_id.clone());
            let payload_json = serde_json::to_string(&snap).map_err(|e| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )))
            })?;
            tx.execute(
                "INSERT INTO snapshots(version_id,branch,event_seq,rule_version,payload,digest)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    snap.version_id,
                    snap.branch,
                    next_seq,
                    snap.rule_version,
                    payload_json,
                    snap.digest
                ],
            )?;
        }

        let effective_version_id = assigned_version_id.or(result_version_id);
        if let Some(version_id) = &effective_version_id {
            tx.execute(
                "INSERT INTO branches(branch,event_seq,version_id) VALUES(?1,?2,?3)
                 ON CONFLICT(branch) DO UPDATE SET event_seq=excluded.event_seq,
                                                  version_id=excluded.version_id",
                params![payload.event.branch, next_seq, version_id],
            )?;
        }

        tx.commit()?;

        Ok(CommitOutcome {
            event: ProjectEvent {
                seq: next_seq,
                id: event_id.clone(),
                branch: payload.event.branch.clone(),
                kind: payload.event.kind.clone(),
                rule_version: crate::engine::RULE_VERSION.to_string(),
                label: payload.event.label.clone(),
                detail_json: payload.event.detail_json.clone(),
                source_file_id: payload.event.source_file_id.clone(),
                parent_event_id: payload.event.parent_event_id.clone(),
                base_version_id,
                result_version_id: effective_version_id.clone(),
                accepted: payload.event.accepted,
                clock_tag: payload.event.clock_tag.clone(),
            },
            version_id: effective_version_id.unwrap_or_default(),
            conflict: None,
        })
    }
}

fn short_tag(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect()
}
