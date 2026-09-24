//! 合并层语义：
//! - 数字 namespace index 只在单文件内有意义，合并按 URI 重映射为全局 ns；
//! - inverse reference 与 forward reference 规范化为同一条边，但不制造缺边；
//! - 依赖缺失保留 unresolved 占位节点；
//! - 多解/不确定状态（重复 NodeId、依赖版本冲突）全部保留候选，不吞掉。

use crate::model::{normalize_quantity, parse_browse_name, parse_node_id, Diagnostic, NodeId, Quantity};
use crate::xml_import::{RawExtension, SourceDoc};
use serde_json::json;
use std::collections::HashMap;

pub const CORE_URI: &str = "http://opcfoundation.org/UA/";

#[derive(Clone, Debug)]
pub struct NsEntry {
    pub doc_id: String,
    pub idx_file: u16,
    pub uri: String,
    pub gns: u16,
}

#[derive(Clone, Debug)]
pub struct MergedNode {
    pub doc_id: String,
    pub gnodeid: String,
    pub file_nodeid: String,
    pub ns_uri: String,
    pub ns_idx_file: u16,
    pub browse_name: String,
    pub node_class: String,
    pub display_name: Option<String>,
    pub value: Option<String>,
    pub xml_start: usize,
    pub xml_end: usize,
    pub raw_xml: String,
    pub attrs_json: String,
}

#[derive(Clone, Debug)]
pub struct MergedRef {
    pub doc_id: String,
    pub src: String,
    pub dst: String,
    pub ref_type: String,
    pub forward_declared: bool,
    pub status: String, // ok | dangling | ambiguous | unresolved_alias | unresolved_ns
    pub candidates: Vec<String>,
    pub xml_start: usize,
    pub xml_end: usize,
}

#[derive(Clone, Debug)]
pub struct MergedAlias {
    pub doc_id: String,
    pub alias: String,
    pub target: String,
    pub status: String, // ok | cycle
}

#[derive(Clone, Debug)]
pub struct MergedDep {
    pub doc_id: String,
    pub model_uri: String,
    pub version_req: String,
    pub status: String, // resolved | conflict | unresolved
    pub detail: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct MergedModel {
    pub ns_entries: Vec<NsEntry>,
    pub nodes: Vec<MergedNode>,
    pub refs: Vec<MergedRef>,
    pub aliases: Vec<MergedAlias>,
    pub dependencies: Vec<MergedDep>,
    pub diagnostics: Vec<Diagnostic>,
    pub quantities: Vec<Quantity>,
    pub extensions: Vec<(String, RawExtension)>,
}

struct Ctx<'a> {
    docs: &'a [(String, SourceDoc)],
    /// (doc_idx, file_ns) -> gns
    ns_map: HashMap<(usize, u16), u16>,
    gns_uri: HashMap<u16, String>,
    diagnostics: Vec<Diagnostic>,
    diag_seq: usize,
}

impl<'a> Ctx<'a> {
    fn diag(&mut self, kind: &str, message: String, doc: Option<&str>, range: Option<(usize, usize)>, obj: Option<String>) {
        self.diag_seq += 1;
        self.diagnostics.push(Diagnostic {
            diag_id: format!("d{:04}", self.diag_seq),
            kind: kind.to_string(),
            message,
            doc_id: doc.map(|s| s.to_string()),
            xml_start: range.map(|r| r.0),
            xml_end: range.map(|r| r.1),
            object_ref: obj,
        });
    }

    /// 文件内 NodeId 文本 -> 全局 NodeId 字符串；alias 名递归解析（带循环检测）。
    fn resolve(
        &mut self,
        doc_idx: usize,
        text: &str,
        depth: &mut Vec<String>,
    ) -> Result<String, String> {
        let (doc_id, doc) = &self.docs[doc_idx];
        if let Some(nid) = parse_node_id(text) {
            let gns = self.gns_for(doc_idx, nid.ns).ok_or_else(|| {
                format!("unresolved_ns: 文件 {} 的 namespace index {} 无对应 URI", doc_id, nid.ns)
            })?;
            return Ok(NodeId { ns: gns, kind: nid.kind }.to_string());
        }
        // 视为 alias
        let alias = text.trim();
        if depth.iter().any(|a| a == alias) {
            let mut cycle = depth.clone();
            cycle.push(alias.to_string());
            return Err(format!("alias_cycle: {}", cycle.join(" -> ")));
        }
        let target = doc
            .aliases
            .iter()
            .find(|a| a.alias == alias)
            .map(|a| a.target.clone())
            .ok_or_else(|| format!("unknown_alias: {}", alias))?;
        depth.push(alias.to_string());
        self.resolve(doc_idx, &target, depth)
    }

