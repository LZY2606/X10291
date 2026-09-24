//! Minimal span-aware XML parser.
//!
//! We intentionally do not use a generic XML tree: the review UI must be able
//! to reopen the exact source range that produced every value, so the parser
//! tracks byte offsets and 1-based line/column for every element.

use super::model::{ParseDiagnostic, SourceRange};

#[derive(Debug, Clone)]
pub struct XmlAttr {
    pub name: String,
    pub value: String,
    pub value_start: usize,
}

#[derive(Debug, Clone)]
pub struct XmlElement {
    /// Local name, namespaces handled by the NodeSet mapping layer.
    pub tag: String,
    /// Original qualified tag as written (`uax:Unit`).
    pub qualified_tag: String,
    pub attrs: Vec<XmlAttr>,
    pub children: Vec<XmlElement>,
    /// Concatenated text content (entities decoded).
    pub text: String,
    pub start_byte: usize,
    /// Byte just after the closing `>` (or self-closing tag end).
    pub end_byte: usize,
    pub start_line: usize,
    pub start_col: usize,
    pub self_closing: bool,
}

impl XmlElement {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_str())
    }
    pub fn child(&self, tag: &str) -> Option<&XmlElement> {
        self.children.iter().find(|c| c.tag == tag)
    }
    pub fn children_tag<'a>(&'a self, tag: &str) -> Vec<&'a XmlElement> {
        self.children.iter().filter(|c| c.tag == tag).collect()
    }
    pub fn range(&self, file_id: &str) -> SourceRange {
        SourceRange {
            file_id: file_id.to_string(),
            start_byte: self.start_byte,
            end_byte: self.end_byte,
            start_line: self.start_line,
            start_col: self.start_col,
            end_line: self.start_line,
        }
    }
}

pub struct ParsedXml {
    pub roots: Vec<XmlElement>,
    pub diagnostics: Vec<ParseDiagnostic>,
}

struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
    file_id: String,
    diagnostics: Vec<ParseDiagnostic>,
}

impl<'a> Cursor<'a> {
    fn new(b: &'a [u8], file_id: &str) -> Self {
        Self { b, pos: 0, line: 1, col: 1, file_id: file_id.to_string(), diagnostics: vec![] }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }
    fn at(&self, s: &str) -> bool {
        self.b[self.pos..].starts_with(s.as_bytes())
    }
    fn bump(&mut self) -> Option<u8> {
        let c = self.b.get(self.pos).copied()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }
    fn diag(&mut self, code: &str, msg: String, start: usize, line: usize, col: usize) {
        self.diagnostics.push(ParseDiagnostic {
            code: code.to_string(),
            message: msg,
            range: SourceRange {
                file_id: self.file_id.clone(),
                start_byte: start,
                end_byte: self.pos,
                start_line: line,
                start_col: col,
                end_line: self.line,
            },
        });
    }

