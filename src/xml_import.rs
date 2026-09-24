//! NodeSet XML 导入：原 XML 节点、namespace table、NodeId、BrowseName、
//! reference、alias、model dependency 与未识别扩展均保留。
//! 导入层不做任何规范化，只记录原始表示 + 源字节范围。

use crate::model::*;
use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ImportError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ImportError> {
    Err(ImportError(msg.into()))
}

pub fn sha256_hex(data: &str) -> String {
    let mut h = Sha256::new();
    h.update(data.as_bytes());
    format!("{:x}", h.finalize())
}

fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw).into_owned();
    match s.rsplit(':').next() {
        Some(l) => l.to_string(),
        None => s,
    }
}

fn attr(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        let key = local_name(a.key.as_ref());
        if key == name {
            return Some(String::from_utf8_lossy(a.value.as_ref()).into_owned());
        }
    }
    None
}

const NODE_CLASSES: &[&str] = &[
    "UAObject", "UAVariable", "UAMethod", "UAObjectType", "UAVariableType",
    "UAReferenceType", "UADataType", "UAView",
];

fn is_node_class(name: &str) -> bool {
    NODE_CLASSES.contains(&name)
}

/// 跳过当前元素直到与之匹配的 End，返回结束后的字节位置。
fn skip_element(reader: &mut Reader<&[u8]>) -> Result<usize, ImportError> {
    let mut depth = 1usize;
    loop {
        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(reader.buffer_position());
                }
            }
            Event::Eof => return err("unexpected EOF inside element"),
            _ => {}
        }
    }
}

/// 读取简单文本元素内容（直到匹配 End）。
fn read_text(reader: &mut Reader<&[u8]>) -> Result<String, ImportError> {
    let mut out = String::new();
    loop {
        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
            Event::Text(t) => out.push_str(&String::from_utf8_lossy(t.as_ref())),
            Event::CData(t) => out.push_str(&String::from_utf8_lossy(t.as_ref())),
            Event::End(_) => return Ok(out.trim().to_string()),
            Event::Eof => return err("unexpected EOF in text element"),
            _ => {}
        }
    }
}

fn parse_references(reader: &mut Reader<&[u8]>) -> Result<Vec<Reference>, ImportError> {
    let mut refs = Vec::new();
    loop {
        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
            Event::Start(e) => {
                if local_name(e.name().as_ref()) == "Reference" {
                    let reference_type_raw =
                        attr(&e, "ReferenceType").unwrap_or_else(|| "i=45".to_string());
                    let is_forward = attr(&e, "IsForward")
                        .map(|v| v != "false" && v != "0")
                        .unwrap_or(true);
                    let target_raw = read_text(reader)?;
                    refs.push(Reference {
                        reference_type_raw,
                        reference_type: None,
                        is_forward,
                        target_raw,
                        target: None,
                        unresolved_reason: None,
                    });
                }
            }
            Event::Empty(e) => {
                if local_name(e.name().as_ref()) == "Reference" {
                    refs.push(Reference {
                        reference_type_raw: attr(&e, "ReferenceType")
                            .unwrap_or_else(|| "i=45".to_string()),
                        reference_type: None,
                        is_forward: attr(&e, "IsForward")
                            .map(|v| v != "false" && v != "0")
                            .unwrap_or(true),
                        target_raw: String::new(),
                        target: None,
                        unresolved_reason: None,
                    });
                }
            }
            Event::End(e) => {
                if local_name(e.name().as_ref()) == "References" {
                    return Ok(refs);
                }
            }
            Event::Eof => return err("unexpected EOF in References"),
            _ => {}
        }
    }
}

