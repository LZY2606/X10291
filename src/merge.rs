//! 合并引擎：把多份 NodeSet 按 namespace URI 重映射后组装成统一图。
//! 规则：
//! - 数字 namespace index 只在单文件内有意义，合并按 URI 重映射；
//! - inverse reference 与 forward reference 规范化但不制造缺边；
//! - 依赖缺失保留 unresolved 节点；
//! - 多解/不确定状态全部保留为候选，不吞掉。

use crate::model::*;
use std::collections::{HashMap, HashSet};

pub struct FileContext<'a> {
    pub source_name: &'a str,
    pub parsed: &'a ParsedNodeSet,
}

#[derive(Debug, Clone)]
pub struct MergedNode {
    pub gid: GlobalNodeId,
    pub element: String,
    pub browse_name: String,
    pub raw_xml: String,
    pub source: String,
    pub start: usize,
    pub end: usize,
    pub attrs: Vec<(String, String)>,
    pub extensions: Vec<RawFragment>,
    pub dup_group: Option<usize>, // 重复 NodeId 组号
}

#[derive(Debug, Clone)]
pub struct Unresolved {
    pub text: String,
    pub reason: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct DependencyRow {
    pub model_uri: String,
    pub required_version: Option<String>,
    pub actual_version: Option<String>,
    pub status: String, // ok / missing / version_conflict
}

#[derive(Debug, Default)]
pub struct MergeOutput {
    pub nodes: Vec<MergedNode>,
    pub edges: Vec<(Edge, String)>, // (边, 来源文件)
    pub unresolved: Vec<Unresolved>,
    pub dependencies: Vec<DependencyRow>,
    pub events: Vec<JudgmentEvent>,
}

/// 解析文件内 NodeId 文本为全局 NodeId。
/// alias 循环检测：沿 alias 链解析，发现环则产生事件并返回 None。
pub fn resolve_nodeid(
    text: &str,
    ctx: &FileContext,
    events: &mut Vec<JudgmentEvent>,
) -> Option<GlobalNodeId> {
    let mut current = text.trim().to_string();
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        // alias?
        if let Some((_, target)) = ctx
            .parsed
            .aliases
            .iter()
            .find(|(a, _)| a == &current)
        {
            if !seen.insert(current.clone()) {
                let mut ev = JudgmentEvent::new(
                    "alias_cycle",
                    &current,
                    "alias 解析沿链进行，发现环则保留 unresolved，不猜测目标",
                    &format!("alias 链出现循环，涉及 {current}（来源 {}）", ctx.source_name),
                );
                ev.inputs = ctx
                    .parsed
                    .aliases
                    .iter()
                    .map(|(a, t)| format!("{a} -> {t}"))
                    .collect();
                ev.candidates = vec![serde_json::json!({"status":"unresolved","reason":"alias_cycle"})];
                events.push(ev);
                return None;
            }
            current = target.trim().to_string();
            continue;
        }
        break;
    }
    parse_global(&current, ctx, events)
}

fn parse_global(
    text: &str,
    ctx: &FileContext,
    events: &mut Vec<JudgmentEvent>,
) -> Option<GlobalNodeId> {
    if let Some(rest) = text.strip_prefix("nsu=") {
        // 已是 URI 形式：nsu=<uri>;<id>
        if let Some(pos) = rest.rfind(';') {
            return Some(GlobalNodeId::from_parts(&rest[..pos], &rest[pos + 1..]));
        }
        return None;
    }
    let (ns_idx, id_part) = if let Some(rest) = text.strip_prefix("ns=") {
        match rest.split_once(';') {
            Some((n, id)) => match n.parse::<usize>() {
                Ok(i) => (i, id.to_string()),
                Err(_) => return None,
            },
            None => return None,
        }
    } else if text.starts_with("i=")
        || text.starts_with("s=")
        || text.starts_with("g=")
        || text.starts_with("b=")
    {
        (0usize, text.to_string())
    } else {
        // 裸文本且不是已知 alias：无法确定，保留 unresolved
        let mut ev = JudgmentEvent::new(
            "unresolved_nodeid",
            text,
            "无法解析的 NodeId 文本保留为 unresolved，不静默丢弃",
            &format!("{text:?} 不是 ns=/nsu=/i=/s=/g=/b= 形式，也不是已知 alias"),
        );
        ev.candidates = vec![serde_json::json!({"status":"unresolved","raw":text})];
        events.push(ev);
        return None;
    };
    match ctx.parsed.namespaces.get(ns_idx) {
        Some(uri) => Some(GlobalNodeId::from_parts(uri, &id_part)),
        None => {
            let mut ev = JudgmentEvent::new(
                "namespace_index_out_of_range",
                text,
                "数字 namespace index 只在单文件内有意义；越界 index 保留 unresolved",
                &format!(
                    "文件 {} 的 namespace 表只有 {} 项，index {ns_idx} 越界",
                    ctx.source_name,
                    ctx.parsed.namespaces.len()
                ),
            );
            ev.candidates = vec![serde_json::json!({"status":"unresolved","raw":text})];
            events.push(ev);
            None
        }
    }
}