    fn skip_ws_and_prolog(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.bump();
                }
                Some(b'<') if self.at("<!--") => self.skip_comment(),
                Some(b'<') if self.at("<?") => self.skip_until("?>"),
                Some(b'<') if self.at("<!") => self.skip_until(">"),
                _ => break,
            }
        }
    }

    fn skip_comment(&mut self) {
        self.skip_until("-->");
    }

    fn skip_until(&mut self, marker: &str) {
        let m = marker.as_bytes();
        while self.pos < self.b.len() {
            if self.b[self.pos..].starts_with(m) {
                for _ in 0..m.len() {
                    self.bump();
                }
                return;
            }
            self.bump();
        }
    }

    fn parse_element(&mut self) -> Option<XmlElement> {
        self.skip_ws_and_prolog();
        if self.peek() != Some(b'<') {
            return None;
        }
        let start = self.pos;
        let line = self.line;
        let col = self.col;
        self.bump(); // <
        let qualified_tag = self.read_name()?;
        let tag = qualified_tag.split(':').next_back().unwrap_or(&qualified_tag).to_string();
        let mut attrs = Vec::new();
        loop {
            self.skip_ws_no_prolog();
            match self.peek() {
                Some(b'>') => {
                    self.bump();
                    break;
                }
                Some(b'/') if self.at("/>") => {
                    self.bump();
                    self.bump();
                    return Some(XmlElement {
                        tag,
                        qualified_tag,
                        attrs,
                        children: vec![],
                        text: String::new(),
                        start_byte: start,
                        end_byte: self.pos,
                        start_line: line,
                        start_col: col,
                        self_closing: true,
                    });
                }
                Some(c) if c.is_ascii_alphabetic() || c == b'_' || c == b':' => {
                    if let Some(a) = self.read_attr() {
                        attrs.push(a);
                    }
                }
                _ => {
                    self.diag("XML_ATTR", "expected attribute or tag end".into(), start, line, col);
                    // recover: advance to next '>'
                    while let Some(c) = self.peek() {
                        if c == b'>' {
                            self.bump();
                            break;
                        }
                        self.bump();
                    }
                    break;
                }
            }
        }
        let (children, text) = self.parse_content(&tag, start, line, col)?;
        Some(XmlElement {
            tag,
            qualified_tag,
            attrs,
            children,
            text,
            start_byte: start,
            end_byte: self.pos,
            start_line: line,
            start_col: col,
            self_closing: false,
        })
    }

    fn skip_ws_no_prolog(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn read_name(&mut self) -> Option<String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'-' | b'.') {
                self.bump();
            } else {
                break;
            }
        }
        if start == self.pos {
            None
        } else {
            Some(String::from_utf8_lossy(&self.b[start..self.pos]).to_string())
        }
    }

    fn read_attr(&mut self) -> Option<XmlAttr> {
        let name = self.read_name()?;
        self.skip_ws_no_prolog();
        if self.peek() != Some(b'=') {
            return Some(XmlAttr { name, value: String::new(), value_start: self.pos });
        }
        self.bump();
        self.skip_ws_no_prolog();
        let quote = self.peek()?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        self.bump();
        let value_start = self.pos;
        let mut raw = String::new();
        while let Some(c) = self.peek() {
            if c == quote {
                self.bump();
                break;
            }
            if c == b'&' {
                raw.push_str(&self.read_entity());
            } else {
                raw.push(c as char);
                self.bump();
            }
        }
        Some(XmlAttr { name, value: raw, value_start })
    }

    fn read_entity(&mut self) -> String {
        self.bump(); // &
        let mut raw = String::new();
        while let Some(c) = self.peek() {
            if c == b';' {
                self.bump();
                break;
            }
            raw.push(c as char);
            self.bump();
        }
        decode_entity(&raw)
    }

    fn parse_content(
        &mut self,
        parent_tag: &str,
        start: usize,
        line: usize,
        col: usize,
    ) -> Option<(Vec<XmlElement>, String)> {
        let mut children = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek() {
                None => {
                    self.diag(
                        "XML_EOF",
                        format!("unclosed element <{parent_tag}>"),
                        start,
                        line,
                        col,
                    );
                    return Some((children, text));
                }
                Some(b'<') if self.at("</") => {
                    self.bump();
                    self.bump();
                    let close = self.read_name().unwrap_or_default();
                    let close_local: String =
                        close.split(':').next_back().unwrap_or(&close).to_string();
                    self.skip_ws_no_prolog();
                    if self.peek() == Some(b'>') {
                        self.bump();
                    }
                    if close_local != parent_tag {
                        self.diag(
                            "XML_MISMATCH",
                            format!("expected </{parent_tag}> found </{close}>"),
                            start,
                            line,
                            col,
                        );
                    }
                    return Some((children, text));
                }
                Some(b'<') if self.at("<!--") => self.skip_comment(),
                Some(b'<') if self.at("<?") => self.skip_until("?>"),
                Some(b'<') if self.at("<!") => {
                    // CDATA or declaration
                    if self.at("<![CDATA[") {
                        for _ in 0.."<![CDATA[".len() {
                            self.bump();
                        }
                        let cdata_start = self.pos;
                        self.skip_until("]]>");
                        let end = self.pos.saturating_sub(3);
                        text.push_str(&String::from_utf8_lossy(&self.b[cdata_start..end]));
                    } else {
                        self.skip_until(">");
                    }
                }
                Some(b'<') => {
                    if let Some(child) = self.parse_element() {
                        children.push(child);
                    } else {
                        return None;
                    }
                }
                Some(b'&') => text.push_str(&self.read_entity()),
                Some(_) => {
                    text.push(self.peek().unwrap() as char);
                    self.bump();
                }
            }
        }
    }
}

fn decode_entity(raw: &str) -> String {
    match raw {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        n if let Some(rest) = n.strip_prefix('#') => {
            let code = if let Some(hex) = rest.strip_prefix('x') {
                u32::from_str_radix(hex, 16).ok()
            } else {
                rest.parse::<u32>().ok()
            };
            code.and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_default()
        }
        other => format!("&{other};"),
    }
}

/// Parse a document, returning every top-level element (normally one
/// `<UANodeSet>`) plus best-effort diagnostics.
pub fn parse_xml(bytes: &[u8], file_id: &str) -> ParsedXml {
    let mut cur = Cursor::new(bytes, file_id);
    let mut roots = Vec::new();
    loop {
        cur.skip_ws_and_prolog();
        if cur.peek().is_none() {
            break;
        }
        match cur.parse_element() {
            Some(e) => roots.push(e),
            None => {
                cur.diag("XML_ROOT", "expected element".into(), cur.pos, cur.line, cur.col);
                break;
            }
        }
    }
    ParsedXml { roots, diagnostics: cur.diagnostics }
}
