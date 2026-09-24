use crate::model::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// 机器可读的判定事件：每条都带稳定 id（入库时分配）、来源范围与结构化依据。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub details: Value,
    pub sources: Vec<SourceSpan>,
}

impl Event {
    pub fn new(kind: &str, summary: impl Into<String>, details: Value, sources: Vec<SourceSpan>) -> Event {
        Event { id: String::new(), kind: kind.to_string(), summary: summary.into(), details, sources }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergedNode {
    pub id: String,
    pub node_class: String,
    pub browse_name: String,
    pub browse_name_raw: String,
    pub display_name: String,
    pub description: Option<String>,
    pub attributes: Vec<(String, String)>,
    pub extensions: Vec<String>,
    pub source: SourceSpan,
    pub origin_file: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub ref_type: String,
    pub normalized_from_inverse: bool,
    pub resolved: bool,
    pub source: SourceSpan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unresolved {
    pub kind: String,
    pub detail: String,
    pub source: Option<SourceSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergedModel {
    pub namespaces: Vec<String>,
    pub nodes: BTreeMap<String, MergedNode>,
    pub edges: Vec<Edge>,
    pub unresolved: Vec<Unresolved>,
    pub events: Vec<Event>,
    pub models: Vec<ModelMeta>,
    pub files: Vec<String>,
}

/// 合并多份 NodeSet。语义红线：
/// - 数字 namespace index 只在单文件内有意义，合并按 URI 重映射；
/// - inverse 与 forward reference 规范化，但不制造缺边；
/// - 依赖缺失 / 目标缺失保留 unresolved 节点，不静默丢弃。
pub fn merge(files: &[NodeSetFile]) -> MergedModel {
    let mut events: Vec<Event> = Vec::new();
    let mut namespaces: Vec<String> = Vec::new();
    let mut maps: Vec<Vec<u16>> = Vec::new();

    // 第一遍：namespace 表按 URI 对齐，记录重映射与重排。
    for f in files {
        let mut map = Vec::new();
        for (i, uri) in f.namespace_uris.iter().enumerate() {
            let file_index = (i + 1) as u16;
            let canon = match namespaces.iter().position(|u| u == uri) {
                Some(p) => (p + 1) as u16,
                None => {
                    namespaces.push(uri.clone());
                    namespaces.len() as u16
                }
            };
            if canon != file_index {
                events.push(Event::new(
                    "NamespaceRemap",
                    format!("{}: ns={file_index} 按 URI 重映射为 ns={canon}（{uri}）", f.file_name),
                    json!({"file": f.file_name, "file_index": file_index, "uri": uri, "canonical_index": canon}),
                    vec![],
                ));
            }
            map.push(canon);
        }
        if map.windows(2).any(|w| w[0] > w[1]) {
            events.push(Event::new(
                "NamespaceReorder",
                format!("{}: namespace 表顺序与合并表不一致，已按 URI 对齐 {:?}", f.file_name, map),
                json!({"file": f.file_name, "file_to_canonical": map}),
                vec![],
            ));
        }
        maps.push(map);
    }

    // 第二遍：节点登记，重复 NodeId 保留先见者并记录。
    let mut nodes: BTreeMap<String, MergedNode> = BTreeMap::new();
    let mut kept: HashMap<(usize, NodeId), String> = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        for n in &f.nodes {
            let Some(gid) = remap(&n.node_id, &maps[fi], f, &mut events, &n.source) else { continue };
            let gid = gid.to_string();
            if let Some(existing) = nodes.get(&gid) {
                events.push(Event::new(
                    "DuplicateNodeId",
                    format!("重复 NodeId {gid}: 保留 {} 的定义，丢弃 {} 的定义", existing.source.file, n.source.file),
                    json!({"node": gid, "kept": existing.source, "dropped": n.source}),
                    vec![existing.source.clone(), n.source.clone()],
                ));
                continue;
            }
            kept.insert((fi, n.node_id.clone()), gid.clone());
            let bn_ns = if n.browse_name.ns == 0 {
                0
            } else {
                maps[fi].get(n.browse_name.ns as usize - 1).copied().unwrap_or(n.browse_name.ns)
            };
            nodes.insert(
                gid.clone(),
                MergedNode {
                    id: gid,
                    node_class: n.node_class.clone(),
                    browse_name: format!("{}:{}", bn_ns, n.browse_name.name),
                    browse_name_raw: n.browse_name.raw.clone(),
                    display_name: n.display_name.clone(),
                    description: n.description.clone(),
                    attributes: n.attributes.clone(),
                    extensions: n.extensions.clone(),
                    source: n.source.clone(),
                    origin_file: f.file_name.clone(),
                },
            );
        }
    }

    // 第三遍：reference 规范化。inverse 归一为 forward 方向，但不补造未声明的边。
    let mut edges: Vec<Edge> = Vec::new();
    for (fi, f) in files.iter().enumerate() {
        for n in &f.nodes {
            let Some(gid) = kept.get(&(fi, n.node_id.clone())).cloned() else { continue };
            for r in &n.references {
                let ref_type = resolve_name(f, &r.ref_type_raw, &mut events, &n.source)
                    .and_then(|id| remap(&id, &maps[fi], f, &mut events, &n.source))
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| format!("unresolved:{}", r.ref_type_raw));
                let target = resolve_name(f, &r.target_raw, &mut events, &n.source)
                    .and_then(|id| remap(&id, &maps[fi], f, &mut events, &n.source));
                let target = target.map(|t| t.to_string()).unwrap_or_else(|| format!("unresolved:{}", r.target_raw));
                let (from, to, inv) = if r.is_forward {
                    (gid.clone(), target, false)
                } else {
                    (target, gid.clone(), true)
                };
                edges.push(Edge {
                    from,
                    to,
                    ref_type,
                    normalized_from_inverse: inv,
                    resolved: true,
                    source: n.source.clone(),
                });
            }
        }
    }

    // 第四遍：悬空目标。ns=0 端点视为 OPC UA 内置基座，不算缺失。
    let mut unresolved: Vec<Unresolved> = Vec::new();
    let mut missing: BTreeMap<String, Option<SourceSpan>> = BTreeMap::new();
    for e in &mut edges {
        for endpoint in [&e.from, &e.to] {
            if endpoint.starts_with("unresolved:") {
                e.resolved = false;
                unresolved.push(Unresolved {
                    kind: "UnresolvedReference".into(),
                    detail: format!("引用端点 `{endpoint}` 无法解析（alias 循环或非法 NodeId）"),
                    source: Some(e.source.clone()),
                });
            } else if !endpoint.starts_with("ns=0;") && !nodes.contains_key(endpoint) {
                e.resolved = false;
                missing.entry(endpoint.clone()).or_insert_with(|| Some(e.source.clone()));
            }
        }
    }
    for (gid, span) in missing {
        unresolved.push(Unresolved {
            kind: "DanglingReference".into(),
            detail: format!("引用目标 {gid} 在合并模型中不存在（删除后悬空或依赖未加载），保留 unresolved 节点"),
            source: span.clone(),
        });
        events.push(Event::new(
            "DanglingReference",
            format!("悬空引用: {gid} 被引用但不存在，保留 unresolved 占位节点"),
            json!({"missing_node": gid}),
            span.clone().into_iter().collect(),
        ));
        nodes.insert(
            gid.clone(),
            MergedNode {
                id: gid.clone(),
                node_class: "Unresolved".into(),
                browse_name: gid.clone(),
                browse_name_raw: gid,
                display_name: String::new(),
                description: None,
                attributes: vec![("unresolved".into(), "true".into())],
                extensions: vec![],
                source: span.unwrap_or(SourceSpan { file: String::new(), start: 0, end: 0 }),
                origin_file: String::new(),
            },
        );
    }

    // 第五遍：model dependency 校验。版本冲突与缺失都保留 unresolved，不吞掉。
    let models: Vec<ModelMeta> = files.iter().flat_map(|f| f.models.clone()).collect();
    for m in &models {
        for req in &m.required_models {
            if req.model_uri == "http://opcfoundation.org/UA/" {
                continue; // 基座模型视为环境自带
            }
            match models.iter().find(|x| x.model_uri == req.model_uri) {
                None => {
                    unresolved.push(Unresolved {
                        kind: "MissingDependency".into(),
                        detail: format!("模型 {} 依赖 {}（版本 {}），未加载", m.model_uri, req.model_uri, req.version),
                        source: None,
                    });
                    events.push(Event::new(
                        "MissingDependency",
                        format!("依赖缺失: {} 需要 {} {}", m.model_uri, req.model_uri, req.version),
                        json!({"model": m.model_uri, "required_uri": req.model_uri, "required_version": req.version}),
                        vec![],
                    ));
                }
                Some(found) if found.version != req.version => {
                    unresolved.push(Unresolved {
                        kind: "DependencyConflict".into(),
                        detail: format!(
                            "模型 {} 需要 {} 版本 {}，实际加载 {}（{}）",
                            m.model_uri, req.model_uri, req.version, found.version, found.source_file
                        ),
                        source: None,
                    });
                    events.push(Event::new(
                        "DependencyConflict",
                        format!("依赖版本冲突: {} 需要 {} {}，实际 {}", m.model_uri, req.model_uri, req.version, found.version),
                        json!({
                            "model": m.model_uri,
                            "required_uri": req.model_uri,
                            "required_version": req.version,
                            "loaded_version": found.version,
                            "loaded_from": found.source_file,
                        }),
                        vec![],
                    ));
                }
                _ => {}
            }
        }
    }

    // 未识别扩展保留记录。
    for f in files {
        let node_ext: usize = f.nodes.iter().map(|n| n.extensions.len()).sum();
        if node_ext > 0 || !f.file_extensions.is_empty() {
            events.push(Event::new(
                "UnknownExtensionPreserved",
                format!("{}: 原样保留 {} 个节点级扩展与 {} 个文件级扩展", f.file_name, node_ext, f.file_extensions.len()),
                json!({"file": f.file_name, "node_extensions": node_ext, "file_extensions": f.file_extensions.len()}),
                vec![],
            ));
        }
    }

    MergedModel {
        namespaces,
        nodes,
        edges,
        unresolved,
        events,
        models,
        files: files.iter().map(|f| f.file_name.clone()).collect(),
    }
}

/// alias 解析：目标可能是 NodeId 或另一个 alias；循环时记录事件并放弃，不猜。
fn resolve_name(file: &NodeSetFile, raw: &str, events: &mut Vec<Event>, span: &SourceSpan) -> Option<NodeId> {
    if let Some(id) = parse_node_id(raw) {
        return Some(id);
    }
    let mut chain: Vec<String> = vec![raw.to_string()];
    let mut cur = raw.to_string();
    loop {
        match file.aliases.iter().find(|(n, _)| *n == cur) {
            None => {
                events.push(Event::new(
                    "UnresolvedAlias",
                    format!("{}: 名称 `{cur}` 既不是 NodeId 也不是已定义 alias", file.file_name),
                    json!({"file": file.file_name, "name": cur, "chain": chain}),
                    vec![span.clone()],
                ));
                return None;
            }
            Some((_, target)) => {
                if let Some(id) = parse_node_id(target) {
                    return Some(id);
                }
                if chain.contains(target) {
                    chain.push(target.clone());
                    events.push(Event::new(
                        "AliasCycle",
                        format!("{}: alias 循环 {}", file.file_name, chain.join(" -> ")),
                        json!({"file": file.file_name, "chain": chain}),
                        vec![span.clone()],
                    ));
                    return None;
                }
                chain.push(target.clone());
                cur = target.clone();
            }
        }
    }
}

/// 单文件 ns index → 合并表 index。越界 index 记录事件并放弃该引用。
fn remap(id: &NodeId, map: &[u16], file: &NodeSetFile, events: &mut Vec<Event>, span: &SourceSpan) -> Option<NodeId> {
    if id.ns == 0 {
        return Some(id.clone());
    }
    let idx = id.ns as usize;
    if idx >= 1 && idx <= map.len() {
        Some(NodeId { ns: map[idx - 1], ident: id.ident.clone() })
    } else {
        events.push(Event::new(
            "UnknownNamespaceIndex",
            format!("{}: ns={} 超出本文件 namespace 表（共 {} 项）", file.file_name, id.ns, map.len()),
            json!({"file": file.file_name, "node_id": id.to_string(), "table_size": map.len()}),
            vec![span.clone()],
        ));
        None
    }
}
