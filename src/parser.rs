//! Streaming NodeSet2 XML parser.
//!
//! Preserves: namespace table, Models/RequiredModel, Aliases, every node with
//! byte span, references with IsForward, unknown extensions with raw XML, and
//! the original Value representation.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::hash::sha256;
use crate::types::*;

const NODE_TAGS: &[&str] = &[
    "UAObject",
    "UAVariable",
    "UAVariableType",
    "UAObjectType",
    "UAReferenceType",
    "UADataType",
    "UAMethod",
    "UAView",
];

pub fn parse_file(file_id: &str, path: &str, content: &[u8]) -> Result<ParsedFile, String> {
    let mut reader = Reader::from_reader(content);
    reader.config_mut().expand_empty_elements = true;

    let mut file = ParsedFile {
        file_id: file_id.to_string(),
        path: path.to_string(),
        sha256: sha256(content),
        namespace_uris: Vec::new(),
        models: Vec::new(),
        required_models: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        unknown_extensions: Vec::new(),
        content: content.to_vec(),
    };

    let mut buf = Vec::new();
    // Stack of (local tag name, element start span).
    let mut stack: Vec<(String, Span)> = Vec::new();
    let mut ext_ord = 0usize;

    loop {
        let before = reader.buffer_position();
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("XML parse error in {path} at byte {before}: {e}"))?;
        match event {
            Event::Start(ref ev) => {
                let span = span_of(ev, before);
                let name = local_name(ev.name());
                stack.push((name.clone(), span));

                match name.as_str() {
                    "Uri" if parent_local(&stack) == Some("NamespaceUris") => {
                        let text = read_inner_text(content, span.end, b"Uri");
                        if !text.trim().is_empty() {
                            file.namespace_uris.push(text.trim().to_string());
                        }
                    }
                    "Model" if parent_local(&stack) == Some("Models") => {
                        let attrs = attr_map(ev);
                        file.models.push(RawModel {
                            model_uri: attrs.get("ModelUri").cloned().unwrap_or_default(),
                            version: attrs.get("Version").cloned().unwrap_or_default(),
                            publication_date: attrs
                                .get("PublicationDate")
                                .cloned()
                                .unwrap_or_default(),
                            prov: prov(&file.file_id, span, content),
                        });
                    }
                    "RequiredModel" if parent_local(&stack) == Some("Model") => {
                        let attrs = attr_map(ev);
                        let owner = file
                            .models
                            .last()
                            .map(|m| m.model_uri.clone())
                            .unwrap_or_default();
                        file.required_models.push(RawRequiredModel {
                            owner_model_uri: owner,
                            model_uri: attrs.get("ModelUri").cloned().unwrap_or_default(),
                            version: attrs.get("Version").cloned().unwrap_or_default(),
                            publication_date: attrs
                                .get("PublicationDate")
                                .cloned()
                                .unwrap_or_default(),
                            prov: prov(&file.file_id, span, content),
                        });
                    }
                    "Alias" if parent_is_aliases(&stack) => {
                        let attrs = attr_map(ev);
                        let aname = attrs.get("Alias").cloned().unwrap_or_default();
                        let text = read_inner_text(content, span.end, b"Alias");
                        file.aliases.push(RawAlias {
                            name: aname,
                            node_id: text.trim().to_string(),
                            prov: prov(&file.file_id, span, content),
                        });
                    }
                    _ => {
                        if NODE_TAGS.contains(&name.as_str()) && in_node_set(&stack) {
                            let node = parse_node(
                                &mut reader,
                                content,
                                &file.file_id,
                                name.as_str(),
                                span,
                                &mut buf,
                                file.nodes.len(),
                            )?;
                            // parse_node consumes through matching End; re-sync stack
                            while matches!(stack.last(), Some((n, _)) if NODE_TAGS.contains(&n.as_str()))
                            {
                                stack.pop();
                            }
                            file.nodes.push(node);
                        } else if is_unknown_extension(&name, &stack) {
                            let ext = parse_extension(
                                &mut reader,
                                content,
                                &file.file_id,
                                name,
                                span,
                                &mut buf,
                                &mut stack,
                                ext_ord,
                            )?;
                            ext_ord += 1;
                            file.unknown_extensions.push(ext);
                        }
                    }
                }
            }
            Event::Empty(ref ev) => {
                // expand_empty_elements = true makes these Start/End, but guard anyway.
                let span = span_of(ev, before);
                let name = local_name(ev.name());
                if name == "Uri" && parent_local(&stack) == Some("NamespaceUris") {
                    file.namespace_uris.push(String::new());
                }
                let _ = span;
            }
            Event::End(ref ev) => {
                let name = local_name(ev.name());
                if let Some(pos) = stack.iter().rposition(|(n, _)| n == &name) {
                    stack.truncate(pos);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(file)
}

fn in_node_set(stack: &[(String, Span)]) -> bool {
    stack
        .iter()
        .any(|(n, _)| n == "UANodeSet" || n == "NodeSet")
}

fn parent_is_aliases(stack: &[(String, Span)]) -> bool {
    parent_local(stack) == Some("Aliases")
}

fn parent_local(stack: &[(String, Span)]) -> Option<&str> {
    stack.get(stack.len().saturating_sub(2)).map(|(n, _)| n.as_str())
}

fn is_unknown_extension(name: &str, stack: &[(String, Span)]) -> bool {
    if NODE_TAGS.contains(&name) {
        return false;
    }
    let known = [
        "UANodeSet",
        "NodeSet",
        "NamespaceUris",
        "Uri",
        "Models",
        "Model",
        "RequiredModel",
        "Aliases",
        "Alias",
        "References",
        "Reference",
        "DisplayName",
        "Description",
        "Category",
        "Value",
        "Definition",
        "InverseName",
    ];
    if known.contains(&name) {
        return false;
    }
    // Only treat direct structural children as top-level extensions.
    parent_local(stack) == Some("UANodeSet") || parent_local(stack) == Some("NodeSet")
}


#[allow(clippy::too_many_arguments)]
fn parse_node(
    reader: &mut Reader<&[u8]>,
    content: &[u8],
    file_id: &str,
    tag: &str,
    node_span: Span,
    buf: &mut Vec<u8>,
    ord: usize,
) -> Result<RawNode, String> {
    let attrs = Vec::new();
    let mut node = RawNode {
        ord,
        kind: NodeKind::parse(tag),
        node_id: String::new(),
        browse_name: String::new(),
        symbolic_name: None,
        data_type: None,
        value_rank: None,
        is_abstract: None,
        references: Vec::new(),
        extra_attributes: attrs,
        extensions: Vec::new(),
        value_text: None,
        value_attributes: Vec::new(),
        prov: prov(file_id, node_span, content),
    };
    // Pull attributes by rescanning the start tag raw bytes.
    if let Some(open) = content.get(node_span.start..node_span.end) {
        if let Ok(text) = std::str::from_utf8(open) {
            node.extra_attributes = raw_attr_pairs(text);
            take_attr(&mut node.extra_attributes, &mut node.node_id, "NodeId");
            take_attr(&mut node.extra_attributes, &mut node.browse_name, "BrowseName");
            take_opt_attr(&mut node.extra_attributes, &mut node.symbolic_name, "SymbolicName");
            take_opt_attr(&mut node.extra_attributes, &mut node.data_type, "DataType");
            take_opt_attr(&mut node.extra_attributes, &mut node.value_rank, "ValueRank");
            take_opt_attr(&mut node.extra_attributes, &mut node.is_abstract, "IsAbstract");
        }
    }

    let mut depth = 1usize;
    let mut in_references = false;
    let mut ext_ord = 0usize;

    while depth > 0 {
        let before = reader.buffer_position();
        let ev = reader
            .read_event_into(buf)
            .map_err(|e| format!("XML parse error inside node {tag}: {e}"))?;
        match ev {
            Event::Start(e) => {
                let start = before;
                let span = span_of(&e, before);
                let name = local_name(e.name());
                depth += 1;
                if name == "References" {
                    in_references = true;
                } else if name == "Reference" && in_references {
                    let map = attr_map(&e);
                    let text = read_inner_text(content, span.end, b"Reference");
                    node.references.push(RawReference {
                        reference_type: map.get("ReferenceType").cloned().unwrap_or_default(),
                        target: text.trim().to_string(),
                        is_forward: map
                            .get("IsForward")
                            .map(|v| v != "false")
                            .unwrap_or(true),
                        prov: prov(file_id, span, content),
                    });
                } else if name == "Value" {
                    let map = attr_map(&e);
                    let (inner_start, inner_end) =
                        skip_to_matching_end(content, reader, b"Value", span.end, buf)?;
                    let raw = String::from_utf8_lossy(&content[inner_start..inner_end]).to_string();
                    node.value_text = Some(raw.trim().to_string());
                    node.value_attributes = map.into_iter().collect();
                    // skip_to_matching_end consumed the Value subtree + End
                    depth -= 1;
                } else if !is_known_child(&name) && depth >= 2 {
                    // Unknown extension within the node: retain full raw XML.
                    let (_, end) = skip_to_matching_end(
                        content,
                        reader,
                        e.name().local_name().as_ref(),
                        span.end,
                        buf,
                    )?;
                    let raw = String::from_utf8_lossy(&content[start..end]).to_string();
                    node.extensions.push(RawExtension {
                        ord: ext_ord,
                        tag: name.clone(),
                        raw_xml: raw,
                        attributes: attr_map(&e).into_iter().collect(),
                        prov: prov(file_id, Span::new(start, end), content),
                    });
                    ext_ord += 1;
                    depth -= 1;
                }
            }
            Event::End(e) => {
                let name = local_name(e.name());
                depth -= 1;
                if name == "References" {
                    in_references = false;
                }
            }
            Event::Empty(e) => {
                let span = span_of(&e, before);
                let name = local_name(e.name());
                if name == "Reference" && in_references {
                    let map = attr_map(&e);
                    node.references.push(RawReference {
                        reference_type: map.get("ReferenceType").cloned().unwrap_or_default(),
                        target: String::new(),
                        is_forward: map
                            .get("IsForward")
                            .map(|v| v != "false")
                            .unwrap_or(true),
                        prov: prov(file_id, span, content),
                    });
                } else if name == "Value" {
                    node.value_text = Some(String::new());
                    node.value_attributes = attr_map(&e).into_iter().collect();
                } else if !is_known_child(&name) {
                    let raw = String::from_utf8_lossy(&content[before..reader.buffer_position()])
                        .to_string();
                    node.extensions.push(RawExtension {
                        ord: ext_ord,
                        tag: name,
                        raw_xml: raw,
                        attributes: attr_map(&e).into_iter().collect(),
                        prov: prov(file_id, Span::new(before, reader.buffer_position()), content),
                    });
                    ext_ord += 1;
                }
            }
            Event::Eof => return Err(format!("unexpected EOF inside node {tag}")),
            _ => {}
        }
        buf.clear();
    }

    Ok(node)
}

#[allow(clippy::too_many_arguments)]
fn parse_extension(
    reader: &mut Reader<&[u8]>,
    content: &[u8],
    file_id: &str,
    name: String,
    span: Span,
    buf: &mut Vec<u8>,
    stack: &mut Vec<(String, Span)>,
    ord: usize,
) -> Result<RawExtension, String> {
    let start = span.start;
    let (_inner, end) = skip_to_matching_end(content, reader, name.as_bytes(), span.end, buf)?;
    let raw = String::from_utf8_lossy(&content[start..end]).to_string();
    if let Some(pos) = stack.iter().rposition(|(n, _)| n == &name) {
        stack.truncate(pos);
    }
    Ok(RawExtension {
        ord,
        tag: name,
        raw_xml: raw,
        attributes: Vec::new(),
        prov: prov(file_id, Span::new(start, end), content),
    })
}

fn is_known_child(name: &str) -> bool {
    matches!(
        name,
        "References"
            | "Reference"
            | "DisplayName"
            | "Description"
            | "Category"
            | "Value"
            | "Definition"
            | "InverseName"
    )
}

/// Reads events from `reader` until the matching closing `tag`, returning the
/// byte range of the *inner* content and the position after the end tag.
fn skip_to_matching_end(
    content: &[u8],
    reader: &mut Reader<&[u8]>,
    tag: &[u8],
    inner_start: usize,
    buf: &mut Vec<u8>,
) -> Result<(usize, usize), String> {
    let mut depth = 1usize;
    let mut inner_end = inner_start;
    while depth > 0 {
        let before = reader.buffer_position();
        let ev = reader
            .read_event_into(buf)
            .map_err(|e| format!("XML parse error while skipping element: {e}"))?;
        match ev {
            Event::Start(e) => {
                if e.name().local_name().as_ref() == tag {
                    depth += 1;
                }
            }
            Event::Empty(e) => {
                if e.name().local_name().as_ref() == tag && depth == 1 {
                    // Self-closed with no inner content.
                    let tag_start = locate_tag_start_before(content, reader.buffer_position());
                    return Ok((inner_start, tag_start));
                }
            }
            Event::End(e) => {
                if e.name().local_name().as_ref() == tag {
                    depth -= 1;
                    if depth == 0 {
                        // inner content ends where this end tag begins.
                        let end_tag_start = locate_tag_start_before(content, reader.buffer_position());
                        return Ok((inner_start, end_tag_start));
                    }
                }
            }
            Event::Eof => return Err("unexpected EOF while skipping element".to_string()),
            _ => {}
        }
        inner_end = reader.buffer_position();
        buf.clear();
    }
    Ok((inner_start, inner_end))
}

/// Find the byte offset of the `<` that opened the XML token that just ended at
/// `pos_after`. Works for both `</tag>` and `<tag/>`.
fn locate_tag_start_before(content: &[u8], pos_after: usize) -> usize {
    let mut i = pos_after.min(content.len());
    while i > 0 {
        i -= 1;
        if content[i] == b'<' {
            return i;
        }
    }
    0
}


/// Byte span of the opening tag `<tag ...>` beginning at `before`.
/// quick-xml reports `buffer_position()` just after the `>`.
fn span_of(ev: &quick_xml::events::BytesStart, before: usize) -> Span {
    Span::new(before, before + ev.name().as_ref().len() + 2)
}

fn local_name(name: quick_xml::name::QName) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).to_string()
}

fn attr_map(ev: &quick_xml::events::BytesStart) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for attr in ev.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.local_name().as_ref()).to_string();
        if let Ok(value) = attr.unescape_value() {
            map.insert(key, value.to_string());
        } else {
            map.insert(key, String::from_utf8_lossy(&attr.value).to_string());
        }
    }
    map
}

