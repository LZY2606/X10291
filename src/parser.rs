//! Streaming NodeSet XML parser.
//!
//! The parser is deliberately permissive and lossless: every known section
//! is captured with its byte range, and anything not recognised is stored
//! verbatim as a `RawUnknown` instead of being dropped.

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Reader;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::model::*;

pub fn parse_file(filename: &str, bytes: &[u8]) -> Result<RawNodeSet, String> {
    let mut doc_id = String::from("sha256:");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    doc_id.push_str(&hex::encode(hasher.finalize()));

    let mut r = Reader::from_reader(bytes);
    r.config_mut().trim_text(true);
    let mut p = Parser {
        r,
        bytes,
        out: RawNodeSet {
            doc_id,
            filename: filename.to_string(),
            bytes: bytes.to_vec(),
            namespace_uris: Vec::new(),
            models: Vec::new(),
            aliases: Vec::new(),
            nodes: Vec::new(),
            unknown: Vec::new(),
        },
    };

    // Locate the UANodeSet root, skipping any prolog/whitespace.
    loop {
        let pos = p.r.buffer_position();
        match p.r.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"UANodeSet" => {
                p.parse_root(pos);
                break;
            }
            Ok(Event::Empty(e)) if e.name().as_ref() == b"UANodeSet" => {
                let _ = e;
                break;
            }
            Ok(Event::Eof) => return Err("no UANodeSet root element".into()),
            Err(e) => return Err(format!("xml error at {pos}: {e}")),
            _ => {}
        }
    }
    Ok(p.out)
}

struct Parser<'a> {
    r: Reader<&'a [u8]>,
    bytes: &'a [u8],
    out: RawNodeSet,
}

impl<'a> Parser<'a> {
    fn doc_id(&self) -> String {
        self.out.doc_id.clone()
    }

    fn attr_map(e: &BytesStart) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        for attr in e.attributes().flatten() {
            let k = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
            let v = attr.unescape_value().map(|c| c.into_owned()).unwrap_or_default();
            m.insert(k, v);
        }
        m
    }

    fn attr_ranges(&self, e: &BytesStart, start: usize) -> BTreeMap<String, Range> {
        let tag_end = self
            .bytes
            .iter()
            .position(|b| *b == b'>')
            .map(|i| i + 1)
            .unwrap_or(self.bytes.len());
        let tag = &self.bytes[start..tag_end.min(self.bytes.len())];
        let mut m = BTreeMap::new();
        for attr in e.attributes().flatten() {
            let k = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
            if let Some(rel) = tag.windows(k.len() + 2).position(|w| {
                w[..k.len()] == k.as_bytes()
                    && w[k.len()] == b'='
                    && (w[k.len() + 1] == b'"' || w[k.len() + 1] == b'\'')
            }) {
                let vstart = start + rel + k.len() + 2;
                let quote = tag[rel + k.len() + 1];
                let mut vend = vstart;
                while vend < tag_end && self.bytes[vend] != quote {
                    vend += 1;
                }
                m.insert(k, Range::new(vstart, vend));
            }
        }
        m
    }

    /// Read text of an element that just started until its matching end.
    fn read_inline_text(&mut self) -> String {
        let mut text = String::new();
        loop {
            match self.r.read_event() {
                Ok(Event::Text(t)) => text.push_str(&t.unescape().unwrap_or_default()),
                Ok(Event::CData(t)) => text.push_str(&String::from_utf8_lossy(t.as_ref())),
                Ok(Event::End(_)) | Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }
        text
    }

    /// Skip a balanced element (start already consumed), returning the
    /// position just past the closing tag. Raw text is captured.
    fn skip_element(&mut self, name: &[u8], start: usize) -> RawUnknown {
        let mut depth = 1usize;
        let mut text = String::new();
        loop {
            match self.r.read_event() {
                Ok(Event::Start(e)) => {
                    if e.name().as_ref() == name {
                        depth += 1;
                    }
                }
                Ok(Event::End(e)) => {
                    depth -= 1;
                    if depth == 0 {
                        let end = self.r.buffer_position();
                        return RawUnknown {
                            path: String::from_utf8_lossy(name).into_owned(),
                            text,
                            attrs: BTreeMap::new(),
                            range: Range::new(start, end),
                        };
                    }
                }
                Ok(Event::Text(t)) => text.push_str(&t.unescape().unwrap_or_default()),
                Ok(Event::Empty(_)) => {}
                Ok(Event::Eof) => {
                    return RawUnknown {
                        path: String::from_utf8_lossy(name).into_owned(),
                        text,
                        attrs: BTreeMap::new(),
                        range: Range::new(start, self.bytes.len()),
                    }
                }
                Err(_) => {}
            }
        }
    }
}