    fn gns_for(&self, doc_idx: usize, file_ns: u16) -> Option<u16> {
        if file_ns == 0 {
            return Some(0);
        }
        self.ns_map.get(&(doc_idx, file_ns)).copied()
    }
}

pub fn merge(docs: &[(String, SourceDoc)]) -> MergedModel {
    // 1) 按 URI 分配全局 namespace（确定性顺序：文档顺序 -> 表内顺序）。
    let mut uri_gns: HashMap<String, u16> = HashMap::new();
    uri_gns.insert(CORE_URI.to_string(), 0);
    let mut next_gns: u16 = 1;
    let mut ns_entries = Vec::new();
    let mut ns_map = HashMap::new();
    let mut gns_uri: HashMap<u16, String> = HashMap::new();
    gns_uri.insert(0, CORE_URI.to_string());
    for (doc_idx, (doc_id, doc)) in docs.iter().enumerate() {
        for (i, uri) in doc.namespaces.iter().enumerate() {
            let idx_file = (i + 1) as u16;
            let gns = *uri_gns.entry(uri.clone()).or_insert_with(|| {
                let g = next_gns;
                next_gns += 1;
                g
            });
            gns_uri.insert(gns, uri.clone());
            ns_map.insert((doc_idx, idx_file), gns);
            ns_entries.push(NsEntry { doc_id: doc_id.clone(), idx_file, uri: uri.clone(), gns });
        }
    }

    let mut ctx = Ctx { docs, ns_map, gns_uri, diagnostics: Vec::new(), diag_seq: 0 };
    let mut nodes: Vec<MergedNode> = Vec::new();
    let mut refs: Vec<MergedRef> = Vec::new();
    let mut aliases: Vec<MergedAlias> = Vec::new();
    let mut dependencies: Vec<MergedDep> = Vec::new();
    let mut quantities: Vec<Quantity> = Vec::new();
    let mut extensions: Vec<(String, RawExtension)> = Vec::new();

    // 2) alias 状态（循环检测）。
    for (doc_idx, (doc_id, doc)) in docs.iter().enumerate() {
        for a in &doc.aliases {
            let mut depth = Vec::new();
            let status = match ctx.resolve(doc_idx, &a.alias, &mut depth) {
                Ok(_) => "ok".to_string(),
                Err(e) if e.starts_with("alias_cycle") => {
                    ctx.diag("alias_cycle", format!("alias 循环：{}（文件 {}）", e, doc_id), Some(doc_id), Some(a.range), Some(a.alias.clone()));
                    "cycle".to_string()
                }
                Err(_) => "unresolved".to_string(),
            };
            aliases.push(MergedAlias { doc_id: doc_id.clone(), alias: a.alias.clone(), target: a.target.clone(), status });
        }
    }

    // 3) 节点重映射 + 重复 NodeId 检测。
    let mut by_gid: HashMap<String, Vec<usize>> = HashMap::new();
    for (doc_idx, (doc_id, doc)) in docs.iter().enumerate() {
        for n in &doc.nodes {
            let (gnodeid, ns_uri, ns_idx_file) = match parse_node_id(&n.node_id) {
                Some(nid) => match ctx.gns_for(doc_idx, nid.ns) {
                    Some(gns) => {
                        let uri = ctx.gns_uri.get(&gns).cloned().unwrap_or_default();
                        (NodeId { ns: gns, kind: nid.kind }.to_string(), uri, nid.ns)
                    }
                    None => {
                        ctx.diag("unresolved_ns", format!("节点 {} 的 namespace index {} 在文件 {} 中无对应 URI", n.node_id, nid.ns, doc_id), Some(doc_id), Some(n.range), Some(n.node_id.clone()));
                        (format!("unresolved:{}", n.node_id), String::new(), nid.ns)
                    }
                },
                None => {
                    ctx.diag("invalid_node_id", format!("无法解析 NodeId: {}", n.node_id), Some(doc_id), Some(n.range), Some(n.node_id.clone()));
                    (format!("invalid:{}", n.node_id), String::new(), 0)
                }
            };
            let bn = parse_browse_name(&n.browse_name);
            let browse_gns = ctx.gns_for(doc_idx, bn.ns).unwrap_or(bn.ns);
            let idx = nodes.len();
            by_gid.entry(gnodeid.clone()).or_default().push(idx);
            nodes.push(MergedNode {
                doc_id: doc_id.clone(),
                gnodeid,
                file_nodeid: n.node_id.clone(),
                ns_uri,
                ns_idx_file,
                browse_name: format!("{}:{}", browse_gns, bn.name),
                node_class: n.node_class.clone(),
                display_name: n.display_name.clone(),
                value: n.value.clone(),
                xml_start: n.range.0,
                xml_end: n.range.1,
                raw_xml: n.raw_xml.clone(),
                attrs_json: serde_json::to_string(&n.attrs).unwrap_or_default(),
            });
        }
    }
    for (gid, idxs) in &by_gid {
        if idxs.len() > 1 {
            let candidates: Vec<String> = idxs.iter().map(|i| format!("{}:{}", nodes[*i].doc_id, gid)).collect();
            ctx.diag(
                "duplicate_node_id",
                format!("NodeId {} 在合并后出现 {} 个候选：{}（全部保留，未做唯一化）", gid, idxs.len(), candidates.join(", ")),
                None,
                None,
                Some(gid.clone()),
            );
        }
    }

    // 4) 引用规范化：forward/inverse 统一为一条规范边，但不制造缺边。
    for (doc_idx, (doc_id, doc)) in docs.iter().enumerate() {
        for n in &doc.nodes {
            let src_gid = match parse_node_id(&n.node_id).and_then(|nid| ctx.gns_for(doc_idx, nid.ns).map(|g| (nid, g))) {
                Some((nid, g)) => NodeId { ns: g, kind: nid.kind }.to_string(),
                None => format!("unresolved:{}", n.node_id),
            };
            for r in &n.references {
                let mut depth = Vec::new();
                let ref_type = match ctx.resolve(doc_idx, &r.ref_type, &mut depth) {
                    Ok(t) => t,
                    Err(e) => {
                        ctx.diag("unresolved_ref_type", e, Some(doc_id), Some(r.range), Some(src_gid.clone()));
                        r.ref_type.clone()
                    }
                };
                let mut depth = Vec::new();
                match ctx.resolve(doc_idx, &r.target, &mut depth) {
                    Ok(target_gid) => {
                        // 规范化：inverse 声明翻转端点，得到唯一规范边。
                        let (src, dst) = if r.is_forward {
                            (src_gid.clone(), target_gid.clone())
                        } else {
                            (target_gid.clone(), src_gid.clone())
                        };
                        let dup_count = by_gid.get(&target_gid).map(|v| v.len()).unwrap_or(0);
                        let status = if !by_gid.contains_key(&target_gid) && !target_gid.starts_with("i=") && !is_core_id(&target_gid) {
                            "dangling"
                        } else if dup_count > 1 {
                            "ambiguous"
                        } else {
                            "ok"
                        };
                        let candidates: Vec<String> = by_gid
                            .get(&target_gid)
                            .map(|v| v.iter().map(|i| format!("{}:{}", nodes[*i].doc_id, target_gid)).collect())
                            .unwrap_or_default();
                        if status == "dangling" {
                            ctx.diag(
                                "dangling_reference",
                                format!("引用目标 {} 不存在（来自 {} 的 {}），保留 unresolved 占位", target_gid, doc_id, src_gid),
                                Some(doc_id),
                                Some(r.range),
                                Some(target_gid.clone()),
                            );
                        }
                        refs.push(MergedRef {
                            doc_id: doc_id.clone(),
                            src,
                            dst: target_gid,
                            ref_type,
                            forward_declared: r.is_forward,
                            status: status.to_string(),
                            candidates,
                            xml_start: r.range.0,
                            xml_end: r.range.1,
                        });
                    }
                    Err(e) => {
                        let kind = if e.starts_with("alias_cycle") { "alias_cycle" } else { "unresolved_alias" };
                        ctx.diag(kind, format!("引用目标解析失败：{}（文件 {}）", e, doc_id), Some(doc_id), Some(r.range), Some(src_gid.clone()));
                        refs.push(MergedRef {
                            doc_id: doc_id.clone(),
                            src: src_gid.clone(),
                            dst: r.target.clone(),
                            ref_type,
                            forward_declared: r.is_forward,
                            status: "unresolved_alias".to_string(),
                            candidates: Vec::new(),
                            xml_start: r.range.0,
                            xml_end: r.range.1,
                        });
                    }
                }
            }
        }
    }

    // 5) 模型依赖：缺失保留 unresolved，版本冲突保留全部候选。
    let available: Vec<(String, String)> = docs
        .iter()
        .flat_map(|(_, d)| d.models.iter().map(|m| (m.model_uri.clone(), m.version.clone())))
        .collect();
    for (doc_id, doc) in docs.iter() {
        for m in &doc.models {
            for req in &m.required {
                let uri_match: Vec<&String> = available
                    .iter()
                    .filter(|(u, _)| u == &req.model_uri)
                    .map(|(_, v)| v)
                    .collect();
                if uri_match.iter().any(|v| **v == req.version) {
                    dependencies.push(MergedDep {
                        doc_id: doc_id.clone(),
                        model_uri: req.model_uri.clone(),
                        version_req: req.version.clone(),
                        status: "resolved".into(),
                        detail: json!({"available": uri_match}),
                    });
                } else if !uri_match.is_empty() {
                    ctx.diag(
                        "dependency_version_conflict",
                        format!("模型 {} 需要版本 {}，可用版本为 {:?}；冲突候选全部保留", req.model_uri, req.version, uri_match),
                        Some(doc_id),
                        Some(m.range),
                        Some(req.model_uri.clone()),
                    );
                    dependencies.push(MergedDep {
                        doc_id: doc_id.clone(),
                        model_uri: req.model_uri.clone(),
                        version_req: req.version.clone(),
                        status: "conflict".into(),
                        detail: json!({"required": req.version, "candidates": uri_match}),
                    });
                } else {
                    ctx.diag(
                        "unresolved_dependency",
                        format!("模型 {}@{} 未导入，保留 unresolved 依赖节点", req.model_uri, req.version),
                        Some(doc_id),
                        Some(m.range),
                        Some(req.model_uri.clone()),
                    );
                    dependencies.push(MergedDep {
                        doc_id: doc_id.clone(),
                        model_uri: req.model_uri.clone(),
                        version_req: req.version.clone(),
                        status: "unresolved".into(),
                        detail: json!({"required": req.version, "candidates": []}),
                    });
                }
            }
        }
    }

    // 6) 领域量规范化（派生层）；无法转换的留在原位置并附诊断。
    for (doc_id, doc) in docs.iter() {
        for n in &doc.nodes {
            if let Some(q) = &n.quantity {
                let raw = n.value.clone().unwrap_or_default();
                let (norm_value, norm_unit, status) = normalize_quantity(&raw, q.unit.as_deref());
                let gid = match parse_node_id(&n.node_id) {
                    Some(nid) => {
                        let doc_idx = docs.iter().position(|(id, _)| id == doc_id).unwrap();
                        match ctx.gns_for(doc_idx, nid.ns) {
                            Some(g) => NodeId { ns: g, kind: nid.kind }.to_string(),
                            None => n.node_id.clone(),
                        }
                    }
                    None => n.node_id.clone(),
                };
                if status == "unconvertible" {
                    ctx.diag(
                        "unit_unconvertible",
                        format!("值 '{}' 单位 {:?} 无法转换，原始表示保留在 {} 原位", raw, q.unit, gid),
                        Some(doc_id),
                        Some(n.range),
                        Some(gid.clone()),
                    );
                }
                quantities.push(Quantity {
                    gnodeid: gid,
                    doc_id: doc_id.clone(),
                    raw_value: raw,
                    unit: q.unit.clone(),
                    basis: q.basis.clone(),
                    norm_value,
                    norm_unit,
                    status: status.to_string(),
                });
            }
        }
    }

    // 7) 未识别扩展全部保留。
    for (doc_id, doc) in docs.iter() {
        for e in &doc.extensions {
            extensions.push((doc_id.clone(), e.clone()));
        }
    }

    MergedModel {
        ns_entries,
        nodes,
        refs,
        aliases,
        dependencies,
        diagnostics: ctx.diagnostics,
        quantities,
        extensions,
    }
}

fn is_core_id(gid: &str) -> bool {
    // ns=0 的核心节点（如 i=58 BaseObjectType）视为存在，不报悬空。
    gid.starts_with("i=")
}
