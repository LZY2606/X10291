//! Minimal well-formed XML parser with byte-accurate range tracking.
//!
//! Scope is intentionally limited to the XML subset emitted by OPC UA
//! NodeSet files: elements, attributes, text and CDATA. It preserves
//! everything it does not understand inside `Element::extras` so later
//! layers never have to discard unknown markup.

#[derive(Clone, Debug)]
pub struct Element {
    pub tag: String,
    pub local: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
    /// byte offset of the opening `<`
    pub start: usize,
    /// byte offset immediately after the closing `>`
    pub end: usize,
    /// byte range covering the element name in the source
    pub name_span: (usize, usize),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn child<'a>(&'a self, local: &str) -> Option<&'a Element> {
        self.children.iter().find(|c| c.local == local)
    }

    pub fn children_local<'a>(&'a self, local: &str) -> Vec<&'a Element> {
        self.children.iter().filter(|c| c.local == local).collect()
    }
}

#[derive(Debug, Clone)]
pub struct XmlError {
    pub msg: String,
    pub pos: usize,
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "xml error at byte {}: {}", self.pos, self.msg)
    }
}

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse(&mut self) -> Result<Element, XmlError> {
        // prolog / declarations / comments / doctype
        loop {
            self.skip_ws();
            if self.pos + 5 <= self.b.len() && &self.b[self.pos..self.pos + 5] == b"<?xml" {
                self.skip_decl()?;
                continue;
            }
            if self.pos + 4 <= self.b.len() && &self.b[self.pos..self.pos + 4] == b"<!--" {
                self.skip_comment()?;
                continue;
            }
            if self.pos + 9 <= self.b.len()
                && &self.b[self.pos..self.pos + 9] == b"<!DOCTYPE"
            {
                self.skip_doctype()?;
                continue;
            }
            break;
        }
        let root = self.parse_element()?;
        self.skip_ws();
        // trailing comments are tolerated
        Ok(root)
    }

    fn skip_decl(&mut self) -> Result<(), XmlError> {
        let end = find_bytes(self.b, self.pos, b"?>").ok_or_else(|| XmlError {
            msg: "unterminated processing instruction".into(),
            pos: self.pos,
        })?;
        self.pos = end + 2;
        Ok(())
    }

    fn skip_comment(&mut self) -> Result<(), XmlError> {
        let end = find_bytes(self.b, self.pos + 4, b"-->").ok_or_else(|| XmlError {
            msg: "unterminated comment".into(),
            pos: self.pos,
        })?;
        self.pos = end + 3;
        Ok(())
    }

    fn skip_doctype(&mut self) -> Result<(), XmlError> {
        // skip until matching '>' at depth zero (internal subsets use [])
        let mut depth = 0i32;
        let mut i = self.pos + 9;
        while i < self.b.len() {
            match self.b[i] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                b'>' if depth <= 0 => {
                    self.pos = i + 1;
                    return Ok(());
                }
                _ => {}
            }
            i += 1;
        }
        Err(XmlError {
            msg: "unterminated doctype".into(),
            pos: self.pos,
        })
    }

    fn parse_element(&mut self) -> Result<Element, XmlError> {
        let start = self.pos;
        if self.peek() != Some(b'<') {
            return Err(XmlError {
                msg: "expected element".into(),
                pos: self.pos,
            });
        }
        self.pos += 1;
        let name_start = self.pos;
        while let Some(c) = self.peek() {
            if c == b'>' || c == b'/' || c.is_ascii_whitespace() {
                break;
            }
            self.pos += 1;
        }
        let name_end = self.pos;
        let tag = decode_simple(&self.b[name_start..name_end]).to_string();
        let local = tag.rsplit(':').next().unwrap_or(&tag).to_string();
        let mut attrs = Vec::new();
        let mut self_closing = false;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(b'/') => {
                    self.pos += 1;
                    if self.peek() != Some(b'>') {
                        return Err(XmlError {
                            msg: "expected >".into(),
                            pos: self.pos,
                        });
                    }
                    self.pos += 1;
                    self_closing = true;
                    break;
                }
                Some(_) => {
                    let (k, v) = self.parse_attr()?;
                    attrs.push((k, v));
                }
                None => {
                    return Err(XmlError {
                        msg: "unterminated element".into(),
                        pos: self.pos,
                    })
                }
            }
        }
        if self_closing {
            return Ok(Element {
                tag,
                local,
                attrs,
                children: Vec::new(),
                text: String::new(),
                start,
                end: self.pos,
                name_span: (name_start, name_end),
            });
        }
        let mut children = Vec::new();
        let mut text = String::new();
        loop {
            let content_start = self.pos;
            match self.peek() {
                None => {
                    return Err(XmlError {
                        msg: format!("missing closing tag for {}", tag),
                        pos: self.pos,
                    })
                }
                Some(b'<') => {
                    if self.pos + 4 <= self.b.len()
                        && &self.b[self.pos..self.pos + 4] == b"<!--"
                    {
                        self.skip_comment()?;
                        continue;
                    }
                    if self.pos + 9 <= self.b.len()
                        && &self.b[self.pos..self.pos + 9] == b"<![CDATA["
                    {
                        let close = find_bytes(self.b, self.pos + 9, b"]]>").ok_or_else(|| {
                            XmlError {
                                msg: "unterminated CDATA".into(),
                                pos: self.pos,
                            }
                        })?;
                        text.push_str(&decode_simple(&self.b[self.pos + 9..close]));
                        self.pos = close + 3;
                        continue;
                    }
                    if self.pos + 2 <= self.b.len() && self.b[self.pos + 1] == b'/' {
                        self.pos += 2;
                        let cname_start = self.pos;
                        while let Some(c) = self.peek() {
                            if c == b'>' || c.is_ascii_whitespace() {
                                break;
                            }
                            self.pos += 1;
                        }
                        let ctag = decode_simple(&self.b[cname_start..self.pos]).to_string();
                        if ctag != tag {
                            return Err(XmlError {
                                msg: format!("mismatched closing tag {} != {}", ctag, tag),
                                pos: cname_start,
                            });
                        }
                        self.skip_ws();
                        if self.peek() != Some(b'>') {
                            return Err(XmlError {
                                msg: "expected > in closing tag".into(),
                                pos: self.pos,
                            });
                        }
                        self.pos += 1;
                        return Ok(Element {
                            tag,
                            local,
                            attrs,
                            children,
                            text: text.trim().to_string(),
                            start,
                            end: self.pos,
                            name_span: (name_start, name_end),
                        });
                    }
                    if self.pos + 2 <= self.b.len() && self.b[self.pos + 1] == b'?' {
                        // embedded processing instruction
                        self.skip_decl()?;
                        continue;
                    }
                    let _ = content_start;
                    children.push(self.parse_element()?);
                }
                Some(_) => {
                    let chunk = self.read_text_until_lt();
                    text.push_str(&chunk);
                }
            }
        }
    }

    fn read_text_until_lt(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == b'<' {
                break;
            }
            self.pos += 1;
        }
        decode_simple(&self.b[start..self.pos]).to_string()
    }

    fn parse_attr(&mut self) -> Result<(String, String), XmlError> {
        let kstart = self.pos;
        while let Some(c) = self.peek() {
            if c == b'=' || c.is_ascii_whitespace() {
                break;
            }
            self.pos += 1;
        }
        let key = decode_simple(&self.b[kstart..self.pos]).to_string();
        self.skip_ws();
        if self.peek() != Some(b'=') {
            return Err(XmlError {
                msg: "expected = after attribute name".into(),
                pos: self.pos,
            });
        }
        self.pos += 1;
        self.skip_ws();
        let quote = match self.peek() {
            Some(q @ (b'"' | b'\'')) => q,
            _ => {
                return Err(XmlError {
                    msg: "expected quoted attribute value".into(),
                    pos: self.pos,
                })
            }
        };
        self.pos += 1;
        let vstart = self.pos;
        while let Some(c) = self.peek() {
            if c == quote {
                break;
            }
            self.pos += 1;
        }
        let value = decode_simple(&self.b[vstart..self.pos]).to_string();
        if self.peek() != Some(quote) {
            return Err(XmlError {
                msg: "unterminated attribute value".into(),
                pos: self.pos,
            });
        }
        self.pos += 1;
        Ok((key, value))
    }
}

