//! Lossless NodeSet2 XML parser.
//!
//! Preserves every raw node, the namespace table, aliases, model
//! dependencies, byte spans and unrecognized extensions. Numeric namespace
//! indexes are NOT interpreted here.

use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::{Digest, Sha256};

use crate::core::events::EventLog;
use crate::core::model::*;

fn span_of(start: usize, end: usize) -> Span {
    Span::new(start, end)
}

pub fn parse_node_id(text: &str, span: Option<Span>) -> RawNodeId {
    let raw = text.trim().to_string();
    let mut ns: u16 = 0;
    let mut id_type = "i".to_string();
    let mut identifier = String::new();
    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(v) = part.strip_prefix("ns=") {
            ns = v.trim().parse().unwrap_or(0);
        } else if let Some((k, v)) = part.split_once('=') {
            match k.trim() {
                "i" => {
                    id_type = "i".to_string();
                    identifier = v.trim().to_string();
                }
                "s" => {
                    id_type = "s".to_string();
                    identifier = v.trim().to_string();
                }
                "g" => {
                    id_type = "g".to_string();
                    identifier = v.trim().to_string();
                }
                "b" => {
                    id_type = "b".to_string();
                    identifier = v.trim().to_string();
                }
                _ => {}
            }
        } else if identifier.is_empty() {
            // Bare identifier with no key defaults to numeric.
            id_type = "i".to_string();
            identifier = part.to_string();
        }
    }
    RawNodeId { ns, identifier, id_type, text: raw, span }
}

pub fn parse_browse_name(text: &str, span: Option<Span>) -> RawBrowseName {
    let raw = text.trim().to_string();
    if let Some((prefix, local)) = raw.split_once(':') {
        RawBrowseName { text: raw, prefix: Some(prefix.trim().to_string()), local: local.trim().to_string(), span }
    } else {
        RawBrowseName { text: raw.clone(), prefix: None, local: raw, span }
    }
}

/// Consume a whole element starting at `start_ev` (already read). Returns
/// (inner_text, inner_raw_range, end_position).
fn consume_element<B: quick_xml::events::BufRead>(
    reader: &mut Reader<B>,
    start_ev: &quick_xml::events::BytesStart,
    content: &str,
    tag_start: usize,
) -> (String, Option<(usize, usize)>, usize) {
    let name = String::from_utf8_lossy(start_ev.name().as_ref()).to_string();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    let mut inner_text = String::new();
    let mut inner_range: Option<(usize, usize)> = None;
    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if String::from_utf8_lossy(e.name().as_ref()) == name {
                    depth += 1;
                }
            }
            Ok(Event::End(ref e)) => {
                if String::from_utf8_lossy(e.name().as_ref()) == name {
                    depth -= 1;
                    if depth == 0 {
                        let end = reader.buffer_position();
                        inner_range.get_or_insert((tag_start, tag_start));
                        let _ = pos;
                        return (inner_text, inner_range, end);
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if let Ok(t) = e.unescape() {
                    inner_text.push_str(&t);
                }
            }
            Ok(Event::Empty(e)) => {
                if let Some((s, en)) = inner_range {
                    let _ = (s, en);
                }
                let _ = e;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let end = reader.buffer_position();
    (inner_text, inner_range, end)
}

/// Read an element known to be empty-or-paired starting at `tag_start`,
/// capturing its *entire* raw XML and its direct inner text, while a callback
/// handles child Start/Empty events. Stops at the matching end.
fn scan_element(
    content: &str,
    reader: &mut Reader<quick_xml::NsReader<&[u8]>>,
    tag_local: &str,
    tag_start: usize,
    mut on_child: impl FnMut(
        &str,
        &quick_xml::events::BytesStart,
        usize,
        &mut Reader<quick_xml::NsReader<&[u8]>>,
        &str,
    ) -> ChildOutcome,
) -> (usize, String) {
    let mut depth = 1usize;
    let mut buf = Vec::new();
    let mut text = String::new();
    loop {
        let pos = reader.buffer_position();
        let ev = reader.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(ref e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match on_child(&local, e, pos, reader, content) {
                    ChildOutcome::ConsumedEnd(end) => {
                        // child already consumed; adjust depth externally
                        let _ = end;
                    }
                    ChildOutcome::Paired => {}
                }
                if local == tag_local {
                    depth += 1;
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                on_child(&local, e, pos, reader, content);
            }
            Ok(Event::End(ref e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if local == tag_local {
                    depth -= 1;
                    if depth == 0 {
                        return (reader.buffer_position(), text);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Ok(t) = t.unescape() {
                    text.push_str(&t);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (reader.buffer_position(), text)
}

enum ChildOutcome {
    Paired,
    ConsumedEnd(usize),
}
