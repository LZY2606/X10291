//! 合并：数字 namespace index 只在单文件内有意义，合并按 URI 重映射；
//! alias 循环保留并诊断；重复 NodeId 保留全部候选；缺失目标保留 unresolved 占位。

use std::collections::HashMap;

use crate::model::*;

pub struct Merger {
    pub global_uris: Vec<String>,
    uri_index: HashMap<String, u16>,
}

impl Merger {
    pub fn new() -> Self {
        let mut m = Merger { global_uris: Vec::new(), uri_index: HashMap::new() };
        m.intern(UA_NS_URI);
        m
    }

    fn intern(&mut self, uri: &str) -> u16 {
        if let Some(i) = self.uri_index.get(uri) {
            return *i;
        }
        let i = self.global_uris.len() as u16;
        self.global_uris.push(uri.to_string());
        self.uri_index.insert(uri.to_string(), i);
        i
    }

    /// 按导入顺序合并多个 NodeSet。
    pub fn merge(&mut self, sets: &[NodeSet]) -> MergedModel {
        let mut ns_maps = Vec::new();
        let mut nodes: Vec<MergedNode> = Vec::new();
        let mut aliases = Vec::new();
        let mut dependencies = Vec::new();
        let mut diagnostics = Vec::new();

        // 每个文件：局部 index -> 全局 index。
        let mut file_maps: Vec<HashMap<u16, u16>> = Vec::new();
        for set in sets {
            let mut map = HashMap::new();
            map.insert(0u16, 0u16);
            map.insert(1u16, 0u16);
            let mut entries = vec![(0u16, 0u16, UA_NS_URI.to_string()), (1, 0, UA_NS_URI.to_string())];
            for (k, uri) in set.namespace_uris.iter().enumerate() {
                let local = (k + 2) as u16;
                let g = self.intern(uri);
                map.insert(local, g);
                entries.push((local, g, uri.clone()));
            }
            // 文件自身 ModelUri 若不在 NamespaceUris 中，也登记（诊断但不丢弃）。
            if let Some(own) = set.models.first() {
                if !set.namespace_uris.contains(&own.model_uri) {
                    let g = self.intern(&own.model_uri);
                    diagnostics.push(Diagnostic {
                        kind: "model_uri_not_in_table".into(),
                        message: format!("ModelUri {} 未出现在 NamespaceUris，已按 URI 登记为全局 ns={g}", own.model_uri),
                        node: None,
                        source: set.filename.clone(),
                        span: None,
                    });
                }
            }
            ns_maps.push(NsMap { source: set.filename.clone(), entries });
            file_maps.push(map);
        }

        // alias 解析（含循环检测）。
        for set in sets {
            let alias_map: HashMap<&str, &str> = set
                .aliases
                .iter()
                .map(|(n, t)| (n.as_str(), t.as_str()))
                .collect();
            for (name, raw) in &set.aliases {
                let status = resolve_alias(&alias_map, raw);
                if let AliasStatus::Cyclic(ref chain) = status {
                    diagnostics.push(Diagnostic {
                        kind: "alias_cycle".into(),
                        message: format!("alias 循环：{} -> {}", name, chain.join(" -> ")),
                        node: None,
                        source: set.filename.clone(),
                        span: None,
                    });
                }
                aliases.push(ResolvedAlias {
                    source: set.filename.clone(),
                    name: name.clone(),
                    raw_target: raw.clone(),
                    status,
                });
            }
        }

        // 节点与引用重映射。
        for (fi, set) in sets.iter().enumerate() {
            let map = &file_maps[fi];
            let alias_map: HashMap<&str, &str> = set
                .aliases
                .iter()
                .map(|(n, t)| (n.as_str(), t.as_str()))
                .collect();
            for n in &set.nodes {
                let gid = remap_nodeid(&n.node_id, map);
                let mut mrefs = Vec::new();
                for r in &n.references {
                    let rt = resolve_ref_endpoint(&r.ref_type_raw, &alias_map, map);
                    let tg = resolve_ref_endpoint(&r.target_raw, &alias_map, map);
                    match (rt, tg) {
                        (Some(rt), Some(tg)) => mrefs.push(MergedRef {
                            ref_type: rt,
                            ref_type_raw: r.ref_type_raw.clone(),
                            is_forward: r.is_forward,
                            target_stable: String::new(),
                            target: tg,
                            target_raw: r.target_raw.clone(),
                            resolved: true,
                        }),
                        (rt, tg) => {
                            diagnostics.push(Diagnostic {
                                kind: "unresolvable_reference".into(),
                                message: format!(
                                    "引用无法解析：type={} target={}",
                                    r.ref_type_raw, r.target_raw
                                ),
                                node: Some(gid.to_local_string()),
                                source: set.filename.clone(),
                                span: Some(n.span),
                            });
                            let fallback = tg.or(rt).unwrap_or(NodeId { ns: 0, ident: Ident::Str("?".into()) });
                            mrefs.push(MergedRef {
                                ref_type: rt.unwrap_or_else(|| NodeId { ns: 0, ident: Ident::Str(r.ref_type_raw.clone()) }),
                                ref_type_raw: r.ref_type_raw.clone(),
                                is_forward: r.is_forward,
                                target_stable: String::new(),
                                target: fallback,
                                target_raw: r.target_raw.clone(),
                                resolved: false,
                            });
                        }
                    }
                }
                let stable = gid.to_stable_string(&self.global_uris);
                let mut mrefs = mrefs;
                for r in &mut mrefs {
                    r.target_stable = r.target.to_stable_string(&self.global_uris);
                }
                nodes.push(MergedNode {
                    id: gid,
                    stable_id: stable,
                    browse_name: (remap_ns(n.browse_name.0, map), n.browse_name.1.clone()),
                    node_class: n.node_class.clone(),
                    display_name: n.display_name.clone(),
                    description: n.description.clone(),
                    attrs: n.attrs.clone(),
                    references: mrefs,
                    extensions: n.extensions.clone(),
                    raw_xml: n.raw_xml.clone(),
                    span: n.span,
                    source: n.source.clone(),
                    status: NodeStatus::Ok,
                });
            }
            diagnostics.extend(set.diagnostics.iter().cloned());
        }

        // 跨文件重复 NodeId：保留全部候选。
        let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, n) in nodes.iter().enumerate() {
            by_id.entry(n.stable_id.clone()).or_default().push(i);
        }
        let mut duplicate_groups = Vec::new();
        for (sid, idxs) in &by_id {
            if idxs.len() > 1 {
                for i in idxs {
                    nodes[*i].status = NodeStatus::Duplicate;
                }
                let candidates = idxs
                    .iter()
                    .map(|i| DuplicateCandidate {
                        source: nodes[*i].source.clone(),
                        span: nodes[*i].span,
                        node_class: nodes[*i].node_class.clone(),
                        display_name: nodes[*i].display_name.clone(),
                    })
                    .collect();
                diagnostics.push(Diagnostic {
                    kind: "duplicate_nodeid".into(),
                    message: format!("重复 NodeId {sid}，保留 {} 个候选", idxs.len()),
                    node: Some(sid.clone()),
                    source: "merge".into(),
                    span: None,
                });
                duplicate_groups.push(DuplicateGroup { stable_id: sid.clone(), candidates });
            }
        }