#[allow(dead_code)]
fn text_of(t: &BytesText) -> String {
    t.unescape().map(|c| c.into_owned()).unwrap_or_default()
}
#[allow(dead_code)]
fn end_name(e: &BytesEnd) -> String {
    String::from_utf8_lossy(e.name().as_ref()).into_owned()
}

impl<'a> Parser<'a> {
    fn parse_root(&mut self, root_start: usize) {
        loop {
            let pos = self.r.buffer_position();
            let ev = match self.r.read_event() {
                Ok(ev) => ev,
                Err(e) => {
                    eprintln!("parse warning: {e}");
                    continue;
                }
            };
            match ev {
                Event::Start(e) => {
                    let name = e.name().as_ref().to_vec();
                    match name.as_slice() {
                        b"NamespaceUris" => self.parse_namespace_uris(),
                        b"Models" => self.parse_models(),
                        b"Aliases" => self.parse_aliases(),
                        b"Extensions" => {
                            let mut u = self.skip_element(b"Extensions", pos);
                            u.path = "Extensions".into();
                            self.out.unknown.push(u);
                        }
                        n if NODE_KINDS
                            .iter()
                            .any(|k| n == k.as_bytes()) =>
                        {
                            let kind = String::from_utf8_lossy(n).into_owned();
                            let node = self.parse_node(kind, e, pos);
                            self.out.nodes.push(node);
                        }
                        other => {
                            let mut u = self.skip_element(other, pos);
                            u.path = format!("UANodeSet/{}", String::from_utf8_lossy(other));
                            self.out.unknown.push(u);
                        }
                    }
                }
                Event::Empty(e) => {
                    let name = e.name().as_ref();
                    match name {
                        b"Model" => self.out.models.push(RawModel {
                            model_uri: String::new(),
                            version: None,
                            publication_date: None,
                            required_model: Vec::new(),
                            attrs: Self::attr_map(&e),
                            range: self.empty_range(pos),
                        }),
                        other => {
                            let raw = String::from_utf8_lossy(
                                &self.bytes[pos..self.r.buffer_position()],
                            )
                            .into_owned();
                            self.out.unknown.push(RawUnknown {
                                path: format!("UANodeSet/{}", String::from_utf8_lossy(other)),
                                text: raw,
                                attrs: Self::attr_map(&e),
                                range: Range::new(pos, self.r.buffer_position()),
                            });
                        }
                    }
                }
                Event::End(e) if e.name().as_ref() == b"UANodeSet" => break,
                Event::Eof => break,
                _ => {}
            }
        }
        let _ = root_start;
    }

    fn empty_range(&self, pos: usize) -> Range {
        Range::new(pos, self.r.buffer_position())
    }

    fn parse_namespace_uris(&mut self) {
        // ns=0 (OPC UA) is implicit and always present.
        self.out
            .namespace_uris
            .push(Located::new("http://opcfoundation.org/UA/", self.doc_id(), Range::new(0, 0)));
        loop {
            let pos = self.r.buffer_position();
            match self.r.read_event() {
                Ok(Event::Start(e)) if e.name().as_ref() == b"Uri" => {
                    let uri = self.read_inline_text();
                    let end = self.r.buffer_position();
                    self.out
                        .namespace_uris
                        .push(Located::new(uri, self.doc_id(), Range::new(pos, end)));
                }
                Ok(Event::Empty(e)) if e.name().as_ref() == b"Uri" => {
                    let _ = e;
                    self.out.namespace_uris.push(Located::new(
                        String::new(),
                        self.doc_id(),
                        Range::new(pos, self.r.buffer_position()),
                    ));
                }
                Ok(Event::End(e)) if e.name().as_ref() == b"NamespaceUris" => break,
                Ok(Event::Eof) => break,
                _ => {}
            }
        }
    }

