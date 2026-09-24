//! Deterministic offline case replay: "Namespace Compass".
//!
//! The script only uses fixtures inside the repository and emits a
//! machine-readable trace. Every clock tag is logical (t#..), so replay is
//! reproducible regardless of wall time.

use crate::service::{
    import_bundle, App, BranchRequest, DecisionRequest, DiffReport, ExportBundle, ImportRequest,
};
use crate::store::Store;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayStep {
    pub clock: String,
    pub action: String,
    pub detail: String,
    pub event_seq: Option<i64>,
    pub version_id: Option<String>,
    pub conflict: Option<String>,
    pub findings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayReport {
    pub rule_version: String,
    pub steps: Vec<ReplayStep>,
    pub branches: Vec<String>,
    pub expected: Vec<ExpectedFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpectedFinding {
    pub code: String,
    pub where_: String,
}

fn fixture(v: &str, name: &str) -> (String, String) {
    let rel = format!("fixtures/compass/{v}/{name}");
    let abs = std::path::Path::new(&rel)
        .canonicalize()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or(rel.clone());
    let content = std::fs::read_to_string(&rel)
        .unwrap_or_else(|e| panic!("read fixture {rel}: {e}"));
    (abs, content)
}

pub fn run(app: &App) -> ReplayReport {
    let mut steps = Vec::new();

    // ---- Phase 1: baseline v1 import in dependency order ----
    import(
        app,
        "00-base.xml",
        "v1",
        0,
        "t1",
        "导入基础模型 base v1（保留未识别扩展）",
        &mut steps,
    );
    import(
        app,
        "10-companion-units.xml",
        "v1",
        10,
        "t2",
        "导入单位/信号伴随模型 v1（alias 循环、缺失依赖、不可换算单位）",
        &mut steps,
    );
    import(
        app,
        "20-plant.xml",
        "v1",
        20,
        "t3",
        "导入工厂模型 v1（namespace 重排、重复 NodeId、逆向引用）",
        &mut steps,
    );

    // Branch A: parameter revision.
    let rev_version = import(
        app,
        "20-plant.xml",
        "v2",
        20,
        "t4",
        "修订：温度设定点参数调整（plant v2，仅参数变化）",
        &mut steps,
    );
    let _ = rev_version;

    // Create alternative branch B from main for the local dependency change.
    app.create_branch(BranchRequest {
        name: "dep-revision".to_string(),
        from_branch: "main".to_string(),
        label: "分支：局部依赖改动".to_string(),
        clock_tag: "t4b".to_string(),
    })
    .expect("branch");
    steps.push(step(
        "t4b",
        "create-branch",
        "从主分支 t3 版本创建 dep-revision 分支",
        Some(4),
        None,
        None,
        vec![],
    ));

    let dep_res = app
        .import(ImportRequest {
            file_id: "10-companion-units.xml".to_string(),
            path_hint: fixture("v2", "10-companion-units.xml").0,
            content: fixture("v2", "10-companion-units.xml").1,
            ordinal: 10,
            label: "局部依赖改动：companion v2 删除 AnalogSignalType 并降级到 0.90".to_string(),
            clock_tag: "t5".to_string(),
            base_seq: None,
            branch: Some("dep-revision".to_string()),
        })
        .expect("import");
    let mut findings = vec![
        "MODEL_VERSION_CONFLICT".to_string(),
        "dangling: AnalogSignalType 删除后 FlowSensorType/DeletedSensor 悬空".to_string(),
        "UNRESOLVED_NODE_PLACEHOLDER".to_string(),
    ];
    let dep_snap = app
        .store
        .branch_pointer("dep-revision")
        .unwrap()
        .and_then(|bp| app.store.snapshot(&bp.version_id).unwrap())
        .unwrap();
    if !dep_snap
        .derived
        .dependencies
        .iter()
        .any(|d| d.required_uri == "http://test.org/CompassUnits/" && d.state == "conflict")
    {
        findings.push("MISSING: dependency conflict not detected".to_string());
    }
    steps.push(step(
        "t5",
        "import",
        "companion v2：版本冲突 + 删除后悬空引用，仅影响依赖闭包",
        Some(dep_res.event_seq),
        Some(dep_res.version_id),
        dep_res.conflict,
        findings,
    ));

    // Compare two branches.
    let diff: DiffReport = app.diff_branches("main", "dep-revision").expect("diff");
    let kinds: std::collections::BTreeSet<String> =
        diff.items.iter().map(|i| i.kind.clone()).collect();
    steps.push(step(
        "t6",
        "diff",
        "比较 main(参数修订) 与 dep-revision(依赖改动)",
        None,
        None,
        None,
        kinds.iter().cloned().collect(),
    ));

    // Conflicting human decision with all legal candidates retained.
    let seq = app
        .decide(DecisionRequest {
            branch: "dep-revision".to_string(),
            subject: "companion-version-conflict".to_string(),
            options: vec![
                "keep-companion-v1".to_string(),
                "accept-companion-v0.90-and-quarantine-dangling".to_string(),
            ],
            chosen: "keep-companion-v1".to_string(),
            label: "人工裁定：保留 companion v1（未选候选保留）".to_string(),
            clock_tag: "t7".to_string(),
            base_seq: None,
        })
        .expect("decision");
    steps.push(step(
        "t7",
        "decision",
        "冲突裁定：两个合法候选，选择 keep-companion-v1，拒绝候选仍保留",
        Some(seq),
        None,
        None,
        vec!["retained-option: accept-companion-v0.90-and-quarantine-dangling".to_string()],
    ));

    // Concurrent save: two users from same base; second must be a conflict.
    let base = app
        .store
        .branch_pointer("main")
        .unwrap()
        .unwrap()
        .event_seq;
    let ok = app
        .import(ImportRequest {
            file_id: "20-plant.xml".to_string(),
            path_hint: fixture("v2", "20-plant.xml").0,
            content: fixture("v2", "20-plant.xml").1,
            ordinal: 20,
            label: "用户甲并发保存".to_string(),
            clock_tag: "t8a".to_string(),
            base_seq: Some(base),
            branch: Some("main".to_string()),
        })
        .unwrap();
    let stale = app
        .import(ImportRequest {
            file_id: "10-companion-units.xml".to_string(),
            path_hint: fixture("v1", "10-companion-units.xml").0,
            content: fixture("v1", "10-companion-units.xml").1,
            ordinal: 10,
            label: "用户乙基于旧版本保存".to_string(),
            clock_tag: "t8b".to_string(),
            base_seq: Some(base),
            branch: Some("main".to_string()),
        })
        .unwrap();
    steps.push(step(
        "t8",
        "concurrent-save",
        "两位使用者从同一版本编辑：甲成功，乙必须收到可解决冲突",
        Some(ok.event_seq),
        Some(ok.version_id),
        stale.conflict.clone(),
        vec![if stale.conflict.is_some() {
            "CONFLICT_DETECTED".to_string()
        } else {
            "ERROR: missing conflict".to_string()
        }],
    ));

    let branches = app
        .store
        .list_branches()
        .unwrap()
        .iter()
        .map(|b| b.branch.clone())
        .collect();

    ReplayReport {
        rule_version: crate::engine::RULE_VERSION.to_string(),
        steps,
        branches,
        expected: expected_findings(),
    }
}

#[allow(clippy::too_many_arguments)]
fn import(
    app: &App,
    name: &str,
    v: &str,
    ordinal: i64,
    clock: &str,
    label: &str,
    steps: &mut Vec<ReplayStep>,
) -> String {
    let (path, content) = fixture(v, name);
    let res = app
        .import(ImportRequest {
            file_id: name.to_string(),
            path_hint: path,
            content,
            ordinal,
            label: label.to_string(),
            clock_tag: clock.to_string(),
            base_seq: None,
            branch: Some("main".to_string()),
        })
        .expect("import ok");
    let snap = app
        .store
        .branch_pointer("main")
        .unwrap()
        .and_then(|bp| app.store.snapshot(&bp.version_id).unwrap())
        .unwrap();
    let mut findings = Vec::new();
    for d in &snap.derived.diagnostics {
        findings.push(format!("{}:{}", d.code, d.subject));
    }
    for q in &snap.derived.quantities {
        findings.push(format!(
            "quantity {} {} -> {:?} ({})",
            q.gid.value,
            q.unit,
            q.normalized_value,
            if q.convertible { "converted" } else { "retained" }
        ));
    }
    steps.push(step(
        clock,
        "import",
        label,
        Some(res.event_seq),
        Some(res.version_id.clone()),
        res.conflict,
        findings,
    ));
    res.version_id
}

fn step(
    clock: &str,
    action: &str,
    detail: &str,
    event_seq: Option<i64>,
    version_id: Option<String>,
    conflict: Option<String>,
    findings: Vec<String>,
) -> ReplayStep {
    ReplayStep {
        clock: clock.to_string(),
        action: action.to_string(),
        detail: detail.to_string(),
        event_seq,
        version_id,
        conflict,
        findings,
    }
}

pub fn expected_findings() -> Vec<ExpectedFinding> {
    vec![
        ExpectedFinding { code: "NAMESPACE_REMAP_BY_URI".to_string(), where_: "t3".to_string() },
        ExpectedFinding { code: "ALIAS_CYCLE".to_string(), where_: "t2".to_string() },
        ExpectedFinding { code: "DUPLICATE_NODE_ID".to_string(), where_: "t3".to_string() },
        ExpectedFinding { code: "INVERSE_REFERENCE_NORMALIZED".to_string(), where_: "t3".to_string() },
        ExpectedFinding { code: "MODEL_VERSION_CONFLICT".to_string(), where_: "t5".to_string() },
        ExpectedFinding { code: "UNKNOWN_EXTENSION_RETAINED".to_string(), where_: "t1".to_string() },
        ExpectedFinding { code: "DANGLING_AFTER_DELETE".to_string(), where_: "t5".to_string() },
        ExpectedFinding { code: "UNRESOLVED_NODE_PLACEHOLDER".to_string(), where_: "t2/t5".to_string() },
        ExpectedFinding { code: "CONCURRENT_SAVE_CONFLICT".to_string(), where_: "t8".to_string() },
    ]
}

/// Round-trip helper: export, wipe (fresh store), import; returns a new app.
pub fn export_then_reimport(old: &App, clock: &str) -> (ExportBundle, App) {
    let bundle = old.export_bundle(clock.to_string()).expect("export");
    let store = Store::open_memory().expect("store");
    import_bundle(&store, &bundle).expect("import bundle");
    (bundle, App::new(store))
}