        // 悬空引用：为目标生成 unresolved 占位节点（不制造缺边，只标记缺失）。
        let known: std::collections::HashSet<String> = nodes.iter().map(|n| n.stable_id.clone()).collect();
        let mut dangling: Vec<String> = Vec::new();
        for n in &nodes {
            for r in &n.references {
                if r.resolved && !known.contains(&r.target_stable) && !dangling.contains(&r.target_stable) {
                    dangling.push(r.target_stable.clone());
                }
            }
        }
        for sid in &dangling {
            diagnostics.push(Diagnostic {
                kind: "dangling_reference".into(),
                message: format!("引用目标 {sid} 缺失，保留 unresolved 占位节点"),
                node: Some(sid.clone()),
                source: "merge".into(),
                span: None,
            });
            let id = parse_stable(sid, &self.global_uris)
                .unwrap_or(NodeId { ns: 0, ident: Ident::Str(sid.clone()) });
            nodes.push(MergedNode {
                id,
                stable_id: sid.clone(),
                browse_name: (0, format!("unresolved:{sid}")),
                node_class: "UnresolvedPlaceholder".into(),
                display_name: format!("未解析 {sid}"),
                description: Some("由悬空引用生成的占位节点；依赖缺失时保留".into()),
                attrs: vec![],
                references: vec![],
                extensions: vec![],
                raw_xml: String::new(),
                span: (0, 0),
                source: "merge".into(),
                status: NodeStatus::Unresolved,
            });
        }

