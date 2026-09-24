//! Streaming NodeSet XML parser.
//!
//! Retains the original element spans and every unrecognized element so the
//! merge layer never has to re-read or guess.

use crate::model::*;
use quick_xml::events::Event;
use quick_xml::Reader;

const NODE_TAGS: &[&str] = &[
    "UAObject",
    "UAVariable",
    "UAMethod",
    "UAObjectType",
    "UAVariableType",
    "UAReferenceType",
    "UADataType",
    "UAView",
];

fn line_at(content: &[u8], offset: usize) -> usize {
    content[..offset.min(content.len())].iter().filter(|&&b| b == b'\n').count() + 1
}

fn local(tag: &[u8]) -> String {
    String::from_utf8_lossy(tag)
        .rsplit_once('}')
        .map(|(_, l)| l)
        .unwrap_or_else(|| std::str::from_utf8(tag).unwrap_or(""))
        .to_string()
}

fn attr_map_start(e: &quick_xml::events::BytesStart) -> std::collections::BTreeMap<String, String> {
    e.attributes().flatten().map(|a| {
        (local(a.key.into_inner()), a.unescape_value().unwrap_or_default().into_owned())
    }).collect()
}
fn attr_map_empty(e: &quick_xml::events::BytesEnd) -> std::collections::BTreeMap<String, String> {
    let _ = e;
    std::collections::BTreeMap::new()
}

fn decode(bytes: &[u8]) -> String {
    quick_xml::Reader::from_str("").decoder().decode(bytes).map(|c| c.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned())
}

struct StackFrame {
    el: RawElement,
    start: usize,
    text: Vec<u8>,
}

/// Parse one NodeSet file. Never fails hard: structural problems become
/// parse diagnostics so nothing is silently dropped.
pub fn parse_file(path: &str, content: &str) -> RawFile {
    let file_id = stable_id("file", &[path, content]);
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut stack: Vec<StackFrame> = Vec::new();
    let mut root_children: Vec<RawElement> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let mut pos = reader.buffer_position() as usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = local(e.name().as_ref());
                let el = RawElement {
                    tag,
                    attrs: attr_map_start(&e),
                    text: None,
                    children: Vec::new(),
                    span: Span { start: pos, end: 0, start_line: line_at(content.as_bytes(), pos), end_line: 0 },
                    unknown: false,
                };
                stack.push(StackFrame { el, start: pos, text: Vec::new() });
            }
            Ok(Event::Empty(e)) => {
                let tag = local(e.name().as_ref());
                let end = reader.buffer_position() as usize;
                let mut el = RawElement {
                    tag,
                    attrs: std::collections::BTreeMap::new(),
                    text: None,
                    children: Vec::new(),
                    span: Span {
                        start: pos,
                        end,
                        start_line: line_at(content.as_bytes(), pos),
                        end_line: line_at(content.as_bytes(), end),
                    },
                    unknown: false,
                };
                attach(&mut stack, &mut root_children, &mut el);
            }
            Ok(Event::Text(t)) => {
                if let Some(frame) = stack.last_mut() {
                    if let Ok(s) = t.unescape() {
                        frame.text = s.into_owned().into_bytes();
                    }
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(frame) = stack.last_mut() {
                    frame.text.extend_from_slice(t.as_ref());
                }
            }
            Ok(Event::End(_)) => {
                let end = reader.buffer_position() as usize;
                if let Some(mut frame) = stack.pop() {
                    let txt = String::from_utf8_lossy(&frame.text).to_string();
                    frame.el.text = if txt.is_empty() { None } else { Some(txt) };
                    frame.el.span.end = end;
                    frame.el.span.end_line = line_at(content.as_bytes(), end);
                    attach(&mut stack, &mut root_children, &mut frame.el);
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                diagnostics.push(Diagnostic {
                    code: "XML_PARSE_ERROR".into(),
                    severity: Severity::Error,
                    message: format!("{err}"),
                    file_id: Some(file_id.clone()),
                    span: Some(Span { start: pos, end: pos, start_line: line_at(content.as_bytes(), pos), end_line: line_at(content.as_bytes(), pos) }),
                    subject: None,
                });
                break;
            }
            _ => {}
        }
        pos = reader.buffer_position() as usize;
    }

    let root = root_children.into_iter().find(|e| e.tag == "UANodeSet");

    let mut namespaces = Vec::new();
    let mut aliases = Vec::new();
    let mut models = Vec::new();
    let mut top_extensions = Vec::new();
    let mut nodes = Vec::new();

    let known_top = ["NamespaceUris", "ServerUris", "Aliases", "Models"];

    let children: Vec<RawElement> = root.map(|r| r.children).unwrap_or_default();
    if root.is_none() {
        diagnostics.push(Diagnostic {
            code: "MISSING_UANODESET_ROOT".into(),
            severity: Severity::Error,
            message: "no UANodeSet root element found".into(),
            file_id: Some(file_id.clone()),
            span: None,
            subject: None,
        });
    }

    for el in children {
        match el.tag.as_str() {
            "NamespaceUris" => {
                for (i, uri_el) in el.children.iter().enumerate() {
                    if uri_el.tag == "Uri" {
                        namespaces.push(NamespaceEntry {
                            index: i as u16 + 1,
                            uri: uri_el.text.clone().unwrap_or_default(),
                            element: uri_el.clone(),
                        });
                    } else {
                        top_extensions.push(mark_unknown(uri_el.clone()));
                    }
                }
            }
            "Aliases" => {
                for alias_el in el.children.iter() {
                    if alias_el.tag == "Alias" {
                        aliases.push(AliasEntry {
                            alias: alias_el.attrs.get("Alias").cloned().unwrap_or_default(),
                            node_id_text: alias_el.text.clone().unwrap_or_default(),
                            element: alias_el.clone(),
                        });
                    } else {
                        top_extensions.push(mark_unknown(alias_el.clone()));
                    }
                }
            }
            "Models" => {
                for model_el in el.children.iter() {
                    if model_el.tag == "Model" || model_el.tag == "RequiredModel" {
                        models.push(ModelDependency {
                            model_uri: model_el.attrs.get("ModelUri").cloned().unwrap_or_default(),
                            version: model_el.attrs.get("Version").cloned(),
                            publication_date: model_el.attrs.get("PublicationDate").cloned(),
                            element: model_el.clone(),
                        });
                    } else {
                        top_extensions.push(mark_unknown(model_el.clone()));
                    }
                }
            }
            tag if NODE_TAGS.contains(&tag) => nodes.push(build_raw_node(tag, el, &aliases, &file_id, &mut diagnostics)),
            tag if known_top.contains(&tag) => {}
            _ => top_extensions.push(mark_unknown(el)),
        }
    }

    RawFile {
        file_id,
        path: path.to_string(),
        content: content.to_string(),
        namespaces,
        aliases,
        models,
        nodes,
        top_extensions,
        parse_diagnostics: diagnostics,
    }
}

