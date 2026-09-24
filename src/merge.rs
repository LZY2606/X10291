//! 跨文件合并语义。
//!
//! 关键不变量：
//! 1. 数字 namespace index 只在单文件内有意义，合并按 URI 重映射。
//! 2. inverse / forward reference 规范化为统一方向，但不制造缺边。
//! 3. 别名经全局别名表解析，检测别名循环与未定义别名，不猜测。
//! 4. 重复 NodeId 产生显式冲突，不静默覆盖。
//! 5. 依赖缺失/版本冲突保留并诊断；未识别扩展与领域量原样保留。

use std::collections::HashMap;

use crate::model::*;

/// 诊断码（机器可读事件依据）。
pub mod codes {
    pub const NS_INDEX_UNBOUND: &str = "NS_INDEX_UNBOUND";
    pub const ALIAS_CYCLE: &str = "ALIAS_CYCLE";
    pub const ALIAS_UNDEFINED: &str = "ALIAS_UNDEFINED";
    pub const DUPLICATE_NODEID: &str = "DUPLICATE_NODEID";
    pub const INVERSE_NORMALIZED: &str = "INVERSE_NORMALIZED";
    pub const DANGLING_REFERENCE: &str = "DANGLING_REFERENCE";
    pub const UNRESOLVED_NODE: &str = "UNRESOLVED_NODE";
    pub const DEP_MISSING: &str = "DEP_MISSING";
    pub const DEP_VERSION_CONFLICT: &str = "DEP_VERSION_CONFLICT";
    pub const UNKNOWN_EXTENSION: &str = "UNKNOWN_EXTENSION";
    pub const QUANTITY_UNCONVERTIBLE: &str = "QUANTITY_UNCONVERTIBLE";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub source_id: String,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<RawRange>,
}

/// 全局节点身份：URI + 标识符。对 Opaque 标识符也成立（按文本）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GlobalId {
    pub namespace_uri: String,
    pub id_code: String,
    pub id_payload: String,
}

impl std::fmt::Display for GlobalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}|{}={}", self.namespace_uri, self.id_code, self.id_payload)
    }
}

impl GlobalId {
    /// 稳定内容寻址 id（短、跨导入稳定）。
    pub fn stable(&self) -> String {
        let mut h: u64 = 1469598103934665603;
        let bytes = self.to_string();
        for b in bytes.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        format!("n{:016x}", h)
    }
}

/// 规范化后的全局引用边。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// 稳定边 id：源->类型->目标，方向已统一为 forward。
    pub edge_id: String,
    pub source_node: String,
    pub target_node: String,
    pub reference_type: String,
    /// 目标在合并时是否尚不存在（悬空）。不制造缺边，边仍保留。
    pub target_unresolved: bool,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<RawRange>,
}

/// 合并后的逻辑节点（类型/实例统一对象）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedNode {
    pub stable_id: String,
    pub global: GlobalId,
    pub class: String,
    pub browse_name: Option<String>,
    pub display_name: Option<String>,
    pub is_type: bool,
    /// 提供该节点的来源（重复 NodeId 时可能多个）。
    pub origins: Vec<NodeOrigin>,
    #[serde(default)]
    pub quantities: Vec<DomainQuantity>,
    #[serde(default)]
    pub unknown_extensions: Vec<UnknownExtension>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeOrigin {
    pub source_id: String,
    pub raw_stable_id: String,
    pub ord: usize,
    pub range: RawRange,
}