/// 解析 Extensions：每个 Extension 的子元素作为未知扩展原样保留；
/// 若发现 EngineeringUnits 则提取单位标签。
fn parse_extensions(
    reader: &mut Reader<&[u8]>,
    xml: &str,
) -> Result<(Vec<UnknownExtension>, Option<String>), ImportError> {
    let mut exts = Vec::new();
    let mut unit = None;
    loop {
        let pos = reader.buffer_position();
        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
            Event::Start(e) => {
                let name = local_name(e.name().as_ref());
                if name == "EngineeringUnits" {
                    if let Some(u) = attr(&e, "Name").or_else(|| attr(&e, "DisplayName")) {
                        unit = Some(u);
                    }
                }
                let end = skip_element(reader)?;
                exts.push(UnknownExtension {
                    element: name,
                    raw_xml: xml[pos..end].to_string(),
                });
            }
            Event::Empty(e) => {
                let name = local_name(e.name().as_ref());
                if name == "EngineeringUnits" {
                    if let Some(u) = attr(&e, "Name").or_else(|| attr(&e, "DisplayName")) {
                        unit = Some(u);
                    }
                }
                let end = reader.buffer_position();
                exts.push(UnknownExtension {
                    element: name,
                    raw_xml: xml[pos..end].to_string(),
                });
            }
            Event::End(e) => {
                let name = local_name(e.name().as_ref());
                if name == "Extensions" || name == "Extension" {
                    return Ok((exts, unit));
                }
            }
            Event::Eof => return err("unexpected EOF in Extensions"),
            _ => {}
        }
    }
}

fn parse_node(
    reader: &mut Reader<&[u8]>,
    xml: &str,
    start: usize,
    e: &quick_xml::events::BytesStart,
    source_id: &str,
    file_name: &str,
) -> Result<Node, ImportError> {
    let class = local_name(e.name().as_ref());
    let node_id_raw = attr(e, "NodeId").unwrap_or_default();
    let browse_name_raw = attr(e, "BrowseName").unwrap_or_default();
    let mut display_name = None;
    let mut description = None;
    let mut references = Vec::new();
    let mut extensions = Vec::new();
    let mut value_raw: Option<String> = None;
    let mut unit_raw: Option<String> = None;
    let end;
    loop {
        let pos = reader.buffer_position();
        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
            Event::Start(c) => {
                let name = local_name(c.name().as_ref());
                match name.as_str() {
                    "DisplayName" => display_name = Some(read_text(reader)?),
                    "Description" => description = Some(read_text(reader)?),
                    "Value" => value_raw = Some(read_text(reader)?),
                    "References" => references = parse_references(reader)?,
                    "Extensions" | "Extension" => {
                        let (mut exts, unit) = parse_extensions(reader, xml)?;
                        extensions.append(&mut exts);
                        if unit.is_some() {
                            unit_raw = unit;
                        }
                    }
                    _ => {
                        let e2 = skip_element(reader)?;
                        extensions.push(UnknownExtension {
                            element: name,
                            raw_xml: xml[pos..e2].to_string(),
                        });
                    }
                }
            }
            Event::Empty(c) => {
                let name = local_name(c.name().as_ref());
                let e2 = reader.buffer_position();
                extensions.push(UnknownExtension {
                    element: name,
                    raw_xml: xml[pos..e2].to_string(),
                });
            }
            Event::End(en) => {
                if local_name(en.name().as_ref()) == class {
                    end = reader.buffer_position();
                    break;
                }
            }
            Event::Eof => return err("unexpected EOF inside node element"),
            _ => {}
        }
    }
    let value = value_raw.map(|raw| NodeValue {
        raw,
        unit_raw: unit_raw.clone(),
        normalized_si: None,
        si_unit: None,
        diagnostic: None,
    });
    Ok(Node {
        id: StableNodeId::new("", ""),
        node_id_raw,
        class,
        browse_name_raw,
        browse_name_global: String::new(),
        display_name,
        description,
        references,
        extensions,
        value,
        source: SourceRange { source_id: source_id.to_string(), start, end },
        source_file: file_name.to_string(),
        raw_xml: xml[start..end].to_string(),
    })
}

