use crate::model::*;
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

const KNOWN_CHILDREN: &[&str] = &[
    "DisplayName",
    "Description",
    "References",
    "Reference",
    "Value",
    "Definition",
    "RolePermissions",
    "AccessRestrictions",
];

fn local_name(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name).into_owned();
    match s.rfind(':') {
        Some(i) => s[i + 1..].to_string(),
        None => s,
    }
}

fn tag_start(xml: &str, tag_end: usize) -> usize {
    xml.as_bytes()[..tag_end.min(xml.len())]
        .iter()
        .rposition(|&b| b == b'<')
        .unwrap_or(0)
}

fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        let k = local_name(a.key.as_ref());
        if k == key {
            return Some(String::from_utf8_lossy(a.value.as_ref()).into_owned());
        }
    }
    None
}

fn text_of(t: &quick_xml::events::BytesText) -> String {
    t.unescape()
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(t.as_ref()).into_owned())
}

pub fn parse_nodeset(file_id: &str, name: &str, xml: &str) -> Result<ParsedFile, String> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(false);
    let mut pf = ParsedFile {
        file_id: file_id.to_string(),
        name: name.to_string(),
        xml: xml.to_string(),
        sha: sha1_hex(xml),
        namespace_uris: Vec::new(),
        aliases: Vec::new(),
        models: Vec::new(),
        nodes: Vec::new(),
    };
    let mut stack: Vec<String> = Vec::new();
    let mut cur_node: Option<RawNode> = None;
    let mut cur_ref: Option<RawRef> = None;
    let mut in_references = false;
    let mut capture: Option<(String, usize, usize)> = None;
    let mut cur_model: Option<ModelDecl> = None;
    let mut cur_alias: Option<String> = None;
    let mut in_uri = false;
    let mut in_display = false;
    let mut in_value = false;
    let mut ord: i64 = 0;

    loop {
        let ev = reader.read_event().map_err(|e| format!("xml parse error: {}", e))?;
        let pos = reader.buffer_position() as usize;
        match ev {
            Event::Start(e) => {
                let ln = local_name(e.name().as_ref());
                if let Some((ref mut cap_name, _, ref mut depth)) = capture {
                    let _ = cap_name;
                    *depth += 1;
                    stack.push(ln);
                    continue;
                }
                let parent = stack.last().cloned().unwrap_or_default();
                match ln.as_str() {
                    n if NODE_CLASSES.contains(&n) && parent == "UANodeSet" => {
                        ord += 1;
                        let start = tag_start(xml, pos);
                        cur_node = Some(RawNode {
                            file_id: file_id.to_string(),
                            ord,
                            local_id: attr(&e, "NodeId").unwrap_or_default(),
                            node_class: n.to_string(),
                            browse_name: attr(&e, "BrowseName").unwrap_or_default(),
                            display_name: String::new(),
                            value_raw: None,
                            xml_fragment: String::new(),
                            span_start: start as i64,
                            span_end: 0,
                            refs: Vec::new(),
                            extensions: Vec::new(),
                        });
                    }
                    "Uri" if parent == "NamespaceUris" => in_uri = true,
                    "Alias" if parent == "Aliases" => {
                        cur_alias = attr(&e, "Alias");
                    }
                    "Model" if parent == "Models" => {
                        cur_model = Some(ModelDecl {
                            model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                            version: attr(&e, "Version").unwrap_or_default(),
                            publication_date: attr(&e, "PublicationDate").unwrap_or_default(),
                            required: Vec::new(),
                        });
                    }
                    "RequiredModel" if cur_model.is_some() => {
                        if let Some(m) = cur_model.as_mut() {
                            m.required.push(RequiredModel {
                                model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                                version: attr(&e, "Version").unwrap_or_default(),
                                publication_date: attr(&e, "PublicationDate").unwrap_or_default(),
                            });
                        }
                    }
                    "DisplayName" if cur_node.is_some() => in_display = true,
                    "Value" if cur_node.is_some() => in_value = true,
                    "References" if cur_node.is_some() => in_references = true,
                    "Reference" if in_references => {
                        cur_ref = Some(RawRef {
                            ref_type: attr(&e, "ReferenceType").unwrap_or_default(),
                            is_forward: attr(&e, "IsForward")
                                .map(|v| v != "false")
                                .unwrap_or(true),
                            target_raw: String::new(),
                        });
                    }
                    other => {
                        if cur_node.is_some() && !KNOWN_CHILDREN.contains(&other) {
                            let start = tag_start(xml, pos);
                            capture = Some((other.to_string(), start, 1));
                        }
                    }
                }
                stack.push(ln);
            }
            Event::Empty(e) => {
                let ln = local_name(e.name().as_ref());
                if capture.is_some() {
                    continue;
                }
                let parent = stack.last().cloned().unwrap_or_default();
                match ln.as_str() {
                    n if NODE_CLASSES.contains(&n) && parent == "UANodeSet" => {
                        ord += 1;
                        let start = tag_start(xml, pos);
                        let node = RawNode {
                            file_id: file_id.to_string(),
                            ord,
                            local_id: attr(&e, "NodeId").unwrap_or_default(),
                            node_class: n.to_string(),
                            browse_name: attr(&e, "BrowseName").unwrap_or_default(),
                            display_name: String::new(),
                            value_raw: None,
                            xml_fragment: xml[start..pos].to_string(),
                            span_start: start as i64,
                            span_end: pos as i64,
                            refs: Vec::new(),
                            extensions: Vec::new(),
                        };
                        pf.nodes.push(node);
                    }
                    "RequiredModel" => {
                        if let Some(m) = cur_model.as_mut() {
                            m.required.push(RequiredModel {
                                model_uri: attr(&e, "ModelUri").unwrap_or_default(),
                                version: attr(&e, "Version").unwrap_or_default(),
                                publication_date: attr(&e, "PublicationDate").unwrap_or_default(),
                            });
                        }
                    }
                    other => {
                        if cur_node.is_some() && !KNOWN_CHILDREN.contains(&other) {
                            let start = tag_start(xml, pos);
                            if let Some(n) = cur_node.as_mut() {
                                n.extensions.push(xml[start..pos].to_string());
                            }
                        }
                    }
                }
            }
            Event::Text(t) => {
                if capture.is_some() {
                    continue;
                }
                let txt = text_of(&t);
                let trimmed = txt.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(r) = cur_ref.as_mut() {
                    r.target_raw.push_str(trimmed);
                } else if in_uri {
                    pf.namespace_uris.push(trimmed.to_string());
                } else if let Some(alias) = cur_alias.clone() {
                    pf.aliases.push((alias, trimmed.to_string()));
                    cur_alias = None;
                } else if in_display {
                    if let Some(n) = cur_node.as_mut() {
                        n.display_name.push_str(trimmed);
                    }
                } else if in_value {
                    if let Some(n) = cur_node.as_mut() {
                        let v = n.value_raw.get_or_insert_with(String::new);
                        v.push_str(trimmed);
                    }
                }
            }
            Event::End(e) => {
                let ln = local_name(e.name().as_ref());
                if let Some((_, start, ref mut depth)) = capture {
                    *depth -= 1;
                    if *depth == 0 {
                        let start = *start;
                        if let Some(n) = cur_node.as_mut() {
                            n.extensions.push(xml[start..pos].to_string());
                        }
                        capture = None;
                    }
                    stack.pop();
                    continue;
                }
                match ln.as_str() {
                    "Uri" => in_uri = false,
                    "DisplayName" => in_display = false,
                    "Value" => in_value = false,
                    "References" => in_references = false,
                    "Reference" => {
                        if let (Some(n), Some(r)) = (cur_node.as_mut(), cur_ref.take()) {
                            n.refs.push(r);
                        }
                    }
                    "Model" => {
                        if let Some(m) = cur_model.take() {
                            pf.models.push(m);
                        }
                    }
                    n if NODE_CLASSES.contains(&n) => {
                        if let Some(mut node) = cur_node.take() {
                            node.span_end = pos as i64;
                            node.xml_fragment =
                                xml[node.span_start as usize..pos].to_string();
                            pf.nodes.push(node);
                        }
                    }
                    _ => {}
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(pf)
}

pub fn extract_attr_from_fragment(frag: &str, key: &str) -> Option<String> {
    let pat = format!("{}=\"", key);
    let i = frag.find(&pat)?;
    let rest = &frag[i + pat.len()..];
    let j = rest.find('"')?;
    Some(rest[..j].to_string())
}