    fn parse_aliases(&mut self) {
        loop {
            let pos = self.r.buffer_position();
            match self.r.read_event() {
                Ok(Event::Start(e)) if e.name().as_ref() == b"Alias" => {
                    let attrs = Self::attr_map(&e);
                    let target = self.read_inline_text();
                    let end = self.r.buffer_position();
                    let alias = attrs.get("Alias").cloned().unwrap_or_default();
                    self.out.aliases.push(RawAlias {
                        alias,
                        node_id_text: target,
                        range: Range::new(pos, end),
                    });
                }
                Ok(Event::End(e)) if e.name().as_ref() == b"Aliases" => break,
                Ok(Event::Eof) => break,
                _ => {}
            }
        }
    }

    fn parse_models(&mut self) {
        loop {
            let pos = self.r.buffer_position();
            match self.r.read_event() {
                Ok(Event::Start(e)) if e.name().as_ref() == b"Model" => {
                    self.out.models.push(self.parse_model(e, pos));
                }
                Ok(Event::End(e)) if e.name().as_ref() == b"Models" => break,
                Ok(Event::Eof) => break,
                _ => {}
            }
        }
    }

    fn parse_model(&mut self, start_el: BytesStart, start: usize) -> RawModel {
        let attrs = Self::attr_map(&start_el);
        let mut required = Vec::new();
        loop {
            let rpos = self.r.buffer_position();
            match self.r.read_event() {
                Ok(Event::Start(e)) if e.name().as_ref() == b"RequiredModel" => {
                    let m = Self::attr_map(&e);
                    let range = self.skip_element(b"RequiredModel", rpos).range;
                    required.push(RawRequiredModel {
                        model_uri: m.get("ModelUri").cloned().unwrap_or_default(),
                        version: m.get("Version").cloned(),
                        publication_date: m.get("PublicationDate").cloned(),
                        range,
                    });
                }
                Ok(Event::Empty(e)) if e.name().as_ref() == b"RequiredModel" => {
                    let m = Self::attr_map(&e);
                    required.push(RawRequiredModel {
                        model_uri: m.get("ModelUri").cloned().unwrap_or_default(),
                        version: m.get("Version").cloned(),
                        publication_date: m.get("PublicationDate").cloned(),
                        range: Range::new(rpos, self.r.buffer_position()),
                    });
                }
                Ok(Event::End(e)) if e.name().as_ref() == b"Model" => {
                    let _ = e;
                    break;
                }
                Ok(Event::Eof) => break,
                _ => {}
            }
        }
        RawModel {
            model_uri: attrs.get("ModelUri").cloned().unwrap_or_default(),
            version: attrs.get("Version").cloned(),
            publication_date: attrs.get("PublicationDate").cloned(),
            required_model: required,
            attrs,
            range: Range::new(start, self.r.buffer_position()),
        }
    }
}

