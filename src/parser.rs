use crate::model::{RawAttribute, RawNode, SourceRange};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::BTreeMap;

#[derive(Default, Clone)]
struct Frame {
    stable_id: String,
    start: usize,
    local_name: String,
    prefix: String,
    xml_namespace: String,
    depth: usize,
    attributes: Vec<RawAttribute>,
    declarations: BTreeMap<String, String>,
    text_parts: Vec<String>,
}

pub fn parse_raw_xml(source_id: &str, file_name: &str, content: &str) -> Result<Vec<RawNode>, String> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<Frame> = Vec::new();
    let mut raw_nodes = Vec::new();
    let mut buffer = Vec::new();

    loop {
        let event_start = reader.buffer_position() as usize;
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("XML parse error in {file_name}: {error}"))?;
        match event {
            Event::Start(start) => {
                let frame = begin_frame(source_id, file_name, &start, event_start, stack.len(), stack.last());
                stack.push(frame);
            }
            Event::Empty(start) => {
                let end = reader.buffer_position() as usize;
                let frame = begin_frame(source_id, file_name, &start, event_start, stack.len(), stack.last());
                raw_nodes.push(finish_frame(source_id, file_name, &frame, stack.last(), content, event_start, end));
            }
            Event::Text(text) => {
                if let Ok(value) = text.unescape() {
                    if let Some(frame) = stack.last_mut() {
                        frame.text_parts.push(value.to_string());
                    }
                }
            }
            Event::End(_) => {
                let end = reader.buffer_position() as usize;
                let frame = stack.pop().ok_or_else(|| format!("Unmatched XML end in {file_name}"))?;
                raw_nodes.push(finish_frame(source_id, file_name, &frame, stack.last(), content, frame.start, end));
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    raw_nodes.sort_by_key(|node| (node.start, node.end));
    Ok(raw_nodes)
}

fn begin_frame(
    source_id: &str,
    file_name: &str,
    start: &BytesStart<'_>,
    start_position: usize,
    depth: usize,
    parent: Option<&Frame>,
) -> Frame {
    let qname = String::from_utf8_lossy(start.name().as_ref()).to_string();
    let mut declarations = parent.map(|frame| frame.declarations.clone()).unwrap_or_default();
    let mut attributes = Vec::new();
    for attribute in start.attributes().flatten() {
        let raw_key = String::from_utf8_lossy(attribute.key.as_ref()).to_string();
        if raw_key == "xmlns" {
            declarations.insert(String::new(), attribute.unescape_value().map(|v| v.to_string()).unwrap_or_default());
        } else if let Some(prefix) = raw_key.strip_prefix("xmlns:") {
            declarations.insert(prefix.to_string(), attribute.unescape_value().map(|v| v.to_string()).unwrap_or_default());
        }
        let (attribute_prefix, attribute_local_name) = split_qname(&raw_key);
        let value = attribute.unescape_value().map(|v| v.to_string()).unwrap_or_default();
        attributes.push(RawAttribute {
            name: attribute_local_name,
            xml_namespace: declarations.get(&attribute_prefix).cloned().unwrap_or_default(),
            value,
            range: SourceRange {
                source_id: source_id.to_string(),
                file_name: file_name.to_string(),
                start: start_position,
                end: start_position,
            },
        });
    }
    let (prefix, local_from_qname) = split_qname(&qname);
    let local_name = String::from_utf8_lossy(start.name().local_name().as_ref()).to_string();
    let xml_namespace = declarations.get(&prefix).cloned().unwrap_or_else(|| {
        declarations.get(&String::new()).cloned().unwrap_or_else(|| {
            if local_from_qname.is_empty() {
                String::new()
            } else {
                "http://opcfoundation.org/UA/2011/03/UANodeSet.xsd".to_string()
            }
        })
    });
    Frame {
        stable_id: format!("{source_id}:{start_position}:{local_name}"),
        start: start_position,
        local_name,
        prefix,
        xml_namespace,
        depth,
        attributes,
        declarations,
        text_parts: Vec::new(),
    }
}

fn split_qname(value: &str) -> (String, String) {
    value
        .split_once(':')
        .map(|(prefix, local)| (prefix.to_string(), local.to_string()))
        .unwrap_or_else(|| (String::new(), value.to_string()))
}

fn finish_frame(
    source_id: &str,
    file_name: &str,
    frame: &Frame,
    parent: Option<&Frame>,
    content: &str,
    start: usize,
    end: usize,
) -> RawNode {
    let kind = if is_semantic_node(&frame.local_name) {
        "opcua_node"
    } else {
        "xml_element"
    };
    RawNode {
        stable_id: frame.stable_id.clone(),
        source_id: source_id.to_string(),
        file_name: file_name.to_string(),
        start,
        end,
        depth: frame.depth,
        kind: kind.to_string(),
        xml_namespace: frame.xml_namespace.clone(),
        local_name: frame.local_name.clone(),
        attributes: frame.attributes.clone(),
        text: frame.text_parts.join(""),
        parent_stable_id: parent.map(|value| value.stable_id.clone()),
        raw_xml: content.get(start..end).unwrap_or_default().to_string(),
    }
}

pub fn is_semantic_node(local_name: &str) -> bool {
    matches!(
        local_name,
        "UAObject" | "UAVariable" | "UAMethod" | "UAObjectType" | "UAVariableType" | "UADataType" | "UAReferenceType"
    )
}

pub fn attribute_value(node: &RawNode, name: &str) -> Option<String> {
    node.attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.clone())
}

pub fn direct_children<'a>(nodes: &'a [RawNode], parent: &RawNode) -> Vec<&'a RawNode> {
    nodes
        .iter()
        .filter(|node| node.parent_stable_id.as_deref() == Some(parent.stable_id.as_str()))
        .collect()
}

pub fn descendant_text(nodes: &[RawNode], ancestor: &RawNode, local_name: &str) -> Option<(String, SourceRange)> {
    let mut stack: Vec<&RawNode> = direct_children(nodes, ancestor);
    while let Some(node) = stack.pop() {
        if node.local_name == local_name {
            return Some((text(nodes, node), range(node)));
        }
        stack.extend(direct_children(nodes, node));
    }
    None
}

pub fn text(nodes: &[RawNode], node: &RawNode) -> String {
    if !node.text.trim().is_empty() {
        return node.text.trim().to_string();
    }
    direct_children(nodes, node)
        .iter()
        .find(|child| child.local_name == "Text")
        .map(|child| child.text.trim().to_string())
        .unwrap_or_default()
}

pub fn range(node: &RawNode) -> SourceRange {
    SourceRange {
        source_id: node.source_id.clone(),
        file_name: node.file_name.clone(),
        start: node.start,
        end: node.end,
    }
}
