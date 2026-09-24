//! Bundled offline case package:
//! "命名空间罗盘：OPC UA NodeSet 合并、引用闭包与模型差异".
//!
//! The replay is deterministic and uses only repository fixtures. Each step is
//! one derivation; decisions keep rejected candidates, so reviewers can trace
//! any final difference back to the source file, ruleset version and decision
//! that produced it.

use crate::derive::{Decision, EventRecord, VersionParams};
use crate::id::stable_id;
use crate::merge::MergedModel;
use crate::model::RawModel;
use crate::parser::parse_nodeset;

pub const BASE_CORE: &str = include_str!("../fixtures/compass/baseline/company-core.xml");
pub const BASE_VENDOR: &str = include_str!("../fixtures/compass/baseline/vendor-compressor.xml");
pub const REORDERED: &str = include_str!("../fixtures/compass/changes/vendor-compressor-reordered.xml");
pub const ALIAS_CYCLE: &str = include_str!("../fixtures/compass/changes/alias-cycle.xml");
pub const DUP_INVERSE: &str = include_str!("../fixtures/compass/changes/duplicate-and-inverse.xml");
pub const CONFLICTS: &str = include_str!("../fixtures/compass/changes/conflicts.xml");
pub const CORE_CHANGED: &str = include_str!("../fixtures/compass/changes/core-local-change.xml");
pub const VENDOR_PARAMS: &str = include_str!("../fixtures/compass/changes/vendor-parameters.xml");

pub struct BuiltVersion {
    pub version: crate::derive::Version,
    pub raw: RawModel,
    pub merged: MergedModel,
    pub changed_sources: Vec<String>,
    pub changed_parameters: Vec<String>,
    pub prior: Option<MergedModel>,
}

pub struct StepSpec {
    pub branch: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub replay_step: Option<i64>,
    pub params: VersionParams,
    /// Files included in this derivation: (source_id, path, xml).
    pub files: &'static [(&'static str, &'static str, &'static str)],
    pub changed_sources: &'static [&'static str],
    pub changed_parameters: &'static [&'static str],
    pub decisions: &'static [DecisionSpec],
    pub note: &'static str,
}

#[derive(Clone)]
pub struct DecisionSpec {
    pub subject: &'static str,
    pub adopted: &'static str,
    pub rejected: &'static [&'static str],
    pub reason: &'static str,
}

pub fn baseline_files() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        ("company-core", "baseline/company-core.xml", BASE_CORE),
        ("vendor-compressor", "baseline/vendor-compressor.xml", BASE_VENDOR),
    ]
}

/// Step 0 baseline, then the narrative revisions.
pub fn main_branch_steps() -> Vec<StepSpec> {
    vec![
        StepSpec {
            branch: "main",
            label: "v1 baseline import",
            kind: "import",
            replay_step: Some(0),
            params: VersionParams::default(),
            files: baseline_files(),
            changed_sources: &[],
            changed_parameters: &[],
            decisions: &[],
            note: "Import both NodeSets and verify all raw material is retained.",
        },
        StepSpec {
            branch: "main",
            label: "v2 parameter revision (pressure kPa / vendor 2.2)",
            kind: "parameter_change",
            replay_step: Some(1),
            params: VersionParams { pressure_unit: "Pa".into(), strict_dependencies: false },
            files: &[
                ("company-core", "baseline/company-core.xml", BASE_CORE),
                ("vendor-parameters", "changes/vendor-parameters.xml", VENDOR_PARAMS),
            ],
            changed_sources: &["vendor-parameters"],
            changed_parameters: &["pressure_unit"],
            decisions: &[],
            note: "One parameter/source revision: normalization closure only.",
        },
        StepSpec {
            branch: "main",
            label: "v3 local dependency change (core 1.1.0, PumpType removed)",
            kind: "source_change",
            replay_step: Some(2),
            params: VersionParams::default(),
            files: &[
                ("company-core-110", "changes/core-local-change.xml", CORE_CHANGED),
                ("vendor-compressor", "baseline/vendor-compressor.xml", BASE_VENDOR),
            ],
            changed_sources: &["company-core-110"],
            changed_parameters: &[],
            decisions: &[],
            note: "Local source change: new property plus deletion-induced dangling refs.",
        },
    ]
}

pub fn branch_steps() -> Vec<StepSpec> {
    vec![
        // Forked from baseline; exercises every boundary condition.
        StepSpec {
            branch: "boundary",
            label: "b1 namespace reorder + alias cycle probe",
            kind: "source_change",
            replay_step: Some(10),
            params: VersionParams::default(),
            files: &[
                ("vendor-reordered", "changes/vendor-compressor-reordered.xml", REORDERED),
                ("alias-cycle", "changes/alias-cycle.xml", ALIAS_CYCLE),
            ],
            changed_sources: &["vendor-reordered", "alias-cycle"],
            changed_parameters: &[],
            decisions: &[],
            note: "Same URIs, reversed numeric indices; alias A<->B cycle retained.",
        },
        StepSpec {
            branch: "boundary",
            label: "b2 duplicate NodeId + inverse reference + unknown extension",
            kind: "source_change",
            replay_step: Some(11),
            params: VersionParams::default(),
            files: &[
                ("vendor-compressor", "baseline/vendor-compressor.xml", BASE_VENDOR),
                ("dup-inverse", "changes/duplicate-and-inverse.xml", DUP_INVERSE),
            ],
            changed_sources: &["dup-inverse"],
            changed_parameters: &[],
            decisions: &[],
            note: "Duplicate CMP-101 candidate; inverse edge normalized without new edge.",
        },
        StepSpec {
            branch: "boundary",
            label: "b3 dependency conflicts + missing package + dangling target",
            kind: "source_change",
            replay_step: Some(12),
            params: VersionParams::default(),
            files: &[
                ("company-core-110", "changes/core-local-change.xml", CORE_CHANGED),
                ("conflicts", "changes/conflicts.xml", CONFLICTS),
            ],
            changed_sources: &["conflicts"],
            changed_parameters: &[],
            decisions: &[DecisionSpec {
                subject: "urn:compass:industrial:core version",
                adopted: "require core 2.0.0",
                rejected: &["require core 1.1.0"],
                reason: "Manual conflicting decision: choose 2.0.0 but keep 1.1.0 candidate.",
            }],
            note: "Version conflict decided explicitly; unresolved safety node retained.",
        },
    ]
}

pub fn build_raw(files: &[(&str, &str, &str)]) -> RawModel {
    let mut raw = RawModel::default();
    for (source_id, path, xml) in files {
        match parse_nodeset(source_id, path, xml) {
            Ok(ns) => raw.sources.push(ns),
            Err(e) => raw.parse_diagnostics.push(crate::model::Diagnostic {
                code: "PARSE_ERROR".into(),
                message: e.to_string(),
                span: None,
            }),
        }
    }
    raw
}

pub fn record_for(seq: u64, kind: &str, code: Option<&str>, pairs: &[(&str, &str)]) -> EventRecord {
    crate::engine::event_record(seq, kind, code, pairs)
}

pub fn decision_for(seq: u64, based_on: &str, spec: &DecisionSpec) -> Decision {
    Decision {
        decision_id: stable_id("dec", &[spec.subject, spec.adopted, &seq.to_string()]),
        subject: spec.subject.to_string(),
        adopted: spec.adopted.to_string(),
        rejected: spec.rejected.iter().map(|s| s.to_string()).collect(),
        reason: spec.reason.to_string(),
        based_on: based_on.to_string(),
    }
}
