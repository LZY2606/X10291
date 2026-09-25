//! NodeSet XML 导入：解析 namespace table、alias、model dependency、节点与引用，
//! 同时保留每个节点的原始 XML 片段、字节范围与未识别扩展。

use anyhow::{anyhow, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use sha2::{Digest, Sha256};

use crate::model::*;

const NODE_CLASSES: &[&str] = &[
    "UAObject",
    "UAVariable",
    "UAMethod",
    "UAObjectType",
    "UAVariableType",
    "UAReferenceType",
    "UADataType",
    "UAView",
];

/// 已识别的子元素；其余作为未识别扩展原样保留。
const KNOWN_CHILDREN: &[&str] = &["DisplayName", "Description", "References", "Reference"];

pub fn content_hash(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    format!("{:x}", h.finalize())[..16].to_string()
}

pub fn parse_nodeset(filename: &str, content: &str) -> Result<NodeSet> {
    let source_id = content_hash(content);
    let mut ns = NodeSet {
        source_id: source_id.clone(),
        filename: filename.to_string(),
        namespace_uris: Vec::new(),
        models: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        diagnostics: Vec::new(),
    };

    // 第一遍：结构化扫描，记录每个顶层节点元素的字节范围。
    let mut reader = Reader::from_str(content);
    reader.trim_text(true);
    let mut depth: usize = 0;
    let mut node_start: Option<(usize, String)> = None;
    let mut in_uris = false;
    let mut in_models = false;
    let mut cur_model: Option<ModelDependency> = None;
    let mut cur_alias: Option<String> = None;
    let mut cur_uri = false;

    loop {
        let pos = reader.buffer_position();
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                depth += 1;
                if depth == 2 {
                    match name.as_str() {
                        "NamespaceUris" => in_uris = true,
                        "Models" => in_models = true,
                        "Aliases" => {}
                        _ => {}
                    }
                }
                if depth == 2 && NODE_CLASSES.contains(&name.as_str()) {
                    // 回溯到 '<' 得到元素起点。
                    let mut s = pos;
                    let bytes = content.as_bytes();
                    let mut i = pos.min(bytes.len());
                    while i > 0 && bytes[i - 1] != b'<' {
                        i -= 1;
                    }
                    s = i - 1;
                    node_start = Some((s, name));
                } else if in_uris && depth == 3 && name == "Uri" {
                    cur_uri = true;
                } else if in_models && depth == 3 && name == "Model" {
                    cur_model = Some(ModelDependency {
                        model_uri: attr_of(&e, "ModelUri").unwrap_or_default(),
                        version: attr_of(&e, "Version").unwrap_or_default(),
                        publication_date: attr_of(&e, "PublicationDate").unwrap_or_default(),
                    });
                } else if depth == 3 && name == "Alias" {
                    cur_alias = attr_of(&e, "Alias");
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if depth == 1 && NODE_CLASSES.contains(&name.as_str()) {
                    let bytes = content.as_bytes();
                    let mut i = pos.min(bytes.len());
                    while i > 0 && bytes[i - 1] != b'<' {
                        i -= 1;
                    }
                    let start = i - 1;
                    let end = reader.buffer_position();
                    let raw = content[start..end].to_string();
                    let node = parse_node_element(&raw, (start, end), &source_id, filename, &name)?;
                    ns.nodes.push(node);
                } else if in_models && depth == 2 && name == "Model" {
                    ns.models.push(ModelDependency {
                        model_uri: attr_of(&e, "ModelUri").unwrap_or_default(),
                        version: attr_of(&e, "Version").unwrap_or_default(),
                        publication_date: attr_of(&e, "PublicationDate").unwrap_or_default(),
                    });
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                if cur_uri && in_uris {
                    ns.namespace_uris.push(text.trim().to_string());
                    cur_uri = false;
                } else if let Some(alias) = cur_alias.clone() {
                    ns.aliases.push((alias, text.trim().to_string()));
                    cur_alias = None;
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if let Some((start, ref start_name)) = node_start {
                    if *start_name == name {
                        let end = reader.buffer_position();
                        let raw = content[start..end].to_string();
                        let node = parse_node_element(&raw, (start, end), &source_id, filename, &name)?;
                        ns.nodes.push(node);
                        node_start = None;
                    }
                }
                if depth == 2 && name == "NamespaceUris" {
                    in_uris = false;
                }
                if depth == 2 && name == "Models" {
                    in_models = false;
                }
                if in_models && depth == 3 && name == "Model" {
                    if let Some(m) = cur_model.take() {
                        ns.models.push(m);
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("XML 解析失败 {filename}@{pos}: {e}")),
            _ => {}
        }
    }

    // 文件内重复 NodeId 诊断（跨文件重复在合并阶段判定）。
    let mut seen: Vec<String> = Vec::new();
    for n in &ns.nodes {
        let k = n.node_id.to_local_string();
        if seen.contains(&k) {
            ns.diagnostics.push(Diagnostic {
                kind: "duplicate_nodeid_in_file".into(),
                message: format!("文件内重复 NodeId {k}"),
                node: Some(k),
                source: filename.into(),
                span: Some(n.span),
            });
        } else {
            seen.push(k);
        }
    }
    Ok(ns)
}

fn attr_of(e: &BytesStart, key: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        if a.key.as_ref() == key.as_bytes() {
            return a.unescape_value().ok().map(|v| v.to_string());
        }
    }
    None
}

fn parse_node_element(
    raw: &str,
    span: (usize, usize),
    _source_id: &str,
    filename: &str,
    class: &str,
) -> Result<UaNode> {
    let mut reader = Reader::from_str(raw);
    reader.trim_text(true);
    let mut node = UaNode {
        node_id: NodeId { ns: 0, ident: Ident::Num(0) },
        browse_name: (0, String::new()),
        node_class: class.to_string(),
        display_name: String::new(),
        description: None,
        attrs: Vec::new(),
        references: Vec::new(),
        extensions: Vec::new(),
        raw_xml: raw.to_string(),
        span,
        source: filename.to_string(),
    };
    let known_attrs = ["NodeId", "BrowseName", "SymbolicName"];
    let mut depth = 0usize;
    let mut in_refs = false;
    let mut cur_ref: Option<(String, bool)> = None;
    let mut cur_display = false;
    let mut cur_desc = false;
    let mut ext: Option<(String, usize, usize)> = None; // (name, depth, start_pos)

    loop {
        let pos = reader.buffer_position();
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                depth += 1;
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if depth == 1 {
                    // 节点自身属性。
                    for a in e.attributes().flatten() {
                        let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                        let v = a.unescape_value().unwrap_or_default().to_string();
                        match k.as_str() {
                            "NodeId" => {
                                node.node_id = NodeId::parse(&v)
                                    .ok_or_else(|| anyhow!("非法 NodeId: {v}"))?
                            }
                            "BrowseName" => node.browse_name = parse_browse_name(&v),
                            _ => {
                                if !known_attrs.contains(&k.as_str()) {
                                    node.attrs.push((k, v));
                                }
                            }
                        }
                    }
                } else if depth == 2 {
                    match name.as_str() {
                        "DisplayName" => cur_display = true,
                        "Description" => cur_desc = true,
                        "References" => in_refs = true,
                        _ => {
                            if !KNOWN_CHILDREN.contains(&name.as_str()) && ext.is_none() {
                                let bytes = raw.as_bytes();
                                let mut i = pos.min(bytes.len());
                                while i > 0 && bytes[i - 1] != b'<' {
                                    i -= 1;
                                }
                                ext = Some((name, depth, i - 1));
                            }
                        }
                    }
                } else if in_refs && depth == 3 && name == "Reference" {
                    let rt = attr_of(&e, "ReferenceType").unwrap_or_default();
                    let fwd = attr_of(&e, "IsForward")
                        .map(|v| v != "false")
                        .unwrap_or(true);
                    cur_ref = Some((rt, fwd));
                } else if let Some((_, d, _)) = &ext {
                    let _ = d; // 扩展内部不再细分
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if in_refs && depth == 2 && name == "Reference" {
                    // 空 Reference 元素无目标，忽略。
                } else if depth >= 2 && !KNOWN_CHILDREN.contains(&name.as_str()) && ext.is_none()
                {
                    let bytes = raw.as_bytes();
                    let mut i = pos.min(bytes.len());
                    while i > 0 && bytes[i - 1] != b'<' {
                        i -= 1;
                    }
                    let end = reader.buffer_position();
                    node.extensions.push(RawExtension {
                        element: name,
                        raw_xml: raw[i - 1..end].to_string(),
                    });
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                if cur_display {
                    node.display_name = text.trim().to_string();
                } else if cur_desc {
                    node.description = Some(text.trim().to_string());
                } else if let Some((rt, fwd)) = cur_ref.clone() {
                    node.references.push(Reference {
                        ref_type_raw: rt,
                        is_forward: fwd,
                        target_raw: text.trim().to_string(),
                    });
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "DisplayName" {
                    cur_display = false;
                } else if name == "Description" {
                    cur_desc = false;
                } else if name == "References" {
                    in_refs = false;
                } else if name == "Reference" {
                    cur_ref = None;
                }
                if let Some((ename, _, start)) = &ext {
                    if *ename == name {
                        let end = reader.buffer_position();
                        node.extensions.push(RawExtension {
                            element: ename.clone(),
                            raw_xml: raw[*start..end].to_string(),
                        });
                        ext = None;
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("节点 XML 解析失败 {filename}: {e}")),
            _ => {}
        }
    }
    Ok(node)
}

fn parse_browse_name(v: &str) -> (u16, String) {
    if let Some((ns, name)) = v.split_once(':') {
        if let Ok(n) = ns.trim().parse::<u16>() {
            return (n, name.trim().to_string());
        }
    }
    (0, v.trim().to_string())
}
