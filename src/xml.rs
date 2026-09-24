//! Byte-faithful XML tokenizer/parser for OPC UA NodeSet files.
//!
//! Every element keeps its byte span in the original file so the UI can jump
//! back to the exact source range. Unknown elements/attributes are preserved
//! verbatim (see [`RawNode`]) instead of being dropped by schema mapping.

use std::collections::HashMap;

/// Half-open byte range `[start,end)` inside one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Attr {
    pub name: String,
    pub value: String,
    pub span: Span,
    pub value_span: Span,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Element {
    /// Local name (prefix stripped).
    pub name: String,
    /// Original qualified name, e.g. `nc:VendorExtension`.
    pub qname: String,
    pub attrs: Vec<Attr>,
    pub attr_map: HashMap<String, Attr>,
    pub children: Vec<Element>,
    /// Concatenated text directly inside this element (trim of surrounding
    /// whitespace is avoided; raw text is stored).
    pub text: String,
    pub span: Span,
    /// Span covering the opening tag, including `<...>`.
    pub open_span: Span,
    /// True for self-closing elements (`<Foo/>`) without an end tag.
    pub self_closing: bool,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attr_map.get(name)
    }
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn children_of<'a>(&'a self, name: &str) -> Vec<&'a Element> {
        self.children.iter().filter(|c| c.name == name).collect()
    }
    /// Trimmed text, convenient for leaf values.
    pub fn text_trim(&self) -> String {
        self.text.trim().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct XmlDoc {
    pub root: Element,
    /// `prefix -> namespace uri` collected from `xmlns:*` attributes on root.
    pub ns_decls: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct XmlError {
    pub message: String,
    pub offset: usize,
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "xml error at byte {}: {}", self.offset, self.message)
    }
}
impl std::error::Error for XmlError {}

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser { b: s.as_bytes(), pos: 0 }
    }

    fn err<T>(&self, msg: &str) -> Result<T, XmlError> {
        Err(XmlError { message: msg.to_string(), offset: self.pos })
    }

    fn skip_ws(&mut self) {
        while self.pos < self.b.len() {
            match self.b[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    /// Parses one document, returning the first element node.
    fn parse_doc(&mut self) -> Result<XmlDoc, XmlError> {
        // prolog / comments / processing instructions
        loop {
            self.skip_ws();
            if self.b[self.pos..].starts_with(b"<?") {
                self.read_until(b"?>")?;
            } else if self.b[self.pos..].starts_with(b"<!--") {
                self.read_comment()?;
            } else if self.b[self.pos..].starts_with(b"<!") {
                // DOCTYPE etc.
                self.read_until(b">")?;
            } else {
                break;
            }
        }
        let root = self.parse_element()?;
        let mut ns_decls = HashMap::new();
        for a in &root.attrs {
            if let Some(prefix) = a.name.strip_prefix("xmlns:") {
                ns_decls.insert(prefix.to_string(), a.value.clone());
            }
        }
        Ok(XmlDoc { root, ns_decls })
    }

    fn read_until(&mut self, needle: &[u8]) -> Result<(), XmlError> {
        let start = self.pos;
        for i in start..self.b.len().saturating_sub(needle.len() - 1) {
            if self.b[i..].starts_with(needle) {
                self.pos = i + needle.len();
                return Ok(());
            }
        }
        self.err("unterminated markup")
    }

    fn read_comment(&mut self) -> Result<(), XmlError> {
        if !self.b[self.pos..].starts_with(b"<!--") {
            return self.err("expected comment");
        }
        let mut i = self.pos + 4;
        while i + 3 <= self.b.len() {
            if self.b[i..].starts_with(b"-->") {
                self.pos = i + 3;
                return Ok(());
            }
            i += 1;
        }
        self.err("unterminated comment")
    }

    fn parse_element(&mut self) -> Result<Element, XmlError> {
        if self.pos >= self.b.len() || self.b[self.pos] != b'<' {
            return self.err("expected '<'");
        }
        let start = self.pos;
        self.pos += 1;
        let (qname, name_start) = self.read_name()?;
        let _ = name_start;
        let mut attrs = Vec::new();
        let mut self_closing = false;
        loop {
            self.skip_ws();
            if self.pos >= self.b.len() {
                return self.err("unterminated opening tag");
            }
            if self.b[self.pos..].starts_with(b"/>") {
                self.pos += 2;
                self_closing = true;
                break;
            }
            if self.b[self.pos] == b'>' {
                self.pos += 1;
                break;
            }
            let attr = self.parse_attr()?;
            attrs.push(attr);
        }
        let open_end = self.pos;
        let local = qname.split(':').last().unwrap_or(&qname).to_string();
        let mut attr_map = HashMap::new();
        for a in &attrs {
            attr_map.insert(a.name.clone(), a.clone());
        }
        let mut el = Element {
            name: local,
            qname,
            attrs,
            attr_map,
            children: Vec::new(),
            text: String::new(),
            span: Span::new(start, open_end),
            open_span: Span::new(start, open_end),
            self_closing,
        };
        if self_closing {
            el.span.end = open_end;
            return Ok(el);
        }
        // content
        let content_start = self.pos;
        let mut text_bytes: Vec<u8> = Vec::new();
        loop {
            if self.pos >= self.b.len() {
                return self.err("unterminated element content");
            }
            if self.b[self.pos..].starts_with(b"</") {
                // record direct text accumulated before this end/child boundary
                self.pos += 2;
                let (end_qn, _) = self.read_name()?;
                self.skip_ws();
                if self.pos >= self.b.len() || self.b[self.pos] != b'>' {
                    return self.err("expected '>' in end tag");
                }
                self.pos += 1;
                if end_qn != el.qname {
                    return self.err(&format!("mismatched end tag </{}>", end_qn));
                }
                el.span.end = self.pos;
                break;
            }
            if self.b[self.pos..].starts_with(b"<!--") {
                self.flush_text(&mut text_bytes, content_start, &mut el)?;
                self.read_comment()?;
                continue;
            }
            if self.b[self.pos..].starts_with(b"<?") {
                self.flush_text(&mut text_bytes, content_start, &mut el)?;
                self.read_until(b"?>")?;
                continue;
            }
            if self.b[self.pos] == b'<' {
                self.flush_text(&mut text_bytes, content_start, &mut el)?;
                let child = self.parse_element()?;
                el.children.push(child);
                continue;
            }
            // accumulate one byte of text (entity decoding happens later)
            text_bytes.push(self.b[self.pos]);
            self.pos += 1;
        }
        el.text = decode_entities(&String::from_utf8_lossy(&text_bytes)).into_owned();
        Ok(el)
    }

    fn flush_text(
        &mut self,
        text_bytes: &mut Vec<u8>,
        _content_start: usize,
        el: &mut Element,
    ) -> Result<(), XmlError> {
        if !text_bytes.is_empty() {
            let chunk = std::mem::take(text_bytes);
            let decoded = decode_entities(&String::from_utf8_lossy(&chunk));
            el.text.push_str(&decoded);
        }
        Ok(())
    }

    fn read_name(&mut self) -> Result<(String, usize), XmlError> {
        let start = self.pos;
        while self.pos < self.b.len() {
            let c = self.b[self.pos];
            if c.is_ascii_alphanumeric() || c == b':' || c == b'_' || c == b'-' || c == b'.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return self.err("expected name");
        }
        Ok((String::from_utf8_lossy(&self.b[start..self.pos]).into_owned(), start))
    }

    fn parse_attr(&mut self) -> Result<Attr, XmlError> {
        let start = self.pos;
        let (raw_name, _) = self.read_name()?;
        self.skip_ws();
        if self.pos >= self.b.len() || self.b[self.pos] != b'=' {
            return self.err("expected '=' after attribute name");
        }
        self.pos += 1;
        self.skip_ws();
        if self.pos >= self.b.len() || (self.b[self.pos] != b'"' && self.b[self.pos] != b'\'') {
            return self.err("expected quoted attribute value");
        }
        let quote = self.b[self.pos];
        self.pos += 1;
        let val_start = self.pos;
        while self.pos < self.b.len() && self.b[self.pos] != quote {
            self.pos += 1;
        }
        if self.pos >= self.b.len() {
            return self.err("unterminated attribute value");
        }
        let val_end = self.pos;
        self.pos += 1; // closing quote
        let value = decode_entities(&String::from_utf8_lossy(&self.b[val_start..val_end]))
            .into_owned();
        let local = raw_name.split(':').last().unwrap_or(&raw_name).to_string();
        Ok(Attr {
            name: local,
            value,
            span: Span::new(start, val_end + 1),
            value_span: Span::new(val_start, val_end),
        })
    }
}

fn decode_entities(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.contains('&') {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with('&') {
            let semi = s[i..].find(';');
            if let Some(rel) = semi {
                let ent = &s[i + 1..i + rel];
                let decoded = match ent {
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "amp" => Some('&'),
                    "apos" => Some('\''),
                    "quot" => Some('"'),
                    _ if ent.starts_with('#') => decode_char_ref(ent),
                    _ => None,
                };
                if let Some(c) = decoded {
                    out.push(c);
                    i += rel + 1;
                    continue;
                }
            }
        }
        out.push_str(&s[i..i + 1]);
        i += 1;
    }
    std::borrow::Cow::Owned(out)
}

fn decode_char_ref(ent: &str) -> Option<char> {
    let num = ent.strip_prefix('#')?;
    let code: u32 = if let Some(hex) = num.strip_prefix(|c| c == 'x' || c == 'X') {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        num.parse().ok()?
    };
    char::from_u32(code)
}

pub fn parse_xml(source: &str) -> Result<XmlDoc, XmlError> {
    let mut p = Parser::new(source);
    p.parse_doc()
}
