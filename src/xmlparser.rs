//! 保真流式解析 UANodeSet XML。
//!
//! 只提取事实，不做任何规范化：namespace 数字 index 保持文件局部语义，
//! alias 不展开，reference 方向按原文，未识别扩展整段保留（含原始字节区间）。

use crate::model::*;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

/// OPC UA 内置命名空间（ns=0 永远指它，与文件无关）。
pub const OPC_UA_CORE_URI: &str = "http://opcfoundation.org/UA/";

#[derive(Debug, Clone)]
enum Payload {
    None,
    Node {
        ordinal: usize,
        node_id_raw: String,
        browse_name_raw: String,
        parent_raw: Option<String>,
        display: Option<String>,
        description: Option<String>,
        attrs: BTreeMap<String, String>,
    },
    Model {
        model_uri: String,
        version: Option<String>,
        publication_date: Option<String>,
        required: Vec<RequiredModel>,
    },
}

#[derive(Debug, Clone)]
struct Frame {
    local: String,
    qname: String,
    start: usize,
    attrs: BTreeMap<String, String>,
    text: String,
    payload: Payload,
    /// 该元素已被整体当作未识别扩展保留，子树不再单独处理。
    suppressed: bool,
}

struct ParseState {
    doc: ParsedDocument,
    stack: Vec<Frame>,
    prefix_map: BTreeMap<String, String>,
    node_ordinal: usize,
    /// 当前打开的 UA 节点 ordinal 栈（用于把子元素挂到正确节点）。
    open_nodes: Vec<usize>,
}

impl ParseState {
    fn enclosing_raw_id(&self) -> Option<String> {
        self.open_nodes.last().map(|o| format!("raw:{}:{}", self.doc.file_id, o))
    }
}

fn decode_attr(value: &[u8]) -> String {
    let raw = String::from_utf8_lossy(value).into_owned();
    quick_xml::escape::unescape(&raw).map(|c| c.into_owned()).unwrap_or(raw)
}

fn decode_text(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes).into_owned();
    quick_xml::escape::unescape(&raw).map(|c| c.into_owned()).unwrap_or(raw)
}

fn split_qname(qname: &str) -> (Option<String>, String) {
    match qname.split_once(':') {
        Some((p, l)) => (Some(p.to_string()), l.to_string()),
        None => (None, qname.to_string()),
    }
}

/// 解析一份 NodeSet XML，返回保真事实集合。
pub fn parse_nodeset(file_id: &str, path: &str, xml: &str, sha256: &str) -> Result<ParsedDocument, String> {
    let mut st = ParseState {
        doc: ParsedDocument {
            file_id: file_id.to_string(),
            path: path.to_string(),
            sha256: sha256.to_string(),
            ..Default::default()
        },
        stack: Vec::new(),
        prefix_map: BTreeMap::new(),
        node_ordinal: 0,
        open_nodes: Vec::new(),
    };
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    loop {
        let pos = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|e| format!("XML 解析失败 {path}: {e}"))?;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let qname = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let (_, local) = split_qname(&qname);
                let start = pos.saturating_sub(e.len());
                let mut attrs = BTreeMap::new();
                for a in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                    let val = decode_attr(&a.value);
                    if let Some(prefix) = key.strip_prefix("xmlns:") {
                        st.prefix_map.insert(prefix.to_string(), val.clone());
                    } else if key == "xmlns" {
                        st.prefix_map.insert(String::new(), val.clone());
                    }
                    attrs.insert(key, val);
                }
                let suppressed = st.stack.iter().any(|f| f.suppressed)
                    || is_unknown_extension(&st.stack, &local);
                let payload = if NodeKind::from_tag(&local).is_some() && is_node_context(&st.stack) {
                    st.node_ordinal += 1;
                    st.open_nodes.push(st.node_ordinal);
                    Payload::Node {
                        ordinal: st.node_ordinal,
                        node_id_raw: attrs.get("NodeId").cloned().unwrap_or_default(),
                        browse_name_raw: attrs.get("BrowseName").cloned().unwrap_or_default(),
                        parent_raw: attrs.get("ParentNodeId").cloned(),
                        display: None,
                        description: None,
                        attrs: attrs.clone(),
                    }
                } else if local == "Model" && parent_local(&st.stack) == Some("Models") {
                    Payload::Model {
                        model_uri: attrs.get("ModelUri").cloned().unwrap_or_default(),
                        version: attrs.get("Version").cloned(),
                        publication_date: attrs.get("PublicationDate").cloned(),
                        required: Vec::new(),
                    }
                } else {
                    Payload::None
                };
                st.stack.push(Frame { local, qname, start, attrs, text: String::new(), payload, suppressed });
            }
            Event::Empty(e) => {
                let qname = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let (_, local) = split_qname(&qname);
                let start = pos.saturating_sub(e.len());
                let mut attrs = BTreeMap::new();
                for a in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                    attrs.insert(key, decode_attr(&a.value));
                }
                let suppressed = st.stack.iter().any(|f| f.suppressed)
                    || is_unknown_extension(&st.stack, &local);
                let frame = Frame { local, qname, start, attrs, text: String::new(), payload: Payload::None, suppressed };
                close_frame(&mut st, frame, pos, xml);
            }
            Event::End(_) => {
                if let Some(frame) = st.stack.pop() {
                    close_frame(&mut st, frame, pos, xml);
                }
            }
            Event::Text(e) => {
                if let Some(top) = st.stack.last_mut() {
                    top.text.push_str(&decode_text(e.as_ref()));
                }
            }
            Event::CData(e) => {
                if let Some(top) = st.stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(e.as_ref()));
                }
            }
            _ => {}
        }
    }
    Ok(st.doc)
}

