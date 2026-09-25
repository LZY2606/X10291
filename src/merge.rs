//! 合并语义：
//! - 数字 namespace index 只在单文件内有意义，合并按 URI 重映射；
//! - inverse reference 与 forward reference 规范化，但不制造缺边；
//! - 依赖缺失保留 unresolved 节点；
//! - 重复 NodeId、alias 循环、单位不可转换等多解/不确定状态全部保留并附诊断。

use crate::model::*;
use std::collections::{BTreeMap, BTreeSet};

const UA_BASE_URI: &str = "http://opcfoundation.org/UA/";

struct FileCtx<'a> {
    raw: &'a RawNodeSet,
    /// 文件内序号 -> 全局序号
    local_to_global: BTreeMap<u32, u32>,
    alias_map: BTreeMap<String, String>,
}

/// 解析 NodeId 文本 -> (ns 部分, ident 部分)。ns 部分: Some(数字) / None(=0)。
fn split_node_id(text: &str) -> (Option<u32>, String) {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("ns=") {
        if let Some(sep) = rest.find(';') {
            let n: u32 = rest[..sep].parse().unwrap_or(0);
            return (Some(n), rest[sep + 1..].to_string());
        }
    }
    if let Some(rest) = t.strip_prefix("nsu=") {
        // 已是全局形式：nsu=uri;ident —— 调用方需特殊处理，这里返回 uri 标记。
        if let Some(sep) = rest.find(';') {
            return (Some(u32::MAX), format!("{};{}", &rest[..sep], &rest[sep + 1..]));
        }
    }
    (None, t.to_string())
}

fn global_id(uri: &str, ident: &str) -> String {
    format!("nsu={};{}", uri, ident)
}

/// alias 解析：目标可能是另一个 alias；检测循环，循环时返回 None（保留原文）。
fn resolve_alias<'a>(
    aliases: &'a BTreeMap<String, String>,
    name: &str,
) -> Option<&'a str> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut cur = name;
    loop {
        if !seen.insert(cur) {
            return None; // 循环
        }
        match aliases.get(cur) {
            Some(next) if !next.contains('=') => cur = next,
            Some(next) => return Some(next.as_str()),
            None => return None,
        }
    }
}

fn alias_cycle_exists(aliases: &BTreeMap<String, String>, name: &str) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut cur = name;
    while let Some(next) = aliases.get(cur) {
        if !seen.insert(cur) {
            return true;
        }
        if next.contains('=') {
            return false;
        }
        cur = next;
    }
    false
}

/// 单位规范化到 SI（派生层）。无法转换返回 None。
fn to_si(text: &str, unit_raw: Option<&str>) -> Option<(f64, &'static str)> {
    let v: f64 = text.trim().parse().ok()?;
    match unit_raw {
        None => Some((v, "1")),
        Some("°C") | Some("degC") | Some("4408652") => Some((v + 273.15, "K")),
        Some("K") | Some("4407876") => Some((v, "K")),
        Some("bar") => Some((v * 100_000.0, "Pa")),
        Some("Pa") => Some((v, "Pa")),
        Some("m") => Some((v, "m")),
        Some(_) => None,
    }
}

