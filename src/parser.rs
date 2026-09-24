//! NodeSet XML 保真解析器。
//!
//! 策略：先用 quick_xml 构建一棵带字节偏移的通用元素树，所有解释都基于这棵树，
//! 未知元素用 `raw[start..end]` 原样切片保留，绝不丢弃。

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::model::*;

#[derive(Debug, Clone)]
struct El {
    tag: String,
    attrs: Vec<(String, String)>,
    start: usize,
    end: usize,
    text: String,
    children: Vec<usize>,
}

impl El {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    fn local_tag(&self) -> &str {
        self.tag.rsplit(':').next().unwrap_or(&self.tag)
    }
}

#[derive(Debug, Clone)]
struct Tree {
    els: Vec<El>,
    roots: Vec<usize>,
    xml: String,
    line_starts: Vec<usize>,
}

impl Tree {
    fn line_of(&self, pos: usize) -> usize {
        match self.line_starts.binary_search(&pos) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
    fn range(&self, el: &El) -> RawRange {
        let end_line_pos = el.end.saturating_sub(1).min(self.xml.len().saturating_sub(1));
        RawRange {
            start: el.start,
            end: el.end,
            start_line: self.line_of(el.start),
            end_line: self.line_of(end_line_pos),
        }
    }
    fn outer_xml(&self, el: &El) -> String {
        self.xml[el.start.min(self.xml.len())..el.end.min(self.xml.len())].to_string()
    }
    fn kids<'a>(&'a self, el: &'a El, name: &str) -> Vec<usize> {
        el.children
            .iter()
            .copied()
            .filter(|&i| self.els[i].local_tag() == name)
            .collect()
    }
    fn first<'a>(&'a self, el: &'a El, name: &str) -> Option<usize> {
        el.children
            .iter()
            .copied()
            .find(|&i| self.els[i].local_tag() == name)
    }
}

