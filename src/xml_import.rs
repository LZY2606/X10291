//! NodeSet XML importer.
//!
//! Preserves everything: original nodes, namespace table, NodeIds, BrowseNames,
//! references, aliases, model dependencies and unrecognized extensions (kept as
//! raw XML fragments). Every node records its source span (file + line range).

use crate::model::*;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

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

fn line_of(xml: &str, byte: usize) -> usize {
    xml.bytes().take(byte).filter(|b| *b == b'\n').count() + 1
}

fn span(file: &str, xml: &str, start: usize, end: usize) -> SourceSpan {
    SourceSpan {
        file: file.to_string(),
        byte_start: start,
        byte_end: end,
        line_start: line_of(xml, start),
        line_end: line_of(xml, end),
    }
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.as_ref() == name.as_bytes() {
            Some(String::from_utf8_lossy(&a.value).to_string())
        } else {
            None
        }
    })
}

/// Capture the raw XML of the element whose Start event begins at `start_off`,
/// consuming events until its matching End. Returns (raw_xml, end_offset).
fn capture_raw(
    reader: &Reader<&[u8]>,
    xml: &str,
    start_off: usize,
    first: &BytesStart,
) -> (String, usize) {
    let mut depth = 1usize;
    let name = first.name().as_ref().to_vec();
    let mut r = reader.clone();
    loop {
        match r.read_event() {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(e)) => {
                depth -= 1;
                if depth == 0 && e.name().as_ref() == name.as_slice() {
                    let end = r.buffer_position() as usize;
                    return (xml[start_off..end].to_string(), end);
                }
            }
            Ok(Event::Eof) => {
                let end = r.buffer_position() as usize;
                return (xml[start_off..end.min(xml.len())].to_string(), end);
            }
            _ => {}
        }
    }
}

/// Read events until the matching End of `elem_name`; calls `on_event` for each
/// nested event. Returns the byte offset just past the End event.
fn skip_element(reader: &mut Reader<&[u8]>, elem_name: &[u8]) {
    let mut depth = 1usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            Ok(Event::Eof) => return,
            Err(_) => return,
            _ => {}
        }
    }
    #[allow unreachable_code]
    {
        let _ = elem_name;
    }
}

pub fn parse_nodeset(file_name: &str, xml: &str) -> Result<NodeSetDoc, String> {
    let mut doc = NodeSetDoc {
        file_name: file_name.to_string(),
        namespace_uris: vec![],
        aliases: vec![],
        models: vec![],
        nodes: vec![],
        unknown_top_level: vec![],
        raw_xml: xml.to_string(),
    };
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_vec();
                match name.as_slice() {
                    b"UANodeSet" => {}
                    b"NamespaceUris" => parse_namespace_uris(&mut reader, &mut doc)?,
                    b"Models" => parse_models(&mut reader, &mut doc)?,
                    b"Aliases" => parse_aliases(&mut reader, &mut doc, xml, file_name)?,
                    n if NODE_CLASSES.contains(&std::str::from_utf8(n).unwrap_or("")) => {
                        let node = parse_node(
                            &mut reader,
                            xml,
                            file_name,
                            pos,
                            &e,
                            std::str::from_utf8(n).unwrap(),
                        )?;
                        doc.nodes.push(node);
                    }
                    _ => {
                        // Unknown top-level element: preserve raw.
                        let (raw, _) = capture_raw(&reader, xml, pos, &e);
                        doc.unknown_top_level.push(raw);
                        skip_element(&mut reader, &name);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => return Err(format!("XML parse error in {file_name}: {err}")),
            _ => {}
        }
    }
    Ok(doc)
}

fn parse_namespace_uris(reader: &mut Reader<&[u8]>, doc: &mut NodeSetDoc) -> Result<(), String> {
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"Uri" => {
                if let Ok(Event::Text(t)) = reader.read_event() {
                    doc.namespace_uris.push(t.decode().unwrap_or_default().to_string());
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"NamespaceUris" => return Ok(()),
            Ok(Event::Eof) => return Ok(()),
            Err(err) => return Err(format!("XML error: {err}")),
            _ => {}
        }
    }
}

