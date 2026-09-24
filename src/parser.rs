//! Streaming NodeSet XML parser that records source byte ranges and retains
//! every unmodeled element verbatim. Numeric namespace indices are kept as
//! written and never interpreted across files.

use crate::domain::*;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;

const UA_NS: &str = "http://opcfoundation.org/UA/2011/03/UANodeSet.xsd";

#[derive(Debug)]
pub struct ParseOutcome {
    pub file: ParsedFile,
    pub warnings: Vec<String>,
}

pub fn line_col(bytes: &[u8], pos: u64) -> (u32, u32) {
    let upto = (pos as usize).min(bytes.len());
    let mut line = 1u32;
    let mut col = 1u32;
    for &b in &bytes[..upto] {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

pub fn make_range(bytes: &[u8], start: u64, end: u64) -> SourceRange {
    let (sl, sc) = line_col(bytes, start);
    let (el, ec) = line_col(bytes, end);
    SourceRange {
        start_byte: start,
        end_byte: end,
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
    }
}

#[derive(Clone)]
struct Frame {
    role: Role,
    local: String,
    ns_uri: String,
    start: u64,
}

#[derive(Clone, PartialEq)]
enum Role {
    Root,
    Other,
    NamespaceTable,
    Uri,
    Models,
    Model,
    RequiredModel,
    Aliases,
    Alias,
    Node(usize),
    References(usize),
    Reference(usize),
    DisplayName(usize),
    InverseName(usize),
    Value(usize),
    Scalar(usize),
    Quantity(usize),
    Unknown(usize), // usize::MAX == top-level
}

pub fn parse_nodeset(file_id: &str, path_hint: &str, bytes: &[u8]) -> Result<ParseOutcome, String> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut warnings = Vec::new();
    let mut file = ParsedFile {
        file_id: file_id.to_string(),
        path_hint: path_hint.to_string(),
        model_uri: String::new(),
        model_version: String::new(),
        publication_date: None,
        namespace_table: Vec::new(),
        requirements: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        unknown_extensions: Vec::new(),
    };

    let mut stack: Vec<Frame> = Vec::new();
    let mut ns_stack: Vec<HashMap<String, String>> = Vec::new();
    let mut text_buf = String::new();
    let mut node_ordinal: u32 = 0;
    let mut pending_ref: Option<(u64, bool, String)> = None;
    let mut pending_alias: Option<(u64, String)> = None;
    let mut pending_required: Option<(u64, String, String, Option<String>)> = None;

    loop {
        let start_pos = reader.buffer_position() as u64;
        let event = reader
            .read_event()
            .map_err(|e| format!("xml parse error in {path_hint} at byte {start_pos}: {e}"))?;
        let end_pos = reader.buffer_position() as u64;

        match event {
            Event::Start(e) => {
                let decls = collect_declarations(&e);
                let (local, ns_uri) = local_ns(&e, &reader, &ns_stack);
                let role = open_role(
                    &local,
                    &ns_uri,
                    &e,
                    &stack,
                    &decls,
                    &ns_stack,
                    &mut file,
                    &mut node_ordinal,
                    start_pos,
                    bytes,
                    &mut pending_ref,
                    &mut pending_alias,
                    &mut pending_required,
                );
                stack.push(Frame {
                    role,
                    local,
                    ns_uri,
                    start: start_pos,
                });
                ns_stack.push(decls);
                text_buf.clear();
            }
            Event::Empty(e) => {
                let decls = collect_declarations(&e);
                let (local, ns_uri) = local_ns(&e, &reader, &ns_stack);
                empty_element(
                    &local,
                    &ns_uri,
                    &e,
                    &stack,
                    &decls,
                    &ns_stack,
                    &mut file,
                    &mut node_ordinal,
                    start_pos,
                    end_pos,
                    bytes,
                );
                ns_stack.push(decls);
                ns_stack.pop();
                text_buf.clear();
            }
            Event::Text(t) => {
                if let Ok(t) = t.unescape() {
                    text_buf.push_str(&t);
                }
            }
            Event::End(_) => {
                if let Some(frame) = stack.pop() {
                    close_role(
                        frame,
                        &text_buf,
                        end_pos,
                        &mut file,
                        bytes,
                        &mut pending_ref,
                        &mut pending_alias,
                        &mut pending_required,
                    );
                }
                ns_stack.pop();
                text_buf.clear();
            }
            Event::Eof => break,
            _ => text_buf.clear(),
        }
    }

    if file.model_uri.is_empty() {
        warnings.push(format!("{path_hint}: no <Model> entry found"));
    }
    Ok(ParseOutcome { file, warnings })
}

fn attr_str(e: &BytesStart, name: &str) -> Option<String> {
    e.try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|a| a.unescape_value().ok().map(|v| v.to_string()))
}