pub fn merge(files: &[RawNodeSet]) -> MergedModel {
    let mut namespaces: Vec<String> = vec![UA_BASE_URI.to_string()];
    let mut ns_index: BTreeMap<String, u32> = BTreeMap::new();
    ns_index.insert(UA_BASE_URI.to_string(), 0);
    let mut ns_mappings = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut ctxs: Vec<FileCtx> = Vec::new();

    // 1) 全局 namespace 注册：按 URI 重映射，文件内序号仅用于本文件解析。
    for raw in files {
        let mut local_to_global = BTreeMap::new();
        local_to_global.insert(0u32, 0u32);
        for (i, uri) in raw.namespace_uris.iter().enumerate() {
            let local = (i + 1) as u32;
            let g = *ns_index.entry(uri.clone()).or_insert_with(|| {
                namespaces.push(uri.clone());
                (namespaces.len() - 1) as u32
            });
            local_to_global.insert(local, g);
            ns_mappings.push(NsMapping {
                file: raw.source_name.clone(),
                local_index: local,
                uri: uri.clone(),
                global_index: g,
            });
        }
        let alias_map: BTreeMap<String, String> = raw.aliases.iter().cloned().collect();
        // alias 循环诊断（保留原文，不强行解析）
        for (name, _) in &raw.aliases {
            if alias_cycle_exists(&alias_map, name) {
                diagnostics.push(Diagnostic {
                    id: stable_id(&["AliasCycle", &raw.source_name, name]),
                    kind: "AliasCycle".into(),
                    message: format!(
                        "{}: alias '{}' 形成循环，保留原文不做解析",
                        raw.source_name, name
                    ),
                    subjects: vec![raw.source_name.clone(), name.clone()],
                });
            }
        }
        ctxs.push(FileCtx {
            raw,
            local_to_global,
            alias_map,
        });
    }

    let to_global = |ctx: &FileCtx, text: &str| -> Option<String> {
        let t = text.trim();
        if let Some(rest) = t.strip_prefix("nsu=") {
            return Some(format!("nsu={}", rest));
        }
        let (ns, ident) = split_node_id(t);
        let local = ns.unwrap_or(0);
        if local == u32::MAX {
            return Some(t.to_string());
        }
        let g = *ctx.local_to_global.get(&local).unwrap_or(&0);
        Some(global_id(&namespaces[g as usize], &ident))
    };

    // 2) 节点：转换全局身份；重复 NodeId 保留全部候选。
    let mut nodes: BTreeMap<String, MergedNode> = BTreeMap::new();
    for ctx in &ctxs {
        for rn in &ctx.raw.nodes {
            let gid = match to_global(ctx, &rn.node_id) {
                Some(g) => g,
                None => continue,
            };
            let (bns, bname) = match rn.browse_name.split_once(':') {
                Some((n, name)) => {
                    let local: u32 = n.parse().unwrap_or(0);
                    let g = *ctx.local_to_global.get(&local).unwrap_or(&0);
                    (Some(namespaces[g as usize].clone()), name.to_string())
                }
                None => (None, rn.browse_name.clone()),
            };
            let cand = NodeCandidate {
                file: ctx.raw.source_name.clone(),
                line_start: rn.line_start,
                line_end: rn.line_end,
                display_name: rn.display_name.clone(),
            };
            let (value_si, value_si_unit) = match &rn.value {
                Some(v) if !v.text.is_empty() => match to_si(&v.text, v.unit_raw.as_deref()) {
                    Some((si, unit)) => (Some(si), Some(unit.to_string())),
                    None => {
                        diagnostics.push(Diagnostic {
                            id: stable_id(&["UnconvertibleUnit", &gid]),
                            kind: "UnconvertibleUnit".into(),
                            message: format!(
                                "{}: 值 '{}' 的单位 '{}' 无法转换，原值保留在原位置",
                                gid,
                                v.text,
                                v.unit_raw.clone().unwrap_or_default()
                            ),
                            subjects: vec![gid.clone()],
                        });
                        (None, None)
                    }
                },
                _ => (None, None),
            };
            match nodes.get_mut(&gid) {
                Some(existing) => {
                    existing.status = NodeStatus::Duplicate;
                    existing.candidates.push(cand);
                }
                None => {
                    nodes.insert(
                        gid.clone(),
                        MergedNode {
                            global_id: gid,
                            class: rn.class.clone(),
                            browse_name: bname,
                            browse_uri: bns,
                            display_name: rn.display_name.clone(),
                            description: rn.description.clone(),
                            value_raw: rn.value.clone(),
                            value_si,
                            value_si_unit,
                            status: NodeStatus::Ok,
                            candidates: vec![cand],
                            extensions: rn.extensions.clone(),
                            source_file: ctx.raw.source_name.clone(),
                            line_start: rn.line_start,
                            line_end: rn.line_end,
                        },
                    );
                }
            }
        }
    }
    for (gid, n) in &nodes {
        if n.status == NodeStatus::Duplicate {
            diagnostics.push(Diagnostic {
                id: stable_id(&["DuplicateNodeId", gid]),
                kind: "DuplicateNodeId".into(),
                message: format!(
                    "{}: 重复 NodeId，{} 个候选全部保留，未做唯一化",
                    gid,
                    n.candidates.len()
                ),
                subjects: vec![gid.clone()],
            });
        }
    }

    // 3) 引用：alias 解析 -> 全局重映射 -> 正/逆向规范化（不制造缺边）。
    let mut edge_set: BTreeSet<Edge> = BTreeSet::new();
    let mut declared_pairs: BTreeSet<(String, String, String)> = BTreeSet::new();
    for ctx in &ctxs {
        for rn in &ctx.raw.nodes {
            let src_gid = match to_global(ctx, &rn.node_id) {
                Some(g) => g,
                None => continue,
            };
            for r in &rn.references {
                // ReferenceType 也可能是 alias
                let rt_text = if r.ref_type.contains('=') {
                    r.ref_type.clone()
                } else {
                    match resolve_alias(&ctx.alias_map, &r.ref_type) {
                        Some(t) => t.to_string(),
                        None => r.ref_type.clone(),
                    }
                };
                let ref_type = to_global(ctx, &rt_text).unwrap_or(rt_text.clone());
                let tgt_text = if r.target.contains('=') {
                    r.target.clone()
                } else {
                    match resolve_alias(&ctx.alias_map, &r.target) {
                        Some(t) => t.to_string(),
                        None => {
                            diagnostics.push(Diagnostic {
                                id: stable_id(&["UnresolvedAlias", &src_gid, &r.target]),
                                kind: "UnresolvedAlias".into(),
                                message: format!(
                                    "{}: 引用目标 alias '{}' 无法解析（循环或缺失），保留原文",
                                    src_gid, r.target
                                ),
                                subjects: vec![src_gid.clone(), r.target.clone()],
                            });
                            continue;
                        }
                    }
                };
                let dst_gid = match to_global(ctx, &tgt_text) {
                    Some(g) => g,
                    None => continue,
                };
                let (src, dst, origin) = if r.is_forward {
                    (src_gid.clone(), dst_gid.clone(), "Forward")
                } else {
                    // 逆向声明规范化：target -> node，方向与正向声明一致。
                    (dst_gid.clone(), src_gid.clone(), "InverseNormalized")
                };
                declared_pairs.insert((src.clone(), dst.clone(), ref_type.clone()));
                edge_set.insert(Edge {
                    src,
                    dst,
                    ref_type,
                    origin: origin.to_string(),
                    source_file: ctx.raw.source_name.clone(),
                });
            }
        }
    }

    // 4) 悬空/缺失目标：保留 unresolved 占位节点，不丢边。
    let edges: Vec<Edge> = edge_set.into_iter().collect();
    for e in &edges {
        for endpoint in [&e.src, &e.dst] {
            if !nodes.contains_key(endpoint) {
                let uri = endpoint
                    .strip_prefix("nsu=")
                    .and_then(|s| s.split(';').next())
                    .unwrap_or("");
                let dep_declared = ctxs.iter().any(|c| {
                    c.raw.models.iter().any(|m| {
                        m.model_uri == uri
                            || m.required.iter().any(|d| d.model_uri == uri)
                    })
                });
                let kind = if dep_declared || uri == UA_BASE_URI {
                    "UnresolvedReference"
                } else {
                    "DependencyMissing"
                };
                diagnostics.push(Diagnostic {
                    id: stable_id(&[kind, endpoint]),
                    kind: kind.into(),
                    message: format!(
                        "{}: 目标节点缺失，保留 unresolved 占位节点（来源 {}）",
                        endpoint, e.source_file
                    ),
                    subjects: vec![endpoint.to_string()],
                });
                nodes.insert(
                    endpoint.to_string(),
                    MergedNode {
                        global_id: endpoint.to_string(),
                        class: "Unresolved".into(),
                        browse_name: endpoint.to_string(),
                        browse_uri: None,
                        display_name: None,
                        description: None,
                        value_raw: None,
                        value_si: None,
                        value_si_unit: None,
                        status: NodeStatus::Unresolved,
                        candidates: vec![],
                        extensions: vec![],
                        source_file: e.source_file.clone(),
                        line_start: 0,
                        line_end: 0,
                    },
                );
            }
        }
    }

    // 5) 模型依赖版本冲突：保留冲突双方，不吞掉。
    let loaded: BTreeMap<String, String> = files
        .iter()
        .flat_map(|f| f.models.iter().map(|m| (m.model_uri.clone(), m.version.clone())))
        .collect();
    for f in files {
        for m in &f.models {
            for dep in &m.required {
                if let Some(have) = loaded.get(&dep.model_uri) {
                    if *have != dep.version {
                        diagnostics.push(Diagnostic {
                            id: stable_id(&["DependencyVersionConflict", &m.model_uri, &dep.model_uri]),
                            kind: "DependencyVersionConflict".into(),
                            message: format!(
                                "模型 {} 要求 {}@{}，已加载 {}；双方版本均保留",
                                m.model_uri, dep.model_uri, dep.version, have
                            ),
                            subjects: vec![m.model_uri.clone(), dep.model_uri.clone()],
                        });
                    }
                }
            }
        }
    }

    MergedModel {
        namespaces,
        ns_mappings,
        nodes: nodes.into_values().collect(),
        edges,
        diagnostics,
    }
}
