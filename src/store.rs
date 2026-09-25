//! SQLite event-sourced store.
//!
//! Guarantees:
//! - Every derivation is a new version; the branch head pointer switches only
//!   inside the same transaction that writes the full version row.
//! - An interrupted write leaves either the old version or a complete new one.
//! - Two users saving from the same version produce a resolvable conflict;
//!   the losing candidate is kept on its own branch, never overwritten.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CURRENT_SCHEMA: i64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub branch: String,
    pub version_id: Option<String>,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionMeta {
    pub id: String,
    pub branch: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Archive {
    pub schema: i64,
    pub events: Vec<Event>,
    pub versions: Vec<VersionRow>,
    pub heads: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRow {
    pub meta: VersionMeta,
    pub state_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SaveOutcome {
    Committed { version_id: String },
    Conflict { current_head: String, kept_version_id: String, kept_branch: String },
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Store> {
        let conn = Connection::open(path)?;
        let s = Store { conn };
        s.migrate()?;
        Ok(s)
    }

    pub fn open_memory() -> rusqlite::Result<Store> {
        let conn = Connection::open_in_memory()?;
        let s = Store { conn };
        s.migrate()?;
        Ok(s)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);",
        )?;
        let v: i64 = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(),
            )
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if v < 1 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS events(
                    seq INTEGER PRIMARY KEY AUTOINCREMENT,
                    branch TEXT NOT NULL,
                    version_id TEXT,
                    kind TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    created_at INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS versions(
                    id TEXT PRIMARY KEY,
                    branch TEXT NOT NULL,
                    parent_id TEXT,
                    label TEXT NOT NULL,
                    state_json TEXT NOT NULL,
                    created_at INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS heads(
                    branch TEXT PRIMARY KEY,
                    version_id TEXT NOT NULL);
                 INSERT OR REPLACE INTO meta(key,value) VALUES('schema_version','1');",
            )?;
        }
        if v < 2 {
            // v2: derived-artifact cache for dependency-propagation recompute.
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS artifacts(
                    version_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    depends_on TEXT NOT NULL,
                    data_json TEXT NOT NULL,
                    PRIMARY KEY(version_id, name));
                 INSERT OR REPLACE INTO meta(key,value) VALUES('schema_version','2');",
            )?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> i64 {
        self.conn
            .query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| {
                r.get::<_, String>()
            })
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn append_event(
        &self,
        branch: &str,
        version_id: Option<&str>,
        kind: &str,
        payload: &serde_json::Value,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO events(branch,version_id,kind,payload,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![branch, version_id, kind, payload.to_string(), now_millis()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn row_to_event(r: &rusqlite::Row) -> rusqlite::Result<Event> {
        let payload: String = r.get(4)?;
        Ok(Event {
            seq: r.get(0)?,
            branch: r.get(1)?,
            version_id: r.get(2)?,
            kind: r.get(3)?,
            payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
            created_at: r.get(5)?,
        })
    }

    pub fn events(&self) -> rusqlite::Result<Vec<Event>> {
        let mut st = self.conn.prepare(
            "SELECT seq,branch,version_id,kind,payload,created_at FROM events ORDER BY seq",
        )?;
        let rows = st.query_map([], Self::row_to_event)?;
        rows.collect()
    }

    pub fn events_up_to(&self, seq: i64) -> rusqlite::Result<Vec<Event>> {
        let mut st = self.conn.prepare(
            "SELECT seq,branch,version_id,kind,payload,created_at FROM events WHERE seq<=?1 ORDER BY seq",
        )?;
        let rows = st.query_map([], Self::row_to_event)?;
        rows.collect()
    }

    pub fn head(&self, branch: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT version_id FROM heads WHERE branch=?1",
                params![branch],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
    }

    fn next_version_id_tx(tx: &rusqlite::Transaction, branch: &str) -> rusqlite::Result<String> {
        let n: i64 = tx.query_row("SELECT COUNT(*) FROM versions", [], |r| r.get(0))?;
        Ok(format!("v{:04}-{}", n + 1, branch))
    }

    /// Atomic save: version row + head pointer switch in one transaction.
    pub fn save_version(
        &self,
        branch: &str,
        expected_parent: Option<&str>,
        label: &str,
        state_json: &str,
    ) -> rusqlite::Result<SaveOutcome> {
        let current = self.head(branch)?;
        if current.as_deref() != expected_parent {
            // Concurrent save conflict: keep this candidate on its own branch.
            let kept_branch = format!("{}~conflict", branch);
            let tx = self.conn.unchecked_transaction()?;
            let id = Self::next_version_id_tx(&tx, &kept_branch)?;
            tx.execute(
                "INSERT INTO versions(id,branch,parent_id,label,state_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
                params![id, kept_branch, expected_parent, label, state_json, now_millis()],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO heads(branch,version_id) VALUES(?1,?2)",
                params![kept_branch, id],
            )?;
            tx.execute(
                "INSERT INTO events(branch,version_id,kind,payload,created_at) VALUES(?1,?2,?3,?4,?5)",
                params![
                    branch,
                    id,
                    "ConflictDetected",
                    serde_json::json!({
                        "expected_parent": expected_parent,
                        "current_head": current,
                        "kept_branch": kept_branch,
                        "kept_version_id": id,
                    })
                    .to_string(),
                    now_millis()
                ],
            )?;
            tx.commit()?;
            return Ok(SaveOutcome::Conflict {
                current_head: current.unwrap_or_default(),
                kept_version_id: id,
                kept_branch,
            });
        }
        let tx = self.conn.unchecked_transaction()?;
        let id = Self::next_version_id_tx(&tx, branch)?;
        tx.execute(
            "INSERT INTO versions(id,branch,parent_id,label,state_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, branch, expected_parent, label, state_json, now_millis()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO heads(branch,version_id) VALUES(?1,?2)",
            params![branch, id],
        )?;
        tx.commit()?;
        Ok(SaveOutcome::Committed { version_id: id })
    }

    /// Test hook: simulate a process crash mid-write (insert without commit).
    #[doc(hidden)]
    pub fn simulate_interrupted_write(
        &self,
        branch: &str,
        label: &str,
        state_json: &str,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let id = Self::next_version_id_tx(&tx, branch)?;
        tx.execute(
            "INSERT INTO versions(id,branch,parent_id,label,state_json,created_at) VALUES(?1,?2,NULL,?3,?4,?5)",
            params![id, branch, label, state_json, now_millis()],
        )?;
        // Crash before head switch and before commit: drop rolls back.
        drop(tx);
        Ok(())
    }

    pub fn version_state(&self, id: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT state_json FROM versions WHERE id=?1", params![id], |r| {
                r.get(0)
            })
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
    }

    pub fn versions(&self) -> rusqlite::Result<Vec<VersionMeta>> {
        let mut st = self
            .conn
            .prepare("SELECT id,branch,parent_id,label,created_at FROM versions ORDER BY created_at, id")?;
        let rows = st.query_map([], |r| {
            Ok(VersionMeta {
                id: r.get(0)?,
                branch: r.get(1)?,
                parent_id: r.get(2)?,
                label: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn heads(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut st = self
            .conn
            .prepare("SELECT branch,version_id FROM heads ORDER BY branch")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    pub fn put_artifact(
        &self,
        version_id: &str,
        name: &str,
        depends_on: &[String],
        data_json: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO artifacts(version_id,name,depends_on,data_json) VALUES(?1,?2,?3,?4)",
            params![version_id, name, depends_on.join(","), data_json],
        )
    }

    pub fn export_archive(&self) -> rusqlite::Result<Archive> {
        let events = self.events()?;
        let mut st = self.conn.prepare(
            "SELECT id,branch,parent_id,label,state_json,created_at FROM versions ORDER BY created_at, id",
        )?;
        let versions = st
            .query_map([], |r| {
                Ok(VersionRow {
                    meta: VersionMeta {
                        id: r.get(0)?,
                        branch: r.get(1)?,
                        parent_id: r.get(2)?,
                        label: r.get(3)?,
                        created_at: r.get(4)?,
                    },
                    state_json: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let heads = self.heads()?;
        Ok(Archive { schema: CURRENT_SCHEMA, events, versions, heads })
    }

    /// Import an archive into this (expected-empty) store, preserving
    /// identity (version ids, event seq), order and provenance.
    pub fn import_archive(&self, archive: &Archive) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for v in &archive.versions {
            tx.execute(
                "INSERT INTO versions(id,branch,parent_id,label,state_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    v.meta.id,
                    v.meta.branch,
                    v.meta.parent_id,
                    v.meta.label,
                    v.state_json,
                    v.meta.created_at
                ],
            )?;
        }
        for e in &archive.events {
            tx.execute(
                "INSERT INTO events(seq,branch,version_id,kind,payload,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    e.seq,
                    e.branch,
                    e.version_id,
                    e.kind,
                    e.payload.to_string(),
                    e.created_at
                ],
            )?;
        }
        for (branch, head) in &archive.heads {
            tx.execute(
                "INSERT OR REPLACE INTO heads(branch,version_id) VALUES(?1,?2)",
                params![branch, head],
            )?;
        }
        tx.commit()
    }
}
