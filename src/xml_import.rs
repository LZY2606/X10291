//! NodeSet XML import. Raw layer: everything preserved, byte spans recorded.

use crate::model::*;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

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

/// Find the byte offset of the `<name` opening that ends at or before `pos`.
fn tag_start(raw: &str, pos: usize, name: &str, closing: bool) -> usize {
    let pat = if closing { format!("</{}", name) } else { format!("<{}", name) };
    raw[..pos.min(raw.len())].rfind(&pat).unwrap_or(pos)
}

pub fn parse_nodeset(file_id: &str, raw_xml: &str) -> Result<NodeSetFile, ImportError> {
    let mut reader = Reader::from_str(raw_xml);
    reader.config_mut().trim_text(true);
    let mut file = NodeSetFile {
        file_id: file_id.to_string(),
        namespaces: vec!["http://opcfoundation.org/UA/".to_string()],
        aliases: vec![],
        models: vec![],
        nodes: vec![],
        raw_xml: raw_xml.to_string(),
    };
    let mut buf = Vec::new();
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "NamespaceUris" => parse_namespace_uris(&mut reader, raw_xml, &mut file)?,
                    "Aliases" => parse_aliases(&mut reader, raw_xml, file_id, &mut file)?,
                    "Models" => parse_models(&mut reader, raw_xml, file_id, &mut file)?,
                    n if n.starts_with("UA") && n != "UANodeSet" => {
                        let start = tag_start(raw_xml, pos, &n, false);
                        let node = parse_node(&mut reader, raw_xml, file_id, &n, start)?;
                        file.nodes.push(node);
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return err(format!("XML parse error in {}: {}", file_id, e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(file)
}

fn read_text_until(reader: &mut Reader<&[u8]>, end_name: &str) -> Result<String, ImportError> {
    let mut text = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(e)) => text.push_str(&e.unescape().unwrap_or_default()),
            Ok(Event::CData(e)) => text.push_str(&String::from_utf8_lossy(&e)),
            Ok(Event::End(e)) if e.name().as_ref() == end_name.as_bytes() => break,
            Ok(Event::Eof) => return err(format!("unexpected EOF inside {}", end_name)),
            Err(e) => return err(format!("XML error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(text)
}

fn parse_namespace_uris(
    reader: &mut Reader<&[u8]>,
    _raw: &str,
    file: &mut NodeSetFile,
) -> Result<(), ImportError> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.name().as_ref() == b"Uri" => {
                let uri = read_text_until(reader, "Uri")?;
                file.namespaces.push(uri);
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"NamespaceUris" => break,
            Ok(Event::Eof) => return err("EOF in NamespaceUris"),
            Err(e) => return err(format!("XML error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn parse_aliases(
    reader: &mut Reader<&[u8]>,
    raw: &str,
    file_id: &str,
    file: &mut NodeSetFile,
) -> Result<(), ImportError> {
    let mut buf = Vec::new();
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"Alias" => {
                let start = tag_start(raw, pos, "Alias", false);
                let mut alias = String::new();
                for a in e.attributes().flatten() {
                    if a.key.as_ref() == b"Alias" {
                        alias = String::from_utf8_lossy(&a.value).to_string();
                    }
                }
                let target = read_text_until(reader, "Alias")?;
                let end = reader.buffer_position() as usize;
                file.aliases.push(AliasEntry {
                    alias,
                    target_text: target,
                    span: SourceSpan { file_id: file_id.to_string(), start, end },
                });
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"Aliases" => break,
            Ok(Event::Eof) => return err("EOF in Aliases"),
            Err(e) => return err(format!("XML error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn parse_models(
    reader: &mut Reader<&[u8]>,
    raw: &str,
    file_id: &str,
    file: &mut NodeSetFile,
) -> Result<(), ImportError> {
    let mut buf = Vec::new();
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"Model" => {
                let start = tag_start(raw, pos, "Model", false);
                let mut entry = ModelEntry {
                    model_uri: String::new(),
                    version: None,
                    publication_date: None,
                    required_models: vec![],
                    span: SourceSpan { file_id: file_id.to_string(), start, end: start },
                };
                for a in e.attributes().flatten() {
                    let v = String::from_utf8_lossy(&a.value).to_string();
                    match a.key.as_ref() {
                        b"ModelUri" => entry.model_uri = v,
                        b"Version" => entry.version = Some(v),
                        b"PublicationDate" => entry.publication_date = Some(v),
                        _ => {}
                    }
                }
                // read RequiredModel children until </Model>
                let mut inner = Vec::new();
                loop {
                    match reader.read_event_into(&mut inner) {
                        Ok(Event::Start(re)) | Ok(Event::Empty(re))
                            if re.name().as_ref() == b"RequiredModel" =>
                        {
                            let mut rm = RequiredModel {
                                model_uri: String::new(),
                                version: None,
                                publication_date: None,
                            };
                            for a in re.attributes().flatten() {
                                let v = String::from_utf8_lossy(&a.value).to_string();
                                match a.key.as_ref() {
                                    b"ModelUri" => rm.model_uri = v,
                                    b"Version" => rm.version = Some(v),
                                    b"PublicationDate" => rm.publication_date = Some(v),
                                    _ => {}
                                }
                            }
                            entry.required_models.push(rm);
                        }
                        Ok(Event::End(re)) if re.name().as_ref() == b"Model" => break,
                        Ok(Event::Eof) => return err("EOF in Model"),
                        Err(e) => return err(format!("XML error: {}", e)),
                        _ => {}
                    }
                    inner.clear();
                }
                entry.span.end = reader.buffer_position() as usize;
                file.models.push(entry);
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"Models" => break,
            Ok(Event::Eof) => return err("EOF in Models"),
            Err(e) => return err(format!("XML error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn node_class(tag: &str) -> Option<NodeClass> {
    Some(match tag {
        "UAObject" => NodeClass::Object,
        "UAVariable" => NodeClass::Variable,
        "UAMethod" => NodeClass::Method,
        "UAObjectType" => NodeClass::ObjectType,
        "UAVariableType" => NodeClass::VariableType,
        "UAReferenceType" => NodeClass::ReferenceType,
        "UADataType" => NodeClass::DataType,
        "UAView" => NodeClass::View,
        _ => return None,
    })
}

pub fn parse_browse_name(text: &str) -> QualifiedName {
    if let Some((ns, name)) = text.split_once(':') {
        if let Ok(n) = ns.parse::<u16>() {
            return QualifiedName { ns: n, name: name.to_string() };
        }
    }
    QualifiedName { ns: 0, name: text.to_string() }
}

fn parse_node(
    reader: &mut Reader<&[u8]>,
    raw: &str,
    file_id: &str,
    tag: &str,
    start: usize,
) -> Result<RawNode, ImportError> {
    // The Start event was already consumed; re-read attributes from raw slice.
    let tag_open_end = raw[start..]
        .find('>')
        .map(|i| start + i + 1)
        .ok_or_else(|| ImportError("malformed tag".into()))?;
    let header = &raw[start..tag_open_end];
    let mut node = RawNode {
        node_class: node_class(tag).ok_or_else(|| ImportError(format!("bad class {}", tag)))?,
        node_id_text: String::new(),
        browse_name: QualifiedName { ns: 0, name: String::new() },
        display_name: None,
        description: None,
        attrs: BTreeMap::new(),
        references: vec![],
        extensions: vec![],
        span: SourceSpan { file_id: file_id.to_string(), start, end: tag_open_end },
    };
    // Parse attributes of the open tag via quick-xml on the header slice.
    let mut hr = Reader::from_str(header);
    let mut hbuf = Vec::new();
    if let Ok(Event::Start(e)) | Ok(Event::Empty(e)) = hr.read_event_into(&mut hbuf) {
        for a in e.attributes().flatten() {
            let key = String::from_utf8_lossy(a.key.as_ref()).to_string();
            let val = a.unescape_value().unwrap_or_default().to_string();
            match key.as_str() {
                "NodeId" => node.node_id_text = val,
                "BrowseName" => node.browse_name = parse_browse_name(&val),
                _ => {
                    node.attrs.insert(key, val);
                }
            }
        }
    }
    let mut buf = Vec::new();
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "DisplayName" => {
                        node.display_name = Some(read_text_until(reader, "DisplayName")?)
                    }
                    "Description" => {
                        node.description = Some(read_text_until(reader, "Description")?)
                    }
                    "References" => parse_references(reader, &mut node)?,
                    other => {
                        // Unknown child element: preserve raw XML.
                        let estart = tag_start(raw, pos, other, false);
                        let raw_xml = capture_element(reader, raw, other, estart)?;
                        node.extensions.push(RawExtension {
                            element_name: other.to_string(),
                            raw_xml,
                        });
                    }
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == tag.as_bytes() => {
                node.span.end = reader.buffer_position() as usize;
                break;
            }
            Ok(Event::Eof) => return err(format!("EOF inside {}", tag)),
            Err(e) => return err(format!("XML error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(node)
}

/// Capture the raw XML of an element whose Start was just consumed, tracking depth.
fn capture_element(
    reader: &mut Reader<&[u8]>,
    raw: &str,
    name: &str,
    start: usize,
) -> Result<String, ImportError> {
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if e.name().as_ref() == name.as_bytes() {
                    depth += 1;
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == name.as_bytes() {
                    depth -= 1;
                    if depth == 0 {
                        let end = reader.buffer_position() as usize;
                        return Ok(raw[start..end].to_string());
                    }
                }
            }
            Ok(Event::Eof) => return err(format!("EOF capturing {}", name)),
            Err(e) => return err(format!("XML error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
}

fn parse_references(reader: &mut Reader<&[u8]>, node: &mut RawNode) -> Result<(), ImportError> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == b"Reference" => {
                let mut reference_type = String::new();
                let mut is_forward = true;
                for a in e.attributes().flatten() {
                    let v = a.unescape_value().unwrap_or_default().to_string();
                    match a.key.as_ref() {
                        b"ReferenceType" => reference_type = v,
                        b"IsForward" => is_forward = v != "false",
                        _ => {}
                    }
                }
                let target = read_text_until(reader, "Reference")?;
                node.references.push(RawReference { reference_type, is_forward, target });
            }
            Ok(Event::End(e)) if e.name().as_ref() == b"References" => break,
            Ok(Event::Eof) => return err("EOF in References"),
            Err(e) => return err(format!("XML error: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// Parse a local NodeId text like "ns=2;i=1001", "i=45", "s=Foo".
pub fn parse_local_node_id(text: &str) -> Option<LocalNodeId> {
    let mut ns = 0u16;
    let mut id: Option<NodeIdKind> = None;
    for part in text.trim().split(';') {
        if let Some(v) = part.strip_prefix("ns=") {
            ns = v.parse().ok()?;
        } else if let Some(v) = part.strip_prefix("i=") {
            id = Some(NodeIdKind::Numeric(v.parse().ok()?));
        } else if let Some(v) = part.strip_prefix("s=") {
            id = Some(NodeIdKind::Text(v.to_string()));
        } else if let Some(v) = part.strip_prefix("g=") {
            id = Some(NodeIdKind::Guid(v.to_string()));
        } else if let Some(v) = part.strip_prefix("b=") {
            id = Some(NodeIdKind::Opaque(v.to_string()));
        }
    }
    id.map(|id| LocalNodeId { ns, id })
}
