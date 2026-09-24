use crate::db::{Database, StoreError};
use crate::model::*;
use std::path::{Path, PathBuf};

pub fn fixtures_dir() -> PathBuf {
    if let Ok(path) = std::env::var("NAMESPACE_COMPASS_FIXTURES") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/namespace-compass")
}

pub fn read_input(dir: &str, files: &[&str]) -> Vec<SourceInput> {
    let root = fixtures_dir().join(dir);
    files
        .iter()
        .map(|file| SourceInput {
            file_name: (*file).to_string(),
            content: std::fs::read_to_string(root.join(file)).expect("fixture NodeSet XML exists"),
        })
        .collect()
}

pub fn seed_demo(database: &Database) -> Result<Vec<i64>, StoreError> {
    if !database.versions()?.is_empty() {
        return Ok(Vec::new());
    }
    let baseline = database.import_snapshot(&ImportRequest {
        label: "baseline: three-file industrial NodeSet".into(),
        sources: read_input("v1", &["shared.xml", "core.xml", "line.xml"]),
        parameters: Some(RuleParameters::default()),
        expected_version: None,
        branch_id: Some("main".into()),
    })?;
    let parameter_version = database.decide(&DecisionRequest {
        expected_version: baseline.record.version_id,
        decision: "parameter_adjusted".into(),
        candidate_fact_id: "pressure-normalization-policy".into(),
        label: Some("parameter revision: report pressure in kPa".into()),
        parameters: Some(RuleParameters { rule_version: "rules-2026.09.2".into(), pressure_unit: "kPa".into() }),
    })?;
    let local_change = database.import_snapshot(&ImportRequest {
        label: "local source revision: line dependency and deletion".into(),
        sources: read_input("v2", &["shared.xml", "core.xml", "line.xml"]),
        parameters: Some(parameter_version.record.parameters.clone()),
        expected_version: Some(parameter_version.record.version_id),
        branch_id: Some("main".into()),
    })?;
    database.import_snapshot(&ImportRequest {
        label: "edge replay branch: all seven boundary conditions".into(),
        sources: read_input("v3", &["edge-core.xml", "edge-line.xml"]),
        parameters: Some(RuleParameters { rule_version: "rules-2026.09.2".into(), pressure_unit: "kPa".into() }),
        expected_version: None,
        branch_id: Some("edge".into()),
    })?;
    Ok(vec![baseline.record.version_id, parameter_version.record.version_id, local_change.record.version_id])
}

pub fn fixture_path(path: &str) -> PathBuf {
    Path::new(&fixtures_dir()).join(path)
}