fn collect_declarations(e: &BytesStart) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for attr in e.attributes().flatten() {
        let raw = attr.key.as_ref();
        if let Some(rest) = raw.strip_prefix(b"xmlns:") {
            map.insert(
                String::from_utf8_lossy(rest).to_string(),
                attr.unescape_value().unwrap_or_default().to_string(),
            );
        } else if raw == b"xmlns" {
            map.insert(
                String::new(),
                attr.unescape_value().unwrap_or_default().to_string(),
            );
        }
    }
    map
}

fn local_ns<'a>(
    e: &'a BytesStart,
    reader: &'a Reader<&'a [u8]>,
    ns_stack: &[HashMap<String, String>],
) -> (String, String) {
    let qname = reader
        .decoder()
        .decode(e.name().as_ref())
        .map(|cow| cow.to_string())
        .unwrap_or_default();
    let (prefix, local) = match qname.split_once(':') {
        Some((p, l)) => (Some(p), l.to_string()),
        None => (None, qname),
    };
    let ns_uri = prefix
        .and_then(|p| lookup_ns(ns_stack, p))
        .or_else(|| lookup_ns(ns_stack, ""))
        .unwrap_or_default();
    (local, ns_uri)
}

fn lookup_ns(stack: &[HashMap<String, String>], prefix: &str) -> Option<String> {
    stack.iter().rev().find_map(|m| m.get(prefix).cloned())
}

fn is_scalar_leaf(local: &str) -> bool {
    matches!(
        local,
        "String"
            | "Boolean"
            | "Byte"
            | "SByte"
            | "Int16"
            | "UInt16"
            | "Int32"
            | "UInt32"
            | "Int64"
            | "UInt64"
            | "Float"
            | "Double"
            | "DateTime"
            | "ByteString"
            | "Guid"
            | "NodeId"
            | "ExpandedNodeId"
    )
}

#[allow(clippy::too_many_arguments)]
fn open_role(
    local: &str,
    ns_uri: &str,
    e: &BytesStart,
    stack: &[Frame],
    decls: &HashMap<String, String>,
    ns_stack: &[HashMap<String, String>],
    file: &mut ParsedFile,
    node_ordinal: &mut u32,
    start_pos: u64,
    bytes: &[u8],
    pending_ref: &mut Option<(u64, bool, String)>,
    pending_alias: &mut Option<(u64, String)>,
    pending_required: &mut Option<(u64, String, String, Option<String>)>,
) -> Role {
    let parent = stack.last();
    let prole = parent.map(|f| f.role.clone()).unwrap_or(Role::Root);

    // Anything inside an Unknown subtree is Other so we only capture the outer
    // element verbatim (and do not model its children separately).
    if matches!(prole, Role::Unknown(_)) {
        return Role::Other;
    }

    match prole {
        Role::Root => {
            if local == "UANodeSet" {
                return Role::Root;
            }
            if local == "NamespaceTable" {
                return Role::NamespaceTable;
            }
            if local == "Models" {
                return Role::Models;
            }
            if local == "Aliases" {
                return Role::Aliases;
            }
            if let Some(kind) = NodeKind::from_element(local) {
                if ns_uri != UA_NS {
                    return Role::Unknown(usize::MAX);
                }
                let idx = start_node(e, kind, file, node_ordinal, start_pos, bytes, decls, ns_stack);
                return Role::Node(idx);
            }
            Role::Unknown(usize::MAX)
        }
        Role::NamespaceTable => {
            if local == "Uri" {
                Role::Uri
            } else {
                Role::Other
            }
        }
        Role::Models => {
            if local == "Model" && ns_uri == UA_NS {
                file.model_uri = attr_str(e, "ModelUri").unwrap_or_default();
                file.model_version = attr_str(e, "Version").unwrap_or_default();
                file.publication_date = attr_str(e, "PublicationDate");
                Role::Model
            } else {
                Role::Other
            }
        }
        Role::Model => {
            if local == "RequiredModel" && ns_uri == UA_NS {
                *pending_required = Some((
                    start_pos,
                    attr_str(e, "ModelUri").unwrap_or_default(),
                    attr_str(e, "Version").unwrap_or_default(),
                    attr_str(e, "PublicationDate"),
                ));
                Role::RequiredModel
            } else {
                Role::Other
            }
        }
        Role::Aliases => {
            if local == "Alias" && ns_uri == UA_NS {
                *pending_alias = Some((start_pos, attr_str(e, "Name").unwrap_or_default()));
                Role::Alias
            } else {
                Role::Other
            }
        }
        Role::Node(ni) => {
            if local == "References" {
                Role::References(ni)
            } else if local == "DisplayName" {
                Role::DisplayName(ni)
            } else if local == "InverseName" {
                Role::InverseName(ni)
            } else if local == "Value" {
                Role::Value(ni)
            } else if local == "CompassQuantity" && ns_uri != UA_NS && !ns_uri.is_empty() {
                let q = RawQuantity {
                    value_text: String::new(),
                    unit: attr_str(e, "unit").unwrap_or_default(),
                    coordinate_basis: attr_str(e, "coordinateBasis").unwrap_or_default(),
                    range: make_range(bytes, start_pos, start_pos),
                };
                file.nodes[ni].quantities.push(q);
                Role::Quantity(ni)
            } else if ns_uri == UA_NS && (local == "Description" || local == "Category") {
                Role::Other
            } else {
                Role::Unknown(ni)
            }
        }
        Role::References(ni) => {
            if local == "Reference" && ns_uri == UA_NS {
                let is_forward = attr_str(e, "IsForward")
                    .map(|v| v.trim() != "false")
                    .unwrap_or(true);
                *pending_ref = Some((start_pos, is_forward, attr_str(e, "ReferenceType").unwrap_or_default()));
                Role::Reference(ni)
            } else {
                Role::Unknown(ni)
            }
        }
        Role::Value(ni) => {
            if is_scalar_leaf(local) {
                Role::Scalar(ni)
            } else {
                Role::Unknown(ni)
            }
        }
        _ => Role::Other,
    }
}

