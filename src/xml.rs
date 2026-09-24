//! NodeSet XML parser. Preserves raw XML, source byte/line ranges, namespace
//! table, aliases, model dependencies and unknown extensions verbatim.

use anyhow::{anyhow, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NodeClass {
    Object,
    Variable,
    Method,
    ObjectType,
    VariableType,
    DataType,
    ReferenceType,
    View,
}

impl NodeClass {
    pub fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "UAObject" => NodeClass::Object,
            "UAVariable" => NodeClass::Variable,
            "UAMethod" => NodeClass::Method,
            "UAObjectType" => NodeClass::ObjectType,
            "UAVariableType" => NodeClass::VariableType,
            "UADataType" => NodeClass::DataType,
            "UAReferenceType" => NodeClass::ReferenceType,
            "UAView" => NodeClass::View,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeClass::Object => "UAObject",
            NodeClass::Variable => "UAVariable",
            NodeClass::Method => "UAMethod",
            NodeClass::ObjectType => "UAObjectType",
            NodeClass::VariableType => "UAVariableType",
            NodeClass::DataType => "UADataType",
            NodeClass::ReferenceType => "UAReferenceType",
            NodeClass::View => "UAView",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RawReference {
    pub ref_type: String,
    pub is_forward: bool,
    pub target: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RawNode {
    pub class: NodeClass,
    pub node_id: String,
    pub browse_name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub references: Vec<RawReference>,
    /// Unrecognized child elements (incl. Extensions) preserved verbatim.
    pub extensions_raw: Vec<String>,
    /// All attributes other than NodeId/BrowseName, preserved verbatim.
    pub extra_attrs: Vec<(String, String)>,
    pub raw_xml: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RequiredModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required: Vec<RequiredModel>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeSetFile {
    pub name: String,
    pub raw_xml: String,
    /// NamespaceUris[i] is the URI for local index i+1 (index 0 is the OPC UA core).
    pub namespace_uris: Vec<String>,
    pub aliases: Vec<(String, String)>,
    pub models: Vec<ModelInfo>,
    pub nodes: Vec<RawNode>,
    /// Unrecognized top-level elements preserved verbatim.
    pub unknown_top_level: Vec<String>,
}

pub fn offset_to_line(xml: &str, offset: usize) -> usize {
    xml.bytes().take(offset).filter(|b| *b == b'\n').count() + 1
}

fn tag_start(xml: &str, scan_from: usize) -> usize {
    xml[scan_from..]
        .find('<')
        .map(|i| scan_from + i)
        .unwrap_or(scan_from)
}

fn local_name(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name);
    s.rsplit(':').next().unwrap_or(&s).to_string()
}

fn read_attrs(e: &quick_xml::events::BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .filter_map(|a| a.ok())
        .map(|a| {
            (
                local_name(a.key.as_ref()),
                a.unescape_value().map(|v| v.into_owned()).unwrap_or_default(),
            )
        })
        .collect()
}

fn get_attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// Consume events until the end tag matching `depth` balance; returns raw slice end.
fn skip_element(reader: &mut Reader<&[u8]>, xml: &str, start: usize) -> Result<usize> {
    let mut depth = 1usize;
    loop {
        match reader.read_event()? {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(reader.buffer_position() as usize);
                }
            }
            Event::Empty(_) => {}
            Event::Eof => return Err(anyhow!("unexpected EOF inside element at {}", start)),
            _ => {}
        }
    }
}

fn parse_references(reader: &mut Reader<&[u8]>, xml: &str) -> Result<Vec<RawReference>> {
    let mut refs = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Start(e) if local_name(e.name().as_ref()) == "Reference" => {
                let attrs = read_attrs(&e);
                let ref_type = get_attr(&attrs, "ReferenceType").unwrap_or("").to_string();
                let is_forward = get_attr(&attrs, "IsForward")
                    .map(|v| v.eq_ignore_ascii_case("true"))
                    .unwrap_or(true);
                let mut target = String::new();
                loop {
                    match reader.read_event()? {
                        Event::Text(t) => target.push_str(&t.unescape().unwrap_or_default()),
                        Event::End(_) => break,
                        Event::Eof => return Err(anyhow!("EOF inside Reference")),
                        _ => {}
                    }
                }
                refs.push(RawReference { ref_type, is_forward, target: target.trim().to_string() });
            }
            Event::Empty(e) if local_name(e.name().as_ref()) == "Reference" => {
                let attrs = read_attrs(&e);
                refs.push(RawReference {
                    ref_type: get_attr(&attrs, "ReferenceType").unwrap_or("").to_string(),
                    is_forward: get_attr(&attrs, "IsForward")
                        .map(|v| v.eq_ignore_ascii_case("true"))
                        .unwrap_or(true),
                    target: String::new(),
                });
            }
            Event::End(e) if local_name(e.name().as_ref()) == "References" => return Ok(refs),
            Event::Eof => return Err(anyhow!("EOF inside References")),
            _ => {}
        }
    }
    let _ = xml;
}