fn parent_local(stack: &[Frame]) -> Option<&str> {
    stack.last().map(|f| f.local.as_str())
}

fn is_node_context(stack: &[Frame]) -> bool {
    matches!(stack.last(), Some(f) if f.local == "UANodeSet")
}

fn is_unknown_extension(stack: &[Frame], local: &str) -> bool {
    let parent = match stack.last() {
        Some(p) => p,
        None => return false,
    };
    if parent.suppressed {
        return true;
    }
    if NodeKind::from_tag(&parent.local).is_some() {
        !matches!(
            local,
            "DisplayName"
                | "Description" | "References"
                | "Extensions" | "Extension"
                | "Value" | "Category"
                | "WriteMask" | "UserWriteMask"
                | "ReleaseStatus" | "RolePermissions"
                | "AccessRestrictions"
        )
    } else if parent.local == "UANodeSet" {
        !matches!(local, "NamespaceUris" | "Aliases" | "Models" | "Extensions")
            && NodeKind::from_tag(local).is_none()
    } else {
        false
    }
}

fn close_frame(st: &mut ParseState, frame: Frame, end_pos: usize, xml: &str) {
    let range = ByteRange::new(frame.start, end_pos);
    let parent_local_name = st.stack.last().map(|p| p.local.clone()).unwrap_or_default();

    match frame.local.as_str() {
        "Uri" if parent_local_name == "NamespaceUris" => {
            let local_index = st.doc.namespaces.len() as u16 + 1;
            st.doc.namespaces.push(NamespaceRow {
                file_id: st.doc.file_id.clone(),
                local_index,
                uri: frame.text.trim().to_string(),
                range,
            });
        }
        "Alias" if parent_local_name == "Aliases" => {
            let name = frame.attrs.get("Alias").cloned().unwrap_or_default();
            st.doc.aliases.push(AliasRow {
                file_id: st.doc.file_id.clone(),
                name,
                target_raw: frame.text.trim().to_string(),
                range,
            });
        }
        "RequiredModel" if parent_local_name == "Model" => {
            let req = RequiredModel {
                model_uri: frame.attrs.get("ModelUri").cloned().unwrap_or_default(),
                version: frame.attrs.get("Version").cloned(),
                publication_date: frame.attrs.get("PublicationDate").cloned(),
                range,
            };
            if let Some(p) = st.stack.last_mut() {
                if let Payload::Model { required, .. } = &mut p.payload {
                    required.push(req);
                }
            }
        }
        "DisplayName" | "Description" if NodeKind::from_tag(&parent_local_name).is_some() => {
            if let Some(p) = st.stack.last_mut() {
                if let Payload::Node { display, description, .. } = &mut p.payload {
                    if frame.local == "DisplayName" {
                        *display = Some(frame.text.trim().to_string());
                    } else {
                        *description = Some(frame.text.trim().to_string());
                    }
                }
            }
        }
        "Reference" if parent_local_name == "References" => {
            let is_forward = frame
                .attrs
                .get("IsForward")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);
            st.doc.references.push(RawReference {
                file_id: st.doc.file_id.clone(),
                node_raw_id: st.enclosing_raw_id().unwrap_or_default(),
                reference_type_raw: frame.attrs.get("ReferenceType").cloned().unwrap_or_default(),
                target_raw: frame.text.trim().to_string(),
                is_forward,
                range,
            });
        }
        "Extension" => {
            let raw_slice = xml.get(frame.start..end_pos.min(xml.len())).unwrap_or("").to_string();
            st.doc.extensions.push(Extension {
                file_id: st.doc.file_id.clone(),
                node_raw_id: st.enclosing_raw_id(),
                local_name: "Extension".to_string(),
                ns_uri: String::new(),
                attrs: frame.attrs.clone(),
                text: Some(raw_slice),
                range,
            });
        }
        _ => {
            match frame.payload {
                Payload::Node { ordinal, node_id_raw, browse_name_raw, parent_raw, display, description, attrs } => {
                    st.open_nodes.pop();
                    st.doc.nodes.push(RawNode {
                        raw_id: format!("raw:{}:{}", st.doc.file_id, ordinal),
                        file_id: st.doc.file_id.clone(),
                        kind: NodeKind::from_tag(&frame.local).unwrap(),
                        node_id_raw,
                        browse_name_raw,
                        parent_raw,
                        display_name: display,
                        description,
                        attrs,
                        ordinal,
                        range,
                    });
                }
                Payload::Model { model_uri, version, publication_date, required } => {
                    st.doc.models.push(ModelDecl {
                        file_id: st.doc.file_id.clone(),
                        model_uri,
                        version,
                        publication_date,
                        range,
                        required,
                    });
                }
                Payload::None => {
                    if frame.suppressed {
                        let (prefix, local) = split_qname(&frame.qname);
                        let ns_uri = prefix
                            .as_ref()
                            .and_then(|p| st.prefix_map.get(p).cloned())
                            .or_else(|| st.prefix_map.get("").cloned())
                            .unwrap_or_default();
                        let raw_slice =
                            xml.get(frame.start..end_pos.min(xml.len())).unwrap_or("").to_string();
                        st.doc.extensions.push(Extension {
                            file_id: st.doc.file_id.clone(),
                            node_raw_id: st.enclosing_raw_id(),
                            local_name: local,
                            ns_uri,
                            attrs: frame.attrs.clone(),
                            text: Some(raw_slice),
                            range,
                        });
                    }
                }
            }
        }
    }
}

