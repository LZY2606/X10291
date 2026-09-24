//! SQLite store. Every derivation is an independent, immutable version; the
//! `current` pointer switches only after a version is fully written, inside
//! the same transaction. After an interrupted write a restart sees either the
//! old version or a complete new one, never a partial state.

use crate::derive::{self, Derivation};
use crate::merge::{Event, MergedModel};
use crate::xml::NodeSetFile;
use anyhow::{anyhow, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionPayload {
    pub files: Vec<NodeSetFile>,
    pub model: MergedModel,
    pub derivation: Derivation,
    pub label: String,
    pub edit: serde_json::Value,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub id: i64,
    pub parent: Option<i64>,
    pub label: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Edit {
    ImportFile { name: String, xml: String },
    ReplaceFile { name: String, xml: String },
    RemoveFile { name: String },
    Decision { action: String, subject: String, note: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SaveOutcome {
    Current { version_id: i64 },
    Conflict { current_version: i64, branch_version: i64 },
}

pub struct Store {
    pub conn: Connection,
}

fn now() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms}")
}

impl Store {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn schema_version(&self) -> Result<i64> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    fn migrate(&self) -> Result<()> {
        let v = self.schema_version()?;
        if v == 0 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);
                 CREATE TABLE IF NOT EXISTS versions(
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   parent INTEGER, label TEXT, status TEXT NOT NULL,
                   created_at TEXT NOT NULL, payload TEXT);
                 CREATE TABLE IF NOT EXISTS pointer(
                   id INTEGER PRIMARY KEY CHECK(id=1), version_id INTEGER NOT NULL);
                 CREATE TABLE IF NOT EXISTS decisions(
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   version_id INTEGER, seq INTEGER, kind TEXT,
                   payload TEXT, created_at TEXT);",
            )?;
            self.set_schema_version(SCHEMA_VERSION)?;
        } else if v == 1 {
            // v1 -> v2: versions gains `status`, decisions table is added.
            self.conn.execute_batch(
                "ALTER TABLE versions ADD COLUMN status TEXT NOT NULL DEFAULT 'complete';
                 CREATE TABLE IF NOT EXISTS decisions(
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   version_id INTEGER, seq INTEGER, kind TEXT,
                   payload TEXT, created_at TEXT);",
            )?;
            self.set_schema_version(SCHEMA_VERSION)?;
        }
        Ok(())
    }

    fn set_schema_version(&self, v: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key,value) VALUES('schema_version',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [v.to_string()],
        )?;
        Ok(())
    }

    pub fn current_pointer(&self) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT version_id FROM pointer WHERE id=1", [], |r| r.get(0))
            .optional()?)
    }

    pub fn list_versions(&self) -> Result<Vec<VersionInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,parent,label,status,created_at FROM versions ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(VersionInfo {
                id: r.get(0)?,
                parent: r.get(1)?,
                label: r.get(2)?,
                status: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn load_payload(&self, version_id: i64) -> Result<VersionPayload> {
        let (status, payload): (String, Option<String>) = self
            .conn
            .query_row(
                "SELECT status, payload FROM versions WHERE id=?1",
                [version_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| anyhow!("version {version_id} not found"))?;
        if status != "complete" {
            return Err(anyhow!("version {version_id} is incomplete (interrupted write)"));
        }
        let payload = payload.ok_or_else(|| anyhow!("version {version_id} has no payload"))?;
        Ok(serde_json::from_str(&payload)?)
    }

    /// Insert a complete version and optionally move the pointer, atomically.
    fn commit_version(
        &self,
        parent: Option<i64>,
        label: &str,
        payload: &VersionPayload,
        move_pointer: bool,
    ) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO versions(parent,label,status,created_at,payload)
             VALUES(?1,?2,'building',?3,NULL)",
            rusqlite::params![parent, label, now()],
        )?;
        let id = tx.last_insert_rowid();
        let blob = serde_json::to_string(payload)?;
        // The version becomes visible only with its full payload, and the
        // pointer switches in the same transaction.
        tx.execute(
            "UPDATE versions SET status='complete', payload=?1 WHERE id=?2",
            rusqlite::params![blob, id],
        )?;
        if move_pointer {
            tx.execute(
                "INSERT INTO pointer(id,version_id) VALUES(1,?1)
                 ON CONFLICT(id) DO UPDATE SET version_id=excluded.version_id",
                [id],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Test/crash helper: begin a write and abandon it mid-transaction.
    pub fn simulate_interrupted_write(&self, label: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO versions(parent,label,status,created_at,payload)
             VALUES(NULL,?1,'building',?2,NULL)",
            rusqlite::params![label, now()],
        )?;
        drop(tx); // rollback, as if the process died here
        Ok(())
    }

    /// Remove leftover 'building' rows after a crash; returns removed count.
    pub fn recover(&self) -> Result<usize> {
        let n = self
            .conn
            .execute("DELETE FROM versions WHERE status='building'", [])?;
        Ok(n)
    }

    pub fn record_event(
        &self,
        version_id: Option<i64>,
        kind: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        let seq: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(seq),-1)+1 FROM decisions
                 WHERE version_id IS ?1",
                [version_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        self.conn.execute(
            "INSERT INTO decisions(version_id,seq,kind,payload,created_at)
             VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![version_id, seq, kind, payload.to_string(), now()],
        )?;
        Ok(())
    }

    pub fn events_of(&self, version_id: Option<i64>) -> Result<Vec<(String, serde_json::Value)>> {
        let mut stmt = self.conn.prepare(
            "SELECT kind,payload FROM decisions WHERE version_id IS ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([version_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter_map(|(k, p)| serde_json::from_str(&p).ok().map(|v| (k, v)))
            .collect())
    }

    /// Seed the very first version (no parent).
    pub fn seed(&self, label: &str, files: Vec<NodeSetFile>) -> Result<i64> {
        let model = crate::merge::merge_files(&files);
        let derivation = derive::recompute(&model, None, None);
        let payload = VersionPayload {
            files,
            events: model.events.clone(),
            model,
            derivation,
            label: label.to_string(),
            edit: serde_json::json!({"kind": "seed"}),
        };
        let id = self.commit_version(None, label, &payload, true)?;
        self.record_event(
            Some(id),
            "VersionCommitted",
            &serde_json::json!({"label": label, "parent": null}),
        )?;
        Ok(id)
    }

    fn build_payload(
        &self,
        base_version: i64,
        label: &str,
        edit: Edit,
    ) -> Result<VersionPayload> {
        let base = self.load_payload(base_version)?;
        let mut files = base.files.clone();
        let mut extra_events: Vec<Event> = Vec::new();
        let edit_json = serde_json::to_value(&edit)?;
        match &edit {
            Edit::ImportFile { name, xml } => {
                files.push(crate::xml::parse_nodeset(name, xml)?);
            }
            Edit::ReplaceFile { name, xml } => {
                let pos = files
                    .iter()
                    .position(|f| &f.name == name)
                    .ok_or_else(|| anyhow!("file {name} not imported"))?;
                files[pos] = crate::xml::parse_nodeset(name, xml)?;
            }
            Edit::RemoveFile { name } => {
                files.retain(|f| &f.name != name);
            }
            Edit::Decision { action, subject, note } => {
                extra_events.push(Event {
                    seq: 0,
                    kind: "Decision",
                    message: format!("decision '{action}' on {subject}: {note}"),
                    subjects: vec![subject.clone()],
                    data: serde_json::json!({
                        "action": action, "subject": subject, "note": note,
                    }),
                });
            }
        }
        let model = crate::merge::merge_files(&files);
        let derivation = derive::recompute(&model, Some(&base.derivation), Some(&base.model));
        let mut events = model.events.clone();
        let offset = events.len();
        for (i, mut e) in extra_events.into_iter().enumerate() {
            e.seq = offset + i;
            events.push(e);
        }
        Ok(VersionPayload {
            files,
            model,
            derivation,
            label: label.to_string(),
            edit: edit_json,
            events,
        })
    }

    /// Apply an edit on top of `base_version`. If the current pointer has
    /// moved past `base_version`, the edit becomes a sibling branch and a
    /// resolvable conflict is returned; the unchosen candidate is kept.
    pub fn apply_edit(&self, base_version: i64, label: &str, edit: Edit) -> Result<SaveOutcome> {
        let payload = self.build_payload(base_version, label, edit.clone())?;
        let current = self.current_pointer()?;
        if current == Some(base_version) || current.is_none() {
            let id = self.commit_version(Some(base_version), label, &payload, true)?;
            self.record_event(
                Some(id),
                "VersionCommitted",
                &serde_json::json!({"label": label, "parent": base_version}),
            )?;
            Ok(SaveOutcome::Current { version_id: id })
        } else {
            let branch = self.commit_version(Some(base_version), label, &payload, false)?;
            self.record_event(
                Some(branch),
                "EditConflict",
                &serde_json::json!({
                    "base": base_version,
                    "current": current,
                    "branch": branch,
                    "resolution": "pending; both candidates kept",
                }),
            )?;
            Ok(SaveOutcome::Conflict {
                current_version: current.unwrap(),
                branch_version: branch,
            })
        }
    }

    /// Apply an edit as a side branch without moving the pointer (used by the
    /// counter-example archive: every variant changes exactly one source).
    pub fn apply_edit_as_branch(
        &self,
        base_version: i64,
        label: &str,
        edit: Edit,
    ) -> Result<i64> {
        let payload = self.build_payload(base_version, label, edit)?;
        let id = self.commit_version(Some(base_version), label, &payload, false)?;
        self.record_event(
            Some(id),
            "BranchCommitted",
            &serde_json::json!({"label": label, "parent": base_version}),
        )?;
        Ok(id)
    }

    /// Choose which branch continues. The unchosen candidate stays in the
    /// version table and remains fully loadable.
    pub fn resolve_branch(&self, chosen_version: i64, note: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO pointer(id,version_id) VALUES(1,?1)
             ON CONFLICT(id) DO UPDATE SET version_id=excluded.version_id",
            [chosen_version],
        )?;
        tx.commit()?;
        self.record_event(
            Some(chosen_version),
            "BranchResolved",
            &serde_json::json!({"chosen": chosen_version, "note": note}),
        )?;
        Ok(())
    }

    /// Replay: move the pointer back to a historical version. The version's
    /// stored conclusions are restored as-is, never recomputed over history.
    pub fn replay_to(&self, version_id: i64) -> Result<()> {
        self.load_payload(version_id)?; // must be complete
        self.resolve_branch(version_id, "replay")
    }

    pub fn export(&self) -> Result<serde_json::Value> {
        let versions = self.list_versions()?;
        let mut payloads = Vec::new();
        for v in &versions {
            let p: Option<String> = self
                .conn
                .query_row("SELECT payload FROM versions WHERE id=?1", [v.id], |r| {
                    r.get(0)
                })
                .optional()?
                .flatten();
            payloads.push(serde_json::json!({
                "id": v.id, "parent": v.parent, "label": v.label,
                "status": v.status, "created_at": v.created_at,
                "payload": p.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
            }));
        }
        let mut decisions = Vec::new();
        let mut stmt = self
            .conn
            .prepare("SELECT id,version_id,seq,kind,payload,created_at FROM decisions ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "version_id": r.get::<_, Option<i64>>(1)?,
                "seq": r.get::<_, i64>(2)?,
                "kind": r.get::<_, String>(3)?,
                "payload": serde_json::from_str::<serde_json::Value>(
                    &r.get::<_, String>(4)?
                ).unwrap_or(serde_json::Value::Null),
                "created_at": r.get::<_, String>(5)?,
            }))
        })?;
        for r in rows.flatten() {
            decisions.push(r);
        }
        Ok(serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "pointer": self.current_pointer()?,
            "versions": payloads,
            "decisions": decisions,
        }))
    }

    pub fn import(&self, dump: &serde_json::Value) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(
            "DELETE FROM decisions; DELETE FROM pointer; DELETE FROM versions;",
        )?;
        for v in dump["versions"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO versions(id,parent,label,status,created_at,payload)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    v["id"].as_i64(),
                    v["parent"].as_i64(),
                    v["label"].as_str().unwrap_or(""),
                    v["status"].as_str().unwrap_or("complete"),
                    v["created_at"].as_str().unwrap_or(""),
                    v["payload"].as_object().map(|_| v["payload"].to_string()),
                ],
            )?;
        }
        if let Some(p) = dump["pointer"].as_i64() {
            tx.execute(
                "INSERT INTO pointer(id,version_id) VALUES(1,?1)
                 ON CONFLICT(id) DO UPDATE SET version_id=excluded.version_id",
                [p],
            )?;
        }
        for d in dump["decisions"].as_array().cloned().unwrap_or_default() {
            tx.execute(
                "INSERT INTO decisions(id,version_id,seq,kind,payload,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    d["id"].as_i64(),
                    d["version_id"].as_i64(),
                    d["seq"].as_i64(),
                    d["kind"].as_str().unwrap_or(""),
                    d["payload"].to_string(),
                    d["created_at"].as_str().unwrap_or(""),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
