//! Minimal XML tooling for OPC UA NodeSet files.
//!
//! The parser intentionally does NOT deserialize into a fixed OPC UA schema:
//! every element (including unknown extensions from vendor namespaces) is kept
//! as a [`DomElement`] with its original source byte span, so downstream layers
//! can point a reviewer back at the exact XML range and nothing is dropped.

use std::collections::BTreeMap;

/// Half-open byte range inside the original source file: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    pub start: u64,
    pub end: u64,
}

impl Span {
    pub fn new(start: u64, end: u64) -> Self {
        Span { start, end }
    }
    pub fn len(&self) -> u64 {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// A preserved XML element: local tag, namespace prefix, attributes, direct
/// text and children, plus its original [`Span`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DomElement {
    pub tag: String,
    pub prefix: Option<String>,
    pub attrs: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<DomElement>,
    pub span: Span,
}

impl DomElement {
    pub fn qname(&self) -> String {
        match &self.prefix {
            Some(p) if !p.is_empty() => format!("{p}:{}", self.tag),
            _ => self.tag.clone(),
        }
    }

    pub fn first(&self, tag: &str) -> Option<&DomElement> {
        self.children.iter().find(|c| c.tag == tag)
    }

    pub fn all<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a DomElement> + 'a {
        self.children.iter().filter(move |c| c.tag == tag)
    }

    pub fn attr(&self, s: &str) -> Option<&str> {
        self.attrs.get(s).map(String::as_str)
    }
}

#[derive(Debug)]
pub enum XmlError {
    Pos(u64, String),
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XmlError::Pos(p, m) => write!(f, "XML error at byte {p}: {m}"),
        }
    }
}
impl std::error::Error for XmlError {}

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Result as QResult};

/// Parse a whole document and return the single root element with byte spans.
pub fn parse_root(xml: &[u8]) -> Result<DomElement, XmlError> {
    let mut reader = Reader::from_reader(xml);
    let mut stack: Vec<DomElement> = Vec::new();
    let mut root: Option<DomElement> = None;
    loop {
        let pos = reader.buffer_position() as u64;
        let event = reader
            .read_event()
            .map_err(|e| XmlError::Pos(reader.buffer_position() as u64, e.to_string()))?;
        match event {
            Event::Start(e) => {
                stack.push(build_start(&e, pos));
            }
            Event::Empty(e) => {
                let mut el = build_start(&e, pos);
                el.span = Span::new(pos, reader.buffer_position() as u64);
                attach(&mut stack, &mut root, el);
            }
            Event::End(_) => {
                let mut el = stack
                    .pop()
                    .ok_or_else(|| XmlError::Pos(pos, "unexpected closing tag".to_string()))?;
                el.span = Span::new(pos, reader.buffer_position() as u64);
                attach(&mut stack, &mut root, el);
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    if let Ok(s) = t.unescape() {
                        top.text.push_str(&s);
                    } else {
                        top.text.push_str(&String::from_utf8_lossy(t.as_ref()));
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| XmlError::Pos(0, "empty document".to_string()))
}

fn attach(stack: &mut Vec<DomElement>, root: &mut Option<DomElement>, mut el: DomElement) {
    match stack.last_mut() {
        Some(parent) => {
            parent.children.push(el);
        }
        None => match root {
            Some(_) => {}
            None => *root = Some(el),
        },
    }
}

fn build_start(e: &BytesStart, pos: u64) -> DomElement {
    let q = e.name().as_ref().to_vec();
    let q = String::from_utf8_lossy(&q).to_string();
    let (prefix, tag) = match q.split_once(':') {
        Some((p, t)) => (Some(p.to_string()), t.to_string()),
        None => (None, q),
    };
    let mut attrs = BTreeMap::new();
    for attr in e.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
        let val = attr
            .decode_and_unescape_value()
            .map(|c| c.to_string())
            .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
        attrs.insert(key, val);
    }
    DomElement {
        tag,
        prefix,
        attrs,
        text: String::new(),
        children: Vec::new(),
        span: Span::new(pos, pos),
    }
}

/// Decode an XML-encoded scalar the way NodeSet files write attribute values
/// (plain text, entities already decoded by the parser).
pub fn decode_bytes(v: &[u8]) -> String {
    String::from_utf8_lossy(v).to_string()
}

#[allow(dead_code)]
fn force_q<T>(r: QResult<T>) -> QResult<T> {
    r
}