/// 解析 NodeId 文本（含 alias 可能）。
pub fn parse_node_id(text: &str, alias_names: &[String]) -> RawNodeId {
    let text = text.trim();
    if alias_names.iter().any(|a| a == text) {
        return RawNodeId {
            text: text.to_string(),
            ns_index: None,
            ns_uri: None,
            id_kind: IdKind::String,
            id_value: text.to_string(),
            alias_name: Some(text.to_string()),
        };
    }
    let mut ns_index = None;
    let mut ns_uri = None;
    let mut id_kind = IdKind::Numeric;
    let mut id_value = String::new();
    let mut has_id = false;
    for part in text.split(';') {
        if let Some(v) = part.strip_prefix("ns=") {
            ns_index = v.parse::<u16>().ok();
        } else if let Some(v) = part.strip_prefix("nsu=") {
            ns_uri = Some(v.to_string());
        } else if let Some(v) = part.strip_prefix("i=") {
            id_kind = IdKind::Numeric;
            id_value = v.to_string();
            has_id = true;
        } else if let Some(v) = part.strip_prefix("s=") {
            id_kind = IdKind::String;
            id_value = v.to_string();
            has_id = true;
        } else if let Some(v) = part.strip_prefix("g=") {
            id_kind = IdKind::Guid;
            id_value = v.to_string();
            has_id = true;
        } else if let Some(v) = part.strip_prefix("b=") {
            id_kind = IdKind::Opaque;
            id_value = v.to_string();
            has_id = true;
        }
    }
    if !has_id {
        id_kind = IdKind::String;
        id_value = text.to_string();
    }
    RawNodeId { text: text.to_string(), ns_index, ns_uri, id_kind, id_value, alias_name: None }
}

/// 解析 BrowseName，返回 (命名空间前缀 index, 名称)。
pub fn parse_browse_name(text: &str) -> (Option<u16>, String) {
    match text.split_once(':') {
        Some((ns, name)) => (ns.parse::<u16>().ok(), name.to_string()),
        None => (None, text.to_string()),
    }
}