/// 未解析（悬空目标）占位节点：保留 unresolved，不画丢。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedNode {
    pub stable_id: String,
    pub global: GlobalId,
    pub referenced_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepRequirement {
    pub required_by: String,
    pub model_uri: String,
    pub wanted_version: Option<String>,
    pub range: RawRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPresent {
    pub model_uri: String,
    pub version: Option<String>,
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedModel {
    /// source_id -> 该文件 URI 列表（按文件内 index 顺序）。
    pub namespace_map: Vec<NamespaceMapping>,
    pub global_aliases: Vec<GlobalAlias>,
    pub nodes: Vec<MergedNode>,
    pub edges: Vec<Edge>,
    pub unresolved: Vec<UnresolvedNode>,
    pub models_present: Vec<ModelPresent>,
    pub requirements: Vec<DepRequirement>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceMapping {
    pub source_id: String,
    pub file_index: u32,
    pub uri: String,
    pub range: RawRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalAlias {
    pub name: String,
    pub source_id: String,
    /// 解析后的全局引用类型 id；循环/未定义时为空。
    pub resolved_type: Option<String>,
    pub state: AliasState,
    pub range: RawRange,
}

#[derive(Debug,Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AliasState {
    Resolved,
    Cycle(Vec<String>),
    Undefined,
}

/// 合并多份已解析文档。`docs` 的顺序即来源顺序（s0001..）。
pub fn merge_documents(docs: &[RawDocument]) -> MergedModel {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut namespace_map: Vec<NamespaceMapping> = Vec::new();

    // 1) namespace：保留每个文件的 index->URI 映射，合并按 URI 对齐。
    for d in docs {
        for ns in &d.namespaces {
            namespace_map.push(NamespaceMapping {
                source_id: d.source_id.clone(),
                file_index: ns.file_index,
                uri: ns.uri.clone(),
                range: ns.range.clone(),
            });
        }
    }

    // 2) 全局别名表：alias 名 -> 候选条目。重复别名也保留（多解）。
    // 先建立 (source, alias) -> 文件内 NodeId 文本 的索引。
    #[derive(Clone)]
    struct ARef {
        source: String,
        alias: String,
        node: NodeId,
        range: RawRange,
    }
    let mut alias_by_name: HashMap<String, Vec<ARef>> = HashMap::new();
    for d in docs {
        for a in &d.aliases {
            alias_by_name
                .entry(a.alias.clone())
                .or_default()
                .push(ARef {
                    source: d.source_id.clone(),
                    alias: a.alias.clone(),
                    node: a.node_id.clone(),
                    range: a.range.clone(),
                });
        }
    }

    // 文件内把 NodeId 提升为 GlobalId 的闭包。
    let lift = |source_id: &str, nid: &NodeId| -> Result<GlobalId, String> {
        let doc = docs.iter().find(|d| d.source_id == source_id);
        let doc = doc.ok_or_else(|| "source missing".to_string())?;
        let uri = doc
            .uri_for_index(nid.namespace_index)
            .map(str::to_string)
            .ok_or_else(|| format!("namespace index ns={} unbound", nid.namespace_index))?;
        Ok(GlobalId {
            namespace_uri: uri,
            id_code: nid.identifier.code().to_string(),
            id_payload: nid.identifier.payload(),
        })
    };

    // 3) 解析别名（含循环检测）。别名可指向别名：按 (source,alias) 链追踪。
    let mut global_aliases: Vec<GlobalAlias> = Vec::new();
    let mut resolved_alias_type: HashMap<(String, String), Option<String>> = HashMap::new();
    for d in docs {
        for a in &d.aliases {
            let mut seen: Vec<String> = Vec::new();
            let mut cur_source = d.source_id.clone();
            let mut cur_name = a.alias.clone();
            // a.node_id 是该别名的初始目标（通常是真 NodeId；若又是别名文本则继续）。
            let mut cur_node: NodeId = a.node_id.clone();
            let state;
            let mut resolved: Option<String> = None;
            loop {
                let key = cur_source.clone();
                let _ = key;
                if seen.iter().any(|s| *s == format!("{}:{}", cur_source, cur_name)) {
                    seen.push(format!("{}:{}", cur_source, cur_name));
                    state = AliasState::Cycle(seen.clone());
                    diagnostics.push(Diagnostic {
                        code: codes::ALIAS_CYCLE.into(),
                        message: format!("alias cycle detected: {}", seen.join(" -> ")),
                        source_id: d.source_id.clone(),
                        subject: a.alias.clone(),
                        range: Some(a.range.clone()),
                    });
                    break;
                }
                seen.push(format!("{}:{}", cur_source, cur_name));
                // 若当前节点是 Opaque 且文本命中某别名，则继续追。
                if let Identifier::Opaque(text) = &cur_node.identifier {
                    if alias_by_name.contains_key(text) {
                        let cands = &alias_by_name[text];
                        // 别名默认在当前文件内解析。
                        if let Some(next) =
                            cands.iter().find(|c| c.source == cur_source).or_else(|| cands.first())
                        {
                            cur_source = next.source.clone();
                            cur_name = next.alias.clone();
                            cur_node = next.node.clone();
                            continue;
                        }
                    }
                    state = AliasState::Undefined;
                    diagnostics.push(Diagnostic {
                        code: codes::ALIAS_UNDEFINED.into(),
                        message: format!("alias '{}' undefined", text),
                        source_id: d.source_id.clone(),
                        subject: text.clone(),
                        range: Some(a.range.clone()),
                    });
                    break;
                }
                match lift(&cur_source, &cur_node) {
                    Ok(g) => {
                        resolved = Some(g.stable());
                        state = AliasState::Resolved;
                    }
                    Err(e) => {
                        state = AliasState::Undefined;
                        diagnostics.push(Diagnostic {
                            code: codes::NS_INDEX_UNBOUND.into(),
                            message: e,
                            source_id: d.source_id.clone(),
                            subject: cur_name.clone(),
                            range: Some(a.range.clone()),
                        });
                    }
                }
                break;
            }
            resolved_alias_type.insert((d.source_id.clone(), a.alias.clone()), resolved.clone());
            global_aliases.push(GlobalAlias {
                name: a.alias.clone(),
                source_id: d.source_id.clone(),
                resolved_type: resolved,
                state,
                range: a.range.clone(),
            });
        }
    }

    // 4) 节点：提升为 GlobalId 并做内容寻址 stable_id；重复 NodeId 记录冲突。
    let mut by_global: HashMap<String, Vec<usize>> = HashMap::new();
    let mut nodes: Vec<MergedNode> = Vec::new();

    for d in docs {
        for rn in &d.nodes {
            let global = match lift(d.source_id.clone().as_str(), &rn.node_id) {
                Ok(g) => g,
                Err(e) => {
                    diagnostics.push(Diagnostic {
                        code: codes::NS_INDEX_UNBOUND.into(),
                        message: e,
                        source_id: d.source_id.clone(),
                        subject: rn.node_id.raw.clone(),
                        range: Some(rn.range.clone()),
                    });
                    GlobalId {
                        namespace_uri: format!("<unbound:{}>", d.source_id),
                        id_code: rn.node_id.identifier.code().into(),
                        id_payload: rn.node_id.identifier.payload(),
                    }
                }
            };
            let stable = global.stable();
            let browse_uri = rn
                .browse_name
                .as_ref()
                .map(|qn| {
                    let uri = d.uri_for_index(qn.namespace_index).unwrap_or("");
                    format!("{}:{}", uri, qn.name)
                });
            let origin = NodeOrigin {
                source_id: d.source_id.clone(),
                raw_stable_id: format!("{}#{}", d.source_id, rn.ord),
                ord: rn.ord,
                range: rn.range.clone(),
            };
            let key = stable.clone();
            if let Some(list) = by_global.get(&key) {
                let first = list[0];
                nodes[first].origins.push(origin.clone());
                diagnostics.push(Diagnostic {
                    code: codes::DUPLICATE_NODEID.into(),
                    message: format!(
                        "duplicate NodeId {} also present in {}",
                        global, nodes[first].origins[0].source_id
                    ),
                    source_id: d.source_id.clone(),
                    subject: global.to_string(),
                    range: Some(rn.range.clone()),
                });
            } else {
                by_global.entry(key.clone()).or_default().push(nodes.len());
                nodes.push(MergedNode {
                    stable_id: stable,
                    global,
                    class: rn.class.label(),
                    browse_name: browse_uri,
                    display_name: rn.display_name.clone(),
                    is_type: rn.class.is_type(),
                    origins: vec![origin],
                    quantities: rn.quantities.clone(),
                    unknown_extensions: rn.unknown_extensions.clone(),
                });
            }
            // 未知扩展诊断（保真但提示）。
            for ux in &rn.unknown_extensions {
                diagnostics.push(Diagnostic {
                    code: codes::UNKNOWN_EXTENSION.into(),
                    message: format!("unrecognized extension <{}> preserved verbatim", ux.name),
                    source_id: d.source_id.clone(),
                    subject: rn.node_id.raw.clone(),
                    range: Some(ux.range.clone()),
                });
            }
            for q in &rn.quantities {
                if q.normalized_value.is_none() {
                    diagnostics.push(Diagnostic {
                        code: codes::QUANTITY_UNCONVERTIBLE.into(),
                        message: format!(
                            "quantity '{}' value '{}' (unit {:?}) not converted; kept in place",
                            q.kind, q.raw_value, q.raw_unit
                        ),
                        source_id: d.source_id.clone(),
                        subject: rn.node_id.raw.clone(),
                        range: Some(q.range.clone()),
                    });
                }
            }
        }
    }

    // 5) 引用：方向规范化为 forward；别名解析；悬空目标保留 unresolved。
    let mut present: HashMap<String, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        present.insert(n.stable_id.clone(), i);
    }
    let mut edges: Vec<Edge> = Vec::new();
    let mut edge_seen: HashMap<String, usize> = HashMap::new();
    let mut unresolved: Vec<UnresolvedNode> = Vec::new();
    let mut unresolved_seen: HashMap<String, usize> = HashMap::new();

    for d in docs {
        for rn in &d.nodes {
            let src_global = lift(d.source_id.clone().as_str(), &rn.node_id)
                .unwrap_or_else(|_| GlobalId {
                    namespace_uri: format!("<unbound:{}>", d.source_id),
                    id_code: rn.node_id.identifier.code().into(),
                    id_payload: rn.node_id.identifier.payload(),
                });
            let src_stable = src_global.stable();
            for r in &rn.references {
                let tgt_global = match lift(d.source_id.clone().as_str(), &r.target) {
                    Ok(g) => g,
                    Err(e) => {
                        diagnostics.push(Diagnostic {
                            code: codes::NS_INDEX_UNBOUND.into(),
                            message: e,
                            source_id: d.source_id.clone(),
                            subject: r.target.raw.clone(),
                            range: Some(r.range.clone()),
                        });
                        GlobalId {
                            namespace_uri: format!("<unbound:{}>", d.source_id),
                            id_code: r.target.identifier.code().into(),
                            id_payload: r.target.identifier.payload(),
                        }
                    }
                };
                let tgt_stable = tgt_global.stable();

                // 引用类型：别名优先。
                let type_stable = if let Some(alias) = &r.via_alias {
                    match resolved_alias_type.get(&(d.source_id.clone(), alias.clone())) {
                        Some(Some(id)) => id.clone(),
                        _ => GlobalId {
                            namespace_uri: format!("alias:{}", d.source_id),
                            id_code: "s".into(),
                            id_payload: alias.clone(),
                        }
                        .stable(),
                    }
                } else {
                    lift(d.source_id.clone().as_str(), &r.reference_type)
                        .map(|g| g.stable())
                        .unwrap_or_else(|_| r.reference_type.raw.clone())
                };

                // 方向规范化：inverse -> 交换端点。不制造新的引用语义。
                let (from, to) = if r.is_forward {
                    (src_stable.clone(), tgt_stable.clone())
                } else {
                    (tgt_stable.clone(), src_stable.clone())
                };
                if !r.is_forward {
                    diagnostics.push(Diagnostic {
                        code: codes::INVERSE_NORMALIZED.into(),
                        message: format!(
                            "inverse reference from {} normalized to forward {} -[{}]-> {}",
                            src_stable, from, type_stable, to
                        ),
                        source_id: d.source_id.clone(),
                        subject: r.target.raw.clone(),
                        range: Some(r.range.clone()),
                    });
                }

                // 目标缺失：保留占位 + 边，绝不丢边。
                if !present.contains_key(&to) && !unresolved_seen.contains_key(&to) {
                    unresolved_seen.insert(to.clone(), unresolved.len());
                    unresolved.push(UnresolvedNode {
                        stable_id: to.clone(),
                        global: tgt_global.clone(),
                        referenced_by: vec![from.clone()],
                    });
                    diagnostics.push(Diagnostic {
                        code: codes::UNRESOLVED_NODE.into(),
                        message: format!("reference target {} not defined; kept unresolved", to),
                        source_id: d.source_id.clone(),
                        subject: to.clone(),
                        range: Some(r.range.clone()),
                    });
                } else if let Some(ui) = unresolved_seen.get(&to) {
                    if !unresolved[*ui].referenced_by.contains(&from) {
                        unresolved[*ui].referenced_by.push(from.clone());
                    }
                }
                if !present.contains_key(&to) {
                    diagnostics.push(Diagnostic {
                        code: codes::DANGLING_REFERENCE.into(),
                        message: format!("dangling edge {} -[{}]-> {}", from, type_stable, to),
                        source_id: d.source_id.clone(),
                        subject: format!("{}->{}", from, to),
                        range: Some(r.range.clone()),
                    });
                }

                let edge_identity = format!("{}|{}|{}", from, type_stable, to);
                let mut h: u64 = 1469598103934665603;
                for b in edge_identity.bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(1099511628211);
                }
                let edge_id = format!("e{:016x}", h);
                if edge_seen.contains_key(&edge_id) {
                    continue;
                }
                edge_seen.insert(edge_id.clone(), edges.len());
                edges.push(Edge {
                    edge_id,
                    source_node: from,
                    target_node: to,
                    reference_type: type_stable,
                    target_unresolved: !present.contains_key(&to),
                    source_id: d.source_id.clone(),
                    range: Some(r.range.clone()),
                });
            }
        }
    }

    // 6) 模型依赖：在场模型 vs 需求；版本冲突显式保留。
    let mut models_present: Vec<ModelPresent> = Vec::new();
    let mut requirements: Vec<DepRequirement> = Vec::new();
    for d in docs {
        for m in &d.models {
            if !m.model_uri.is_empty() {
                models_present.push(ModelPresent {
                    model_uri: m.model_uri.clone(),
                    version: m.version.clone(),
                    source_id: d.source_id.clone(),
                });
            }
            for req in &m.required {
                requirements.push(DepRequirement {
                    required_by: d.source_id.clone(),
                    model_uri: req.model_uri.clone(),
                    wanted_version: req.version.clone(),
                    range: req.range.clone(),
                });
            }
        }
    }
    for req in &requirements {
        let offers: Vec<&ModelPresent> = models_present
            .iter()
            .filter(|p| p.model_uri == req.model_uri)
            .collect();
        if offers.is_empty() {
            diagnostics.push(Diagnostic {
                code: codes::DEP_MISSING.into(),
                message: format!("required model {} not imported", req.model_uri),
                source_id: req.required_by.clone(),
                subject: req.model_uri.clone(),
                range: Some(req.range.clone()),
            });
            continue;
        }
        if let Some(wanted) = &req.wanted_version {
            let mismatched: Vec<&&ModelPresent> =
                offers.iter().filter(|p| p.version.as_deref() != Some(wanted.as_str())).collect();
            if !mismatched.is_empty() {
                let got = offers
                    .iter()
                    .map(|p| p.version.clone().unwrap_or_else(|| "?".into()))
                    .collect::<Vec<_>>()
                    .join(",");
                diagnostics.push(Diagnostic {
                    code: codes::DEP_VERSION_CONFLICT.into(),
                    message: format!(
                        "model {} required {} but present {}",
                        req.model_uri, wanted, got
                    ),
                    source_id: req.required_by.clone(),
                    subject: req.model_uri.clone(),
                    range: Some(req.range.clone()),
                });
            }
        }
    }

    // 稳定排序：来源顺序 + 文件内 ord（身份与顺序可复现）。
    nodes.sort_by(|a, b| {
        a.origins[0]
            .source_id
            .cmp(&b.origins[0].source_id)
            .then(a.origins[0].ord.cmp(&b.origins[0].ord))
    });
    edges.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then(a.edge_id.cmp(&b.edge_id))
    });
    unresolved.sort_by(|a, b| a.stable_id.cmp(&b.stable_id));
    diagnostics.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then(a.code.cmp(&b.code))
            .then(a.subject.cmp(&b.subject))
    });

    MergedModel {
        namespace_map,
        global_aliases,
        nodes,
        edges,
        unresolved,
        models_present,
        requirements,
        diagnostics,
    }
}
