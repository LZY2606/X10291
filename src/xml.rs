//! NodeSet XML 导入：保留原 XML 节点、namespace table、NodeId、BrowseName、
//! reference、alias、model dependency 与未识别扩展（含字节范围，可回读源片段）。

use crate::model::*;
use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Clone, Debug)]
pub struct XmlElem {
    pub name: String,  // 本地名（去前缀）
    pub qname: String, // 原始限定名
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlElem>,
    pub text: String,
    pub span: (usize, usize),
}

impl XmlElem {
    pub fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| local_name(k) == local)
            .map(|(_, v)| v.as_str())
    }
    pub fn children_named(&self, local: &str) -> impl Iterator<Item = &XmlElem> {
        self.children.iter().filter(move |c| c.name == local)
    }
    pub fn child(&self, local: &str) -> Option<&XmlElem> {
        self.children.iter().find(|c| c.name == local)
    }
}

pub fn local_name(q: &str) -> &str {
    match q.rfind(':') {
        Some(i) => &q[i + 1..],
        None => q,
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// 解析为保留字节范围的通用元素树。
pub fn parse_tree(source: &str) -> Result<XmlElem, String> {
    let mut reader = Reader::from_str(source);
    let mut stack: Vec<XmlElem> = Vec::new();
    let mut root: Option<XmlElem> = None;
    loop {
        let pos = reader.buffer_position() as usize;
        let ev = reader.read_event().map_err(|e| format!("XML 解析失败 @{}: {}", pos, e))?;
        match ev {
            Event::Start(e) => {
                let qname = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let mut attrs = Vec::new();
                for a in e.attributes().flatten() {
                    let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                    let v = unescape(&String::from_utf8_lossy(a.value.as_ref()));
                    attrs.push((k, v));
                }
                stack.push(XmlElem {
                    name: local_name(&qname).to_string(),
                    qname,
                    attrs,
                    children: Vec::new(),
                    text: String::new(),
                    span: (pos, pos),
                });
            }
            Event::Empty(e) => {
                let end = reader.buffer_position() as usize;
                let qname = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let mut attrs = Vec::new();
                for a in e.attributes().flatten() {
                    let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                    let v = unescape(&String::from_utf8_lossy(a.value.as_ref()));
                    attrs.push((k, v));
                }
                let el = XmlElem {
                    name: local_name(&qname).to_string(),
                    qname,
                    attrs,
                    children: Vec::new(),
                    text: String::new(),
                    span: (pos, end),
                };
                match stack.last_mut() {
                    Some(p) => p.children.push(el),
                    None => {
                        root = Some(el);
                    }
                }
            }
            Event::End(_) => {
                let end = reader.buffer_position() as usize;
                if let Some(mut el) = stack.pop() {
                    el.span.1 = end;
                    match stack.last_mut() {
                        Some(p) => p.children.push(el),
                        None => root = Some(el),
                    }
                }
            }
            Event::Text(t) => {
                let raw = String::from_utf8_lossy(t.as_ref()).to_string();
                if let Some(p) = stack.last_mut() {
                    p.text.push_str(&unescape(raw.trim()));
                }
            }
            Event::CData(t) => {
                let raw = String::from_utf8_lossy(t.as_ref()).to_string();
                if let Some(p) = stack.last_mut() {
                    p.text.push_str(&raw);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| "空文档：没有根元素".to_string())
}

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

/// 节点内已识别的子元素；其余一律作为未识别扩展原样保留。
const KNOWN_NODE_CHILDREN: &[&str] = &["DisplayName", "Description", "References", "Value", "Definition"];

/// 派生层认识的领域扩展（导入层仍按扩展保留原文）。
pub const QUANTITY_EXT: &str = "CompassQuantity";

pub fn import_file(name: &str, source: &str) -> RawFile {
    let mut file = RawFile {
        name: name.to_string(),
        source: source.to_string(),
        fingerprint: fnv64(source),
        namespace_uris: Vec::new(),
        aliases: Vec::new(),
        models: Vec::new(),
        nodes: Vec::new(),
        diagnostics: Vec::new(),
    };
    let root = match parse_tree(source) {
        Ok(r) => r,
        Err(e) => {
            file.diagnostics.push(Diagnostic {
                id: format!("XmlParse:{}", fnv64(&e)),
                kind: DiagKind::XmlParse,
                message: e,
                subject: None,
                source: None,
                candidates: vec![],
                caused_by: vec![],
            });
            return file;
        }
    };
    if let Some(ns) = root.child("NamespaceUris") {
        for uri in ns.children_named("Uri") {
            file.namespace_uris.push(uri.text.trim().to_string());
        }
    }
    if let Some(models) = root.child("Models") {
        for m in models.children_named("Model") {
            let mut required = Vec::new();
            for r in m.children_named("RequiredModel") {
                required.push(RequiredModel {
                    uri: r.attr("ModelUri").unwrap_or("").to_string(),
                    version: r.attr("Version").unwrap_or("").to_string(),
                    publication_date: r.attr("PublicationDate").unwrap_or("").to_string(),
                });
            }
            file.models.push(ModelInfo {
                uri: m.attr("ModelUri").unwrap_or("").to_string(),
                version: m.attr("Version").unwrap_or("").to_string(),
                publication_date: m.attr("PublicationDate").unwrap_or("").to_string(),
                required,
                source: SourceRef { file: name.into(), start: m.span.0, end: m.span.1 },
            });
        }
    }
    if let Some(aliases) = root.child("Aliases") {
        for a in aliases.children_named("Alias") {
            file.aliases.push(AliasDef {
                alias: a.attr("Alias").unwrap_or("").to_string(),
                target_raw: a.text.trim().to_string(),
                source: SourceRef { file: name.into(), start: a.span.0, end: a.span.1 },
            });
        }
    }
    for el in &root.children {
        if NODE_CLASSES.contains(&el.name.as_str()) {
            file.nodes.push(import_node(name, source, el));
        }
    }
    file
}

fn import_node(file: &str, source: &str, el: &XmlElem) -> RawNode {
    let src = SourceRef { file: file.into(), start: el.span.0, end: el.span.1 };
    let mut references = Vec::new();
    if let Some(refs) = el.child("References") {
        for r in refs.children_named("Reference") {
            let forward = match r.attr("IsForward") {
                Some(v) => v != "false" && v != "0",
                None => true,
            };
            references.push(Reference {
                kind_raw: r.attr("ReferenceType").unwrap_or("").to_string(),
                kind_node: None,
                forward,
                target: RefTarget::Unresolved {
                    raw: r.text.trim().to_string(),
                    reason: "尚未解析（导入层原样保留）".into(),
                },
                source_span: r.span,
            });
        }
    }
    let mut extensions = Vec::new();
    for c in &el.children {
        if !KNOWN_NODE_CHILDREN.contains(&c.name.as_str()) {
            extensions.push(Extension {
                name: c.qname.clone(),
                xml: source[c.span.0..c.span.1].to_string(),
                span: c.span,
            });
        }
    }
    RawNode {
        node_class: el.name.clone(),
        node_id_raw: el.attr("NodeId").unwrap_or("").to_string(),
        node_id: None,
        browse_name_raw: el.attr("BrowseName").unwrap_or("").to_string(),
        browse_name: None,
        display_name: el.child("DisplayName").map(|d| d.text.trim().to_string()),
        attrs: el.attrs.clone(),
        references,
        extensions,
        source: src,
    }
}