fn parse_node_element(
    reader: &mut Reader<&[u8]>,
    xml: &str,
    class: NodeClass,
    attrs: Vec<(String, String)>,
    start: usize,
) -> Result<RawNode> {
    let mut node = RawNode {
        class,
        node_id: get_attr(&attrs, "NodeId").unwrap_or("").to_string(),
        browse_name: get_attr(&attrs, "BrowseName").unwrap_or("").to_string(),
        display_name: None,
        description: None,
        references: Vec::new(),
        extensions_raw: Vec::new(),
        extra_attrs: attrs
            .iter()
            .filter(|(k, _)| k != "NodeId" && k != "BrowseName")
            .cloned()
            .collect(),
        raw_xml: String::new(),
        byte_start: start,
        byte_end: start,
        line_start: 0,
        line_end: 0,
    };
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "DisplayName" => loop {
                        match reader.read_event()? {
                            Event::Text(t) => {
                                node.display_name =
                                    Some(t.unescape().unwrap_or_default().into_owned())
                            }
                            Event::End(_) => break,
                            Event::Eof => return Err(anyhow!("EOF inside DisplayName")),
                            _ => {}
                        }
                    },
                    "Description" => loop {
                        match reader.read_event()? {
                            Event::Text(t) => {
                                node.description =
                                    Some(t.unescape().unwrap_or_default().into_owned())
                            }
                            Event::End(_) => break,
                            Event::Eof => return Err(anyhow!("EOF inside Description")),
                            _ => {}
                        }
                    },
                    "References" => node.references = parse_references(reader, xml)?,
                    _ => {
                        // Unknown child (Extensions, vendor elements...): preserve raw.
                        let s = tag_start(xml, pos);
                        let end = skip_element(reader, xml, s)?;
                        node.extensions_raw.push(xml[s..end].to_string());
                    }
                }
            }
            Event::Empty(e) => {
                let name = local_name(e.name().as_ref());
                if name != "DisplayName" && name != "Description" && name != "References" {
                    let s = tag_start(xml, pos);
                    let end = reader.buffer_position() as usize;
                    node.extensions_raw.push(xml[s..end].to_string());
                }
            }
            Event::End(_) => {
                node.byte_end = reader.buffer_position() as usize;
                node.raw_xml = xml[node.byte_start..node.byte_end].to_string();
                node.line_start = offset_to_line(xml, node.byte_start);
                node.line_end = offset_to_line(xml, node.byte_end);
                return Ok(node);
            }
            Event::Eof => return Err(anyhow!("EOF inside node element")),
            _ => {}
        }
    }
}

