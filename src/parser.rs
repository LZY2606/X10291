//! Lenient, span-preserving NodeSet2 XML parser.
//!
//! Never drops information: unknown elements/extensions are captured as raw
//! outer XML with byte spans; invalid NodeIds produce diagnostics instead of
//! panicking.

use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::{Digest, Sha256};

use crate::model::*;

#[derive(Debug, Clone)]
struct Elem {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Elem>,
    text: String,
    span: Span,
}

fn line_at(content: &str, byte_pos: usize) -> u32 {
    content[..byte_pos.min(content.len())].matches('\n').count() as u32 + 1
}

/// Parse into a generic span-aware DOM tree.
fn build_tree(content: &str) -> Result<Elem, String> {
    let mut r = Reader::from_str(content);
    *r.config_mut() = quick_xml::Config::default().trim_text(true).expand_empty_elements(false);

    struct Frame {
        el: Elem,
        text: Vec<u8>,
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<Elem> = None;

    loop {
        let pos_before = r.buffer_position();
        let ev = r.read_event_into(&mut Vec::new()).map_err(|e| format!("{e}"))?;
        match ev {
            Event::Start(e) => {
                let (name, attrs) = decode_start(&e);
                stack.push(Frame {
                    el: Elem {
                        name,
                        attrs,
                        children: Vec::new(),
                        text: String::new(),
                        span: Span { start: pos_before, end: 0, line: line_at(content, pos_before) },
                    },
                    text: Vec::new(),
                });
            }
            Event::Empty(e) => {
                let (name, attrs) = decode_start(&e);
                let el = Elem {
                    name,
                    attrs,
                    children: Vec::new(),
                    text: String::new(),
                    span: Span {
                        start: pos_before,
                        end: r.buffer_position(),
                        line: line_at(content, pos_before),
                    },
                };
                attach(&mut stack, &mut root, el);
            }
            Event::Text(t) => {
                if let Some(frame) = stack.last_mut() {
                    if let Ok(s) = t.unescape() {
                        frame.text.extend_from_slice(s.as_bytes());
                    }
                }
            }
            Event::CData(t) => {
                if let Some(frame) = stack.last_mut()) {
                    frame.text.extend_from_slice(t);
                }
            }
            Event::End(_) => {
                let end = r.buffer_position();
                if let Some(mut frame) = stack.pop() {
                    frame.el.span.end = end;
                    frame.el.text = String::from_utf8_lossy(&frame.text).trim().to_string();
                    attach(&mut stack, &mut root, frame.el);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| "empty document".to_string())
}

fn decode_start(
    e: &quick_xml::events::BytesStart,
) -> (String, Vec<(String, String)>) {
    let raw = String::from_utf8_lossy(e.name().as_ref()).to_string();
    let name = raw.rsplit(':').next().unwrap_or(&raw).to_string();
    let mut attrs = Vec::new();
    for attr in e.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
        let val = attr.unescape_value().map(|c| c.into_owned()).unwrap_or_default();
        attrs.push((key, val));
    }
    (name, attrs)
}

fn attach(stack: &mut Vec<impl FrameLike>, root: &mut Option<Elem>, el: Elem) {
    if let Some(top) = stack.last_mut() {
        top.frame_el().children.push(el);
    } else if root.is_none() {
        *root = Some(el);
    }
}

trait FrameLike {
    fn frame_el(&mut self) -> &mut Elem;
}
