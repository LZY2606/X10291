//! NodeSet XML 导入层：只解析、不规范化。
//! 原 XML 节点、namespace table、NodeId、BrowseName、reference、alias、
//! model dependency 与未识别扩展均按原样保留（含字节范围，可回溯源 XML）。

use roxmltree::Document;

pub const NODE_CLASSES: [&str; 8] = [
    "UAObjectType",
    "UAVariableType",
    "UAObject",
    "UAVariable",
    "UAMethod",
    "UAReferenceType",
    "UADataType",
    "UAView",
];

#[derive(Clone, Debug)]
pub struct SourceDoc {
    pub name: String,
    pub xml: String,
    /// 文件内 namespace table，index 从 1 开始（0 固定为 OPC UA 核心 URI）。
    pub namespaces: Vec<String>,
    pub models: Vec<ModelDecl>,
    pub aliases: Vec<AliasDecl>,
    pub nodes: Vec<RawNode>,
    /// 未识别扩展（文档级与节点级），原样保留。
    pub extensions: Vec<RawExtension>,
}

#[derive(Clone, Debug)]
pub struct ModelDecl {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub required: Vec<RequiredModel>,
    pub range: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct RequiredModel {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
}

#[derive(Clone, Debug)]
pub struct AliasDecl {
    pub alias: String,
    pub target: String,
    pub range: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct RawNode {
    pub node_class: String,
    pub node_id: String,
    pub browse_name: String,
    pub parent_node_id: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub data_type: Option<String>,
    pub references: Vec<RawReference>,
    /// 已识别的领域量扩展（CompassQuantity）：单位/坐标基准。
    pub quantity: Option<RawQuantity>,
    /// 其余全部属性原样保留。
    pub attrs: Vec<(String, String)>,
    pub range: (usize, usize),
    pub raw_xml: String,
}

#[derive(Clone, Debug)]
pub struct RawReference {
    pub ref_type: String,
    pub is_forward: bool,
    pub target: String,
    pub range: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct RawQuantity {
    pub unit: Option<String>,
    pub basis: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RawExtension {
    pub owner_node: Option<String>,
    pub tag: String,
    pub range: (usize, usize),
    pub raw_xml: String,
}

const KNOWN_NODE_ATTRS: [&str; 6] = [
    "NodeId",
    "BrowseName",
    "ParentNodeId",
    "DataType",
    "SymbolicName",
    "ValueRank",
];

const KNOWN_NODE_CHILDREN: [&str; 4] = ["DisplayName", "Description", "References", "Value"];

pub fn parse_nodeset(name: &str, xml: &str) -> Result<SourceDoc, String> {
    let parsed = Document::parse(xml).map_err(|e| format!("XML 解析失败: {}", e))?;
    let root = parsed.root_element();
    if root.tag_name().name() != "UANodeSet" {
        return Err(format!("根元素应为 UANodeSet，实际为 {}", root.tag_name().name()));
    }
    let mut doc = SourceDoc {
        name: name.to_string(),
        xml: xml.to_string(),
        namespaces: Vec::new(),
        models: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        extensions: Vec::new(),
    };
    for child in root.children().filter(|n| n.is_element()) {
        let tag = child.tag_name().name().to_string();
        match tag.as_str() {
            "NamespaceUris" => {
                for uri in child.children().filter(|n| n.is_element()) {
                    doc.namespaces.push(uri.text().unwrap_or("").trim().to_string());
                }
            }
            "Models" => {
                for m in child.children().filter(|n| n.is_element()) {
                    if m.tag_name().name() != "Model" {
                        doc.extensions.push(RawExtension {
                            owner_node: None,
                            tag: m.tag_name().name().to_string(),
                            range: (m.range().start, m.range().end),
                            raw_xml: xml[m.range()].to_string(),
                        });
                        continue;
                    }
                    let mut required = Vec::new();
                    for r in m.children().filter(|n| n.is_element()) {
                        if r.tag_name().name() == "RequiredModel" {
                            required.push(RequiredModel {
                                model_uri: r.attribute("ModelUri").unwrap_or("").to_string(),
                                version: r.attribute("Version").unwrap_or("").to_string(),
                                publication_date: r
                                    .attribute("PublicationDate")
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                    }
                    doc.models.push(ModelDecl {
                        model_uri: m.attribute("ModelUri").unwrap_or("").to_string(),
                        version: m.attribute("Version").unwrap_or("").to_string(),
                        publication_date: m
                            .attribute("PublicationDate")
                            .unwrap_or("")
                            .to_string(),
                        required,
                        range: (m.range().start, m.range().end),
                    });
                }
            }
            "Aliases" => {
                for a in child.children().filter(|n| n.is_element()) {
                    doc.aliases.push(AliasDecl {
                        alias: a.attribute("Alias").unwrap_or("").to_string(),
                        target: a.text().unwrap_or("").trim().to_string(),
                        range: (a.range().start, a.range().end),
                    });
                }
            }
            t if NODE_CLASSES.contains(&t) => {
                doc.nodes.push(parse_node(&child, xml, &mut doc.extensions));
            }
            _ => {
                // 未识别的文档级元素：保留原文与范围。
                doc.extensions.push(RawExtension {
                    owner_node: None,
                    tag,
                    range: (child.range().start, child.range().end),
                    raw_xml: xml[child.range()].to_string(),
                });
            }
        }
    }
    Ok(doc)
}

fn parse_node(
    el: &roxmltree::Node,
    xml: &str,
    extensions: &mut Vec<RawExtension>,
) -> RawNode {
    let node_id = el.attribute("NodeId").unwrap_or("").to_string();
    let mut node = RawNode {
        node_class: el.tag_name().name().to_string(),
        node_id: node_id.clone(),
        browse_name: el.attribute("BrowseName").unwrap_or("").to_string(),
        parent_node_id: el.attribute("ParentNodeId").map(|s| s.to_string()),
        display_name: None,
        description: None,
        value: None,
        data_type: el.attribute("DataType").map(|s| s.to_string()),
        references: Vec::new(),
        quantity: None,
        attrs: el
            .attributes()
            .filter(|a| !KNOWN_NODE_ATTRS.contains(&a.name()))
            .map(|a| (a.name().to_string(), a.value().to_string()))
            .collect(),
        range: (el.range().start, el.range().end),
        raw_xml: xml[el.range()].to_string(),
    };
    for child in el.children().filter(|n| n.is_element()) {
        let tag = child.tag_name().name();
        match tag {
            "DisplayName" => node.display_name = child.text().map(|s| s.to_string()),
            "Description" => node.description = child.text().map(|s| s.to_string()),
            "Value" => {
                // 保留第一个值子元素的文本（原始表示）。
                if let Some(v) = child.children().find(|n| n.is_element()) {
                    node.value = v.text().map(|s| s.trim().to_string());
                } else {
                    node.value = child.text().map(|s| s.trim().to_string());
                }
            }
            "References" => {
                for r in child.children().filter(|n| n.is_element()) {
                    if r.tag_name().name() != "Reference" {
                        extensions.push(RawExtension {
                            owner_node: Some(node_id.clone()),
                            tag: r.tag_name().name().to_string(),
                            range: (r.range().start, r.range().end),
                            raw_xml: xml[r.range()].to_string(),
                        });
                        continue;
                    }
                    node.references.push(RawReference {
                        ref_type: r.attribute("ReferenceType").unwrap_or("i=31").to_string(),
                        is_forward: r
                            .attribute("IsForward")
                            .map(|v| v != "false")
                            .unwrap_or(true),
                        target: r.text().unwrap_or("").trim().to_string(),
                        range: (r.range().start, r.range().end),
                    });
                }
            }
            "Extensions" => {
                for ext in child.children().filter(|n| n.is_element()) {
                    let inner = ext.children().find(|n| n.is_element());
                    let (tag, range) = match &inner {
                        Some(i) => (i.tag_name().name().to_string(), (i.range().start, i.range().end)),
                        None => (ext.tag_name().name().to_string(), (ext.range().start, ext.range().end)),
                    };
                    if tag == "CompassQuantity" {
                        let i = inner.unwrap();
                        node.quantity = Some(RawQuantity {
                            unit: i.attribute("Unit").map(|s| s.to_string()),
                            basis: i.attribute("Basis").map(|s| s.to_string()),
                        });
                    } else {
                        extensions.push(RawExtension {
                            owner_node: Some(node_id.clone()),
                            tag,
                            range,
                            raw_xml: xml[range.0..range.1].to_string(),
                        });
                    }
                }
            }
            _ => {
                // 未识别的节点级子元素：保留。
                extensions.push(RawExtension {
                    owner_node: Some(node_id.clone()),
                    tag: tag.to_string(),
                    range: (child.range().start, child.range().end),
                    raw_xml: xml[child.range()].to_string(),
                });
            }
        }
    }
    node
}
