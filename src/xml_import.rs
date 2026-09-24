//! NodeSet XML import. Everything is preserved: original XML fragments,
//! namespace table, NodeIds, BrowseNames, references, aliases, model
//! dependencies and unrecognized extensions. Source byte ranges are recorded
//! so every displayed value can be traced back to the source XML.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

#[derive(Debug, Clone, Default)]
pub struct ParsedNodeSet {
    pub namespace_uris: Vec<String>,
    pub models: Vec<ParsedModel>,
    pub aliases: Vec<(String, String)>,
    pub nodes: Vec<ParsedNode>,
    /// Unrecognized top-level elements, preserved verbatim.
    pub unknown_top: Vec<ParsedExtension>,
}

#[derive(Debug, Clone)]
pub struct ParsedModel {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
    pub required: Vec<ParsedRequiredModel>,
}

#[derive(Debug, Clone)]
pub struct ParsedRequiredModel {
    pub model_uri: String,
    pub version: String,
    pub publication_date: String,
}

#[derive(Debug, Clone)]
pub struct ParsedNode {
    pub node_class: String,
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub attrs: Vec<(String, String)>,
    pub references: Vec<ParsedReference>,
    pub extensions: Vec<ParsedExtension>,
    /// Raw XML fragment of the whole node element, byte-for-byte.
    pub raw_xml: String,
    pub src_start: usize,
    pub src_end: usize,
    pub ord: usize,
}

#[derive(Debug, Clone)]
pub struct ParsedReference {
    pub ref_type: String,
    pub is_forward: bool,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct ParsedExtension {
    pub element: String,
    pub raw_xml: String,
    pub src_start: usize,
    pub src_end: usize,
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

fn all_attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .flatten()
        .map(|a| {
            (
                String::from_utf8_lossy(a.key.as_ref()).to_string(),
                String::from_utf8_lossy(&a.value).to_string(),
            )
        })
        .collect()
}

fn local_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_string()
}

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

/// Known children of a UANode element; anything else is an extension and is
/// preserved verbatim.
const KNOWN_CHILDREN: &[&str] = &["DisplayName", "Description", "References", "Value"];

/// Consume events until the matching end tag for `name`; returns the byte
/// offset just past the end tag.
fn skip_element(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>, name: &[u8]) -> usize {
    let mut depth = 1usize;
    loop {
        match reader.read_event_into(buf) {
            Ok(Event::Start(e)) if e.name().as_ref() == name => depth += 1,
            Ok(Event::End(e)) if e.name().as_ref() == name => {
                depth -= 1;
                if depth == 0 {
                    return reader.buffer_position();
                }
            }
            Ok(Event::Eof) => return reader.buffer_position(),
            Ok(_) => {}
            Err(_) => return reader.buffer_position(),
        }
    }
}

