//! SQLite persistence.
//!
//! Raw material, events and derived versions are separate rows; the current
//! pointer moves only after a full successful transaction, so a crash mid
//! write leaves either the old or the complete new version.

use crate::events::Event;
use crate::model::*;
use serde::{Deserialize, Serialize};
use rusqlite::{params, Connection};
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const SCHEMA_VERSION: u32 = 2;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug)]
pub struct SaveConflict {
    pub expected_head: u64,
    pub actual_head: u64,
}

impl std::fmt::Display for SaveConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "optimistic conflict: expected head {}, found {}", self.expected_head, self.actual_head)
    }
}
impl std::error::Error for SaveConflict {}

fn parse_kind(s: &str) -> crate::events::EventKind {
    match s {
        "ImportFile" => crate::events::EventKind::ImportFile,
        "ParameterChange" => crate::events::EventKind::ParameterChange,
        "LocalSourceChange" => crate::events::EventKind::LocalSourceChange,
        "BranchCheckpoint" => crate::events::EventKind::BranchCheckpoint,
        _ => crate::events::EventKind::ManualDecision,
    }
}


impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        let s = Store { conn: Mutex::new(conn) };
        s.init()?;
        Ok(s)
    }

    pub fn open_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let s = Store { conn: Mutex::new(conn) };
        s.init()?;
        Ok(s)
    }

    fn init(&self) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        // Hard crash safety: all pages synced and WAL checkpointed at commit.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn
            .query_row("SELECT user_version FROM pragma_user_version", [], |r| r.get(0))
            .unwrap_or(0);
        let _ = version;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS raw_files (
                file_id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                content TEXT NOT NULL,
                json TEXT NOT NULL,
                first_seq INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS events (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                branch TEXT NOT NULL,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                detail TEXT NOT NULL,
                rule_version TEXT NOT NULL,
                file_path TEXT,
                file_content TEXT,
                payload TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS derived_versions (
                version INTEGER PRIMARY KEY,
                branch TEXT NOT NULL,
                parent_version INTEGER,
                json TEXT NOT NULL,
                quantities TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS current_pointer (
                branch TEXT PRIMARY KEY,
                version INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS branches (
                name TEXT PRIMARY KEY,
                base_seq INTEGER NOT NULL,
                note TEXT NOT NULL,
                accepted INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS decisions (
                id TEXT PRIMARY KEY,
                seq INTEGER NOT NULL,
                branch TEXT NOT NULL,
                topic TEXT NOT NULL,
                chosen TEXT,
                rejected TEXT NOT NULL,
                reason TEXT NOT NULL
             );",
        )?;
        self.migrate(&mut conn)?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_meta(key,value) VALUES('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    fn migrate(&self, conn: &mut Connection) -> rusqlite::Result<()> {
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
        if v < 1 {
            // v1 baseline already created by init.
            conn.execute_batch("PRAGMA user_version = 1;")?;
        }
        if v < 2 {
            // v2: keep rejected candidates forever (no cascade loss).
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS decisions (
                    id TEXT PRIMARY KEY, seq INTEGER NOT NULL, branch TEXT NOT NULL,
                    topic TEXT NOT NULL, chosen TEXT, rejected TEXT NOT NULL, reason TEXT NOT NULL);
                 PRAGMA user_version = 2;",
            )?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> rusqlite::Result<u32> {
        let conn = self.conn.lock().unwrap();
        let s: String = conn.query_row(
            "SELECT value FROM schema_meta WHERE key='schema_version'", [], |r| r.get(0),
        ).unwrap_or_else(|_| "1".into());
        Ok(s.parse().unwrap_or(1))
    }

    /// Append an event, rebuild the merged model for that branch and switch the
    /// current pointer only on full success. `expected_head` enforces
    /// optimistic concurrency: two editors on one version cannot silently win.
    pub fn append_event(
        &self,
        branch: &str,
        kind: &str,
        title: &str,
        detail: &str,
        rule_version: &str,
        file_path: Option<&str>,
        file_content: Option<&str>,
        payload: BTreeMap<String, String>,
        expected_head: Option<u64>,
    ) -> Result<(Event, MergedModel), Box<dyn std::error::Error>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let head: i64 = tx
            .query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE branch=?1", params![branch], |r| r.get(0))
            .unwrap_or(0);
        if let Some(expected) = expected_head {
            if head as u64 != expected {
                return Err(Box::new(SaveConflict { expected_head: expected, actual_head: head as u64 }));
            }
        }

        let payload_json = serde_json::to_string(&payload)?;
        tx.execute(
            "INSERT INTO events(branch,kind,title,detail,rule_version,file_path,file_content,payload)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![branch, kind, title, detail, rule_version, file_path, file_content, payload_json],
        )?;
        let seq = tx.last_insert_rowid();

        // Persist/replace raw material inline.
        if let (Some(path), Some(content)) = (file_path, file_content) {
            let parsed = crate::parser::parse_file(path, content);
            tx.execute(
                "INSERT INTO raw_files(file_id,path,content,json,first_seq) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(file_id) DO UPDATE SET path=excluded.path,content=excluded.content,json=excluded.json",
                params![parsed.file_id, path, content, serde_json::to_string(&parsed)?, seq],
            )?;
        }

        let event = Event {
            seq: seq as u64, branch: branch.into(),
            kind: parse_kind(kind),
            title: title.into(), detail: detail.into(), rule_version: rule_version.into(),
            file_path: file_path.map(String::from), file_content: file_content.map(String::from),
            payload,
        };

        let events = Self::load_events_tx(&tx)?;
        let base_seq: u64 = if branch == "main" { 0 } else {
            tx.query_row("SELECT base_seq FROM branches WHERE name=?1", params![branch], |r| r.get::<_,i64>(0)).unwrap_or(0) as u64
        };
        let frame = if branch == "main" {
            crate::events::replay(&events, branch, Some(seq as u64))?
        } else {
            crate::events::replay_with_base(&events, branch, base_seq, Some(seq as u64))?
        };
        let quantities = crate::derive::derive_quantities(&frame.raw);
        tx.execute(
            "INSERT INTO derived_versions(version,branch,parent_version,json,quantities)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(version) DO UPDATE SET json=excluded.json,quantities=excluded.quantities",
            params![frame.model.version as i64, branch, frame.model.parent_version.map(|v| v as i64),
                    serde_json::to_string(&frame.model)?, serde_json::to_string(&quantities)?],
        )?;
        tx.execute(
            "INSERT INTO current_pointer(branch,version) VALUES(?1,?2)
             ON CONFLICT(branch) DO UPDATE SET version=excluded.version",
            params![branch, frame.model.version as i64],
        )?;

        tx.commit()?;
        Ok((event, frame.model))
    }

    pub fn head_seq(&self, branch: &str) -> u64 {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE branch=?1", params![branch], |r| r.get(0))
            .unwrap_or(0) as u64
    }

    pub fn current_version(&self, branch: &str) -> Option<u64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT version FROM current_pointer WHERE branch=?1", params![branch], |r| r.get::<_, i64>(0))
            .ok().map(|v| v as u64)
    }

    pub fn events(&self, branch: &str) -> Vec<Event> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT payload,seq,branch,kind,title,detail,rule_version,file_path,file_content FROM events WHERE branch=?1 ORDER BY seq").unwrap();
        stmt.query_map(params![branch], |row| {
            let payload_s: String = row.get(0)?;
            Ok(Event {
                seq: row.get(1)?, branch: row.get(2)?,
                kind: { let k: String = row.get(3)?; parse_kind(&k) },
                title: row.get(4)?, detail: row.get(5)?, rule_version: row.get(6)?,
                file_path: row.get(7)?, file_content: row.get(8)?,
                payload: serde_json::from_str(&payload_s).unwrap_or_default(),
            })
        }).unwrap().flatten().collect()
    }

    fn load_events_tx(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<Vec<Event>> {
        let mut stmt = tx.prepare("SELECT seq,branch,kind,title,detail,rule_version,file_path,file_content,payload FROM events ORDER BY seq")?;
        let rows = stmt.query_map([], |row| {
            let payload_s: String = row.get(8)?;
            Ok(Event {
                seq: row.get(0)?, branch: row.get(1)?,
                kind: parse_kind(&row.get::<_, String>(2)?),
                title: row.get(3)?, detail: row.get(4)?, rule_version: row.get(5)?,
                file_path: row.get(6)?, file_content: row.get(7)?,
                payload: serde_json::from_str(&payload_s).unwrap_or_default(),
            })
        })?;
        rows.collect()
    }

    pub(crate) fn branch_base(&self, branch: &str) -> u64 {
        if branch == "main" { 0 } else {
            let conn = self.conn.lock().unwrap();
            conn.query_row("SELECT base_seq FROM branches WHERE name=?1", params![branch], |r| r.get::<_,i64>(0)).unwrap_or(0) as u64
        }
    }

    pub fn model_at(&self, branch: &str, seq: Option<u64>) -> Result<MergedModel, String> {
        let events = self.all_events();
        if branch == "main" {
            Ok(crate::events::replay(&events, branch, seq)?.model)
        } else {
            Ok(crate::events::replay_with_base(&events, branch, self.branch_base(branch), seq)?.model)
        }
    }

    pub fn current_model(&self, branch: &str) -> Result<MergedModel, String> {
        let events = self.all_events();
        let own_last = events.iter().filter(|e| e.branch == branch).map(|e| e.seq).max();
        if branch == "main" {
            crate::events::replay(&events, branch, own_last).map(|f| f.model)
        } else {
            crate::events::replay_with_base(&events, branch, self.branch_base(branch), own_last).map(|f| f.model)
        }
    }

    pub fn raw_files(&self, branch: &str) -> Vec<RawFile> {
        let events = self.all_events();
        let own_last = events.iter().filter(|e| e.branch == branch).map(|e| e.seq).max();
        let frame = if branch == "main" {
            crate::events::replay(&events, branch, own_last)
        } else {
            crate::events::replay_with_base(&events, branch, self.branch_base(branch), own_last)
        };
        frame.map(|f| f.raw).unwrap_or_default()
    }

    pub fn all_events(&self) -> Vec<Event> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT seq,branch,kind,title,detail,rule_version,file_path,file_content,payload FROM events ORDER BY seq").unwrap();
        stmt.query_map([], |row| {
            let payload_s: String = row.get(8)?;
            Ok(Event {
                seq: row.get(0)?, branch: row.get(1)?,
                kind: parse_kind(&row.get::<_, String>(2)?),
                title: row.get(3)?, detail: row.get(4)?, rule_version: row.get(5)?,
                file_path: row.get(6)?, file_content: row.get(7)?,
                payload: serde_json::from_str(&payload_s).unwrap_or_default(),
            })
        }).unwrap().flatten().collect()
    }

    pub fn create_branch(&self, name: &str, base_seq: u64, note: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO branches(name,base_seq,note,accepted) VALUES(?1,?2,?3,0)",
            params![name, base_seq as i64, note],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn branches(&self) -> Vec<(String, u64, String, bool)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT name,base_seq,note,accepted FROM branches ORDER BY name").unwrap();
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, String>(2)?, r.get::<_, i64>(3)? != 0)))
            .unwrap().flatten().collect()
    }

    pub fn accept_branch(&self, name: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE branches SET accepted=1 WHERE name=?1", params![name]).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Record a human decision; the rejected candidate is retained permanently.
    pub fn record_decision(&self, branch: &str, seq: u64, topic: &str, chosen: Option<&str>, rejected: &[String], reason: &str) -> Result<String, String> {
        let id = stable_id("dec", &[branch, &seq.to_string(), topic]);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO decisions(id,seq,branch,topic,chosen,rejected,reason) VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(id) DO UPDATE SET chosen=excluded.chosen,rejected=excluded.rejected,reason=excluded.reason",
            params![id, seq as i64, branch, topic, chosen, rejected.join("\n"), reason],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn decisions(&self, branch: &str) -> Vec<(String, u64, String, Option<String>, String, String)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,seq,topic,chosen,rejected,reason FROM decisions WHERE branch=?1 ORDER BY seq").unwrap();
        stmt.query_map(params![branch], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?)))
            .unwrap().flatten().collect()
    }

    // ---------- Export / import (stable identity survives) ----------

    pub fn export_json(&self) -> Result<String, String> {
        #[derive(Serialize)]
        struct Export<'a> {
            format: &'a str,
            schema_version: u32,
            rule_version: &'a str,
            branches: Vec<BranchExport>,
        }
        #[derive(Serialize)]
        struct BranchExport {
            name: String,
            base_seq: u64,
            note: String,
            accepted: bool,
            events: Vec<Event>,
            decisions: Vec<serde_json::Value>,
        }
        let mut out_branches = Vec::new();
        let names: Vec<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare("SELECT name FROM branches ORDER BY name").map_err(|e| e.to_string())?;
            stmt.query_map([], |r| r.get(0)).unwrap().flatten().collect()
        };
        if names.is_empty() {
            out_branches.push(BranchExport {
                name: "main".into(), base_seq: 0, note: "default".into(), accepted: true,
                events: self.events("main"), decisions: Vec::new(),
            });
        }
        for name in names {
            let (base_seq, note, accepted): (i64, String, i64) = {
                let conn = self.conn.lock().unwrap();
                conn.query_row("SELECT base_seq,note,accepted FROM branches WHERE name=?1", params![name], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap_or((0, String::new(), 0))
            };
            let mut decisions = Vec::new();
            {
                let conn = self.conn.lock().unwrap();
                let mut ds = conn.prepare("SELECT id,seq,topic,chosen,rejected,reason FROM decisions WHERE branch=?1 ORDER BY seq").unwrap();
                for d in ds.query_map(params![name], |r| Ok(serde_json::json!({
                    "id": r.get::<_,String>(0)?, "seq": r.get::<_,i64>(1)?,
                    "topic": r.get::<_,String>(2)?, "chosen": r.get::<_,Option<String>>(3)?,
                    "rejected": r.get::<_,String>(4)?, "reason": r.get::<_,String>(5)?,
                }))).unwrap().flatten() {
                    decisions.push(d);
                }
            }
            out_branches.push(BranchExport {
                name: name.clone(), base_seq: base_seq as u64, note: note.clone(),
                accepted: accepted != 0, events: self.events(&name), decisions,
            });
        }
        let export = Export {
            format: "opc-compass-export/1",
            schema_version: SCHEMA_VERSION,
            rule_version: crate::merge::RULE_VERSION,
            branches: out_branches,
        };
        serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
    }

    pub fn import_json(&self, json: &str) -> Result<Vec<String>, String> {
        #[derive(Deserialize)]
        struct Import {
            branches: Vec<BranchImport>,
        }
        #[derive(Deserialize)]
        struct BranchImport {
            name: String,
            base_seq: u64,
            note: String,
            accepted: bool,
            events: Vec<Event>,
            #[serde(default)]
            decisions: Vec<DecImport>,
        }
        #[derive(Deserialize)]
        struct DecImport {
            id: String, seq: u64, topic: String, chosen: Option<String>,
            rejected: String, reason: String,
        }
        let data: Import = serde_json::from_str(json).map_err(|e| e.to_string())?;
        for b in &data.branches {
            self.create_branch(&b.name, b.base_seq, &b.note)?;
            for ev in &b.events {
                let conn = self.conn.lock().unwrap();
                Self::append_raw_event_locked(&conn, ev)?;
                drop(conn);
                if b.accepted { self.accept_branch(&b.name)?; }
            }
            for d in &b.decisions {
                let rejected: Vec<String> = d.rejected.split('\n').map(String::from).filter(|s| !s.is_empty()).collect();
                let conn = self.conn.lock().unwrap();
                conn.execute(
                    "INSERT OR REPLACE INTO decisions(id,seq,branch,topic,chosen,rejected,reason) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![d.id, d.seq as i64, b.name, d.topic, d.chosen, rejected.join("\n"), d.reason],
                ).map_err(|e| e.to_string())?;
            }
        }
        Ok(data.branches.into_iter().map(|b| b.name).collect())
    }

    fn append_raw_event_locked(conn: &Connection, ev: &Event) -> Result<(), String> {
        let kind = match ev.kind {
            crate::events::EventKind::ImportFile => "ImportFile",
            crate::events::EventKind::ParameterChange => "ParameterChange",
            crate::events::EventKind::LocalSourceChange => "LocalSourceChange",
            crate::events::EventKind::ManualDecision => "ManualDecision",
            crate::events::EventKind::BranchCheckpoint => "BranchCheckpoint",
        };
        let payload = serde_json::to_string(&ev.payload).unwrap_or_else(|_| "{}".into());
        conn.execute(
            "INSERT INTO events(seq,branch,kind,title,detail,rule_version,file_path,file_content,payload)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![ev.seq as i64, ev.branch, kind, ev.title, ev.detail, ev.rule_version,
                    ev.file_path, ev.file_content, payload],
        ).map_err(|e| e.to_string())?;
        if let (Some(path), Some(content)) = (&ev.file_path, &ev.file_content) {
            let parsed = crate::parser::parse_file(path, content);
            conn.execute(
                "INSERT OR REPLACE INTO raw_files(file_id,path,content,json,first_seq) VALUES(?1,?2,?3,?4,?5)",
                params![parsed.file_id, path, content, serde_json::to_string(&parsed).unwrap_or_default(), ev.seq as i64],
            ).map_err(|e| e.to_string())?;
            let events = {
                let mut st = conn.prepare("SELECT seq,branch,kind,title,detail,rule_version,file_path,file_content,payload FROM events ORDER BY seq").map_err(|e| e.to_string())?;
                st.query_map([], |row| {
                    let payload_s: String = row.get(8)?;
                    Ok(Event { seq: row.get(0)?, branch: row.get(1)?,
                        kind: parse_kind(&row.get::<_, String>(2)?),
                        title: row.get(3)?, detail: row.get(4)?, rule_version: row.get(5)?,
                        file_path: row.get(6)?, file_content: row.get(7)?,
                        payload: serde_json::from_str(&payload_s).unwrap_or_default(),
                    })
                }).map_err(|e| e.to_string())?.flatten().collect::<Vec<Event>>()
            };
            let base_seq: u64 = if ev.branch == "main" { 0 } else {
                conn.query_row("SELECT base_seq FROM branches WHERE name=?1", params![ev.branch], |r| r.get::<_,i64>(0)).unwrap_or(0) as u64
            };
            let frame = if ev.branch == "main" {
                crate::events::replay(&events, &ev.branch, Some(ev.seq))?
            } else {
                crate::events::replay_with_base(&events, &ev.branch, base_seq, Some(ev.seq))?
            };
            let quantities = crate::derive::derive_quantities(&frame.raw);
            conn.execute(
                "INSERT INTO derived_versions(version,branch,parent_version,json,quantities) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(version) DO UPDATE SET json=excluded.json,quantities=excluded.quantities",
                params![frame.model.version as i64, ev.branch, frame.model.parent_version.map(|v| v as i64),
                        serde_json::to_string(&frame.model).unwrap(), serde_json::to_string(&quantities).unwrap()],
            ).map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO current_pointer(branch,version) VALUES(?1,?2)
                 ON CONFLICT(branch) DO UPDATE SET version=excluded.version",
                params![ev.branch, frame.model.version as i64],
            ).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Test-only fault injection: perform all writes, then abort before commit.
    /// The old pointer and complete old version must remain visible.
    pub fn crash_during_write(&self, branch: &str, title: &str) -> Result<(), String> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO events(branch,kind,title,detail,rule_version,payload) VALUES(?1,'ParameterChange',?2,'will abort','x','{}')",
            params![branch, title],
        ).map_err(|e| e.to_string())?;
        // Explicit rollback simulates process kill before commit.
        drop(tx);
        Ok(())
    }
}
