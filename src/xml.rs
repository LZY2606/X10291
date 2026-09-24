//! 极简 XML 解析器：保留每个元素在源文本中的字节范围（span），
//! 使每个节点都能回溯到源 XML 范围。仅支持本工具所需的 XML 子集：
//! 元素、属性、文本、注释、CDATA、处理指令、自闭合标签、实体转义。

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlNode>,
    /// 整个元素（含标签）在源文本中的字节范围
    pub span: (usize, usize),
    /// 元素内容（开标签与闭标签之间）的字节范围
    pub inner_span: (usize, usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum XmlNode {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct XmlError {
    pub message: String,
    pub offset: usize,
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "XML 错误 @{}: {}", self.offset, self.message)
    }
}
impl std::error::Error for XmlError {}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            XmlNode::Element(e) => Some(e),
            _ => None,
        })
    }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.elements().filter(move |e| e.name == name)
    }
    pub fn first(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }
    pub fn text(&self) -> String {
        let mut out = String::new();
        for c in &self.children {
            if let XmlNode::Text(t) = c {
                out.push_str(t);
            }
        }
        out.trim().to_string()
    }
    pub fn raw<'a>(&self, src: &'a str) -> &'a str {
        &src[self.span.0..self.span.1]
    }
}

pub fn parse(src: &str) -> Result<Element, XmlError> {
    let b = src.as_bytes();
    let mut pos = 0usize;
    // 跳过序言（处理指令、注释、DOCTYPE）
    loop {
        skip_ws(b, &mut pos);
        if starts_with(b, pos, b"<?") {
            pos = find_from(b, pos + 2, b"?>").ok_or_else(|| err("未闭合的处理指令", pos))? + 2;
        } else if starts_with(b, pos, b"<!--") {
            pos = find_from(b, pos + 4, b"-->").ok_or_else(|| err("未闭合的注释", pos))? + 3;
        } else if starts_with(b, pos, b"<!DOCTYPE") || starts_with(b, pos, b"<!doctype") {
            pos = find_byte(b, pos, b'>').ok_or_else(|| err("未闭合的 DOCTYPE", pos))? + 1;
        } else {
            break;
        }
    }
    let root = parse_element(src, &mut pos)?;
    Ok(root)
}

fn err(msg: &str, offset: usize) -> XmlError {
    XmlError { message: msg.to_string(), offset }
}

fn starts_with(b: &[u8], pos: usize, pat: &[u8]) -> bool {
    pos + pat.len() <= b.len() && &b[pos..pos + pat.len()] == pat
}

fn find_from(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    (from..=b.len().saturating_sub(pat.len())).find(|&i| &b[i..i + pat.len()] == pat)
}

fn find_byte(b: &[u8], from: usize, byte: u8) -> Option<usize> {
    (from..b.len()).find(|&i| b[i] == byte)
}

fn skip_ws(b: &[u8], pos: &mut usize) {
    while *pos < b.len() && (b[*pos] as char).is_ascii_whitespace() {
        *pos += 1;
    }
}

fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.' || c == b':'
}

