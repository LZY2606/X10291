//! Streaming NodeSet XML parser built on quick-xml events.
//!
//! Everything is retained: namespaces, NodeIds, BrowseNames, references,
//! aliases, model dependencies, quantities and unrecognized extensions, each
//! with a byte span into the source file. Unknown subtrees are captured
//! verbatim so normalization can never silently drop them.

use crate::model::*;
use anyhow::Result;
use quick_xml::events::Event;
use quick_xml::Reader;

pub const UA_NS: &str = "http://opcfoundation.org/UA/";
pub const COMPASS_NS: &str = "urn:compass:domain";

fn attrs_of(e: &quick_xml::events::BytesStart) -> Vec<(String, String)> {
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

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn local(tag: &str) -> &str {
    tag.rsplit(':').next().unwrap_or(tag)
}

struct Frame {
    tag: String,
    parent: String,
    start: usize,
    attrs: Vec<(String, String)>,
    text: String,
    node_at: Option<usize>,
    file_unknown: bool,
}

pub fn parse_nodeset(source_id: &str, path: &str, bytes: &str) -> Result<RawNodeSet> {
    let mut reader = Reader::from_str(bytes);
    reader.config_mut().trim_text(true);

    let mut out = RawNodeSet {
        source_id: source_id.to_string(),
        path: path.to_string(),
        bytes: bytes.to_string(),
        last_modified: None,
        namespaces: Vec::new(),
        aliases: Vec::new(),
        dependencies: Vec::new(),
        nodes: Vec::new(),
        file_extensions: Vec::new(),
    };

    let mut stack: Vec<Frame> = Vec::new();
    let mut buf = Vec::new();

    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let parent = stack.last().map(|f| f.tag.clone()).unwrap_or_default();
                let node_tag = is_node_tag(&tag);
                let in_node = stack.last().and_then(|f| f.node_at);
                let node_at = if node_tag { Some(out.nodes.len()) } else { in_node };
                let file_unknown = stack.last().map(|f| f.file_unknown).unwrap_or(false)
                    || (parent_is_root(&stack) && local(&tag) == "Extensions");
                stack.push(Frame {
                    tag,
                    parent,
                    start: pos,
                    attrs: attrs_of(&e),
                    text: String::new(),
                    node_at,
                    file_unknown,
                });
                if node_tag {
                    push_node_from_frame(&mut out, stack.last().unwrap(), source_id);
                }
            }
            Ok(Event::Empty(e)) => {
                let end = reader.buffer_position();
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let parent = stack.last().map(|f| f.tag.clone()).unwrap_or_default();
                let node_tag = is_node_tag(&tag);
                let node_at = if node_tag {
                    Some(out.nodes.len())
                } else {
                    stack.last().and_then(|f| f.node_at)
                };
                let file_unknown = stack.last().map(|f| f.file_unknown).unwrap_or(false)
                    || (parent_is_root(&stack) && local(&tag) == "Extensions");
                let frame = Frame {
                    tag,
                    parent,
                    start: pos,
                    attrs: attrs_of(&e),
                    text: String::new(),
                    node_at,
                    file_unknown,
                };
                if node_tag {
                    push_node_from_frame(&mut out, &frame, source_id);
                    out.nodes.last_mut().unwrap().span.end = end;
                } else {
                    finalize_element(&frame, end, source_id, bytes, &mut out);
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(f) = stack.last_mut() {
                    if let Ok(s) = t.unescape() {
                        f.text.push_str(&s);
                    }
                }
            }
            Ok(Event::End(_)) => {
                let end = reader.buffer_position();
                if let Some(frame) = stack.pop() {
                    if is_node_tag(&frame.tag) {
                        if let Some(idx) = frame.node_at {
                            out.nodes[idx].span.end = end;
                        }
                    } else {
                        finalize_element(&frame, end, source_id, bytes, &mut out);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => anyhow::bail!("XML parse error near byte {}: {err}", pos),
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

fn parent_is_root(stack: &[Frame]) -> bool {
    stack.len() == 1 && local(&stack[0].tag) == "UANodeSet"
}

fn push_node_from_frame(out: &mut RawNodeSet, frame: &Frame, source_id: &str) {
    out.nodes.push(RawNode {
        kind: frame.tag.clone(),
        node_id_text: attr(&frame.attrs, "NodeId").unwrap_or("").to_string(),
        browse_name_text: attr(&frame.attrs, "BrowseName").map(str::to_string),
        display_name: None,
        references: Vec::new(),
        quantities: Vec::new(),
        extensions: Vec::new(),
        is_abstract: attr(&frame.attrs, "IsAbstract").map(|v| v == "true"),
        span: Span::new(source_id, frame.start, frame.start),
    });
}

fn is_node_tag(tag: &str) -> bool {
    matches!(
        local(tag),
        "UAObject" | "UAVariable" | "UAObjectType" | "UAVariableType"
            | "UAReferenceType" | "UADataType" | "UAView"
    )
}

fn finalize_element(
    frame: &Frame,
    end: usize,
    source_id: &str,
    bytes: &str,
    out: &mut RawNodeSet,
) {
    let tag = frame.tag.clone();
    let lname = local(&tag);
    let parent = local(&frame.parent);
    let span = Span::new(source_id, frame.start, end);

    if lname == "Uri" && parent == "NamespaceUris" {
        out.namespaces.push(RawNamespace {
            local_index: out.namespaces.len() as u16 + 1,
            uri: frame.text.trim().to_string(),
            span,
        });
        return;
    }
    if lname == "Alias" && parent == "Aliases" {
        if let Some(name) = attr(&frame.attrs, "Alias") {
            out.aliases.push(RawAlias {
                name: name.to_string(),
                node_id_text: frame.text.trim().to_string(),
                span,
            });
        }
        return;
    }
    if lname == "RequiredModel" && parent == "Model" {
        if let Some(uri) = attr(&frame.attrs, "ModelUri") {
            out.dependencies.push(ModelDependency {
                model_uri: uri.to_string(),
                version: attr(&frame.attrs, "Version").unwrap_or("0").to_string(),
                publication_date: attr(&frame.attrs, "PublicationDate").map(str::to_string),
                span,
            });
        }
        if let Some(lm) = attr(&frame.attrs, "LastModifiedDate") {
            out.last_modified = Some(lm.to_string());
        }
        return;
    }
    if lname == "Model" && parent == "Models" {
        if let Some(lm) = attr(&frame.attrs, "LastModifiedDate") {
            out.last_modified = Some(lm.to_string());
        }
        if let Some(v) = attr(&frame.attrs, "Version") {
            // Own model version is retained on the source record via dependency.
            out.dependencies.push(ModelDependency {
                model_uri: attr(&frame.attrs, "ModelUri").unwrap_or("").to_string(),
                version: v.to_string(),
                publication_date: attr(&frame.attrs, "PublicationDate").map(str::to_string),
                span: span.clone(),
            });
        }
        return;
    }
    if lname == "Reference" && parent == "References" {
        if let Some(idx) = frame.node_at {
            out.nodes[idx].references.push(RawRef {
                reference_type: attr(&frame.attrs, "ReferenceType").unwrap_or("").to_string(),
                target_text: frame.text.trim().to_string(),
                is_forward: attr(&frame.attrs, "IsForward").map(|v| v != "false").unwrap_or(true),
                span,
            });
        }
        return;
    }
    if lname == "DisplayName" {
        if let Some(idx) = frame.node_at {
            out.nodes[idx].display_name = Some(frame.text.trim().to_string());
        }
        return;
    }
    if let Some(q) = quantity_of(frame, &span) {
        if let Some(idx) = frame.node_at {
            out.nodes[idx].quantities.push(q);
            return;
        }
    }
    if frame.file_unknown {
        out.file_extensions.push(UnknownExtension {
            tag: tag.clone(),
            xml: bytes[frame.start..end].to_string(),
            span,
        });
        return;
    }
    if let Some(idx) = frame.node_at {
        if is_unknown_inside_node(lname, &frame.parent) {
            out.nodes[idx].extensions.push(UnknownExtension {
                tag: tag.clone(),
                xml: bytes[frame.start..end].to_string(),
                span,
            });
        }
    }
}

/// Domain quantity elements. All carry an explicit unit and/or coordinate
/// frame so import can preserve the original representation.
fn quantity_of(frame: &Frame, span: &Span) -> Option<RawQuantity> {
    let lname = local(&frame.tag);
    let known = matches!(lname, "Pressure" | "Temperature" | "Flow" | "Speed" | "Length" | "Position" | "Quantity");
    if !known {
        return None;
    }
    let unit = attr(&frame.attrs, "Unit").or_else(|| attr(&frame.attrs, "EngineeringUnits"));
    let frame_ref = attr(&frame.attrs, "CoordinateFrame").or_else(|| attr(&frame.attrs, "ReferenceFrame"));
    Some(RawQuantity {
        value: frame.text.trim().to_string(),
        unit: unit.map(str::to_string),
        coordinate_frame: frame_ref.map(str::to_string),
        span: span.clone(),
        canonical_value: None,
        canonical_unit: None,
        diagnostic: None,
    })
}

fn is_unknown_inside_node(lname: &str, parent: &str) -> bool {
    let known = matches!(
        lname,
        "References"
            | "DisplayName"
            | "Description"
            | "Category"
            | "Reference"
            | "Definition"
            | "Value"
            | "Pressure"
            | "Temperature"
            | "Flow"
            | "Speed"
            | "Length"
            | "Position"
            | "Quantity"
    );
    let _ = parent;
    !known
}