#[allow(clippy::too_many_arguments)]
fn empty_element(
    local: &str,
    ns_uri: &str,
    e: &BytesStart,
    stack: &[Frame],
    decls: &HashMap<String, String>,
    ns_stack: &[HashMap<String, String>],
    file: &mut ParsedFile,
    node_ordinal: &mut u32,
    start_pos: u64,
    end_pos: u64,
    bytes: &[u8],
) {
    let prole = stack.last().map(|f| f.role.clone()).unwrap_or(Role::Root);
    if matches!(prole, Role::Unknown(_)) {
        return;
    }
    match prole {
        Role::Root => {
            if let Some(kind) = NodeKind::from_element(local) {
                if ns_uri == UA_NS {
                    start_node(e, kind, file, node_ordinal, start_pos, bytes, decls, ns_stack);
                    if let Some(node) = file.nodes.last_mut() {
                        node.range = make_range(bytes, start_pos, end_pos);
                    }
                } else {
                    push_top_extension(local, ns_uri, file, start_pos, end_pos, bytes);
                }
            } else if local != "UANodeSet" {
                push_top_extension(local, ns_uri, file, start_pos, end_pos, bytes);
            }
        }
        Role::Model => {
            if local == "RequiredModel" && ns_uri == UA_NS {
                file.requirements.push(ModelRequirement {
                    model_uri: attr_str(e, "ModelUri").unwrap_or_default(),
                    version: attr_str(e, "Version").unwrap_or_default(),
                    publication_date: attr_str(e, "PublicationDate"),
                    range: make_range(bytes, start_pos, end_pos),
                });
            }
        }
        _ => {}
    }
}

fn push_top_extension(
    local: &str,
    ns_uri: &str,
    file: &mut ParsedFile,
    start: u64,
    end: u64,
    bytes: &[u8],
) {
    let id = format!("{}#ext-top-{}", file.file_id, file.unknown_extensions.len());
    let raw_xml = String::from_utf8_lossy(&bytes[start as usize..end as usize]).to_string();
    file.unknown_extensions.push(UnknownExtension {
        id,
        parent_kind: "top-level".to_string(),
        parent_ref: String::new(),
        element_local: local.to_string(),
        ns_uri: ns_uri.to_string(),
        raw_xml: raw_xml.trim().to_string(),
        range: make_range(bytes, start, end),
    });
}