pub fn parse_nodeset(xml: &str) -> Result<ParsedNodeSet, String> {
    let bytes = xml.as_bytes();
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut out = ParsedNodeSet::default();
    let mut buf = Vec::new();
    let mut ord = 0usize;

    loop {
        let pos = reader.buffer_position();
        let ev = match reader.read_event_into(&mut buf) {
            Ok(e) => e,
            Err(e) => return Err(format!("XML parse error at byte {}: {}", pos, e)),
        };
        match ev {
            Event::Start(e) => {
                let name = local_name(&e);
                match name.as_str() {
                    "UANodeSet" => {}
                    "NamespaceUris" => {
                        let end = skip_element(&mut reader, &mut buf, e.name().as_ref());
                        let frag = &xml[pos..end];
                        let mut inner = Reader::from_str(frag);
                        inner.config_mut().trim_text(true);
                        let mut b2 = Vec::new();
                        loop {
                            match inner.read_event_into(&mut b2) {
                                Ok(Event::Start(e2)) if local_name(&e2) == "Uri" => {
                                    if let Ok(Event::Text(t)) =
                                        inner.read_event_into(&mut b2)
                                    {
                                        out.namespace_uris
                                            .push(t.decode().unwrap_or_default().to_string());
                                    }
                                }
                                Ok(Event::Eof) => break,
                                _ => {}
                            }
                        }
                    }
                    "Models" => {
                        let end = skip_element(&mut reader, &mut buf, e.name().as_ref());
                        parse_models(&xml[pos..end], &mut out);
                    }
                    "Aliases" => {
                        let end = skip_element(&mut reader, &mut buf, e.name().as_ref());
                        let mut inner = Reader::from_str(&xml[pos..end]);
                        inner.config_mut().trim_text(true);
                        let mut b2 = Vec::new();
                        loop {
                            match inner.read_event_into(&mut b2) {
                                Ok(Event::Start(e2)) if local_name(&e2) == "Alias" => {
                                    let alias =
                                        attr(&e2, "Alias").unwrap_or_default();
                                    if let Ok(Event::Text(t)) =
                                        inner.read_event_into(&mut b2)
                                    {
                                        out.aliases.push((
                                            alias,
                                            t.decode().unwrap_or_default().to_string(),
                                        ));
                                    }
                                }
                                Ok(Event::Eof) => break,
                                _ => {}
                            }
                        }
                    }
                    n if NODE_CLASSES.contains(&n) => {
                        let end = skip_element(&mut reader, &mut buf, e.name().as_ref());
                        let mut node = parse_node(&xml[pos..end], n)?;
                        node.src_start = pos;
                        node.src_end = end;
                        node.ord = ord;
                        ord += 1;
                        out.nodes.push(node);
                    }
                    _ => {
                        // Unknown top-level element: preserve verbatim.
                        let end = skip_element(&mut reader, &mut buf, e.name().as_ref());
                        out.unknown_top.push(ParsedExtension {
                            element: name,
                            raw_xml: xml[pos..end].to_string(),
                            src_start: pos,
                            src_end: end,
                        });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn parse_models(frag: &str, out: &mut ParsedNodeSet) {
    let mut reader = Reader::from_str(frag);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut current: Option<ParsedModel> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(&e).as_str() {
                "Model" => {
                    current = Some(ParsedModel {
                        model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                        version: attr(&e, "Version").unwrap_or_default(),
                        publication_date: attr(&e, "PublicationDate").unwrap_or_default(),
                        required: Vec::new(),
                    });
                }
                "RequiredModel" => {
                    if let Some(m) = current.as_mut() {
                        m.required.push(ParsedRequiredModel {
                            model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                            version: attr(&e, "Version").unwrap_or_default(),
                            publication_date: attr(&e, "PublicationDate")
                                .unwrap_or_default(),
                        });
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) => {
                if local_name_bytes(e.name().as_ref()) == "Model" {
                    if let Some(m) = current.take() {
                        out.models.push(m);
                    }
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
}

fn local_name_bytes(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    s.rsplit(':').next().unwrap_or(&s).to_string()
}

fn parse_node(frag: &str, node_class: &str) -> Result<ParsedNode, String> {
    let mut reader = Reader::from_str(frag);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut node = ParsedNode {
        node_class: node_class.to_string(),
        node_id: String::new(),
        browse_name: String::new(),
        display_name: String::new(),
        attrs: Vec::new(),
        references: Vec::new(),
        extensions: Vec::new(),
        raw_xml: frag.to_string(),
        src_start: 0,
        src_end: 0,
        ord: 0,
    };
    // Root element of the fragment.
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                node.node_id = attr(&e, "NodeId").unwrap_or_default();
                node.browse_name = attr(&e, "BrowseName").unwrap_or_default();
                node.attrs = all_attrs(&e);
                break;
            }
            Ok(Event::Eof) => return Ok(node),
            _ => {}
        }
        buf.clear();
    }
    // Children.
    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(&e);
                match name.as_str() {
                    "DisplayName" => {
                        if let Ok(Event::Text(t)) = reader.read_event_into(&mut buf) {
                            node.display_name = t.decode().unwrap_or_default().to_string();
                        }
                    }
                    "References" => {
                        let end = skip_element(&mut reader, &mut buf, e.name().as_ref());
                        parse_references(&frag[pos..end], &mut node);
                    }
                    "Description" | "Value" => {
                        // Known child: kept inside raw_xml of the node; also
                        // preserved individually so the derived layer can read
                        // values without re-parsing.
                        let end = skip_element(&mut reader, &mut buf, e.name().as_ref());
                        node.extensions.push(ParsedExtension {
                            element: name,
                            raw_xml: frag[pos..end].to_string(),
                            src_start: pos,
                            src_end: end,
                        });
                    }
                    _ => {
                        // Unrecognized extension: preserved verbatim.
                        let end = skip_element(&mut reader, &mut buf, e.name().as_ref());
                        node.extensions.push(ParsedExtension {
                            element: name,
                            raw_xml: frag[pos..end].to_string(),
                            src_start: pos,
                            src_end: end,
                        });
                    }
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(node)
}

fn parse_references(frag: &str, node: &mut ParsedNode) {
    let mut reader = Reader::from_str(frag);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if local_name(&e) == "Reference" => {
                let ref_type = attr(&e, "ReferenceType").unwrap_or_default();
                let is_forward = attr(&e, "IsForward")
                    .map(|v| v != "false")
                    .unwrap_or(true);
                if let Ok(Event::Text(t)) = reader.read_event_into(&mut buf) {
                    node.references.push(ParsedReference {
                        ref_type,
                        is_forward,
                        target: t.decode().unwrap_or_default().to_string(),
                    });
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
}
