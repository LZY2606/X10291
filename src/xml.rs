//! NodeSet XML 解析：原 XML 节点、namespace table、NodeId、BrowseName、
//! reference、alias、model dependency 与未识别扩展均保留（含源字节范围）。

use crate::model::*;
use anyhow::{bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

pub const OPC_UA_CORE_URI: &str = "http://opcfoundation.org/UA/";

const KNOWN_TOP: &[&str] = &["NamespaceUris", "Aliases", "Models"];
const KNOWN_CHILD: &[&str] = &["DisplayName", "Description", "References", "Value"];

fn local(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name).into_owned();
    s.rsplit(':').next().unwrap_or(&s).to_string()
}

fn attrs_of(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .flatten()
        .map(|a| {
            (
                local(a.key.as_ref()),
                String::from_utf8_lossy(&a.value).into_owned(),
            )
        })
        .collect()
}

fn get_attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// 跳过当前已打开元素的剩余部分，返回其结束位置（`</name>` 之后）。
fn skip_element(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Result<usize> {
    let mut depth = 1usize;
    loop {
        match reader.read_event_into(buf)? {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(reader.buffer_position());
                }
            }
            Event::Eof => bail!("unexpected EOF while skipping element"),
            _ => {}
        }
        buf.clear();
    }
}

fn model_from_attrs(attrs: &[(String, String)]) -> ModelDependency {
    ModelDependency {
        model_uri: get_attr(attrs, "ModelUri").unwrap_or_default().to_string(),
        version: get_attr(attrs, "Version").map(|s| s.to_string()),
        publication_date: get_attr(attrs, "PublicationDate").map(|s| s.to_string()),
    }
}

