//! NodeSet XML 的解析模型。原则：原 XML 节点、namespace table、NodeId、
//! BrowseName、reference、alias、model dependency 与未识别扩展均保留。

use crate::xml::{self, Element};

pub const CORE_URI: &str = "http://opcfoundation.org/UA/";

pub const NODE_CLASSES: &[&str] = &[
    "UAObject",
    "UAVariable",
    "UAMethod",
    "UAObjectType",
    "UAVariableType",
    "UAReferenceType",
    "UADataType",
    "UAView",
];

#[derive(Debug, Clone)]
pub struct NodeSetFile {
    pub name: String,
    pub source: String,
    /// 文件内 namespace table：索引 i 对应 URI（文件内数字 index 仅在本文件有意义）
    pub namespace_uris: Vec<String>,
    pub models: Vec<ModelInfo>,
    pub aliases: Vec<AliasDef>,
    pub nodes: Vec<RawNode>,
    /// 未识别的顶层扩展元素，原样保留
    pub extensions: Vec<RawExtension>,
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub dependencies: Vec<ModelDep>,
    pub span: (usize, usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelDep {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
}

#[derive(Debug, Clone)]
pub struct AliasDef {
    pub name: String,
    /// 原始 NodeId 文本（可能指向另一个 alias）
    pub target_text: String,
    pub span: (usize, usize),
}

#[derive(Debug, Clone)]
pub struct RawNode {
    pub class: String,
    /// 原始 NodeId 文本（可能是 alias 名）
    pub node_id_text: String,
    pub browse_name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// 其余属性原样保留（含 EngineeringUnits、Value 等领域量，携带单位/坐标基准）
    pub attrs: Vec<(String, String)>,
    pub references: Vec<RawReference>,
    /// 未识别的子元素（含 <Value> 等），原样保留
    pub extensions: Vec<RawExtension>,
    pub span: (usize, usize),
    pub raw_xml: String,
}

#[derive(Debug, Clone)]
pub struct RawReference {
    pub ref_type_text: String,
    pub is_forward: bool,
    pub target_text: String,
    pub span: (usize, usize),
}

#[derive(Debug, Clone)]
pub struct RawExtension {
    pub name: String,
    pub raw_xml: String,
    pub span: (usize, usize),
}

/// 解析一份 NodeSet XML。未知顶层元素与未知子元素全部保留为扩展。
pub fn parse_nodeset(name: &str, source: &str) -> Result<NodeSetFile, xml::XmlError> {
    let root = xml::parse(source)?;
    let mut file = NodeSetFile {
        name: name.to_string(),
        source: source.to_string(),
        namespace_uris: Vec::new(),
        models: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        extensions: Vec::new(),
    };
    for child in root.elements() {
        match child.name.as_str() {
            "NamespaceUris" => {
                for uri_el in child.children_named("Uri") {
                    file.namespace_uris.push(uri_el.text());
                }
            }
            "Models" => {
                for m in child.children_named("Model") {
                    file.models.push(parse_model(m));
                }
            }
            "Aliases" => {
                for a in child.children_named("Alias") {
                    file.aliases.push(AliasDef {
                        name: a.attr("Alias").unwrap_or("").to_string(),
                        target_text: a.text(),
                        span: a.span,
                    });
                }
            }
            n if NODE_CLASSES.contains(&n) => {
                file.nodes.push(parse_node(child, source));
            }
            _ => {
                file.extensions.push(RawExtension {
                    name: child.name.clone(),
                    raw_xml: child.raw(source).to_string(),
                    span: child.span,
                });
            }
        }
    }
    Ok(file)
}

fn parse_model(el: &Element) -> ModelInfo {
    let mut deps = Vec::new();
    for d in el.children_named("RequiredModel") {
        deps.push(ModelDep {
            model_uri: d.attr("ModelUri").unwrap_or("").to_string(),
            version: d.attr("Version").unwrap_or("").to_string(),
            publication_date: d.attr("PublicationDate").unwrap_or("").to_string(),
        });
    }
    ModelInfo {
        model_uri: el.attr("ModelUri").unwrap_or("").to_string(),
        version: el.attr("Version").unwrap_or("").to_string(),
        publication_date: el.attr("PublicationDate").unwrap_or("").to_string(),
        dependencies: deps,
        span: el.span,
    }
}

fn parse_node(el: &Element, source: &str) -> RawNode {
    let mut refs = Vec::new();
    let mut extensions = Vec::new();
    let mut display_name = None;
    let mut description = None;
    for child in el.elements() {
        match child.name.as_str() {
            "DisplayName" => display_name = Some(child.text()),
            "Description" => description = Some(child.text()),
            "References" => {
                for r in child.children_named("Reference") {
                    refs.push(RawReference {
                        ref_type_text: r.attr("ReferenceType").unwrap_or("").to_string(),
                        is_forward: r.attr("IsForward").map(|v| v != "false").unwrap_or(true),
                        target_text: r.text(),
                        span: r.span,
                    });
                }
            }
            _ => extensions.push(RawExtension {
                name: child.name.clone(),
                raw_xml: child.raw(source).to_string(),
                span: child.span,
            }),
        }
    }
    let attrs: Vec<(String, String)> = el
        .attrs
        .iter()
        .filter(|(k, _)| k != "NodeId" && k != "BrowseName")
        .cloned()
        .collect();
    RawNode {
        class: el.name.clone(),
        node_id_text: el.attr("NodeId").unwrap_or("").to_string(),
        browse_name: el.attr("BrowseName").unwrap_or("").to_string(),
        display_name,
        description,
        attrs,
        references: refs,
        extensions,
        span: el.span,
        raw_xml: el.raw(source).to_string(),
    }
}