/// 解析 BrowseName 的数字前缀为 namespace URI（保留原始表示，派生层另存 URI 形式）。
pub fn browse_name_uri(browse: &str, ctx: &FileContext) -> Option<String> {
    let (prefix, _) = browse.split_once(':')?;
    let idx: usize = prefix.parse().ok()?;
    ctx.parsed.namespaces.get(idx).cloned()
}

pub fn merge(files: &[FileContext]) -> MergeOutput {
    let mut out = MergeOutput::default();
    let mut events = Vec::new();

    // 1. 节点：解析 NodeId，检测重复
    let mut gid_to_indices: HashMap<GlobalNodeId, Vec<usize>> = HashMap::new();
    let mut pending_nodes: Vec<(MergedNode, &FileContext)> = Vec::new();
    for ctx in files {
        for raw in &ctx.parsed.nodes {
            match resolve_nodeid(&raw.node_id_text, ctx, &mut events) {
                Some(gid) => {
                    let node = MergedNode {
                        gid: gid.clone(),
                        element: raw.element.clone(),
                        browse_name: raw.browse_name.clone(),
                        raw_xml: raw.raw_xml.clone(),
                        source: ctx.source_name.to_string(),
                        start: raw.start,
                        end: raw.end,
                        attrs: raw.attrs.clone(),
                        extensions: raw.extensions.clone(),
                        dup_group: None,
                    };
                    gid_to_indices.entry(gid).or_default().push(pending_nodes.len());
                    pending_nodes.push((node, ctx));
                }
                None => out.unresolved.push(Unresolved {
                    text: raw.node_id_text.clone(),
                    reason: "nodeid_unresolved".into(),
                    source: ctx.source_name.into(),
                }),
            }
        }
    }
    // 重复 NodeId：全部保留为候选，不吞掉
    let mut dup_group_counter = 0usize;
    for (gid, idxs) in &gid_to_indices {
        if idxs.len() > 1 {
            let mut ev = JudgmentEvent::new(
                "duplicate_nodeid",
                gid.as_str(),
                "同一全局 NodeId 出现多次：全部候选保留，标记 dup_group，不自动取舍",
                &format!("{} 在 {} 个来源中重复定义", gid.as_str(), idxs.len()),
            );
            ev.inputs = idxs
                .iter()
                .map(|i| {
                    format!(
                        "{}@{}",
                        pending_nodes[*i].0.source, pending_nodes[*i].0.start
                    )
                })
                .collect();
            ev.candidates = idxs
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "source": pending_nodes[*i].0.source,
                        "start": pending_nodes[*i].0.start,
                        "browse_name": pending_nodes[*i].0.browse_name,
                    })
                })
                .collect();
            events.push(ev);
            for i in idxs {
                pending_nodes[*i].0.dup_group = Some(dup_group_counter);
            }
            dup_group_counter += 1;
        }
    }

    // 2. 引用：inverse 规范化为 forward 方向，去重但不制造缺边
    let mut edge_set: HashSet<(String, String, String)> = HashSet::new();
    for (node, ctx) in &pending_nodes {
        let raw_node = ctx
            .parsed
            .nodes
            .iter()
            .find(|r| r.raw_xml == node.raw_xml && r.start == node.start);
        let Some(raw_node) = raw_node else { continue };
        for r in &raw_node.references {
            let Some(ref_type) = resolve_nodeid(&r.ref_type_text, ctx, &mut events) else {
                out.unresolved.push(Unresolved {
                    text: r.ref_type_text.clone(),
                    reason: "reference_type_unresolved".into(),
                    source: ctx.source_name.into(),
                });
                continue;
            };
            let Some(target) = resolve_nodeid(&r.target_text, ctx, &mut events) else {
                out.unresolved.push(Unresolved {
                    text: r.target_text.clone(),
                    reason: "reference_target_unresolved".into(),
                    source: ctx.source_name.into(),
                });
                continue;
            };
            let (src, dst) = if r.is_forward {
                (node.gid.clone(), target)
            } else {
                // inverse：规范化方向为 forward（target -> node）
                let mut ev = JudgmentEvent::new(
                    "inverse_normalized",
                    &format!("{} -> {}", target.as_str(), node.gid.as_str()),
                    "inverse reference 规范化为 forward 方向存储；不制造未声明的缺边",
                    &format!(
                        "{} 上声明了 IsForward=false 的引用，规范化后源为 {}",
                        node.gid.as_str(),
                        target.as_str()
                    ),
                );
                ev.inputs = vec![format!("{}@{}", ctx.source_name, raw_node.start)];
                ev.candidates = vec![serde_json::json!({
                    "src": target.as_str(), "dst": node.gid.as_str(),
                    "ref_type": ref_type.as_str(), "normalized_from": "inverse",
                })];
                ev.chosen = Some(0);
                events.push(ev);
                (target, node.gid.clone())
            };
            let key = (
                src.as_str().to_string(),
                dst.as_str().to_string(),
                ref_type.as_str().to_string(),
            );
            if edge_set.insert(key) {
                out.edges.push((
                    Edge { src, dst, ref_type },
                    ctx.source_name.to_string(),
                ));
            } else {
                let mut ev = JudgmentEvent::new(
                    "duplicate_edge_deduped",
                    &format!("{:?}", edge_set.iter().next_back()),
                    "forward 与 inverse 声明产生同一条边：保留一条，不制造重复边",
                    "同一 (src,dst,ref_type) 边被声明多次，规范化后去重",
                );
                ev.candidates = vec![serde_json::json!({"deduped": true})];
                ev.chosen = Some(0);
                events.push(ev);
            }
        }
    }

    // 3. 模型依赖：按 URI 匹配，版本冲突与缺失都保留
    let available: Vec<&ModelDependency> =
        files.iter().filter_map(|f| f.parsed.model.as_ref()).collect();
    for ctx in files {
        for req in &ctx.parsed.required_models {
            let found = available.iter().find(|m| m.model_uri == req.model_uri);
            match found {
                None => {
                    out.dependencies.push(DependencyRow {
                        model_uri: req.model_uri.clone(),
                        required_version: req.version.clone(),
                        actual_version: None,
                        status: "missing".into(),
                    });
                    let mut ev = JudgmentEvent::new(
                        "dependency_missing",
                        &req.model_uri,
                        "依赖缺失保留 unresolved 节点，不阻断合并",
                        &format!("{} 依赖的模型 {} 未提供", ctx.source_name, req.model_uri),
                    );
                    ev.candidates = vec![serde_json::json!({"status":"unresolved","model":req.model_uri})];
                    events.push(ev);
                }
                Some(actual) => {
                    let conflict = req.version.is_some()
                        && actual.version.is_some()
                        && req.version != actual.version;
                    if conflict {
                        out.dependencies.push(DependencyRow {
                            model_uri: req.model_uri.clone(),
                            required_version: req.version.clone(),
                            actual_version: actual.version.clone(),
                            status: "version_conflict".into(),
                        });
                        let mut ev = JudgmentEvent::new(
                            "dependency_version_conflict",
                            &req.model_uri,
                            "依赖版本冲突：要求版本与实际版本都保留为候选，不自动选择",
                            &format!(
                                "{} 要求 {} @ {:?}，实际提供 {:?}",
                                ctx.source_name, req.model_uri, req.version, actual.version
                            ),
                        );
                        ev.candidates = vec![
                            serde_json::json!({"required": req.version, "by": ctx.source_name}),
                            serde_json::json!({"actual": actual.version}),
                        ];
                        events.push(ev);
                    } else {
                        out.dependencies.push(DependencyRow {
                            model_uri: req.model_uri.clone(),
                            required_version: req.version.clone(),
                            actual_version: actual.version.clone(),
                            status: "ok".into(),
                        });
                    }
                }
            }
        }
    }

    // 4. 未识别扩展保留事件（顶层）
    for ctx in files {
        for ext in &ctx.parsed.top_level_extensions {
            let mut ev = JudgmentEvent::new(
                "unknown_extension_preserved",
                &ext.element,
                "未识别扩展原样保留，不参与规范化",
                &format!("{} 中的顶层扩展 <{}> 已原样保留", ctx.source_name, ext.element),
            );
            ev.candidates = vec![serde_json::json!({"preserved": true, "element": ext.element})];
            ev.chosen = Some(0);
            events.push(ev);
        }
    }

    out.nodes = pending_nodes.into_iter().map(|(n, _)| n).collect();
    out.events = events;
    out
}