        // 模型依赖状态。
        let loaded: HashMap<String, String> = sets
            .iter()
            .filter_map(|s| s.models.first().map(|m| (m.model_uri.clone(), m.version.clone())))
            .collect();
        for set in sets {
            for (di, dep) in set.models.iter().enumerate() {
                if di == 0 {
                    continue; // 第一个 Model 是文件自身
                }
                let status = match loaded.get(&dep.model_uri) {
                    None => DepStatus::Missing,
                    Some(v) if *v == dep.version => DepStatus::Satisfied { loaded_version: v.clone() },
                    Some(v) => DepStatus::VersionConflict { loaded_version: v.clone() },
                };
                match &status {
                    DepStatus::Missing => diagnostics.push(Diagnostic {
                        kind: "dependency_missing".into(),
                        message: format!("依赖 {} ({}) 缺失，保留 unresolved", dep.model_uri, dep.version),
                        node: None,
                        source: set.filename.clone(),
                        span: None,
                    }),
                    DepStatus::VersionConflict { loaded_version } => diagnostics.push(Diagnostic {
                        kind: "dependency_version_conflict".into(),
                        message: format!(
                            "依赖版本冲突：{} 需要 {}，已加载 {}",
                            dep.model_uri, dep.version, loaded_version
                        ),
                        node: None,
                        source: set.filename.clone(),
                        span: None,
                    }),
                    _ => {}
                }
                dependencies.push(DependencyStatus {
                    required_by: set.filename.clone(),
                    dep: dep.clone(),
                    status,
                });
            }
        }

        MergedModel {
            global_uris: self.global_uris.clone(),
            ns_maps,
            nodes,
            aliases,
            dependencies,
            duplicate_groups,
            diagnostics,
        }
    }
}

fn remap_ns(local: u16, map: &HashMap<u16, u16>) -> u16 {
    *map.get(&local).unwrap_or(&local)
}

fn remap_nodeid(id: &NodeId, map: &HashMap<u16, u16>) -> NodeId {
    NodeId { ns: remap_ns(id.ns, map), ident: id.ident.clone() }
}

/// alias 解析：alias 的目标可以是 NodeId 字符串或另一个 alias 名；循环时保留整条链。
pub fn resolve_alias(alias_map: &HashMap<&str, &str>, raw: &str) -> AliasStatus {
    if NodeId::parse(raw).is_some() {
        return AliasStatus::Resolved(raw.to_string());
    }
    let mut chain = vec![raw.to_string()];
    let mut cur = raw.to_string();
    for _ in 0..64 {
        match alias_map.get(cur.as_str()) {
            None => return AliasStatus::Unresolved,
            Some(next) => {
                if NodeId::parse(next).is_some() {
                    return AliasStatus::Resolved(next.to_string());
                }
                if chain.iter().any(|c| c == *next) {
                    chain.push(next.to_string());
                    return AliasStatus::Cyclic(chain);
                }
                chain.push(next.to_string());
                cur = next.to_string();
            }
        }
    }
    AliasStatus::Cyclic(chain)
}

fn resolve_ref_endpoint(
    raw: &str,
    alias_map: &HashMap<&str, &str>,
    map: &HashMap<u16, u16>,
) -> Option<NodeId> {
    if let Some(id) = NodeId::parse(raw) {
        return Some(remap_nodeid(&id, map));
    }
    match resolve_alias(alias_map, raw) {
        AliasStatus::Resolved(s) => NodeId::parse(&s).map(|id| remap_nodeid(&id, map)),
        _ => None,
    }
}

/// 从稳定身份（nsu=uri;...）还原全局 NodeId。
pub fn parse_stable(s: &str, global_uris: &[String]) -> Option<NodeId> {
    if let Some(rest) = s.strip_prefix("nsu=") {
        let (uri, body) = rest.split_once(';')?;
        let ns = global_uris.iter().position(|u| u == uri)? as u16;
        let mut id = NodeId::parse(body)?;
        id.ns = ns;
        Some(id)
    } else {
        NodeId::parse(s)
    }
}
