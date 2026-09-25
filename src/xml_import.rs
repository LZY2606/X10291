//! UANodeSet XML 导入：保留原 XML 节点、namespace table、NodeId、BrowseName、
//! reference、alias、model dependency 与未识别扩展。
//! 文件内数字 namespace index 不做任何全局解释，仅原样记录。

use crate::model::*;
use anyhow::{anyhow, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

const NODE_CLASSES: &[&str] = &[
    "UAObject",
    "UAVariable",
    "UAMethod",
    "UAObjectType",
    "UAVariableType",
    "UADataType",
    "UAReferenceType",
    "UAView",
];

fn line_of(xml: &str, byte_pos: usize) -> usize {
    xml[..byte_pos.min(xml.len())].matches('\n').count() + 1
}

fn attr(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.as_ref() == name.as_bytes() {
            Some(String::from_utf8_lossy(&a.value).into_owned())
        } else {
            None
        }
    })
}

/// 捕获一个未知元素的原始 XML 片段（含起止标签，支持同名嵌套）。
fn capture_raw(reader: &Reader<&[u8]>, xml: &str, start_pos: usize) -> String {
    let end = reader.buffer_position();
    xml[start_pos..end].to_string()
}

pub fn parse_nodeset(source_name: &str, xml: &str) -> Result<RawNodeSet> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut ns = RawNodeSet {
        source_name: source_name.to_string(),
        source_xml: xml.to_string(),
        namespace_uris: Vec::new(),
        models: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        extensions: Vec::new(),
    };
    let mut buf = Vec::new();
    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                match name.as_str() {
                    "NamespaceUris" => read_namespace_uris(&mut reader, &mut ns)?,
                    "Models" => read_models(&mut reader, &mut ns)?,
                    "Aliases" => read_aliases(&mut reader, &mut ns)?,
                    n if NODE_CLASSES.contains(&n) => {
                        let node = read_node(&mut reader, xml, pos, &e, n)?;
                        ns.nodes.push(node);
                    }
                    "UANodeSet" => {}
                    _ => {
                        // 未识别扩展：保留原始 XML 片段。
                        reader.read_to_end_into(e.name(), &mut Vec::new())?;
                        ns.extensions.push(capture_raw(&reader, xml, pos));
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if ns.nodes.is_empty() && ns.namespace_uris.is_empty() {
        return Err(anyhow!("{}: 不是有效的 UANodeSet（无节点且无 namespace table）", source_name));
    }
    Ok(ns)
}

fn read_namespace_uris(reader: &mut Reader<&[u8]>, ns: &mut RawNodeSet) -> Result<()> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == b"Uri" => {
                let text = reader.read_text(e.name())?.into_owned();
                ns.namespace_uris.push(text);
            }
            Event::End(e) if e.name().as_ref() == b"NamespaceUris" => break,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn read_models(reader: &mut Reader<&[u8]>, ns: &mut RawNodeSet) -> Result<()> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == b"Model" => {
                let mut m = RawModel {
                    model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                    version: attr(&e, "Version").unwrap_or_default(),
                    publication_date: attr(&e, "PublicationDate").unwrap_or_default(),
                    required: Vec::new(),
                };
                // Model 的子元素（RequiredModel）读到 </Model>
                let mut inner = Vec::new();
                loop {
                    match reader.read_event_into(&mut inner)? {
                        Event::Empty(ie) | Event::Start(ie) => {
                            if ie.name().as_ref() == b"RequiredModel" {
                                m.required.push(RawModelDep {
                                    model_uri: attr(&ie, "ModelUri").unwrap_or_default(),
                                    version: attr(&ie, "Version").unwrap_or_default(),
                                });
                            }
                            if matches!(reader.read_event_into(&mut Vec::new()), Ok(Event::Eof)) {
                                break;
                            }
                        }
                        Event::End(ie) if ie.name().as_ref() == b"Model" => break,
                        Event::Eof => break,
                        _ => {}
                    }
                    inner.clear();
                }
                ns.models.push(m);
            }
            Event::End(e) if e.name().as_ref() == b"Models" => break,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn read_aliases(reader: &mut Reader<&[u8]>, ns: &mut RawNodeSet) -> Result<()> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == b"Alias" => {
                let name = attr(&e, "Alias").unwrap_or_default();
                let target = reader.read_text(e.name())?.into_owned();
                ns.aliases.push((name, target));
            }
            Event::End(e) if e.name().as_ref() == b"Aliases" => break,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn read_node(
    reader: &mut Reader<&[u8]>,
    xml: &str,
    start_pos: usize,
    e: &quick_xml::events::BytesStart,
    class: &str,
) -> Result<RawNode> {
    let mut node = RawNode {
        class: class.to_string(),
        node_id: attr(e, "NodeId").unwrap_or_default(),
        browse_name: attr(e, "BrowseName").unwrap_or_default(),
        display_name: None,
        description: None,
        value: None,
        references: Vec::new(),
        extensions: Vec::new(),
        xml_start: start_pos,
        xml_end: start_pos,
        line_start: line_of(xml, start_pos),
        line_end: 0,
    };
    let mut buf = Vec::new();
    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(&mut buf)? {
            Event::Start(ce) => {
                let cname = String::from_utf8_lossy(ce.name().as_ref()).into_owned();
                match cname.as_str() {
                    "DisplayName" => {
                        node.display_name = Some(reader.read_text(ce.name())?.into_owned())
                    }
                    "Description" => {
                        node.description = Some(reader.read_text(ce.name())?.into_owned())
                    }
                    "Value" => read_value(reader, &mut node)?,
                    "References" => read_references(reader, &mut node)?,
                    _ => {
                        // 未识别扩展（含 EngineeringUnits 等）：保留原始 XML，
                        // 同时尝试从中提取单位与坐标基准（不转换，仅记录原文）。
                        reader.read_to_end_into(ce.name(), &mut Vec::new())?;
                        let raw = capture_raw(reader, xml, pos);
                        extract_units(&raw, &mut node);
                        node.extensions.push(raw);
                    }
                }
            }
            Event::Empty(ce) => {
                let cname = String::from_utf8_lossy(ce.name().as_ref()).into_owned();
                if !matches!(cname.as_str(), "DisplayName" | "Description") {
                    let raw = capture_raw(reader, xml, pos);
                    extract_units(&raw, &mut node);
                    node.extensions.push(raw);
                }
            }
            Event::End(ee) if ee.name().as_ref() == class.as_bytes() => break,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    node.xml_end = reader.buffer_position();
    node.line_end = line_of(xml, node.xml_end);
    Ok(node)
}

fn read_value(reader: &mut Reader<&[u8]>, node: &mut RawNode) -> Result<()> {
    let mut buf = Vec::new();
    let mut text = String::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Text(t) => text.push_str(&t.decode()?.into_owned()),
            Event::End(e) if e.name().as_ref() == b"Value" => break,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let v = node.value.get_or_insert(RawValue {
        text: String::new(),
        unit_raw: None,
        coordinate_basis: None,
    });
    v.text = text.trim().to_string();
    Ok(())
}

fn read_references(reader: &mut Reader<&[u8]>, node: &mut RawNode) -> Result<()> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == b"Reference" => {
                let r = RawReference {
                    ref_type: attr(&e, "ReferenceType").unwrap_or_default(),
                    is_forward: attr(&e, "IsForward")
                        .map(|v| v != "false")
                        .unwrap_or(true),
                    target: String::new(),
                };
                let target = reader.read_text(e.name())?.into_owned();
                node.references.push(RawReference {
                    target,
                    ..r
                });
            }
            Event::End(e) if e.name().as_ref() == b"References" => break,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// 从未识别扩展原文中提取单位与坐标基准（仅记录，不转换）。
fn extract_units(raw: &str, node: &mut RawNode) {
    let get_attr = |key: &str| -> Option<String> {
        let pat = format!("{}=\"", key);
        raw.find(&pat).map(|i| {
            let rest = &raw[i + pat.len()..];
            rest[..rest.find('"').unwrap_or(rest.len())].to_string()
        })
    };
    if raw.contains("EngineeringUnits") || raw.contains("unitId") || raw.contains("displayName") {
        let unit = get_attr("displayName").or_else(|| get_attr("unitId"));
        let basis = get_attr("coordinateBasis");
        if unit.is_some() || basis.is_some() {
            let v = node.value.get_or_insert(RawValue {
                text: String::new(),
                unit_raw: None,
                coordinate_basis: None,
            });
            if v.unit_raw.is_none() {
                v.unit_raw = unit;
            }
            if v.coordinate_basis.is_none() {
                v.coordinate_basis = basis;
            }
        }
    }
}
