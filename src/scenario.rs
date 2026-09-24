//! 命名空间罗盘 case pack: a scripted, replayable narrative.
//!
//! 1. baseline import (core + line + station) with everything preserved;
//! 2. a parameter revision (TempSensor1 25 -> 30 degC);
//! 3. a local source change (station adds Valve1) affecting only its subtree;
//! 4. two conflicting human decisions from the same version (both kept);
//! 5. the counter-example archive: from the baseline, seven branches each
//!    changing exactly one source (ns reorder, alias cycle, duplicate NodeId,
//!    inverse reference, dependency version conflict, unknown extension,
//!    dangling reference).

use crate::store::{Edit, SaveOutcome, Store};
use crate::xml::parse_nodeset;
use anyhow::Result;

pub const FIXTURES: &[(&str, &str)] = &[
    ("core.nodeset.xml", include_str!("../fixtures/compass/core.nodeset.xml")),
    ("line.nodeset.xml", include_str!("../fixtures/compass/line.nodeset.xml")),
    ("line_v2.nodeset.xml", include_str!("../fixtures/compass/line_v2.nodeset.xml")),
    ("station.nodeset.xml", include_str!("../fixtures/compass/station.nodeset.xml")),
    ("station_v2.nodeset.xml", include_str!("../fixtures/compass/station_v2.nodeset.xml")),
    ("variant_ns_reorder.nodeset.xml", include_str!("../fixtures/compass/variant_ns_reorder.nodeset.xml")),
    ("variant_alias_cycle.nodeset.xml", include_str!("../fixtures/compass/variant_alias_cycle.nodeset.xml")),
    ("variant_dup_nodeid.nodeset.xml", include_str!("../fixtures/compass/variant_dup_nodeid.nodeset.xml")),
    ("variant_inverse_ref.nodeset.xml", include_str!("../fixtures/compass/variant_inverse_ref.nodeset.xml")),
    ("variant_dep_conflict.nodeset.xml", include_str!("../fixtures/compass/variant_dep_conflict.nodeset.xml")),
    ("variant_unknown_ext.nodeset.xml", include_str!("../fixtures/compass/variant_unknown_ext.nodeset.xml")),
    ("variant_dangling.nodeset.xml", include_str!("../fixtures/compass/variant_dangling.nodeset.xml")),
];

pub fn fixture(name: &str) -> Option<&'static str> {
    FIXTURES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

fn import(name: &str) -> Edit {
    Edit::ImportFile {
        name: name.to_string(),
        xml: fixture(name).unwrap().to_string(),
    }
}

fn replace(name: &str) -> Edit {
    Edit::ReplaceFile {
        // ReplaceFile targets the imported file name; v2 fixtures keep the
        // same logical source name as the file they replace.
        name: name.replace("_v2", "").to_string(),
        xml: fixture(name).unwrap().to_string(),
    }
}

pub struct CaseResult {
    pub log: Vec<String>,
    pub baseline: i64,
    pub main_tip: i64,
    pub rejected_candidate: i64,
    pub archive_variants: Vec<(String, i64)>,
}

pub fn run_case_pack(store: &Store) -> Result<CaseResult> {
    let mut log = Vec::new();
    let mut say = |s: String| log.push(s);

    // 1. baseline import
    let files = vec![
        parse_nodeset("core.nodeset.xml", fixture("core.nodeset.xml").unwrap())?,
        parse_nodeset("line.nodeset.xml", fixture("line.nodeset.xml").unwrap())?,
        parse_nodeset("station.nodeset.xml", fixture("station.nodeset.xml").unwrap())?,
    ];
    let baseline = store.seed("v1 基线导入: core + line + station (raw preserved)", files)?;
    say(format!("v{baseline} 基线导入完成: 3 份 NodeSet, namespace 按 URI 重映射"));

    // 2. parameter revision
    let v2 = match store.apply_edit(
        baseline,
        "v2 参数修订: TempSensor1 25 -> 30 degC",
        replace("line_v2.nodeset.xml"),
    )? {
        SaveOutcome::Current { version_id } => version_id,
        _ => unreachable!(),
    };
    say(format!("v{v2} 参数修订: 仅 line 源文件变化, 派生层按依赖图局部重算"));

    // 3. local source change
    let v3 = match store.apply_edit(
        v2,
        "v3 局部源改动: station 增加 Valve1",
        replace("station_v2.nodeset.xml"),
    )? {
        SaveOutcome::Current { version_id } => version_id,
        _ => unreachable!(),
    };
    say(format!("v{v3} 局部源改动: station 子树重算, line 派生事实保持有效"));

    // 4. conflicting human decisions from the same version
    let v4 = match store.apply_edit(
        v3,
        "v4 决定A: 采纳 30 degC 设定 (user A)",
        Edit::Decision {
            action: "accept".into(),
            subject: "nsu=urn:compass:line;i=1201".into(),
            note: "采纳 30 degC 设定值".into(),
        },
    )? {
        SaveOutcome::Current { version_id } => version_id,
        _ => unreachable!(),
    };
    let v5 = match store.apply_edit(
        v3,
        "v5 决定B: 拒绝 30 degC 设定 (user B, 与A冲突)",
        Edit::Decision {
            action: "reject".into(),
            subject: "nsu=urn:compass:line;i=1201".into(),
            note: "拒绝, 要求维持 25 degC".into(),
        },
    )? {
        SaveOutcome::Conflict { branch_version, .. } => branch_version,
        _ => unreachable!(),
    };
    say(format!("v{v4} 与 v{v5} 冲突: 同一基线的两个人工决定, 两个候选都保留"));
    store.resolve_branch(v4, "复核选择继续 A; B 作为未采纳候选保留")?;
    say(format!("裁定: 继续 v{v4}; v{v5} 未采纳但可载入、可回溯"));

    // 5. counter-example archive: each variant changes exactly one source.
    let variants = [
        ("variant_ns_reorder.nodeset.xml", "档案A: namespace 重排"),
        ("variant_alias_cycle.nodeset.xml", "档案B: alias 循环"),
        ("variant_dup_nodeid.nodeset.xml", "档案C: 重复 NodeId"),
        ("variant_inverse_ref.nodeset.xml", "档案D: 逆向引用"),
        ("variant_dep_conflict.nodeset.xml", "档案E: 依赖版本冲突"),
        ("variant_unknown_ext.nodeset.xml", "档案F: 未知扩展"),
        ("variant_dangling.nodeset.xml", "档案G: 删除后悬空引用"),
    ];
    let mut archive = Vec::new();
    for (file, label) in variants {
        let id = store.apply_edit_as_branch(baseline, label, import(file))?;
        say(format!("v{id} {label}: 基于 v{baseline} 只改动一个直接来源"));
        archive.push((label.to_string(), id));
    }

    Ok(CaseResult {
        log,
        baseline,
        main_tip: v4,
        rejected_candidate: v5,
        archive_variants: archive,
    })
}