fn prov(file_id: &str, span: Span, content: &[u8]) -> Provenance {
    let fragment = String::from_utf8_lossy(&content[span.start..span.end.min(content.len())])
        .chars()
        .take(160)
        .collect();
    Provenance::new(file_id, span, fragment)
}

/// Extract the decoded attribute pairs directly from a raw `<tag ...>` string,
/// preserving even attributes quick-xml would treat the same way. Order kept.
fn raw_attr_pairs(tag_text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let bytes = tag_text.as_bytes();
    let mut i = tag_text
        .find(|c: char| c.is_whitespace())
        .unwrap_or(tag_text.len());
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && bytes[i] != b'>' && !(bytes[i] as char).is_whitespace()
        {
            i += 1;
        }
        let key = &tag_text[key_start..i];
        while i < bytes.len() && (bytes[i] as char).is_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            if key.is_empty() {
                if i < bytes.len() && bytes[i] == b'>' {
                    break;
                }
                continue;
            }
            // Bare attribute (e.g. minimized) — retain with empty value.
            pairs.push((key.to_string(), String::new()));
            continue;
        }
        i += 1; // skip '='
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if quote == b'"' || quote == b'\'' {
            i += 1;
            let val_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let value = decode_entities(&tag_text[val_start..i]);
            if !key.is_empty() {
                pairs.push((key.to_string(), value));
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            let val_start = i;
            while i < bytes.len() && bytes[i] != b'>' && !(bytes[i] as char).is_whitespace() {
                i += 1;
            }
            if !key.is_empty() {
                pairs.push((key.to_string(), tag_text[val_start..i].to_string()));
            }
        }
    }
    pairs
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn take_attr(pairs: &mut Vec<(String, String)>, out: &mut String, key: &str) {
    if let Some(pos) = pairs.iter().position(|(k, _)| k == key) {
        *out = pairs.remove(pos).1;
    }
}