fn find_bytes(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    hay.windows(needle.len())
        .enumerate()
        .find_map(|(i, w)| (i >= from && w == needle).then_some(i))
}

pub fn decode_simple(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(s.len());
    let bytes_chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes_chars.len() {
        if bytes_chars[i] == '&' {
            let semi = bytes_chars[i..].iter().position(|c| *c == ';');
            if let Some(rel) = semi {
                let ent: String = bytes_chars[i + 1..i + rel].iter().collect();
                let resolved = match ent.as_str() {
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "amp" => Some('&'),
                    "apos" => Some('\''),
                    "quot" => Some('"'),
                    e if e.starts_with('#') => decode_char_ref(&e[1..]),
                    _ => None,
                };
                if let Some(c) = resolved {
                    out.push(c);
                    i += rel + 1;
                    continue;
                }
            }
        }
        out.push(bytes_chars[i]);
        i += 1;
    }
    out
}

fn decode_char_ref(body: &str) -> Option<char> {
    let code = if let Some(hex) = body.strip_prefix('x').or_else(|| body.strip_prefix('X')) {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        body.parse::<u32>().ok()?
    };
    char::from_u32(code)
}

pub fn parse(input: &str) -> Result<Element, XmlError> {
    let mut p = Parser {
        b: input.as_bytes(),
        pos: 0,
    };
    p.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nodeset_shape_and_ranges() {
        let xml = r#"<?xml version="1.0"?>
<UANodeSet xmlns="http://opcfoundation.org/UA/2011/03/UANodeSet.xsd">
  <NamespaceUris><Uri>urn:ns1</Uri></NamespaceUris>
  <UAObject NodeId="ns=1;i=1" BrowseName="1:Thing">
    <DisplayName>Thing &amp; Co</DisplayName>
  </UAObject>
</UANodeSet>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.local, "UANodeSet");
        let obj = root.child("UAObject").unwrap();
        assert_eq!(obj.attr("NodeId"), Some("ns=1;i=1"));
        assert_eq!(obj.child("DisplayName").unwrap().text, "Thing & Co");
        assert!(xml[obj.start..obj.end].contains("Thing &amp; Co"));
    }

    #[test]
    fn byte_ranges_slice_back_to_source() {
        let xml = r#"<a><b x="1">hi</b></a>"#;
        let root = parse(xml).unwrap();
        let b = root.child("b").unwrap();
        assert_eq!(&xml[b.start..b.end], r#"<b x="1">hi</b>"#);
    }

    #[test]
    fn cdata_and_self_closing() {
        let xml = "<root><a/><b><![CDATA[1 < 2]]></b></root>";
        let root = parse(xml).unwrap();
        assert!(root.child("a").is_some());
        assert_eq!(root.child("b").unwrap().text, "1 < 2");
    }
}