fn parse_models(reader: &mut Reader<&[u8]>, doc: &mut NodeSetDoc) -> Result<(), String> {
    loop {
        match reader.read_event() {
            Ok(Event::Empty(e)) if e.name().as_ref() == b"Model" => {
                doc.models.push(ModelInfo {
                    model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                    version: attr(&e, "Version"),
                    publication_date: attr(&e, "PublicationDate"),
                    required_models: vec![],
                });
            }
            Ok(Event::Start(e)) if e.name().as_ref() == b"Model" => {
                let mut model = ModelInfo {
                    model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                    version: attr(&e, "Version"),
                    publication_date: attr(&e, "PublicationDate"),
                    required_models: vec![],
                };
                loop {
                    match reader.read_event() {
                        Ok(Event::Empty(r)) | Ok(Event::Start(r))
                            if r.name().as_ref() == b"RequiredModel" =>
                        {
                            model.required_models.push(ModelDependency {
                                model_uri: attr(&r, "ModelUri").unwrap_or_default(),
                                version: attr(&r, "Version"),
                                publication_date: attr(&r, "PublicationDate"),
                            });
                        }
                        Ok(Event::End(x)) if x.name().as_ref() == b"Model" => break,
                        Ok(Event::Eof) => break,
                        Err(err) => return Err(format!("XML error: {err}")),
                        _ => {}
                    }
                }
                doc.models.push(model);
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"Models" => return Ok(()),
            Ok(Event::Eof) => return Ok(()),
            Err(err) => return Err(format!("XML error: {err}")),
            _ => {}
        }
    }
}

fn parse_aliases(
    reader: &mut Reader<&[u8]>,
    doc: &mut NodeSetDoc,
    xml: &str,
    file_name: &str,
) -> Result<(), String> {
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"Alias" => {
                let name = attr(&e, "Alias").unwrap_or_default();
                let mut target = String::new();
                if let Ok(Event::Text(t)) = reader.read_event() {
                    target = t.decode().unwrap_or_default().to_string();
                }
                let end = reader.buffer_position() as usize;
                doc.aliases.push(Alias {
                    name,
                    target_raw: target,
                    span: span(file_name, xml, pos, end),
                });
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"Aliases" => return Ok(()),
            Ok(Event::Eof) => return Ok(()),
            Err(err) => return Err(format!("XML error: {err}")),
            _ => {}
        }
    }
}