#[allow(clippy::too_many_arguments)]
fn start_node(
    e: &BytesStart,
    kind: NodeKind,
    file: &mut ParsedFile,
    node_ordinal: &mut u32,
    start_pos: u64,
    bytes: &[u8],
    decls: &HashMap<String, String>,
    ns_stack: &[HashMap<String, String>],
) -> usize {
    const KNOWN_ATTRS: [&str; 4] = ["NodeId", "BrowseName", "DataType", "ParentNodeId"];
    let mut attrs = Vec::new();
    for attr in e.attributes().flatten() {
        let raw_key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
        if raw_key == "xmlns" || raw_key.starts_with("xmlns:") {
            continue;
        }
        let (prefix, local_name) = match raw_key.split_once(':') {
            Some((p, l)) => (Some(p), l.to_string()),
            None => (None, raw_key.clone()),
        };
        if KNOWN_ATTRS.contains(&local_name.as_str()) && prefix.is_none() {
            continue;
        }
        let ns_uri = match prefix {
            Some(p) => lookup_ns(ns_stack, p)
                .or_else(|| decls.get(p).cloned())
                .unwrap_or_else(|| format!("__unresolved_prefix__:{p}")),
            None => String::new(),
        };
        attrs.push(RawAttribute {
            name: local_name,
            value: attr.unescape_value().unwrap_or_default().to_string(),
            ns_uri,
        });
    }
    let idx = file.nodes.len();
    file.nodes.push(RawNode {
        local_id: format!("{}#n{}", file.file_id, node_ordinal),
        kind,
        node_id_raw: attr_str(e, "NodeId").unwrap_or_default(),
        browse_name_raw: attr_str(e, "BrowseName").unwrap_or_default(),
        display_name: None,
        data_type_raw: attr_str(e, "DataType"),
        parent_node_raw: attr_str(e, "ParentNodeId"),
        references: Vec::new(),
        attributes: attrs,
        scalar_value_text: None,
        quantities: Vec::new(),
        range: make_range(bytes, start_pos, start_pos),
        ordinal: *node_ordinal,
    });
    *node_ordinal += 1;
    idx
}

#[allow(clippy::too_many_arguments)]
fn close_role(
    frame: Frame,
    text: &str,
    end_pos: u64,
    file: &mut ParsedFile,
    bytes: &[u8],
    pending_ref: &mut Option<(u64, bool, String)>,
    pending_alias: &mut Option<(u64, String)>,
    pending_required: &mut Option<(u64, String, String, Option<String>)>,
) {
    match frame.role {
        Role::Uri => {
            file.namespace_table.push(text.trim().to_string());
        }
        Role::Alias => {
            if let Some((start, name)) = pending_alias.take() {
                file.aliases.push(RawAlias {
                    name,
                    target_raw: text.trim().to_string(),
                    range: make_range(bytes, start, end_pos),
                });
            }
        }
        Role::RequiredModel => {
            if let Some((start, uri, version, pubdate)) = pending_required.take() {
                file.requirements.push(ModelRequirement {
                    model_uri: uri,
                    version,
                    publication_date: pubdate,
                    range: make_range(bytes, start, end_pos),
                });
            }
        }
        Role::Node(ni) => {
            if let Some(node) = file.nodes.get_mut(ni) {
                node.range = make_range(bytes, frame.start, end_pos);
            }
        }
        Role::Reference(ni) => {
            if let Some((start, is_forward, ref_type)) = pending_ref.take() {
                if let Some(node) = file.nodes.get_mut(ni) {
                    node.references.push(RawReference {
                        ref_type_raw: ref_type,
                        is_forward,
                        target_raw: text.trim().to_string(),
                        range: make_range(bytes, start, end_pos),
                    });
                }
            }
        }
        Role::DisplayName(ni) => {
            if let Some(node) = file.nodes.get_mut(ni) {
                node.display_name = Some(text.trim().to_string());
            }
        }
        Role::Scalar(ni) => {
            if let Some(node) = file.nodes.get_mut(ni) {
                node.scalar_value_text = Some(text.trim().to_string());
            }
        }
        Role::Quantity(ni) => {
            if let Some(node) = file.nodes.get_mut(ni) {
                if let Some(q) = node.quantities.last_mut() {
                    q.value_text = text.trim().to_string();
                    q.range = make_range(bytes, frame.start, end_pos);
                }
            }
        }
        Role::Unknown(ni) => {
            let raw_xml = String::from_utf8_lossy(&bytes[frame.start as usize..end_pos as usize])
                .trim()
                .to_string();
            if ni == usize::MAX {
                let id = format!(
                    "{}#ext-top-{}",
                    file.file_id,
                    file.unknown_extensions
                        .iter()
                        .filter(|x| x.parent_kind == "top-level")
                        .count()
                );
                file.unknown_extensions.push(UnknownExtension {
                    id,
                    parent_kind: "top-level".to_string(),
                    parent_ref: String::new(),
                    element_local: frame.local,
                    ns_uri: frame.ns_uri,
                    raw_xml,
                    range: make_range(bytes, frame.start, end_pos),
                });
            } else if let Some(node) = file.nodes.get(ni) {
                let parent_ref = node.local_id.clone();
                let seq = file
                    .unknown_extensions
                    .iter()
                    .filter(|x| x.parent_ref == parent_ref)
                    .count();
                let id = format!("{parent_ref}#ext-{seq}");
                file.unknown_extensions.push(UnknownExtension {
                    id,
                    parent_kind: "node".to_string(),
                    parent_ref,
                    element_local: frame.local,
                    ns_uri: frame.ns_uri,
                    raw_xml,
                    range: make_range(bytes, frame.start, end_pos),
                });
            }
        }
        _ => {}
    }
}