impl<'a> Parser<'a> {
    fn parse_node(&mut self, kind: String, start_el: BytesStart, start: usize) -> RawNode {
        let attrs = Self::attr_map(&start_el);
        let attr_ranges = self.attr_ranges(&start_el, start);
        let mut node = RawNode {
            id: String::new(),
            kind,
            node_id: attrs.get("NodeId").cloned().unwrap_or_default(),
            browse_name: attrs.get("BrowseName").cloned().unwrap_or_default(),
            display_name: None,
            description: None,
            write_mask: attrs.get("WriteMask").cloned(),
            user_write_mask: attrs.get("UserWriteMask").cloned(),
            value: None,
            data_type: attrs.get("DataType").cloned(),
            value_rank: attrs.get("ValueRank").cloned(),
            references: Vec::new(),
            unknown: Vec::new(),
            attrs,
            attr_ranges,
            range: Range::new(start, start),
        };

        loop {
            let pos = self.r.buffer_position();
            let ev = match self.r.read_event() {
                Ok(ev) => ev,
                Err(_) => continue,
            };
            match ev {
                Event::Start(e) => match e.name().as_ref() {
                    b"DisplayName" => node.display_name = Some(self.read_inline_text()),
                    b"Description" => node.description = Some(self.read_inline_text()),
                    b"References" => self.parse_references(&mut node),
                    b"Value" => {
                        // Preserve the *original* representation verbatim;
                        // the derived layer is the only place that parses it.
                        let raw = self.capture_raw(b"Value", pos);
                        let inner = self
                            .bytes
                            .get(raw.range.start..raw.range.end)
                            .map(|b| String::from_utf8_lossy(b).into_owned())
                            .unwrap_or_default();
                        node.value = Some(RawValue { text: inner, range: raw.range });
                    }
                    other => {
                        let mut u = self.skip_element(other, pos);
                        u.path = format!("{}/{}", node.kind, String::from_utf8_lossy(other));
                        node.unknown.push(u);
                    }
                },
                Event::Empty(e) => {
                    let raw = String::from_utf8_lossy(&self.bytes[pos..self.r.buffer_position()])
                        .into_owned();
                    node.unknown.push(RawUnknown {
                        path: format!("{}/{}", node.kind, String::from_utf8_lossy(e.name().as_ref())),
                        text: raw,
                        attrs: Self::attr_map(&e),
                        range: Range::new(pos, self.r.buffer_position()),
                    });
                }
                Event::End(e) if e.name().as_ref() == node.kind.as_bytes() => {
                    node.range = Range::new(start, self.r.buffer_position());
                    break;
                }
                Event::Eof => {
                    node.range = Range::new(start, self.r.buffer_position());
                    break;
                }
                _ => {}
            }
        }
        node
    }

    /// Like skip_element but returns the span and discards buffered text;
    /// used for <Value>, whose raw bytes are read straight from the file.
    fn capture_raw(&mut self, name: &[u8], start: usize) -> RawUnknown {
        let mut depth = 1usize;
        loop {
            match self.r.read_event() {
                Ok(Event::Start(e)) if e.name().as_ref() == name => depth += 1,
                Ok(Event::End(e)) => {
                    if e.name().as_ref() == name {
                        depth -= 1;
                        if depth == 0 {
                            return RawUnknown {
                                path: String::new(),
                                text: String::new(),
                                attrs: BTreeMap::new(),
                                range: Range::new(start, self.r.buffer_position()),
                            };
                        }
                    }
                }
                Ok(Event::Eof) => {
                    return RawUnknown {
                        path: String::new(),
                        text: String::new(),
                        attrs: BTreeMap::new(),
                        range: Range::new(start, self.bytes.len()),
                    }
                }
                Err(_) => {}
                _ => {}
            }
        }
    }

    fn parse_references(&mut self, node: &mut RawNode) {
        loop {
            let pos = self.r.buffer_position();
            match self.r.read_event() {
                Ok(Event::Start(e)) if e.name().as_ref() == b"Reference" => {
                    let attrs = Self::attr_map(&e);
                    let target = self.read_inline_text();
                    let end = self.r.buffer_position();
                    let is_forward = attrs
                        .get("IsForward")
                        .map(|v| v != "false")
                        .unwrap_or(true);
                    node.references.push(RawRef {
                        reference_type: attrs.get("ReferenceType").cloned().unwrap_or_default(),
                        is_forward,
                        target_text: target,
                        range: Range::new(pos, end),
                    });
                }
                Ok(Event::Empty(e)) if e.name().as_ref() == b"Reference" => {
                    let attrs = Self::attr_map(&e);
                    let is_forward = attrs
                        .get("IsForward")
                        .map(|v| v != "false")
                        .unwrap_or(true);
                    node.references.push(RawRef {
                        reference_type: attrs.get("ReferenceType").cloned().unwrap_or_default(),
                        is_forward,
                        target_text: String::new(),
                        range: Range::new(pos, self.r.buffer_position()),
                    });
                }
                Ok(Event::End(e)) if e.name().as_ref() == b"References" => {
                    let _ = e;
                    break;
                }
                Ok(Event::Eof) => break,
                _ => {}
            }
        }
    }
}
