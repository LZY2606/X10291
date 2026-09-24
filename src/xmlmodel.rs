//! Raw XML tree that preserves everything: tags, attributes, text, and the byte
//! range of every element in the source file. Unrecognized extensions are kept
//! verbatim as `RawXml` nodes so nothing is lost during import.

use anyhow::{anyhow, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawXml {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<RawXml>,
    pub text: Option<String>,
    /// Byte range [start, end) in the source file.
    pub span: (usize, usize),
}

impl RawXml {
    fn new(tag: String) -> Self {
        RawXml { tag, attrs: Vec::new(), children: Vec::new(), text: None, span: (0, 0) }
    }

    /// Local name without namespace prefix.
    pub fn local(&self) -> &str {
        local_name(&self.tag)
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name || local_name(k) == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn children_named(&self, local: &str) -> impl Iterator<Item = &RawXml> {
        self.children.iter().filter(move |c| c.local() == local)
    }

    pub fn first_child(&self, local: &str) -> Option<&RawXml> {
        self.children.iter().find(|c| c.local() == local)
    }

    pub fn text_of(&self, local: &str) -> Option<String> {
        self.first_child(local).and_then(|c| c.text.clone()).map(|t| t.trim().to_string())
    }
}

pub fn local_name(tag: &str) -> &str {
    match tag.rfind(':') {
        Some(i) => &tag[i + 1..],
        None => tag,
    }
}

/// Parse a whole XML document into a RawXml tree with byte spans.
pub fn parse_raw(content: &str) -> Result<RawXml> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<RawXml> = Vec::new();
    let mut root: Option<RawXml> = None;
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let mut node = RawXml::new(String::from_utf8_lossy(e.name().as_ref()).into_owned());
                node.span.0 = pos;
                for attr in e.attributes().flatten() {
                    node.attrs.push((
                        String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                        String::from_utf8_lossy(&attr.value).into_owned(),
                    ));
                }
                stack.push(node);
            }
            Ok(Event::Empty(e)) => {
                let mut node = RawXml::new(String::from_utf8_lossy(e.name().as_ref()).into_owned());
                node.span = (pos, reader.buffer_position() as usize);
                for attr in e.attributes().flatten() {
                    node.attrs.push((
                        String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                        String::from_utf8_lossy(&attr.value).into_owned(),
                    ));
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => {
                        if root.is_some() {
                            return Err(anyhow!("multiple root elements"));
                        }
                        root = Some(node);
                    }
                }
            }
            Ok(Event::End(_)) => {
                let mut node = stack.pop().ok_or_else(|| anyhow!("unbalanced XML"))?;
                node.span.1 = reader.buffer_position() as usize;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Ok(Event::Text(e)) => {
                let text = e.decode().unwrap_or_default().into_owned();
                if !text.trim().is_empty() {
                    if let Some(top) = stack.last_mut() {
                        match &mut top.text {
                            Some(existing) => {
                                existing.push_str(&text);
                            }
                            None => top.text = Some(text),
                        }
                    }
                }
            }
            Ok(Event::CData(e)) => {
                let text = String::from_utf8_lossy(e.as_ref()).into_owned();
                if let Some(top) = stack.last_mut() {
                    top.text = Some(text);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("XML parse error at {}: {}", pos, e)),
            Ok(_) => {}
        }
    }
    root.ok_or_else(|| anyhow!("empty document"))
}