fn parse_models(
    reader: &mut Reader<&[u8]>,
    source_id: &str,
) -> Result<Vec<ModelInfo>, ImportError> {
    let mut models = Vec::new();
    loop {
        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
            Event::Empty(e) => {
                if local_name(e.name().as_ref()) == "Model" {
                    models.push(ModelInfo {
                        model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                        version: attr(&e, "Version"),
                        publication_date: attr(&e, "PublicationDate"),
                        required: Vec::new(),
                        source_id: source_id.to_string(),
                    });
                }
            }
            Event::Start(e) => {
                let name = local_name(e.name().as_ref());
                if name == "Model" {
                    let mut m = ModelInfo {
                        model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                        version: attr(&e, "Version"),
                        publication_date: attr(&e, "PublicationDate"),
                        required: Vec::new(),
                        source_id: source_id.to_string(),
                    };
                    // 读取 RequiredModel 子元素
                    loop {
                        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
                            Event::Start(r) | Event::Empty(r) => {
                                if local_name(r.name().as_ref()) == "RequiredModel" {
                                    m.required.push(RequiredModel {
                                        model_uri: attr(&r, "ModelUri").unwrap_or_default(),
                                        version: attr(&r, "Version"),
                                        publication_date: attr(&r, "PublicationDate"),
                                    });
                                }
                            }
                            Event::End(en) => {
                                if local_name(en.name().as_ref()) == "Model" {
                                    break;
                                }
                            }
                            Event::Eof => return err("unexpected EOF in Model"),
                            _ => {}
                        }
                    }
                    models.push(m);
                }
            }
            Event::End(e) => {
                if local_name(e.name().as_ref()) == "Models" {
                    return Ok(models);
                }
            }
            Event::Eof => return err("unexpected EOF in Models"),
            _ => {}
        }
    }
}

/// 导入一份 NodeSet XML，全部原始信息保留。
pub fn import_nodeset(
    source_id: &str,
    file_name: &str,
    xml: &str,
) -> Result<ImportedNodeSet, ImportError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut out = ImportedNodeSet {
        source_id: source_id.to_string(),
        file_name: file_name.to_string(),
        sha256: sha256_hex(xml),
        namespace_uris: Vec::new(),
        aliases: Vec::new(),
        models: Vec::new(),
        nodes: Vec::new(),
        unknown_root_elements: Vec::new(),
    };
    loop {
        let pos = reader.buffer_position();
        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
            Event::Start(e) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "NamespaceUris" => loop {
                        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
                            Event::Start(u)
                                if local_name(u.name().as_ref()) == "Uri" =>
                            {
                                out.namespace_uris.push(read_text(&mut reader)?);
                            }
                            Event::End(en)
                                if local_name(en.name().as_ref()) == "NamespaceUris" =>
                            {
                                break;
                            }
                            Event::Eof => return err("unexpected EOF in NamespaceUris"),
                            _ => {}
                        }
                    },
                    "Aliases" => loop {
                        match reader.read_event().map_err(|e| ImportError(e.to_string()))? {
                            Event::Start(a) if local_name(a.name().as_ref()) == "Alias" => {
                                let alias_name =
                                    attr(&a, "Alias").unwrap_or_default();
                                let target = read_text(&mut reader)?;
                                out.aliases.push((alias_name, target));
                            }
                            Event::End(en) if local_name(en.name().as_ref()) == "Aliases" => {
                                break;
                            }
                            Event::Eof => return err("unexpected EOF in Aliases"),
                            _ => {}
                        }
                    },
                    "Models" => {
                        out.models = parse_models(&mut reader, source_id)?;
                    }
                    n if is_node_class(n) => {
                        let node =
                            parse_node(&mut reader, xml, pos, &e, source_id, file_name)?;
                        out.nodes.push(node);
                    }
                    "UANodeSet" => {}
                    _ => {
                        let end = skip_element(&mut reader)?;
                        out.unknown_root_elements.push(UnknownExtension {
                            element: name,
                            raw_xml: xml[pos..end].to_string(),
                        });
                    }
                }
            }
            Event::Empty(e) => {
                let name = local_name(e.name().as_ref());
                if is_node_class(&name) {
                    let end = reader.buffer_position();
                    let node = Node {
                        id: StableNodeId::new("", ""),
                        node_id_raw: attr(&e, "NodeId").unwrap_or_default(),
                        class: name,
                        browse_name_raw: attr(&e, "BrowseName").unwrap_or_default(),
                        browse_name_global: String::new(),
                        display_name: None,
                        description: None,
                        references: Vec::new(),
                        extensions: Vec::new(),
                        value: None,
                        source: SourceRange {
                            source_id: source_id.to_string(),
                            start: pos,
                            end,
                        },
                        source_file: file_name.to_string(),
                        raw_xml: xml[pos..end].to_string(),
                    };
                    out.nodes.push(node);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}
