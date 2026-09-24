//! Minimal preserving XML parser: generic element tree with byte-offset spans
//! so every derived value can point back at its source XML range.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub value: String,
    /// span of the attribute name
    pub span: (usize, usize),
    /// span of the attribute value excluding quotes
    pub val_span: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Element,
    Text,
    CData,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    /// element name for Element nodes; empty for text/cdata
    pub name: String,
    pub text: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
    /// [start, end) byte range covering the whole element, or text payload
    pub span: (usize, usize),
    /// span covering only the opening tag
    pub open_span: (usize, usize),
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_str())
    }
    pub fn attr_span(&self, name: &str) -> Option<(usize, usize)> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.val_span)
    }
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn children_named(&self, name: &str) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(move |c| c.name == name)
    }
    pub fn text_trimmed(&self) -> String {
        let mut out = String::new();
        for c in &self.children {
            if matches!(c.kind, NodeKind::Text | NodeKind::CData) {
                out.push_str(&c.text);
            }
        }
        out.trim().to_string()
    }
}

pub fn parse(input: &[u8]) -> Result<Node, String> {
    let s = std::str::from_utf8(input).map_err(|e| format!("non-utf8 xml: {e}"))?;
    let mut p = Parser { s, i: 0 };
    p.skip_prolog()?;
    p.skip_misc()?;
    let root = p.parse_element()?;
    p.skip_misc()?;
    if p.i != p.s.len() {
        return Err(format!("trailing data at byte {}", p.i));
    }
    Ok(root)
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self, off: usize) -> Option<char> {
        self.s[self.i..].chars().nth(off)
    }
    fn starts(&self, lit: &str) -> bool {
        self.s[self.i..].starts_with(lit)
    }
    fn err<T>(&self, msg: &str) -> Result<T, String> {
        Err(format!("xml parse error at byte {}: {msg}", self.i))
    }

    fn skip_prolog(&mut self) -> Result<(), String> {
        if self.starts("<?xml") {
            let end = self.s[self.i..]
                .find("?>")
                .ok_or("unclosed xml declaration")?;
            self.i += end + 2;
        }
        Ok(())
    }

    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            while self.peek(0).is_some_and(|c| c.is_whitespace()) {
                self.i += self.peek(0).unwrap().len_utf8();
            }
            if self.starts("<!--") {
                let end = self.s[self.i + 4..]
                    .find("-->")
                    .ok_or("unclosed comment")?;
                self.i += 4 + end + 3;
            } else if self.starts("<?") {
                let end = self.s[self.i + 2..].find("?>").ok_or("unclosed pi")?;
                self.i += 2 + end + 2;
            } else if self.starts("<!DOCTYPE") {
                let end = self.s[self.i + 9..].find('>').ok_or("unclosed doctype")?;
                self.i += 9 + end + 1;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_name(&mut self) -> Result<String, String> {
        let start = self.i;
        while let Some(c) = self.peek(0) {
            if c.is_alphanumeric() || matches!(c, ':' | '_' | '-' | '.' | 0xB7) {
                self.i += c.len_utf8();
            } else {
                break;
            }
        }
        if start == self.i {
            return self.err("expected name");
        }
        Ok(self.s[start..self.i].to_string())
    }

    fn parse_element(&mut self) -> Result<Node, String> {
        let start = self.i;
        if self.peek(0) != Some('<') {
            return self.err("expected '<'");
        }
        self.i += 1;
        let name = self.parse_name()?;
        let mut attrs = Vec::new();
        loop {
            loop {
                if let Some(c) = self.peek(0) {
                    if c.is_whitespace() {
                        self.i += c.len_utf8();
                        continue;
                    }
                }
                break;
            }
            if self.peek(0) == Some('/') {
                self.i += 1;
                if self.peek(0) != Some('>') {
                    return self.err("expected '>'");
                }
                self.i += 1;
                return Ok(Node {
                    kind: NodeKind::Element,
                    name,
                    text: String::new(),
                    attrs,
                    children: Vec::new(),
                    span: (start, self.i),
                    open_span: (start, self.i),
                });
            }
            if self.peek(0) == Some('>') {
                self.i += 1;
                break;
            }
            let aname_span_start = self.i;
            let aname = self.parse_name()?;
            let aname_span_end = self.i;
            loop {
                if let Some(c) = self.peek(0) {
                    if c.is_whitespace() {
                        self.i += c.len_utf8();
                    } else {
                        break;
                    }
                }
            }
            if self.peek(0) != Some('=') {
                return self.err("expected '=' in attribute");
            }
            self.i += 1;
            loop {
                if let Some(c) = self.peek(0) {
                    if c.is_whitespace() {
                        self.i += c.len_utf8();
                    } else {
                        break;
                    }
                }
            }
            let quote = match self.peek(0) {
                Some('"') | Some('\'') => self.peek(0).unwrap(),
                _ => return self.err("expected quoted attribute value"),
            };
            self.i += 1;
            let val_start = self.i;
            let val = self.parse_attr_value(quote)?;
            let val_end = self.i - 1;
            attrs.push(Attr {
                name: aname,
                value: val,
                span: (aname_span_start, aname_span_end),
                val_span: (val_start, val_end),
            });
        }
        let open_end = self.i;
        let mut children = Vec::new();
        loop {
            if self.starts("</") {
                self.i += 2;
                let close = self.parse_name()?;
                if close != name {
                    return self.err(&format!("mismatched closing tag </{close}> for <{name}>"));
                }
                loop {
                    if let Some(c) = self.peek(0) {
                        if c.is_whitespace() {
                            self.i += c.len_utf8();
                            continue;
                        }
                    }
                    break;
                }
                if self.peek(0) != Some('>') {
                    return self.err("expected '>' to close element");
                }
                self.i += 1;
                return Ok(Node {
                    kind: NodeKind::Element,
                    name,
                    text: String::new(),
                    attrs,
                    children,
                    span: (start, self.i),
                    open_span: (start, open_end),
                });
            }
            if self.starts("<!--") {
                let cstart = self.i;
                let end = self.s[self.i + 4..].find("-->").ok_or("unclosed comment")?;
                self.i += 4 + end + 3;
                children.push(Node {
                    kind: NodeKind::Text,
                    name: String::new(),
                    text: String::new(),
                    attrs: Vec::new(),
                    children: Vec::new(),
                    span: (cstart, self.i),
                    open_span: (cstart, self.i),
                });
            } else if self.starts("<![CDATA[") {
                let cstart = self.i;
                let payload = self.i + 9;
                let end = self.s[payload..].find("]]>").ok_or("unclosed cdata")?;
                let text = self.s[payload..payload + end].to_string();
                self.i = payload + end + 3;
                children.push(Node {
                    kind: NodeKind::CData,
                    name: String::new(),
                    text,
                    attrs: Vec::new(),
                    children: Vec::new(),
                    span: (cstart, self.i),
                    open_span: (cstart, self.i),
                });
            } else if self.peek(0) == Some('<') {
                children.push(self.parse_element()?);
            } else {
                let tstart = self.i;
                while let Some(c) = self.peek(0) {
                    if c == '<' {
                        break;
                    }
                    self.i += c.len_utf8();
                }
                let raw = &self.s[tstart..self.i];
                let text = decode_text(raw);
                if !text.chars().all(char::is_whitespace) {
                    children.push(Node {
                        kind: NodeKind::Text,
                        name: String::new(),
                        text,
                        attrs: Vec::new(),
                        children: Vec::new(),
                        span: (tstart, self.i),
                        open_span: (tstart, self.i),
                    });
                }
            }
        }
    }

    fn parse_attr_value(&mut self, quote: char) -> Result<String, String> {
        let _ = quote;
        let start = self.i;
        while let Some(c) = self.peek(0) {
            if c == quote {
                let raw = self.s[start..self.i].to_string();
                self.i += 1;
                return Ok(decode_text(&raw));
            }
            if c == '<' {
                return self.err("'<' inside attribute value");
            }
            self.i += c.len_utf8();
        }
        self.err("unterminated attribute value")
    }
}

