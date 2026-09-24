use crate::model::*;
use roxmltree::{Document, Node};

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

/// 解析一份 NodeSet XML。原 XML 节点、namespace table、NodeId、BrowseName、
/// reference、alias、model dependency 与未识别扩展全部保留。
pub fn parse_nodeset(file_name: &str, xml: &str) -> Result<NodeSetFile, String> {
    let doc = Document::parse(xml).map_err(|e| format!("{file_name}: XML 解析失败: {e}"))?;
    let root = doc.root_element();
    let mut namespace_uris = Vec::new();
    let mut aliases = Vec::new();
    let mut models = Vec::new();
    let mut nodes = Vec::new();
    let mut file_extensions = Vec::new();
    for child in root.children().filter(|c| c.is_element()) {
        match child.tag_name().name() {
            "NamespaceUris" => {
                for uri in child.children().filter(|c| c.is_element()) {
                    namespace_uris.push(uri.text().unwrap_or("").trim().to_string());
                }
            }
            "Aliases" => {
                for a in child.children().filter(|c| c.is_element()) {
                    let name = a.attribute("Alias").unwrap_or("").to_string();
                    aliases.push((name, a.text().unwrap_or("").trim().to_string()));
                }
            }
            "Models" => {
                for m in child.children().filter(|c| c.is_element()) {
                    models.push(parse_model(m, file_name));
                }
            }
            name if NODE_CLASSES.contains(&name) => {
                nodes.push(parse_node(&child, file_name, xml)?);
            }
            _ => file_extensions.push(xml[child.range()].to_string()),
        }
    }
    Ok(NodeSetFile {
        file_name: file_name.to_string(),
        namespace_uris,
        aliases,
        models,
        nodes,
        file_extensions,
        content_hash: sha256_hex(xml),
        raw_xml: xml.to_string(),
    })
}

fn parse_model(el: Node<'_, '_>, file_name: &str) -> ModelMeta {
    let mut required_models = Vec::new();
    for c in el.children().filter(|c| c.is_element()) {
        if c.tag_name().name() == "RequiredModel" {
            required_models.push(ModelDep {
                model_uri: c.attribute("ModelUri").unwrap_or("").to_string(),
                version: c.attribute("Version").unwrap_or("").to_string(),
                publication_date: c.attribute("PublicationDate").unwrap_or("").to_string(),
            });
        }
    }
    ModelMeta {
        model_uri: el.attribute("ModelUri").unwrap_or("").to_string(),
        version: el.attribute("Version").unwrap_or("").to_string(),
        publication_date: el.attribute("PublicationDate").unwrap_or("").to_string(),
        required_models,
        source_file: file_name.to_string(),
    }
}

fn parse_node(el: &Node<'_, '_>, file_name: &str, xml: &str) -> Result<UANode, String> {
    let node_id_raw = el
        .attribute("NodeId")
        .ok_or_else(|| format!("{file_name}: 节点缺少 NodeId 属性"))?;
    let node_id = parse_node_id(node_id_raw)
        .ok_or_else(|| format!("{file_name}: 无法解析 NodeId `{node_id_raw}`"))?;
    let browse_name = parse_browse_name(el.attribute("BrowseName").unwrap_or(""));
    let attributes = el
        .attributes()
        .map(|a| (a.name().to_string(), a.value().to_string()))
        .collect();
    let mut display_name = String::new();
    let mut description = None;
    let mut references = Vec::new();
    let mut extensions = Vec::new();
    for c in el.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "DisplayName" => display_name = c.text().unwrap_or("").trim().to_string(),
            "Description" => description = Some(c.text().unwrap_or("").trim().to_string()),
            "References" => {
                for r in c.children().filter(|c| c.is_element()) {
                    if r.tag_name().name() == "Reference" {
                        references.push(Reference {
                            ref_type_raw: r.attribute("ReferenceType").unwrap_or("").to_string(),
                            is_forward: r.attribute("IsForward").map(|v| v != "false").unwrap_or(true),
                            target_raw: r.text().unwrap_or("").trim().to_string(),
                        });
                    }
                }
            }
            // 未识别扩展：保留原始 XML 片段。
            _ => extensions.push(xml[c.range()].to_string()),
        }
    }
    Ok(UANode {
        node_class: el.tag_name().name().to_string(),
        node_id,
        browse_name,
        display_name,
        description,
        references,
        attributes,
        extensions,
        source: SourceSpan {
            file: file_name.to_string(),
            start: el.range().start,
            end: el.range().end,
        },
    })
}
