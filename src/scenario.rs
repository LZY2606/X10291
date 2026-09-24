//! Reproducible offline case player for "Namespace Compass".
//!
//! The same script runs in tests and in the web demo tour, so reviewers traverse
//! namespace reorder, alias cycle, duplicate NodeId, inverse references,
//! dependency version conflict, unknown extensions and dangling-after-delete.

use crate::events::EventKind;
use serde::Serialize;
use crate::store::Store;
use std::collections::BTreeMap;

pub const MAIN: &str = "main";
pub const BRANCH_B: &str = "draft-a";

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub seq: u64,
    pub title: String,
    pub expected_code: Option<&'static str>,
}

pub fn fixture_path(name: &str) -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("fixtures/case-compass");
    p.push(name);
    p
}

pub fn read(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name)).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn ev(kind: EventKind) -> &'static str {
    match kind {
        EventKind::ImportFile => "ImportFile",
        EventKind::ParameterChange => "ParameterChange",
        EventKind::LocalSourceChange => "LocalSourceChange",
        EventKind::ManualDecision => "ManualDecision",
        EventKind::BranchCheckpoint => "BranchCheckpoint",
    }
}

/// Play the complete narrative. Returns the ordered step titles and the
/// diagnostic codes visible after each material step.
pub fn play(store: &Store) -> Vec<Step> {
    let mut steps = Vec::new();
    let rv = crate::merge::RULE_VERSION;

    let import = |steps: &mut Vec<Step>, store: &Store, rel: &str, title: &str, code: Option<&'static str>| {
        let head = store.head_seq(MAIN);
        let content = read(rel);
        let (e, model) = store
            .append_event(MAIN, ev(EventKind::ImportFile), title, rel, rv, Some(rel), Some(&content), BTreeMap::new(), Some(head))
            .unwrap_or_else(|err| panic!("import {rel}: {err}"));
        steps.push(Step { seq: e.seq, title: title.into(), expected_code: code });
        let _ = model;
    };

    // 1-2 initial v1 baseline
    import(&mut steps, store, "v1/base.xml", "导入 base：完整基线（含未知扩展与单位量）", Some("UNKNOWN_EXTENSION"));
    import(&mut steps, store, "v1/line.xml", "导入 line：namespace 重排 + 逆向引用 + 缺失依赖", Some("DEPENDENCY_UNRESOLVED"));

    // checkpoint for branch B
    let head = store.head_seq(MAIN);
    store.create_branch(BRANCH_B, head, "人工决定草案 A：MachineToolType 另立拓扑类型父类").unwrap();

    // 3 parameter revision
    import(&mut steps, store, "v2/params.xml", "参数修订：AxisXPosition 1200 mm/MachineCoord → 1250 mm/WorkCoord", None);

    // 4 localized source change
    import(&mut steps, store, "v2/local.xml", "局部依赖改动：新增 StatusBoard → ServerStatus", None);

    // 5 duplicate NodeId
    import(&mut steps, store, "v2/duplicate.xml", "重复 NodeId：HydraulicPressure 第二份声明", Some("DUPLICATE_NODEID"));

    // 6 alias cycle (separate branch to keep main meaningful? Keep on main — it
    // is an independent file namespace, cycle is contained to that file.)
    import(&mut steps, store, "v2/aliases_cycle.xml", "Alias 循环：Alpha ↔ Beta，影响仅限该文件", Some("ALIAS_CYCLE"));

    // 7 dependency version conflict + unknown extension
    import(&mut steps, store, "v2/dep_conflict.xml", "依赖版本冲突：base 1.4.0 vs 1.4.2 + 未知扩展保留", Some("DEP_VERSION_CONFLICT"));

    // 8 conflicting human decision lives on branch B
    {
        let head_b = store.head_seq(BRANCH_B);
        let content = read("v2/conflict.xml");
        let (e, _) = store
            .append_event(BRANCH_B, ev(EventKind::ImportFile), "冲突决定草案 A：重定义 MachineToolType 父类", "v2/conflict.xml", rv, Some("v2/conflict.xml"), Some(&content), BTreeMap::new(), Some(head_b))
            .unwrap();
        steps.push(Step { seq: e.seq, title: "分支 draft-a：冲突人工决定（未选候选保留）".into(), expected_code: Some("DUPLICATE_NODEID") });
    }
    store
        .record_decision(BRANCH_B, store.head_seq(BRANCH_B), "MachineToolType 父类型", Some("沿用 i=58（base 现状）"), &["TopologyElementType ns=1;i=1002（draft-a）"], "审阅比较两个分支后维持现状，draft-a 作为未选候选保留")
        .unwrap();

    // 9 dangling after delete: delete base.xml content then check placeholders.
    {
        let head = store.head_seq(MAIN);
        let raw_files = store.raw_files(MAIN);
        let base_id = raw_files.iter().find(|f| f.path.ends_with("v1/base.xml")).map(|f| f.file_id.clone()).unwrap();
        let mut payload = BTreeMap::new();
        payload.insert("delete_file_id".to_string(), base_id);
        let (e, _) = store
            .append_event(MAIN, ev(EventKind::ManualDecision), "删除 base.xml：引用应变为悬空 unresolved", "delete", rv, None, None, payload, Some(head))
            .unwrap();
        steps.push(Step { seq: e.seq, title: "删除来源后悬空引用：节点保留为 unresolved 占位".into(), expected_code: Some("DANGLING_REFERENCE") });
    }

    steps
}

/// Codes present in the current model of a branch.
pub fn codes(store: &Store, branch: &str) -> Vec<String> {
    store
        .current_model(branch)
        .map(|m| m.diagnostics.iter().map(|d| d.code.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect())
        .unwrap_or_default()
}

pub fn expected_tour() -> Vec<Step> {
    vec![
        Step { seq: 1, title: "初始导入：原 XML 节点/namespace/NodeId/BrowseName/reference/alias/dependency/未知扩展全部保留".into(), expected_code: None },
        Step { seq: 2, title: "namespace 重排、逆向引用、缺失依赖 unresolved".into(), expected_code: Some("NS_INDEX_FILE_LOCAL") },
        Step { seq: 3, title: "参数修订（单位/坐标基准原值保留，派生层规范化）".into(), expected_code: None },
        Step { seq: 4, title: "局部依赖改动（依赖图传播只重算受影响项）".into(), expected_code: None },
        Step { seq: 5, title: "重复 NodeId（多份来源全部保留）".into(), expected_code: Some("DUPLICATE_NODEID") },
        Step { seq: 6, title: "alias 循环（影响限制在本文件）".into(), expected_code: Some("ALIAS_CYCLE") },
        Step { seq: 7, title: "依赖版本冲突 + 未知扩展".into(), expected_code: Some("DEP_VERSION_CONFLICT") },
        Step { seq: 8, title: "分支 draft-a：冲突人工决定，未选候选保留".into(), expected_code: Some("MULTIPLE_CANDIDATES_KEPT") },
        Step { seq: 9, title: "删除来源后悬空引用（unresolved 占位，不丢边）".into(), expected_code: Some("DANGLING_REFERENCE") },
    ]
}