fn decode_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw;
    let mut rest = bytes;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let semi = match after.find(';') {
            Some(i) => i,
            None => {
                out.push('&');
                rest = after;
                continue;
            }
        };
        let ent = &after[..semi];
        match ent {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "apos" => out.push('\''),
            "quot" => out.push('"'),
            _ if ent.starts_with('#') => {
                let code = if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X"))
                {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    ent[1..].parse::<u32>().ok()
                };
                if let Some(c) = code.and_then(char::from_u32) {
                    out.push(c);
                } else {
                    out.push_str(&rest[amp..amp + 1 + semi + 1]);
                }
            }
            _ => out.push_str(&rest[amp..amp + 1 + semi + 1]),
        }
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_spans_and_entities() {
        let xml = br#"<root a="x &lt; y"><!-- c --><child z='1'>hi &amp; bye</child></root>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.name, "root");
        assert_eq!(root.attr("a"), Some("x < y"));
        let child = root.child("child").unwrap();
        assert_eq!(child.attr("z"), Some("1"));
        assert_eq!(child.text_trimmed(), "hi & bye");
        assert_eq!(&xml[child.span.0..child.span.1], b"<child z='1'>hi &amp; bye</child>");
    }

    #[test]
    fn self_closing_and_cdata() {
        let xml = br#"<r><a/><![CDATA[1<2]]></r>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[1].text, "1<2");
    }
}
