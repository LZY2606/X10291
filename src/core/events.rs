//! Machine-readable judgment events.
//!
//! Every semantic ruling (including the three governing principles) is
//! emitted as an `Event` so the UI and the replay archive can show *why* a
//! value is the way it is. Events are append-only per derivation version.

use serde::{Deserialize, Serialize};

use crate::core::model::Span;

/// Stable codes for the governing semantic principles.
pub mod principle {
    pub const NS_INDEX_LOCAL: &str = "PRINCIPLE:NS_INDEX_LOCAL_ONLY";
    pub const INVERSE_NORMALIZE_NO_MISSING_EDGE: &str =
        "PRINCIPLE:INVERSE_NORMALIZE_NO_MISSING_EDGE";
    pub const MISSING_DEP_KEEPS_UNRESOLVED: &str = "PRINCIPLE:MISSING_DEP_KEEPS_UNRESOLVED";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    /// Input / finding recorded.
    Observed,
    /// Ruling applied and adopted into the canonical graph.
    Adopted,
    /// Candidate considered but not chosen (kept, never destroyed).
    Rejected,
    /// Cannot be decided uniquely; all legal candidates retained.
    Ambiguous,
    /// Derived item was recomputed because an input changed.
    Recomputed,
    /// Derived item is still valid after a change propagation.
    StillValid,
    /// Awaiting a human decision.
    Pending,
    /// Human chose this candidate.
    Chosen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    pub source_sha: String,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic, stable within a derivation.
    pub seq: u32,
    pub code: String,
    pub status: EventStatus,
    pub severity: Severity,
    pub message: String,
    /// Stable identity of the object most affected (node id / edge id / dep).
    pub subject: Option<String>,
    pub principle: Option<String>,
    /// Legal candidates when `status` is ambiguous/rejected/chosen.
    pub candidates: Vec<String>,
    pub refs: Vec<EventRef>,
}

#[derive(Default)]
pub struct EventLog {
    events: Vec<Event>,
}

impl EventLog {
    pub fn new() -> Self {
        EventLog { events: Vec::new() }
    }
    pub fn emit(
        &mut self,
        code: &str,
        status: EventStatus,
        severity: Severity,
        message: impl Into<String>,
        subject: Option<String>,
        principle: Option<&str>,
        candidates: Vec<String>,
        refs: Vec<EventRef>,
    ) -> &Event {
        let seq = self.events.len() as u32;
        self.events.push(Event {
            seq,
            code: code.to_string(),
            status,
            severity,
            message: message.into(),
            subject,
            principle: principle.map(|s| s.to_string()),
            candidates,
            refs,
        });
        self.events.last().unwrap()
    }

    pub fn into_vec(self) -> Vec<Event> {
        self.events
    }
    pub fn as_slice(&self) -> &[Event] {
        &self.events
    }
}