fn parse_node(
    reader: &mut Reader<&[u8]>,
    xml: &str,
    file_name: &str,
    start_off: usize,
    e: &BytesStart,
    class: &str,
) -> Result<UaNode, String> {
    let mut node = UaNode {
        node_id_raw: attr(e, "NodeId").unwrap_or_default(),
        browse_name_raw: attr(e, "BrowseName").unwrap_or_default(),
        display_name: String::new(),
        node_class: class.to_string(),
        extra_attrs: vec![],
        references: vec![],
        extensions: vec![],
        value: None,
        span: span(file_name, xml, start_off, start_off),
    };
    let known_attrs = ["NodeId", "BrowseName", "SymbolicName"];
    for a in e.attributes().flatten() {
        let key = String::from_utf8_lossy(a.key.as_ref()).to_string();
        if !known_attrs.contains(&key.as_str()) {
            node.extra_attrs
                .push((key, String::from_utf8_lossy(&a.value).to_string()));
        }
    }

    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(c)) => {
                let cname = c.name().as_ref().to_vec();
                match cname.as_slice() {
                    b"DisplayName" => {
                        if let Ok(Event::Text(t)) = reader.read_event() {
                            node.display_name = t.decode().unwrap_or_default().to_string();
                        }
                    }
                    b"References" => parse_references(reader, &mut node, xml, file_name)?,
                    b"Value" => {
                        let (raw, _) = capture_raw(reader, xml, pos, &c);
                        node.value = Some(extract_quantity(&raw));
                        skip_element(reader, &cname);
                    }
                    b"Extensions" => {
                        // Preserve each extension child as raw XML.
                        loop {
                            let epos = reader.buffer_position() as usize;
                            match reader.read_event() {
                                Ok(Event::Start(x)) => {
                                    let (raw, _) = capture_raw(reader, xml, epos, &x);
                                    node.extensions.push(raw);
                                    let xn = x.name().as_ref().to_vec();
                                    skip_element(reader, &xn);
                                }
                                Ok(Event::Empty(x)) => {
                                    let end = reader.buffer_position() as usize;
                                    node.extensions.push(xml[epos..end].to_string());
                                }
                                Ok(Event::End(x)) if x.name().as_ref() == b"Extensions" => break,
                                Ok(Event::Eof) => break,
                                Err(err) => return Err(format!("XML error: {err}")),
                                _ => {}
                            }
                        }
                    }
                    _ => {
                        // Unknown child element: preserve raw as an extension.
                        let (raw, _) = capture_raw(reader, xml, pos, &c);
                        node.extensions.push(raw);
                        skip_element(reader, &cname);
                    }
                }
            }
            Ok(Event::End(c)) if c.name().as_ref() == class.as_bytes() => {
                let end = reader.buffer_position() as usize;
                node.span = span(file_name, xml, start_off, end);
                return Ok(node);
            }
            Ok(Event::Eof) => {
                node.span = span(file_name, xml, start_off, xml.len());
                return Ok(node);
            }
            Err(err) => return Err(format!("XML error: {err}")),
            _ => {}
        }
    }
}

fn parse_references(
    reader: &mut Reader<&[u8]>,
    node: &mut UaNode,
    xml: &str,
    file_name: &str,
) -> Result<(), String> {
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Empty(e)) if e.name().as_ref() == b"Reference" => {
                let end = reader.buffer_position() as usize;
                node.references.push(Reference {
                    reference_type_raw: attr(&e, "ReferenceType").unwrap_or_default(),
                    is_forward: attr(&e, "IsForward").map(|v| v != "false").unwrap_or(true),
                    target_raw: String::new(),
                    span: span(file_name, xml, pos, end),
                });
                continue;
            }
            Ok(Event::Start(e)) if e.name().as_ref() == b"Reference" => {
                let reference_type_raw = attr(&e, "ReferenceType").unwrap_or_default();
                let is_forward = attr(&e, "IsForward").map(|v| v != "false").unwrap_or(true);
                let mut target_raw = String::new();
                match reader.read_event() {
                    Ok(Event::Text(t)) => {
                        target_raw = t.decode().unwrap_or_default().to_string();
                    }
                    Ok(Event::End(_)) => {}
                    _ => {}
                }
                let end = reader.buffer_position() as usize;
                node.references.push(Reference {
                    reference_type_raw,
                    is_forward,
                    target_raw,
                    span: span(file_name, xml, pos, end),
                });
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"References" => return Ok(()),
            Ok(Event::Eof) => return Ok(()),
            Err(err) => return Err(format!("XML error: {err}")),
            _ => {}
        }
    }
}

/// Extract a Quantity from a raw `<Value>` fragment, keeping the raw text and
/// any declared unit/coordinate basis. Normalization is deferred to merge.
fn extract_quantity(raw_value_xml: &str) -> Quantity {
    // Pull the innermost text of the Value element as the raw value.
    let mut reader = Reader::from_str(raw_value_xml);
    reader.config_mut().trim_text(true);
    let mut texts = Vec::new();
    let mut unit = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                for a in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(a.key.as_ref()).to_string();
                    if key == "Unit" || key == "uom" || key == "EngineeringUnits" {
                        unit = Some(String::from_utf8_lossy(&a.value).to_string());
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Ok(s) = t.decode() {
                    if !s.trim().is_empty() {
                        texts.push(s.trim().to_string());
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    Quantity {
        raw_value: texts.join(""),
        unit,
    }
}