fn attach(stack: &mut Vec<StackFrame>, root_children: &mut Vec<RawElement>, el: &mut RawElement) {
    if let Some(parent) = stack.last_mut() {
        parent.el.children.push(el.clone());
    } else {
        root_children.push(el.clone());
    }
}

fn mark_unknown(mut el: RawElement) -> RawElement {
    el.unknown = true;
    el
}

fn build_raw_node(tag: &str, el: RawElement, file_aliases: &[AliasEntry], file_id: &str, diagnostics: &mut Vec<Diagnostic>) -> RawNode {
    let mut references = Vec::new();
    let mut extensions = Vec::new();
    let mut aliases_used = Vec::new();
    let known_child = ["DisplayName", "Description", "Category", "References", "Documentation", "Definition", "RolePermissions"];

    for child in &el.children {
        if child.tag == "References" {
            for ref_el in &child.children {
                if ref_el.tag == "Reference" {
                    let text = ref_el.text.clone().unwrap_or_default();
                    let is_forward = ref_el
                        .attrs
                        .get("IsForward")
                        .map(|v| v != "false")
                        .unwrap_or(true);
                    references.push(RawRef { text: text.clone(), is_forward, element: ref_el.clone() });
                    if file_aliases.iter().any(|a| a.alias == text) {
                        aliases_used.push(text);
                    }
                } else {
                    extensions.push(mark_unknown(ref_el.clone()));
                }
            }
        } else if !known_child.contains(&child.tag.as_str()) {
            extensions.push(mark_unknown(child.clone()));
        }
    }

    if NodeIdValue::parse(&el.attrs.get("NodeId").cloned().unwrap_or_default()).is_none() {
        diagnostics.push(Diagnostic {
            code: "INVALID_NODEID".into(),
            severity: Severity::Error,
            message: format!("node has unparsable NodeId: {:?}", el.attrs.get("NodeId")),
            file_id: Some(file_id.into()),
            span: Some(el.span),
            subject: el.attrs.get("NodeId").cloned(),
        });
    }

    RawNode {
        node_class: tag.to_string(),
        node_id_text: el.attrs.get("NodeId").cloned().unwrap_or_default(),
        browse_name_text: el.attrs.get("BrowseName").cloned().unwrap_or_default(),
        attrs: el.attrs.clone(),
        references,
        aliases_used,
        extensions,
        element: el,
    }
}

pub fn snippet(content: &str, span: Span) -> String {
    content[span.start.min(content.len())..span.end.min(content.len())].to_string()
}