fn take_opt_attr(pairs: &mut Vec<(String, String)>, out: &mut Option<String>, key: &str) {
    if let Some(pos) = pairs.iter().position(|(k, _)| k == key) {
        *out = Some(pairs.remove(pos).1);
    }
}

/// Read trimmed inner text of an element by scanning raw bytes from
/// `inner_start` until the matching close tag, honouring nesting of same-named
/// tags (rare for Uri/Alias but cheap to support).
fn read_inner_text(content: &[u8], inner_start: usize, tag: &[u8]) -> String {
    let tag_name = String::from_utf8_lossy(tag).to_string();
    let mut depth = 1usize;
    let mut i = inner_start;
    let bytes = content;
    let start = i;
    while i < bytes.len() && depth > 0 {
        if bytes[i] == b'<' {
            if bytes.get(i + 1) == Some(&b'/') {
                let rest = &bytes[i + 2..];
                if rest.starts_with(tag) {
                    depth -= 1;
                    if depth == 0 {
                        return decode_entities(
                            String::from_utf8_lossy(&bytes[start..i]).trim(),
                        );
                    }
                    // skip past this end tag
                    if let Some(gt) = bytes[i..].iter().position(|&b| b == b'>') {
                        i += gt + 1;
                        continue;
                    }
                }
            } else {
                // Skip a possible nested start tag with same name.
                let rest = &bytes[i + 1..];
                if rest.starts_with(tag) {
                    depth += 1;
                }
                if let Some(gt) = bytes[i..].iter().position(|&b| b == b'>') {
                    i += gt + 1;
                    continue;
                }
            }
        }
        i += 1;
        let _ = tag_name;
    }
    decode_entities(String::from_utf8_lossy(&bytes[start..i]).trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<UANodeSet xmlns="http://opcfoundation.org/UA/2011/03/UANodeSet.xsd">
  <NamespaceUris>
    <Uri>urn:factory:a</Uri>
    <Uri>urn:factory:b</Uri>
  </NamespaceUris>
  <Models>
    <Model ModelUri="urn:factory:a" Version="1.0.0">
      <RequiredModel ModelUri="http://opcfoundation.org/UA/" Version="1.04"/>
    </Model>
  </Models>
  <Aliases>
    <Alias Alias="Boolean">i=1</Alias>
  </Aliases>
  <UAObject NodeId="ns=1;i=5001" BrowseName="1:MotorA">
    <DisplayName>Motor A</DisplayName>
    <References>
      <Reference ReferenceType="i=47">i=5002</Reference>
      <Reference ReferenceType="i=47" IsForward="false">i=5003</Reference>
    </References>
    <Vendor:CustomRag xmlns:Vendor="urn:vendor:ext" heat="42"/>
  </UAObject>
  <UAVariable NodeId="ns=1;i=5002" BrowseName="1:Speed" DataType="i=1">
    <Value><uax:Float xmlns:uax="http://opcfoundation.org/UA/2008/02/Types.xsd">1500</uax:Float></Value>
  </UAVariable>
</UANodeSet>
"#;

    #[test]
    fn parses_namespace_table_models_aliases() {
        let f = parse_file("f1", "sample.xml", SAMPLE.as_bytes()).unwrap();
        assert_eq!(f.namespace_uris, vec!["urn:factory:a".to_string(), "urn:factory:b".to_string()]);
        assert_eq!(f.models[0].model_uri, "urn:factory:a");
        assert_eq!(f.models[0].version, "1.0.0");
        assert_eq!(f.required_models[0].model_uri, "http://opcfoundation.org/UA/");
        assert_eq!(f.aliases[0].name, "Boolean");
        assert_eq!(f.aliases[0].node_id, "i=1");
    }

    #[test]
    fn preserves_nodes_references_direction_and_extensions() {
        let f = parse_file("f1", "sample.xml", SAMPLE.as_bytes()).unwrap();
        let motor = &f.nodes[0];
        assert_eq!(motor.kind, NodeKind::Object);
        assert_eq!(motor.node_id, "ns=1;i=5001");
        assert_eq!(motor.browse_name, "1:MotorA");
        assert_eq!(motor.references.len(), 2);
        assert!(motor.references[0].is_forward);
        assert!(!motor.references[1].is_forward);
        assert_eq!(motor.extensions.len(), 1);
        assert!(motor.extensions[0].raw_xml.contains("Vendor:CustomRag"));
        let speed = &f.nodes[1];
        assert_eq!(speed.value_text.as_deref(), Some("1500"));
    }

    #[test]
    fn provenance_spans_read_back_source() {
        let f = parse_file("f1", "sample.xml", SAMPLE.as_bytes()).unwrap();
        let p = &f.nodes[0].prov;
        let read = String::from_utf8_lossy(&f.content[p.span.start..p.span.end]);
        assert!(read.starts_with("<UAObject"));
        assert!(read.contains("NodeId=\"ns=1;i=5001\""));
    }
}
