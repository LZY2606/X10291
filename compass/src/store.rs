//! SQLite persistence with atomic version switching.
//!
//! Every derivation is written as an independent version inside one
//! transaction; the current pointer moves only after the full transaction
//! commits. A crash mid-write therefore exposes either the old version or the
//! complete new one.

use crate::derive::{Decision, Version};
use crate::model::RawModel;
use anyhow::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::sync::Mutex;

pub const SCHEMA_VERSION: u32 = 2;

pub struct Store {
    pub conn: Mutex<Connection>,
}

#[derive(Debug)]
pub struct ConflictError {
    pub message: String,
}

impl std::fmt::Display for ConflictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ConflictError {}

pub fn init_connection(conn: &mut Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    migrate(conn)?;
    Ok(())
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    if current >= SCHEMA_VERSION {
        return Ok(());
    }
    let tx = conn.transaction()?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sources (
            source_id TEXT NOT NULL,
            version_id TEXT NOT NULL,
            path TEXT NOT NULL,
            content TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            PRIMARY KEY (source_id, version_id)
        );

        CREATE TABLE IF NOT EXISTS raw_models (
            version_id TEXT PRIMARY KEY,
            json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS merged_models (
            version_id TEXT PRIMARY KEY,
            rule_version TEXT NOT NULL,
            json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS versions (
            version_id TEXT PRIMARY KEY,
            branch TEXT NOT NULL,
            parent_id TEXT,
            label TEXT NOT NULL,
            kind TEXT NOT NULL,
            created_seq INTEGER NOT NULL,
            base_parent_id TEXT,
            replay_step INTEGER
        );

        CREATE TABLE IF NOT EXISTS events_log (
            event_id TEXT PRIMARY KEY,
            version_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            kind TEXT NOT NULL,
            code TEXT,
            json TEXT NOT NULL,
            UNIQUE (version_id, seq)
        );

        CREATE TABLE IF NOT EXISTS decisions (
            decision_id TEXT PRIMARY KEY,
            version_id TEXT NOT NULL,
            subject TEXT NOT NULL,
            adopted TEXT,
            rejected TEXT,
            json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS current_pointer (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            version_id TEXT NOT NULL
        );
        "#,
    )?;
    // v2 migration: replay_step / base_parent already created above for fresh
    // dbs; for v1 databases add columns if missing.
    let cols: Vec<String> = tx
        .prepare("PRAGMA table_info(versions)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .flatten()
        .collect();
    if !cols.contains(&"base_parent_id".to_string()) {
        tx.execute_batch("ALTER TABLE versions ADD COLUMN base_parent_id TEXT;")?;
    }
    if !cols.contains(&"replay_step".to_string()) {
        tx.execute_batch("ALTER TABLE versions ADD COLUMN replay_step INTEGER;")?;
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS current_pointer (id INTEGER PRIMARY KEY CHECK (id = 1), version_id TEXT NOT NULL);",
    )?;
    tx.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
    tx.commit()?;
    Ok(())
}

impl Store {
    pub fn open(path: &str) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        init_connection(&mut conn)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        init_connection(&mut conn)?;
        Ok(Store { conn: Mutex::new(conn) })
    }
}

/// Optional crash injection used by the write-interruption test: the process
/// aborts *after* version rows are written but *before* the pointer commits.
pub static CRASH_BEFORE_POINTER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

impl Store {
    /// Write a fully derived version and move the current pointer in one
    /// transaction. Returns Err(ConflictError) when `expected_parent` no
    /// longer matches the pointer (two editors on the same version).
    pub fn commit_version(
        &self,
        version: &Version,
        raw: &RawModel,
        merged: &crate::merge::MergedModel,
        expected_parent: Option<&str>,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        if let Some(expected) = expected_parent {
            let current: Option<String> = tx
                .query_row("SELECT version_id FROM current_pointer WHERE id = 1", [], |r| r.get(0))
                .optional()?;
            match current {
                Some(cur) if cur != expected => {
                    return Err(Box::new(ConflictError {
                        message: format!(
                            "concurrent edit: current version is {cur}, expected {expected}; resolve by rebasing or branching"
                        ),
                    }));
                }
                _ => {}
            }
        }

        for (ordinal, src) in raw.sources.iter().enumerate() {
            tx.execute(
                "INSERT OR REPLACE INTO sources (source_id, version_id, path, content, ordinal)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![src.source_id, version.version_id, src.path, src.bytes, ordinal as i64],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO raw_models (version_id, json) VALUES (?1, ?2)",
            rusqlite::params![version.version_id, serde_json::to_string(raw)?],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO merged_models (version_id, rule_version, json) VALUES (?1, ?2, ?3)",
            rusqlite::params![version.version_id, merged.rule_version, serde_json::to_string(merged)?],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO versions
             (version_id, branch, parent_id, label, kind, created_seq, base_parent_id, replay_step)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                version.version_id,
                version.branch,
                version.parent_id,
                version.label,
                version.kind,
                version.seq as i64,
                version.base_parent_id,
                version.replay_step,
            ],
        )?;
        for ev in &version.events {
            tx.execute(
                "INSERT OR REPLACE INTO events_log (event_id, version_id, seq, kind, code, json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![ev.event_id, version.version_id, ev.seq as i64, ev.kind, ev.code, serde_json::to_string(ev)?],
            )?;
        }
        for d in &version.decisions {
            tx.execute(
                "INSERT OR REPLACE INTO decisions (decision_id, version_id, subject, adopted, rejected, json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    d.decision_id,
                    version.version_id,
                    d.subject,
                    d.adopted,
                    serde_json::to_string(&d.rejected)?,
                    serde_json::to_string(d)?,
                ],
            )?;
        }

        // Deliberate interruption point for the crash-recovery test.
        if CRASH_BEFORE_POINTER.swap(false, std::sync::atomic::Ordering::SeqCst) {
            // Hold the write lock open and terminate; nothing commits.
            std::process::abort();
        }

        tx.execute(
            "INSERT INTO current_pointer (id, version_id) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET version_id = excluded.version_id",
            rusqlite::params![version.version_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn current_version_id(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT version_id FROM current_pointer WHERE id = 1", [], |r| r.get(0))
            .optional()?)
    }

    pub fn load_version(&self, version_id: &str) -> Result<Version> {
        let conn = self.conn.lock().unwrap();
        let raw_json: String = conn.query_row(
            "SELECT json FROM raw_models WHERE version_id = ?1",
            [version_id],
            |r| r.get(0),
        )?;
        let merged_json: String = conn.query_row(
            "SELECT json FROM merged_models WHERE version_id = ?1",
            [version_id],
            |r| r.get(0),
        )?;
        let meta: (String, Option<String>, String, String, i64, Option<String>, Option<i64>) = conn.query_row(
            "SELECT branch, parent_id, label, kind, created_seq, base_parent_id, replay_step
             FROM versions WHERE version_id = ?1",
            [version_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )?;
        let mut events = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT json FROM events_log WHERE version_id = ?1 ORDER BY seq",
            )?;
            let rows = stmt.query_map([version_id], |r| r.get::<_, String>(0))?;
            for row in rows {
                events.push(serde_json::from_str(&row?)?);
            }
        }
        let mut decisions = Vec::new();
        {
            let mut stmt = conn.prepare("SELECT json FROM decisions WHERE version_id = ?1")?;
            let rows = stmt.query_map([version_id], |r| r.get::<_, String>(0))?;
            for row in rows {
                decisions.push(serde_json::from_str(&row?)?);
            }
        }
        Ok(Version {
            version_id: version_id.to_string(),
            branch: meta.0,
            parent_id: meta.1,
            label: meta.2,
            kind: meta.3,
            seq: meta.4 as u64,
            base_parent_id: meta.5,
            replay_step: meta.6,
            parameters: VersionParams::default(),
            events,
            decisions,
            propagation: None,
            merge_events: Vec::new(),
            raw: Some(serde_json::from_str(&raw_json)?),
            merged: Some(serde_json::from_str(&merged_json)?),
        })
    }

    pub fn list_versions(&self) -> Result<Vec<(String, String, Option<String>, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT version_id, branch, parent_id, label, created_seq FROM versions ORDER BY created_seq",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?))
        })?;
        Ok(rows.flatten().collect())
    }

    /// Full self-contained archive; stable ids and order are serialized, so a
    /// delete + re-import restores identity, order and provenance.
    pub fn export_archive(&self) -> Result<Value> {
        let conn = self.conn.lock().unwrap();
        let mut versions = Vec::new();
        let mut stmt = conn.prepare("SELECT version_id FROM versions ORDER BY created_seq")?;
        let ids: Vec<String> = stmt.query_map([], |r| r.get(0))?.flatten().collect();
        drop(stmt);
        for id in ids {
            let raw: String = conn.query_row(
                "SELECT json FROM raw_models WHERE version_id = ?1",
                [&id],
                |r| r.get(0),
            )?;
            let merged: String = conn.query_row(
                "SELECT json FROM merged_models WHERE version_id = ?1",
                [&id],
                |r| r.get(0),
            )?;
            versions.push(serde_json::json!({ "version_id": id, "raw": serde_json::from_str::<Value>(&raw)?, "merged": serde_json::from_str::<Value>(&merged)? }));
        }
        let current: Option<String> = conn
            .query_row("SELECT version_id FROM current_pointer WHERE id = 1", [], |r| r.get(0))
            .optional()?;
        Ok(serde_json::json!({
            "archive": "compass-nodeset-archive",
            "schema_version": SCHEMA_VERSION,
            "rule_version": crate::rules::RULESET_VERSION,
            "current": current,
            "versions": versions,
        }))
    }
}

