//! Machine-readable judgment records. Every rule the engine applies emits one of
//! these with its input fragment, judgment status and affected objects, so a
//! reviewer can trace which material, which rule version and which decision
//! produced any difference.

use serde::{Deserialize, Serialize};

/// Rule identifiers (stable; treated as the "rule version" surface).
pub mod rules {
    pub const NS_INDEX_LOCAL_ONLY: &str = "ns_index_local_only";
    pub const MERGE_REMAP_BY_URI: &str = "merge_remap_by_uri";
    pub const ALIAS_CYCLE_DETECTED: &str = "alias_cycle_detected";
    pub const DUPLICATE_NODEID: &str = "duplicate_nodeid";
    pub const INVERSE_REF_NORMALIZED: &str = "inverse_ref_normalized";
    pub const NO_SYNTHETIC_EDGES: &str = "no_synthetic_edges";
    pub const DEP_VERSION_CONFLICT: &str = "dep_version_conflict";
    pub const UNRESOLVED_DEP_RETAINED: &str = "unresolved_dep_retained";
    pub const UNKNOWN_EXTENSION_PRESERVED: &str = "unknown_extension_preserved";
    pub const DANGLING_REF_AFTER_DELETE: &str = "dangling_ref_after_delete";
    pub const UNIT_CONVERTED: &str = "unit_converted";
    pub const UNIT_UNCONVERTIBLE_KEPT: &str = "unit_unconvertible_kept";
    pub const PARAM_REVISION_APPLIED: &str = "param_revision_applied";
    pub const DECISION_RECORDED: &str = "decision_recorded";
    pub const CONFLICT_DETECTED: &str = "conflict_detected";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Judgment {
    pub stable_id: String,
    /// Rule identifier, see `rules`.
    pub rule: String,
    /// e.g. "applied" | "conflict" | "unresolved" | "preserved" | "diagnostic"
    pub status: String,
    /// Input fragment the judgment was made from (JSON).
    pub inputs: serde_json::Value,
    /// Stable ids of affected objects.
    pub affected: Vec<String>,
    pub detail: String,
}

impl Judgment {
    pub fn new(
        rule: &str,
        status: &str,
        inputs: serde_json::Value,
        affected: Vec<String>,
        detail: &str,
    ) -> Self {
        let canonical = format!("{}|{}|{}|{}", rule, status, inputs, detail);
        Judgment {
            stable_id: crate::ids::stable_id("judg", &canonical),
            rule: rule.to_string(),
            status: status.to_string(),
            inputs,
            affected,
            detail: detail.to_string(),
        }
    }
}