pub fn parse_nodeset(xml: &str) -> Result<ParsedNodeSet> {
    let mut out = ParsedNodeSet::default();
    out.namespaces.push(OPC_UA_CORE_URI.to_string()); // 文件内 index 0 固定为核心 URI
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);
    let mut buf = Vec::new();
    let mut in_root = false;
    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let name = local(e.name().as_ref());
                if !in_root {
                    if name != "UANodeSet" {
                        bail!("root element must be UANodeSet, got {name}");
                    }
                    in_root = true;
                } else if KNOWN_TOP.contains(&name.as_str()) {
                    parse_known_top(&mut reader, &mut buf, &name, &e, &mut out)?;
                } else if name.starts_with("UA") {
                    let node = parse_node(&mut reader, &mut buf, &e, xml, pos)?;
                    out.nodes.push(node);
                } else {
                    // 顶层未识别扩展：原样保留
                    let end = skip_element(&mut reader, &mut buf)?;
                    out.top_level_extensions.push(RawFragment {
                        element: name,
                        raw_xml: xml[pos..end].to_string(),
                    });
                }
            }
            Event::Empty(e) if in_root => {
                let name = local(e.name().as_ref());
                if name.starts_with("UA") {
                    let node = node_from_parts(&e, xml, pos, reader.buffer_position(), vec![], vec![], vec![])?;
                    out.nodes.push(node);
                } else if !KNOWN_TOP.contains(&name.as_str()) {
                    out.top_level_extensions.push(RawFragment {
                        element: name,
                        raw_xml: xml[pos..reader.buffer_position()].to_string(),
                    });
                }
            }
            Event::End(e) => {
                if local(e.name().as_ref()) == "UANodeSet" {
                    in_root = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn parse_known_top(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    name: &str,
    e: &BytesStart,
    out: &mut ParsedNodeSet,
) -> Result<()> {
    match name {
        "NamespaceUris" => loop {
            match reader.read_event_into(buf)? {
                Event::Start(c) if local(c.name().as_ref()) == "Uri" => {
                    let t = reader.read_text(c.name())?.into_owned();
                    out.namespaces.push(t);
                }
                Event::Empty(c) if local(c.name().as_ref()) == "Uri" => {
                    out.namespaces.push(String::new());
                }
                Event::End(_) => return Ok(()),
                Event::Eof => return Ok(()),
                _ => {}
            }
            buf.clear();
        },
        "Aliases" => loop {
            match reader.read_event_into(buf)? {
                Event::Start(c) if local(c.name().as_ref()) == "Alias" => {
                    let attrs = attrs_of(&c);
                    let alias = get_attr(&attrs, "Alias").unwrap_or_default().to_string();
                    let t = reader.read_text(c.name())?.into_owned();
                    out.aliases.push((alias, t));
                }
                Event::End(_) => return Ok(()),
                Event::Eof => return Ok(()),
                _ => {}
            }
            buf.clear();
        },
        "Models" => {
            let _ = e;
            loop {
                match reader.read_event_into(buf)? {
                    Event::Start(c) if local(c.name().as_ref()) == "Model" => {
                        out.model = Some(model_from_attrs(&attrs_of(&c)));
                        // 读取 RequiredModel 子元素直到 </Model>
                        loop {
                            match reader.read_event_into(buf)? {
                                Event::Empty(r) | Event::Start(r)
                                    if local(r.name().as_ref()) == "RequiredModel" =>
                                {
                                    out.required_models.push(model_from_attrs(&attrs_of(&r)));
                                }
                                Event::End(_) => break,
                                Event::Eof => break,
                                _ => {}
                            }
                            buf.clear();
                        }
                    }
                    Event::Empty(c) if local(c.name().as_ref()) == "Model" => {
                        out.model = Some(model_from_attrs(&attrs_of(&c)));
                    }
                    Event::End(_) => return Ok(()),
                    Event::Eof => return Ok(()),
                    _ => {}
                }
                buf.clear();
            }
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn node_from_parts(
    e: &BytesStart,
    xml: &str,
    start: usize,
    end: usize,
    references: Vec<RawReference>,
    extensions: Vec<RawFragment>,
    texts: Vec<(String, String)>,
) -> Result<RawNode> {
    let attrs = attrs_of(e);
    let node_id = get_attr(&attrs, "NodeId")
        .context("node missing NodeId")?
        .to_string();
    let browse = get_attr(&attrs, "BrowseName").unwrap_or_default().to_string();
    let mut all_attrs = attrs;
    all_attrs.extend(texts);
    Ok(RawNode {
        element: local(e.name().as_ref()),
        node_id_text: node_id,
        browse_name: browse,
        attrs: all_attrs,
        references,
        extensions,
        raw_xml: xml[start..end].to_string(),
        start,
        end,
    })
}

fn parse_node(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    e: &BytesStart,
    xml: &str,
    start: usize,
) -> Result<RawNode> {
    let elem = e.clone();
    let mut refs = Vec::new();
    let mut exts = Vec::new();
    let mut texts: Vec<(String, String)> = Vec::new();
    let end;
    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(buf)? {
            Event::Start(c) => {
                let n = local(c.name().as_ref());
                match n.as_str() {
                    "References" => loop {
                        match reader.read_event_into(buf)? {
                            Event::Start(r) if local(r.name().as_ref()) == "Reference" => {
                                let attrs = attrs_of(&r);
                                let target = reader.read_text(r.name())?.into_owned();
                                refs.push(RawReference {
                                    ref_type_text: get_attr(&attrs, "ReferenceType")
                                        .unwrap_or_default()
                                        .to_string(),
                                    is_forward: get_attr(&attrs, "IsForward")
                                        .map(|v| v != "false")
                                        .unwrap_or(true),
                                    target_text: target,
                                });
                            }
                            Event::Empty(r) if local(r.name().as_ref()) == "Reference" => {
                                let attrs = attrs_of(&r);
                                refs.push(RawReference {
                                    ref_type_text: get_attr(&attrs, "ReferenceType")
                                        .unwrap_or_default()
                                        .to_string(),
                                    is_forward: get_attr(&attrs, "IsForward")
                                        .map(|v| v != "false")
                                        .unwrap_or(true),
                                    target_text: String::new(),
                                });
                            }
                            Event::End(_) => break,
                            Event::Eof => break,
                            _ => {}
                        }
                        buf.clear();
                    },
                    "DisplayName" | "Description" => {
                        let t = reader.read_text(c.name())?.into_owned();
                        texts.push((n, t));
                    }
                    "Value" => {
                        let t = read_inner_xml(reader, buf, xml, pos)?;
                        texts.push(("Value".into(), t));
                    }
                    _ => {
                        // 未识别扩展：原样保留
                        let epos = skip_element(reader, buf)?;
                        exts.push(RawFragment {
                            element: n,
                            raw_xml: xml[pos..epos].to_string(),
                        });
                    }
                }
            }
            Event::Empty(c) => {
                let n = local(c.name().as_ref());
                if !KNOWN_CHILD.contains(&n.as_str()) {
                    exts.push(RawFragment {
                        element: n,
                        raw_xml: xml[pos..reader.buffer_position()].to_string(),
                    });
                }
            }
            Event::End(_) => {
                end = reader.buffer_position();
                break;
            }
            Event::Eof => bail!("unexpected EOF inside node element"),
            _ => {}
        }
        buf.clear();
    }
    node_from_parts(&elem, xml, start, end, refs, exts, texts)
}

/// 读取元素内部 XML（含子标签），返回内部片段文本。
fn read_inner_xml(
    reader: &mut Reader<&[u8]>,
    buf: &mut Vec<u8>,
    xml: &str,
    start_of_elem: usize,
) -> Result<String> {
    let inner_start = xml[start_of_elem..]
        .find('>')
        .map(|i| start_of_elem + i + 1)
        .unwrap_or(start_of_elem);
    let mut depth = 1usize;
    loop {
        match reader.read_event_into(buf)? {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    let end = reader.buffer_position();
                    let close = xml[..end].rfind('<').unwrap_or(end);
                    return Ok(xml[inner_start..close].to_string());
                }
            }
            Event::Eof => bail!("unexpected EOF in Value"),
            _ => {}
        }
        buf.clear();
    }
}