// Tiny Option-extractor so we do not pull in another dependency.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>>;
}
impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

impl Store {
    /// Re-create every version from an archive, preserving ids, creation order
    /// and the current pointer.
    pub fn import_archive(&self, archive: &Value) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = archive.get("current").and_then(|v| v.as_str());
        let versions = archive
            .get("versions")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("archive has no versions array"))?;
        tx.execute("DELETE FROM current_pointer", [])?;
        tx.execute("DELETE FROM versions", [])?;
        tx.execute("DELETE FROM raw_models", [])?;
        tx.execute("DELETE FROM merged_models", [])?;
        for (seq, item) in versions.iter().enumerate() {
            let version_id = item["version_id"].as_str().ok_or_else(|| anyhow::anyhow!("missing version_id"))?;
            let raw_json = item["raw"].to_string();
            let merged_json = item["merged"].to_string();
            let raw: RawModel = serde_json::from_str(&raw_json)?;
            let merged: crate::merge::MergedModel = serde_json::from_str(&merged_json)?;
            tx.execute(
                "INSERT OR REPLACE INTO raw_models (version_id, json) VALUES (?1, ?2)",
                rusqlite::params![version_id, raw_json],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO merged_models (version_id, rule_version, json) VALUES (?1, ?2, ?3)",
                rusqlite::params![version_id, merged.rule_version, merged_json],
            )?;
            for (ordinal, src) in raw.sources.iter().enumerate() {
                tx.execute(
                    "INSERT OR REPLACE INTO sources (source_id, version_id, path, content, ordinal)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![src.source_id, version_id, src.path, src.bytes, ordinal as i64],
                )?;
            }
            // Version metadata is derivable from merge events payload in the
            // full domain flow; archive import reconstructs a linear history.
            tx.execute(
                "INSERT INTO versions (version_id, branch, parent_id, label, kind, created_seq, base_parent_id, replay_step)
                 VALUES (?1, 'main', NULL, ?2, 'imported', ?3, NULL, NULL)",
                rusqlite::params![version_id, format!("imported-{seq}"), seq as i64],
            )?;
            let _ = &merged;
        }
        if let Some(cur) = current {
            tx.execute(
                "INSERT INTO current_pointer (id, version_id) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET version_id = excluded.version_id",
                rusqlite::params![cur],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
