//! NodeSet XML 解析：保留原 XML 范围（字节区间）、未知扩展原文。
use crate::model::*;
use roxmltree::Node;

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

fn slice(source: &str, r: std::ops::Range<usize>) -> String {
    source.get(r).unwrap_or("").to_string()
}

fn known_node_attr(name: &str) -> bool {
    matches!(name, "NodeId" | "BrowseName")
}

pub fn parse_nodeset(name: &str, source: &str) -> Result<NodeSetFile, String> {
    let doc = roxmltree::Document::parse(source).map_err(|e| format!("XML 解析失败: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "UANodeSet" {
        return Err(format!("{}: 根元素不是 UANodeSet", root.tag_name().name()));
    }
    let mut file = NodeSetFile {
        name: name.to_string(),
        source: source.to_string(),
        ..Default::default()
    };
    for child in root.children().filter(|n| n.is_element()) {
        let tag = child.tag_name().name();
        match tag {
            "NamespaceUris" => {
                for u in child.children().filter(|n| n.is_element()) {
                    file.namespace_uris
                        .push(u.text().unwrap_or("").trim().to_string());
                }
            }
            "Aliases" => {
                for a in child.children().filter(|n| n.is_element()) {
                    file.aliases.push(AliasDef {
                        alias: a.attribute("Alias").unwrap_or("").to_string(),
                        target: a.text().unwrap_or("").trim().to_string(),
                        range: (a.range().start, a.range().end),
                    });
                }
            }
            "Models" => {
                for m in child.children().filter(|n| n.is_element()) {
                    let mut decl = ModelDecl {
                        model_uri: m.attribute("ModelUri").unwrap_or("").to_string(),
                        version: m.attribute("Version").unwrap_or("").to_string(),
                        publication_date: m
                            .attribute("PublicationDate")
                            .unwrap_or("")
                            .to_string(),
                        required: vec![],
                        range: (m.range().start, m.range().end),
                    };
                    for r in m.children().filter(|n| n.is_element()) {
                        if r.tag_name().name() == "RequiredModel" {
                            decl.required.push(RequiredModel {
                                model_uri: r.attribute("ModelUri").unwrap_or("").to_string(),
                                version: r.attribute("Version").unwrap_or("").to_string(),
                                publication_date: r
                                    .attribute("PublicationDate")
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                    }
                    file.models.push(decl);
                }
            }
            t if NODE_CLASSES.contains(&t) => {
                file.nodes.push(parse_node(&file.name, source, child, &mut file.diagnostics));
            }
            _ => {
                // 未识别顶层扩展：原文保留
                file.extensions.push(Extension {
                    element: tag.to_string(),
                    xml: slice(source, child.range()),
                });
            }
        }
    }
    Ok(file)
}

fn parse_node(
    file_name: &str,
    source: &str,
    n: Node,
    diags: &mut Vec<Diag>,
) -> RawNode {
    let mut node = RawNode {
        class: n.tag_name().name().to_string(),
        node_id: n.attribute("NodeId").unwrap_or("").to_string(),
        browse_name: n.attribute("BrowseName").unwrap_or("").to_string(),
        display_name: None,
        value: None,
        unit: None,
        attrs: Default::default(),
        references: vec![],
        extensions: vec![],
        range: (n.range().start, n.range().end),
    };
    // 未识别属性同样保留
    for attr in n.attributes() {
        if !known_node_attr(attr.name()) {
            node.attrs
                .insert(attr.name().to_string(), attr.value().to_string());
        }
    }
    if node.node_id.is_empty() {
        diags.push(Diag {
            code: "missing-nodeid".into(),
            message: "节点缺少 NodeId 属性".into(),
            file: file_name.into(),
            range: Some(node.range),
            node: None,
        });
    }
    for child in n.children().filter(|c| c.is_element()) {
        match child.tag_name().name() {
            "DisplayName" => {
                node.display_name = Some(child.text().unwrap_or("").trim().to_string())
            }
            "Description" => {
                node.attrs.insert(
                    "Description".into(),
                    child.text().unwrap_or("").trim().to_string(),
                );
            }
            "References" => {
                for r in child.children().filter(|c| c.is_element()) {
                    if r.tag_name().name() == "Reference" {
                        node.references.push(RawRef {
                            ref_type: r.attribute("ReferenceType").unwrap_or("").to_string(),
                            is_forward: r
                                .attribute("IsForward")
                                .map(|v| v != "false")
                                .unwrap_or(true),
                            target: r.text().unwrap_or("").trim().to_string(),
                            range: (r.range().start, r.range().end),
                        });
                    }
                }
            }
            "Value" => {
                // 原始表示保留：取第一个元素子节点的文本
                if let Some(inner) = child.children().find(|c| c.is_element()) {
                    node.value = Some(inner.text().unwrap_or("").trim().to_string());
                } else {
                    node.value = Some(child.text().unwrap_or("").trim().to_string());
                }
            }
            "Extensions" => {
                for ext in child.children().filter(|c| c.is_element()) {
                    node.extensions.push(Extension {
                        element: ext.tag_name().name().to_string(),
                        xml: slice(source, ext.range()),
                    });
                    // 已知扩展：单位标注（派生层才做规范化）
                    if let Some(unit) = find_unit(*&ext) {
                        node.unit = Some(unit);
                    }
                }
            }
            other => {
                // 未识别子元素：原文保留
                node.extensions.push(Extension {
                    element: other.to_string(),
                    xml: slice(source, child.range()),
                });
            }
        }
    }
    node
}

fn find_unit(ext: Node) -> Option<String> {
    // 在扩展内寻找 <Unit symbol="..."/>（任意命名空间）
    if ext.tag_name().name() == "Unit" {
        if let Some(s) = ext.attribute("symbol") {
            return Some(s.to_string());
        }
    }
    ext.descendants()
        .find(|d| d.is_element() && d.tag_name().name() == "Unit")
        .and_then(|u| u.attribute("symbol"))
        .map(|s| s.to_string())
}