fn build_tree(xml: &str) -> Tree {
    let mut line_starts = vec![0usize];
    for (i, b) in xml.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut els: Vec<El> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut buf = Vec::new();

    loop {
        let before = reader.buffer_position();
        let ev = match reader.read_event_into(&mut buf) {
            Ok(e) => e,
            Err(_) => break,
        };
        match ev {
            Event::Start(ref e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let attrs = e
                    .attributes()
                    .flatten()
                    .map(|a| {
                        (
                            String::from_utf8_lossy(a.key.as_ref()).to_string(),
                            a.unescape_value().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
                let id = els.len();
                els.push(El {
                    tag,
                    attrs,
                    start: before,
                    end: 0,
                    text: String::new(),
                    children: Vec::new(),
                });
                if let Some(&parent) = stack.last() {
                    els[parent].children.push(id);
                } else {
                    roots.push(id);
                }
                stack.push(id);
            }
            Event::Empty(ref e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let attrs = e
                    .attributes()
                    .flatten()
                    .map(|a| {
                        (
                            String::from_utf8_lossy(a.key.as_ref()).to_string(),
                            a.unescape_value().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
                let after = reader.buffer_position();
                let id = els.len();
                els.push(El {
                    tag,
                    attrs,
                    start: before,
                    end: after,
                    text: String::new(),
                    children: Vec::new(),
                });
                if let Some(&parent) = stack.last() {
                    els[parent].children.push(id);
                } else {
                    roots.push(id);
                }
            }
            Event::Text(t) => {
                if let Some(&parent) = stack.last() {
                    if let Ok(s) = t.unescape() {
                        els[parent].text.push_str(&s);
                    }
                }
            }
            Event::End(_) => {
                if let Some(id) = stack.pop() {
                    els[id].end = reader.buffer_position();
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    // 容错：未闭合元素给到文件末尾。
    let len = xml.len();
    for el in els.iter_mut() {
        if el.end == 0 {
            el.end = len;
        }
    }
    Tree {
        els,
        roots,
        xml: xml.to_string(),
        line_starts,
    }
}

pub fn parse_document(source_id: &str, filename: &str, xml: &str) -> RawDocument {
    let tree = build_tree(xml);
    let mut doc = RawDocument {
        source_id: source_id.to_string(),
        filename: filename.to_string(),
        raw_xml: xml.to_string(),
        namespaces: Vec::new(),
        aliases: Vec::new(),
        models: Vec::new(),
        nodes: Vec::new(),
    };

    let root = tree
        .roots
        .iter()
        .copied()
        .find(|&i| tree.els[i].local_tag() == "UANodeSet");

    let root = match root {
        Some(r) => r,
        None => return doc,
    };

    // NamespaceTable / NamespaceUris
    if let Some(nt) = tree.first(&tree.els[root], "NamespaceTable") {
        let mut idx = 1u32;
        for ci in tree.els[nt].children.clone() {
            let child = &tree.els[ci];
            if child.local_tag() == "Uri" || child.local_tag() == "NamespaceUri" {
                doc.namespaces.push(NamespaceEntry {
                    file_index: idx,
                    uri: child.text.trim().to_string(),
                    range: tree.range(child),
                });
                idx += 1;
            }
        }
    }

    // Aliases
    if let Some(at) = tree.first(&tree.els[root], "Aliases") {
        for ci in tree.els[at].children.clone() {
            let child = &tree.els[ci];
            if child.local_tag() == "Alias" {
                let alias = child
                    .attr("Alias")
                    .unwrap_or_else(|| child.attr("Name").unwrap_or(""))
                    .to_string();
                let target = child.text.trim().to_string();
                doc.aliases.push(AliasEntry {
                    alias,
                    node_id: NodeId::parse(&target),
                    range: tree.range(child),
                });
            }
        }
    }

    // Models
    for mi in tree.kids(&tree.els[root], "Model") {
        doc.models.push(parse_model(&tree, mi));
    }

    // Nodes（UANodeSet 直接子节点）
    for ci in tree.els[root].children.clone() {
        let child = &tree.els[ci];
        if is_node_tag(child.local_tag()) {
            doc.nodes.push(parse_node(&tree, ci, NodeClass::from_tag(child.local_tag())));
        }
    }

    doc
}

fn parse_model(tree: &Tree, mi: usize) -> ModelDecl {
    let el = &tree.els[mi];
    let mut required = Vec::new();
    let mut walk: Vec<usize> = el.children.clone();
    while let Some(ci) = walk.pop() {
        let child = &tree.els[ci];
        if child.local_tag() == "RequiredModel" {
            required.push(ModelDependency {
                model_uri: child.attr("ModelUri").unwrap_or("").to_string(),
                version: child.attr("Version").map(str::to_string),
                publication_date: child.attr("PublicationDate").map(str::to_string),
                range: tree.range(child),
            });
        } else {
            walk.extend(child.children.iter().rev().copied());
        }
    }
    ModelDecl {
        model_uri: el.attr("ModelUri").unwrap_or("").to_string(),
        version: el.attr("Version").map(str::to_string),
        publication_date: el.attr("PublicationDate").map(str::to_string),
        range: tree.range(el),
        required,
    }
}

const KNOWN_CHILDREN: &[&str] = &[
    "DisplayName",
    "Description",
    "Category",
    "References",
    "Value",
    "Extensions",
];

fn parse_node(tree: &Tree, ni: usize, class: NodeClass) -> RawNode {
    let el = &tree.els[ni];
    let node_id_raw = el.attr("NodeId").unwrap_or("").to_string();
    let browse_raw = el.attr("BrowseName").map(str::to_string);

    // 其余属性全部原样保留。
    let reserved = ["NodeId", "BrowseName"];
    let mut attributes = Vec::new();
    for (k, v) in &el.attrs {
        if !reserved.contains(&k.as_str()) {
            attributes.push((k.clone(), v.clone()));
        }
    }

    let display_name = tree
        .first(el, "DisplayName")
        .map(|i| tree.els[i].text.trim().to_string());
    let description = tree
        .first(el, "Description")
        .map(|i| tree.els[i].text.trim().to_string());

    let mut references = Vec::new();
    if let Some(ri) = tree.first(el, "References") {
        for ci in tree.els[ri].children.clone() {
            let child = &tree.els[ci];
            if child.local_tag() == "Reference" {
                references.push(parse_reference(tree, ci));
            }
        }
    }

    let mut value_xml = None;
    if let Some(vi) = tree.first(el, "Value") {
        value_xml = Some(tree.outer_xml(&tree.els[vi]));
    }

    let mut quantities = Vec::new();
    let mut unknown_extensions = Vec::new();
    collect_extensions(tree, ni, &mut quantities, &mut unknown_extensions);

    let rn = RawNode {
        stable_id: String::new(),
        ord: 0,
        node_id: NodeId::parse(&node_id_raw),
        browse_name: browse_raw.as_deref().map(QualifiedName::parse),
        display_name,
        description,
        class,
        attributes,
        value_xml,
        references,
        quantities,
        unknown_extensions,
        range: tree.range(el),
    };
    rn
}

fn parse_reference(tree: &Tree, ri: usize) -> RawReference {
    let el = &tree.els[ri];
    let type_text = el.attr("ReferenceType").unwrap_or("").to_string();
    let is_forward = el
        .attr("IsForward")
        .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0"))
        .unwrap_or(true);
    let target_text = el.text.trim().to_string();

    // 别名写法：ReferenceType 文本若不是 NodeId 语法，则视为别名引用。
    let looks_like_nodeid = type_text.contains('=');
    let via_alias = if looks_like_nodeid {
        None
    } else {
        Some(type_text.clone())
    };
    let reference_type = if looks_like_nodeid {
        NodeId::parse(&type_text)
    } else {
        // 别名书写：原样保留别名名，解析为 Opaque，由合并层解析。
        NodeId {
            raw: type_text.clone(),
            namespace_index: 0,
            identifier: Identifier::Opaque(type_text.clone()),
        }
    };

    RawReference {
        target: NodeId::parse(&target_text),
        reference_type,
        is_forward,
        via_alias,
        range: tree.range(el),
    }
}

/// 收集扩展：识别我们的领域量元素，其余作为未知扩展保留原文。
fn collect_extensions(
    tree: &Tree,
    ni: usize,
    quantities: &mut Vec<DomainQuantity>,
    unknown: &mut Vec<UnknownExtension>,
) {
    let el = &tree.els[ni];
    for ci in el.children.clone() {
        let child = &tree.els[ci];
        let tag = child.local_tag();
        if !KNOWN_CHILDREN.contains(&tag) {
            unknown.push(UnknownExtension {
                name: child.tag.clone(),
                outer_xml: tree.outer_xml(child),
                range: tree.range(child),
            });
            if tag == "Quantity" {
                quantities.push(parse_quantity(tree, ci));
            }
        } else if tag == "Extensions" {
            for gi in child.children.clone() {
                let g = &tree.els[gi];
                if g.local_tag() == "Quantity" {
                    quantities.push(parse_quantity(tree, gi));
                } else {
                    unknown.push(UnknownExtension {
                        name: g.tag.clone(),
                        outer_xml: tree.outer_xml(g),
                        range: tree.range(g),
                    });
                }
            }
        }
    }
}

fn parse_quantity(tree: &Tree, qi: usize) -> DomainQuantity {
    let el = &tree.els[qi];
    let kind = el.attr("kind").unwrap_or("unknown").to_string();
    let raw_value = el
        .attr("value")
        .map(str::to_string)
        .unwrap_or_else(|| el.text.trim().to_string());
    let raw_unit = el.attr("unit").map(str::to_string);
    let coordinate_basis = el.attr("coordinateBasis").map(str::to_string);
    let mut q = DomainQuantity {
        kind,
        raw_value,
        raw_unit,
        normalized_value: None,
        normalized_unit: None,
        coordinate_basis,
        diagnostic: None,
        range: tree.range(el),
    };
    crate::derive::normalize_quantity(&mut q);
    q
}