pub fn parse_nodeset(name: &str, xml: &str) -> Result<NodeSetFile> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);
    let mut file = NodeSetFile {
        name: name.to_string(),
        raw_xml: xml.to_string(),
        namespace_uris: Vec::new(),
        aliases: Vec::new(),
        models: Vec::new(),
        nodes: Vec::new(),
        unknown_top_level: Vec::new(),
    };
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) => {
                let tag = local_name(e.name().as_ref());
                match tag.as_str() {
                    "UANodeSet" => {}
                    "NamespaceUris" => loop {
                        match reader.read_event()? {
                            Event::Start(e2) if local_name(e2.name().as_ref()) == "Uri" => {
                                if let Event::Text(t) = reader.read_event()? {
                                    file.namespace_uris
                                        .push(t.unescape().unwrap_or_default().into_owned());
                                }
                            }
                            Event::End(e2) if local_name(e2.name().as_ref()) == "NamespaceUris" => break,
                            Event::Eof => return Err(anyhow!("EOF inside NamespaceUris")),
                            _ => {}
                        }
                    },
                    "Aliases" => loop {
                        match reader.read_event()? {
                            Event::Start(e2) if local_name(e2.name().as_ref()) == "Alias" => {
                                let attrs = read_attrs(&e2);
                                let alias = get_attr(&attrs, "Alias").unwrap_or("").to_string();
                                let mut val = String::new();
                                loop {
                                    match reader.read_event()? {
                                        Event::Text(t) => {
                                            val.push_str(&t.unescape().unwrap_or_default())
                                        }
                                        Event::End(_) => break,
                                        Event::Eof => return Err(anyhow!("EOF inside Alias")),
                                        _ => {}
                                    }
                                }
                                file.aliases.push((alias, val.trim().to_string()));
                            }
                            Event::End(e2) if local_name(e2.name().as_ref()) == "Aliases" => break,
                            Event::Eof => return Err(anyhow!("EOF inside Aliases")),
                            _ => {}
                        }
                    },
                    "Models" => loop {
                        match reader.read_event()? {
                            Event::Start(e2) if local_name(e2.name().as_ref()) == "Model" => {
                                let attrs = read_attrs(&e2);
                                let mut model = ModelInfo {
                                    model_uri: get_attr(&attrs, "ModelUri").unwrap_or("").to_string(),
                                    version: get_attr(&attrs, "Version").map(|s| s.to_string()),
                                    publication_date: get_attr(&attrs, "PublicationDate")
                                        .map(|s| s.to_string()),
                                    required: Vec::new(),
                                };
                                loop {
                                    match reader.read_event()? {
                                        Event::Empty(e3)
                                            if local_name(e3.name().as_ref()) == "RequiredModel" =>
                                        {
                                            let a = read_attrs(&e3);
                                            model.required.push(RequiredModel {
                                                model_uri: get_attr(&a, "ModelUri")
                                                    .unwrap_or("")
                                                    .to_string(),
                                                version: get_attr(&a, "Version")
                                                    .map(|s| s.to_string()),
                                                publication_date: get_attr(&a, "PublicationDate")
                                                    .map(|s| s.to_string()),
                                            });
                                        }
                                        Event::Start(e3)
                                            if local_name(e3.name().as_ref()) == "RequiredModel" =>
                                        {
                                            let a = read_attrs(&e3);
                                            model.required.push(RequiredModel {
                                                model_uri: get_attr(&a, "ModelUri")
                                                    .unwrap_or("")
                                                    .to_string(),
                                                version: get_attr(&a, "Version")
                                                    .map(|s| s.to_string()),
                                                publication_date: get_attr(&a, "PublicationDate")
                                                    .map(|s| s.to_string()),
                                            });
                                            loop {
                                                match reader.read_event()? {
                                                    Event::End(_) => break,
                                                    Event::Eof => {
                                                        return Err(anyhow!(
                                                            "EOF inside RequiredModel"
                                                        ))
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                        Event::End(e3)
                                            if local_name(e3.name().as_ref()) == "Model" =>
                                        {
                                            break
                                        }
                                        Event::Eof => return Err(anyhow!("EOF inside Model")),
                                        _ => {}
                                    }
                                }
                                file.models.push(model);
                            }
                            Event::End(e2) if local_name(e2.name().as_ref()) == "Models" => break,
                            Event::Eof => return Err(anyhow!("EOF inside Models")),
                            _ => {}
                        }
                    },
                    _ => {
                        if let Some(class) = NodeClass::from_tag(&tag) {
                            let attrs = read_attrs(&e);
                            let start = tag_start(xml, pos);
                            let node = parse_node_element(&mut reader, xml, class, attrs, start)?;
                            file.nodes.push(node);
                        } else {
                            // Unknown top-level element: preserve raw.
                            let s = tag_start(xml, pos);
                            let end = skip_element(&mut reader, xml, s)?;
                            file.unknown_top_level.push(xml[s..end].to_string());
                        }
                    }
                }
            }
            Event::Empty(e) => {
                let tag = local_name(e.name().as_ref());
                if let Some(class) = NodeClass::from_tag(&tag) {
                    let attrs = read_attrs(&e);
                    let start = tag_start(xml, pos);
                    let end = reader.buffer_position() as usize;
                    let node = RawNode {
                        class,
                        node_id: get_attr(&attrs, "NodeId").unwrap_or("").to_string(),
                        browse_name: get_attr(&attrs, "BrowseName").unwrap_or("").to_string(),
                        display_name: None,
                        description: None,
                        references: Vec::new(),
                        extensions_raw: Vec::new(),
                        extra_attrs: attrs
                            .iter()
                            .filter(|(k, _)| k != "NodeId" && k != "BrowseName")
                            .cloned()
                            .collect(),
                        raw_xml: xml[start..end].to_string(),
                        byte_start: start,
                        byte_end: end,
                        line_start: offset_to_line(xml, start),
                        line_end: offset_to_line(xml, end),
                    };
                    file.nodes.push(node);
                } else if tag != "UANodeSet" {
                    let s = tag_start(xml, pos);
                    let end = reader.buffer_position() as usize;
                    file.unknown_top_level.push(xml[s..end].to_string());
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(file)
}