fn parse_name(src: &str, pos: &mut usize) -> Result<String, XmlError> {
    let b = src.as_bytes();
    let start = *pos;
    while *pos < b.len() && is_name_byte(b[*pos]) {
        *pos += 1;
    }
    if *pos == start {
        return Err(err("期望名称", *pos));
    }
    Ok(src[start..*pos].to_string())
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = match tail.find(';') {
            Some(e) => e,
            None => {
                out.push_str(tail);
                return out;
            }
        };
        let ent = &tail[1..end];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                if let Ok(n) = u32::from_str_radix(&ent[2..], 16) {
                    if let Some(c) = char::from_u32(n) {
                        out.push(c);
                    }
                }
            }
            _ if ent.starts_with('#') => {
                if let Ok(n) = ent[1..].parse::<u32>() {
                    if let Some(c) = char::from_u32(n) {
                        out.push(c);
                    }
                }
            }
            _ => {
                out.push('&');
                out.push_str(ent);
                out.push(';');
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn parse_element(src: &str, pos: &mut usize) -> Result<Element, XmlError> {
    let b = src.as_bytes();
    skip_ws(b, pos);
    if *pos >= b.len() || b[*pos] != b'<' {
        return Err(err("期望 '<'", *pos));
    }
    let elem_start = *pos;
    *pos += 1;
    let name = parse_name(src, pos)?;
    let mut attrs = Vec::new();
    loop {
        skip_ws(b, pos);
        if *pos >= b.len() {
            return Err(err("文件意外结束", *pos));
        }
        if starts_with(b, *pos, b"/>") {
            *pos += 2;
            return Ok(Element {
                name,
                attrs,
                children: Vec::new(),
                span: (elem_start, *pos),
                inner_span: (*pos, *pos),
            });
        }
        if b[*pos] == b'>' {
            *pos += 1;
            break;
        }
        let aname = parse_name(src, pos)?;
        skip_ws(b, pos);
        if *pos >= b.len() || b[*pos] != b'=' {
            return Err(err("属性缺少 '='", *pos));
        }
        *pos += 1;
        skip_ws(b, pos);
        if *pos >= b.len() || (b[*pos] != b'"' && b[*pos] != b'\'') {
            return Err(err("属性值缺少引号", *pos));
        }
        let quote = b[*pos];
        *pos += 1;
        let vstart = *pos;
        while *pos < b.len() && b[*pos] != quote {
            *pos += 1;
        }
        if *pos >= b.len() {
            return Err(err("属性值未闭合", *pos));
        }
        attrs.push((aname, decode_entities(&src[vstart..*pos])));
        *pos += 1;
    }
    let inner_start = *pos;
    let mut children = Vec::new();
    loop {
        if *pos >= b.len() {
            return Err(err(&format!("元素 <{}> 未闭合", name), *pos));
        }
        if starts_with(b, *pos, b"</") {
            let inner_end = *pos;
            *pos += 2;
            let close = parse_name(src, pos)?;
            if close != name {
                return Err(err(&format!("闭标签 </{}> 与 <{}> 不匹配", close, name), *pos));
            }
            skip_ws(b, pos);
            if *pos >= b.len() || b[*pos] != b'>' {
                return Err(err("闭标签缺少 '>'", *pos));
            }
            *pos += 1;
            return Ok(Element {
                name,
                attrs,
                children,
                span: (elem_start, *pos),
                inner_span: (inner_start, inner_end),
            });
        }
        if starts_with(b, *pos, b"<!--") {
            *pos = find_from(b, *pos + 4, b"-->").ok_or_else(|| err("未闭合的注释", *pos))? + 3;
            continue;
        }
        if starts_with(b, *pos, b"<![CDATA[") {
            let start = *pos + 9;
            let end = find_from(b, start, b"]]>").ok_or_else(|| err("未闭合的 CDATA", *pos))?;
            children.push(XmlNode::Text(src[start..end].to_string()));
            *pos = end + 3;
            continue;
        }
        if starts_with(b, *pos, b"<?") {
            *pos = find_from(b, *pos + 2, b"?>").ok_or_else(|| err("未闭合的处理指令", *pos))? + 2;
            continue;
        }
        if b[*pos] == b'<' {
            children.push(XmlNode::Element(parse_element(src, pos)?));
            continue;
        }
        // 文本
        let tstart = *pos;
        while *pos < b.len() && b[*pos] != b'<' {
            *pos += 1;
        }
        let raw = &src[tstart..*pos];
        if !raw.trim().is_empty() {
            children.push(XmlNode::Text(decode_entities(raw)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spans() {
        let src = "<A x=\"1\"><B>hi</B></A>";
        let root = parse(src).unwrap();
        assert_eq!(root.name, "A");
        assert_eq!(root.raw(src), src);
        let b = root.first("B").unwrap();
        assert_eq!(b.raw(src), "<B>hi</B>");
        assert_eq!(b.text(), "hi");
    }

    #[test]
    fn entities_and_cdata() {
        let src = "<A a=\"&amp;&lt;\"><![CDATA[x<y]]></A>";
        let root = parse(src).unwrap();
        assert_eq!(root.attr("a"), Some("&<"));
        assert_eq!(root.text(), "x<y");
    }
}
