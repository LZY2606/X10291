//! NodeSet2 XML 解析器。
//!
//! 与“直接拿数据”的解析不同，这里每个 token 都带字节范围：
//! 派生界面上的每个值都能追回到源 XML 的具体区间；原始子树（含未识别扩展）
//! 逐字节保留。解析阶段不做 namespace 重映射、不补边，只忠实记录。

use crate::domain::*;
use quick_xml::events::Event;
use quickxml_compat::BytesStartExt;
use quick_xml::Reader;
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct Attr {
    key: String,
    value: String,
    value_start: usize,
    value_end: usize,
}

#[derive(Debug, Clone)]
enum Token {
    Start {
        name: String,
        attrs: Vec<Attr>,
        range: SrcRange,
        empty: bool,
    },
    End {
        name: String,
        pos: usize,
    },
    Text {
        text: String,
        start: usize,
        end: usize,
    },
}

mod quickxml_compat {
    use super::Attr;
    use quick_xml::events::BytesStart;

    pub trait BytesStartExt {
        fn attrs_with_pos(&self, tag_start: usize, content: &str) -> Vec<Attr>;
    }

    impl BytesStartExt for BytesStart<'_> {
        fn attrs_with_pos(&self, tag_start: usize, content: &str) -> Vec<Attr> {
            parse_attr_bytes(tag_start, content, self)
        }
    }

    fn parse_attr_bytes(
        tag_start: usize,
        content: &str,
        e: &quick_xml::events::BytesStart,
    ) -> Vec<Attr> {
        use crate::parser::decode_text;
        let name_end_est = tag_start + 1 + e.name().as_ref().len();
        let tag_end = find_tag_end(content, tag_start);
        let mut out = Vec::new();
        let mut i = name_end_est.min(tag_end);
        let bytes = content.as_bytes();
        while i < tag_end {
            while i < tag_end && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') {
                i += 1;
            }
            let key_start = i;
            while i < tag_end && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let key = decode_text(&content[key_start..i]);
            while i < tag_end && bytes[i] != b'=' {
                i += 1;
            }
            if i >= tag_end {
                break;
            }
            i += 1;
            while i < tag_end && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= tag_end {
                break;
            }
            let quote = bytes[i];
            if quote != b'"' && quote != b'\'' {
                continue;
            }
            let value_start = i + 1;
            i += 1;
            while i < tag_end && bytes[i] != quote {
                i += 1;
            }
            let value_end = i;
            out.push(Attr {
                key,
                value: decode_text(&content[value_start..value_end]),
                value_start,
                value_end,
            });
            if i < tag_end {
                i += 1;
            }
        }
        out
    }

    fn find_tag_end(content: &str, tag_start: usize) -> usize {
        let bytes = content.as_bytes();
        let mut i = tag_start + 1;
        let mut in_quote: Option<u8> = None;
        while i < bytes.len() {
            match (bytes[i], in_quote) {
                (q, None) if q == b'"' || q == b'\'' => in_quote = Some(q),
                (q, Some(cur)) if q == cur => in_quote = None,
                (b'>', None) => return i,
                _ => {}
            }
            i += 1;
        }
        content.len()
    }
}

pub(crate) fn decode_text(raw: &str) -> String {
    // quick-xml 的 unescape 只接受转义实体，原始属性/文本均为合法片段。
    quick_xml::escape::unescape(raw).map(|c| c.to_string()).unwrap_or_else(|_| raw.to_string())
}

struct Tokenizer<'a> {
    content: &'a str,
    file: String,
    tokens: Vec<Token>,
}

impl<'a> Tokenizer<'a> {
    fn range(&self, start: usize, end: usize) -> SrcRange {
        SrcRange::new(self.file.clone(), start, end)
    }

    fn run(&mut self) -> Result<(), String> {
        let mut reader = Reader::from_str(self.content);
        let mut buf = Vec::new();
        loop {
            let start = reader.buffer_position() as usize;
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let end = reader.buffer_position() as usize;
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let attrs = e.attrs_with_pos(start, self.content);
                    self.tokens.push(Token::Start {
                        name,
                        attrs,
                        range: self.range(start, end),
                        empty: false,
                    });
                }
                Ok(Event::Empty(e)) => {
                    let end = reader.buffer_position() as usize;
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let attrs = e.attrs_with_pos(start, self.content);
                    self.tokens.push(Token::Start {
                        name,
                        attrs,
                        range: self.range(start, end),
                        empty: true,
                    });
                }
                Ok(Event::End(e)) => {
                    let after = reader.buffer_position() as usize;
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let bytes = self.content.as_bytes();
                    let mut pos = after;
                    while pos > 0 && bytes[pos - 1] != b'<' {
                        pos -= 1;
                    }
                    self.tokens.push(Token::End { name, pos });
                }
                Ok(Event::Text(_e)) => {
                    let end = reader.buffer_position() as usize;
                    // 起始位置在 build_tree 的 text_span 中按相邻标签精确推算；
                    // 此处仅占位，避免按解码长度反推（实体场景会错位）。
                    self.tokens.push(Token::Text {
                        text: String::new(),
                        start: end,
                        end,
                    });
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("{} at byte {}", err, start)),
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Elem {
    name: String,
    attrs: Vec<Attr>,
    start: usize,
    end: usize,
    ns_uri: Option<String>,
    children: Vec<usize>,
    text: Option<(usize, usize, String)>,
}

impl Elem {
    fn attr(&self, key: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.key == key)
    }
    fn arange(&self, file: &str, a: &Attr) -> AttrRange {
        AttrRange {
            value: a.value.clone(),
            range: SrcRange::new(file, a.value_start, a.value_end),
        }
    }
}

struct Tree {
    elems: Vec<Elem>,
    root: Option<usize>,
}

fn build_tree(file: &str, content: &str, tokens: &[Token]) -> Tree {
    let mut elems: Vec<Elem> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut ns_stack: Vec<HashMap<String, String>> = Vec::new();
    let mut root = None;
    let mut token_i = 0;
    while token_i < tokens.len() {
        match &tokens[token_i] {
            Token::Start { name, attrs, range, empty } => {
                // 元素名按父作用域解析；自身 xmlns 声明只对子元素生效（DOM 语义）。
                let parent_scope = ns_stack.last().cloned().unwrap_or_default();
                let ns_uri = resolve_prefix(&parent_scope, name);
                let mut scope = parent_scope;
                for a in attrs {
                    if a.key == "xmlns" {
                        scope.insert(String::new(), a.value.clone());
                    } else if let Some(p) = a.key.strip_prefix("xmlns:") {
                        scope.insert(p.to_string(), a.value.clone());
                    }
                }
                let idx = elems.len();
                elems.push(Elem {
                    name: name.clone(),
                    attrs: attrs.clone(),
                    start: range.start,
                    end: range.end,
                    ns_uri,
                    children: Vec::new(),
                    text: None,
                });
                if let Some(&p) = stack.last() {
                    elems[p].children.push(idx);
                } else if root.is_none() {
                    root = Some(idx);
                }
                if !*empty {
                    ns_stack.push(scope);
                    stack.push(idx);
                }
            }
            Token::End { name, pos } => {
                // 正常文档下栈顶即匹配元素；若因空白 token 边界造成错位，
                // 按标签名向上找到真正的开元素，保证结构不串台。
                let depth_pos = stack.iter().rposition(|&si| elems[si].name == *name);
                if let Some(rel) = depth_pos {
                    while stack.len() > rel + 1 {
                        stack.pop();
                        ns_stack.pop();
                    }
                    if let Some(idx) = stack.pop() {
                        let mut endp = *pos;
                        while endp < content.len() && content.as_bytes()[endp] != b'>' {
                            endp += 1;
                        }
                        elems[idx].end = (endp + 1).min(content.len());
                        ns_stack.pop();
                    }
                }
            }
            Token::Text { .. } => {
                if let Some(&idx) = stack.last() {
                    let (s, e) = text_span(tokens, token_i, content);
                    if s <= e {
                        let raw = &content[s..e];
                        if !raw.trim().is_empty() {
                            let decoded = crate::parser::decode_text(raw.trim());
                            elems[idx].text = Some((s, e, decoded));
                        }
                    }
                }
            }
        }
        token_i += 1;
    }
    let _ = file;
    Tree { elems, root }
}

fn resolve_prefix(scope: &HashMap<String, String>, qualified: &str) -> Option<String> {
    if let Some(colon) = qualified.find(':') {
        scope.get(&qualified[..colon]).cloned()
    } else {
        scope.get("").cloned()
    }
}

/// 利用相邻标签位置，从原始字节中精确截取非空白文本范围。
fn text_span(tokens: &[Token], i: usize, content: &str) -> (usize, usize) {
    let prev_end = match tokens.get(i.wrapping_sub(1)) {
        Some(Token::Start { range, .. }) => range.end,
        Some(Token::End { pos, .. }) => *pos,
        _ => 0,
    };
    let next_start = match tokens.get(i + 1) {
        Some(Token::Start { range, .. }) => range.start,
        Some(Token::End { pos, .. }) => *pos,
        _ => content.len(),
    };
    // 在 [prev_end, next_start) 内取真正的文本字符；跳过空白与混入的尖括号片段。
    let seg = &content[prev_end..next_start];
    let start_off = seg.find(|c: char| !c.is_whitespace() && c != '<' && c != '>' && c != '/');
    let trimmed = seg.trim_matches(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '/');
    let start_off = start_off.unwrap_or(0);
    let real = trimmed;
    let abs_start = prev_end + start_off;
    let abs_end = abs_start + real.len();
    (abs_start, abs_end)
}

const UA_NS: &str = "http://opcfoundation.org/UA/";
const UA_NODESET_XSD: &str = "http://opcfoundation.org/UA/2011/03/UANodeSet.xsd";
const UA_XSD: &str = "http://www.w3.org/2001/XMLSchema";

fn is_standard_ns(u: &str) -> bool {
    u == UA_NS || u == UA_NODESET_XSD || u == UA_XSD
}
const STANDARD_CHILDREN: &[&str] = &[
    "DisplayName",
    "Description",
    "Category",
    "References",
    "Definition",
    "Value",
    "DataType",
    "InverseName",
    "Documentation",
];

pub fn parse_nodeset(file: &str, content: &str) -> Result<RawNodeSet, String> {
    let mut tk = Tokenizer {
        content,
        file: file.to_string(),
        tokens: Vec::new(),
    };
    tk.run()?;
    let tree = build_tree(file, content, &tk.tokens);
    let root = tree
        .root
        .ok_or_else(|| format!("{file}: 空文档，缺少 UANodeSet 根元素"))?;
    let root_elem = &tree.elems[root];
    if root_elem.name != "UANodeSet" {
        return Err(format!("{file}: 根元素应为 UANodeSet，实际为 {}", root_elem.name));
    }

    let mut out = RawNodeSet {
        file: file.to_string(),
        content: content.to_string(),
        namespaces: Vec::new(),
        self_models: Vec::new(),
        required_models: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        file_extensions: Vec::new(),
        misc: Vec::new(),
    };

    let root_ns = root_elem.ns_uri.clone();
    for &ci in &root_elem.children {
        let child = &tree.elems[ci];
        match child.name.as_str() {
            "NamespaceUris" => {
                for (di, &ui) in child.children.iter().enumerate() {
                    let uri_elem = &tree.elems[ui];
                    if let Some((s, e, text)) = &uri_elem.text {
                        out.namespaces.push(NamespaceUri {
                            decl_index: di,
                            uri: AttrRange {
                                value: text.clone(),
                                range: SrcRange::new(file, *s, *e),
                            },
                        });
                    }
                }
            }
            "Models" => {
                for &mi in &child.children {
                    let m = &tree.elems[mi];
                    let req = model_requirement(file, m, &tree);
                    if m.name == "RequiredModel" {
                        out.required_models.push(req);
                    } else {
                        out.self_models.push(req);
                    }
                }
            }
            "Aliases" => {
                for &ai in &child.children {
                    let a = &tree.elems[ai];
                    if a.name != "Alias" {
                        continue;
                    }
                    let name_attr = a
                        .attr("Alias")
                        .ok_or_else(|| format!("{file}: Alias 缺少 Alias 属性"))?;
                    if let Some((s, e, text)) = &a.text {
                        out.aliases.push(AliasEntry {
                            name: a.arange(file, name_attr),
                            target: AttrRange {
                                value: text.clone(),
                                range: SrcRange::new(file, *s, *e),
                            },
                            range: SrcRange::new(file, a.start, a.end),
                        });
                    }
                }
            }
            tag if NodeKind::from_tag(tag).is_some() => {
                out.nodes.push(parse_node(file, ci, &tree, content, root_ns.as_deref()));
            }
            other => {
                // 顶层未知内容：既保留原文，也登记其它简单文本。
                let xml = content[child.start..child.end].to_string();
                let recognized = child.ns_uri.as_deref().is_some_and(is_standard_ns);
                out.file_extensions.push(RawExtension {
                    name: other.to_string(),
                    ns_uri: child.ns_uri.clone(),
                    recognized,
                    xml,
                    range: SrcRange::new(file, child.start, child.end),
                });
            }
        }
    }
    Ok(out)
}

fn model_requirement(file: &str, m: &Elem, _tree: &Tree) -> ModelRequirement {
    let take = |key: &str| -> Option<AttrRange> {
        m.attr(key).map(|a| AttrRange {
            value: a.value.clone(),
            range: SrcRange::new(file, a.value_start, a.value_end),
        })
    };
    ModelRequirement {
        model_uri: take("ModelUri").expect("Models entry has ModelUri"),
        version: take("Version"),
        publication_date: take("PublicationDate"),
        range: SrcRange::new(file, m.start, m.end),
    }
}

fn parse_node(file: &str, idx: usize, tree: &Tree, content: &str, root_ns: Option<&str>) -> RawNode {
    let e = &tree.elems[idx];
    let kind = NodeKind::from_tag(&e.name).expect("node tag validated");
    let ar = |key: &str| -> Option<AttrRange> {
        e.attr(key).map(|a| AttrRange {
            value: a.value.clone(),
            range: SrcRange::new(file, a.value_start, a.value_end),
        })
    };
    let mut references = Vec::new();
    let mut extensions = Vec::new();
    let mut display_name = ar("DisplayName");
    let mut data_type = ar("DataType");

    for &ci in &e.children {
        let c = &tree.elems[ci];
        match c.name.as_str() {
            "DisplayName" if display_name.is_none() => {
                if let Some((s, end, text)) = &c.text {
                    display_name = Some(AttrRange {
                        value: text.clone(),
                        range: SrcRange::new(file, *s, *end),
                    });
                }
            }
            "DataType" if data_type.is_none() => {
                if let Some((s, end, text)) = &c.text {
                    data_type = Some(AttrRange {
                        value: text.clone(),
                        range: SrcRange::new(file, *s, *end),
                    });
                }
            }
            "References" => {
                for &ri in &c.children {
                    let r = &tree.elems[ri];
                    if let Some((s, end, text)) = &r.text {
                        if text.trim().is_empty() {
                            continue;
                        }
                        let forward = r
                            .attr("IsForward")
                            .map(|a| a.value != "false")
                            .unwrap_or(true);
                        references.push(RawReference {
                            text: AttrRange {
                                value: text.clone(),
                                range: SrcRange::new(file, *s, *end),
                            },
                            reftype: r.arange(
                                file,
                                r.attr("ReferenceType")
                                    .expect("Reference has ReferenceType"),
                            ),
                            forward,
                            range: SrcRange::new(file, r.start, r.end),
                        });
                    }
                }
            }
            other => {
                if STANDARD_CHILDREN.contains(&other) {
                    continue;
                }
                let xml = (
                    content[tree.elems[ci].start..tree.elems[ci].end].to_string(),
                    SrcRange::new(file, tree.elems[ci].start, tree.elems[ci].end),
                );
                let recognized = c
                    .ns_uri
                    .as_deref()
                    .map(|u| is_standard_ns(u) || Some(u) == root_ns)
                    .unwrap_or(true);
                extensions.push(RawExtension {
                    name: other.to_string(),
                    ns_uri: c.ns_uri.clone(),
                    recognized,
                    xml: xml.0,
                    range: xml.1,
                });
            }
        }
    }

    RawNode {
        kind,
        node_id: ar("NodeId").expect("node has NodeId"),
        browse_name: ar("BrowseName").expect("node has BrowseName"),
        display_name,
        data_type,
        references,
        extensions,
        range: SrcRange::new(file, e.start, e.end),
    }
}

/// 读取并解析 NodeSet 文件（文件内容同时原样保存，供界面按范围回读）。
pub fn parse_file(path: &std::path::Path) -> Result<RawNodeSet, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    parse_nodeset(&name, &content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<UANodeSet xmlns="http://opcfoundation.org/UA/2011/03/UANodeSet.xsd">
  <NamespaceUris>
    <Uri>urn:example:plant</Uri>
  </NamespaceUris>
  <Models>
    <Model ModelUri="urn:example:plant" Version="1.0"/>
    <RequiredModel ModelUri="urn:example:devices" Version="2.0"/>
  </Models>
  <Aliases>
    <Alias Alias="Organizes">i=35</Alias>
  </Aliases>
  <UAObject NodeId="ns=1;s=Plant" BrowseName="1:Plant">
    <DisplayName>Plant</DisplayName>
    <References>
      <Reference ReferenceType="i=35">ns=1;s=Pump1</Reference>
      <Reference ReferenceType="i=35" IsForward="false">ns=1;s=Pump2</Reference>
    </References>
  </UAObject>
  <UAVariable NodeId="ns=1;s=Pump1" BrowseName="1:Pump1">
    <psx:EngineeringUnits xmlns:psx="urn:vendor:x">bar</psx:EngineeringUnits>
  </UAVariable>
</UANodeSet>
"#;

    #[test]
    fn preserves_raw_structure_with_ranges() {
        let ns = parse_nodeset("sample.xml", SAMPLE).unwrap();
        assert_eq!(ns.namespaces.len(), 1);
        assert_eq!(ns.self_models.len(), 1);
        assert_eq!(ns.required_models.len(), 1);
        assert_eq!(ns.aliases.len(), 1);
        assert_eq!(ns.nodes.len(), 2);
        let r = &ns.namespaces[0].uri.range;
        assert_eq!(&SAMPLE[r.start..r.end], "urn:example:plant");
        let plant = &ns.nodes[0];
        assert_eq!(plant.references.len(), 2);
        assert!(plant.references[1].forward == false);
        let attr_r = &plant.node_id.range;
        assert_eq!(&SAMPLE[attr_r.start..attr_r.end], "ns=1;s=Plant");
        let var = &ns.nodes[1];
        assert_eq!(var.extensions.len(), 1);
        let ext = &var.extensions[0];
        assert_eq!(ext.ns_uri.as_deref(), Some("urn:vendor:x"));
        assert!(ext.xml.contains("bar"));
    }
}

#[cfg(test)]
mod dbg2 {
    use super::*;
    #[test]
    fn print_tokens() {
        let xml = "<UANodeSet><NamespaceUris>\n  <Uri>urn:example:plant</Uri>\n</NamespaceUris></UANodeSet>";
        let mut tk = Tokenizer { content: xml, file: "x".into(), tokens: vec![] };
        tk.run().unwrap();
        for (i, t) in tk.tokens.iter().enumerate() {
            match t {
                Token::Start { name, range, empty, .. } => {
                    eprintln!("{i} START {name} {}..{} empty={empty}", range.start, range.end);
                }
                Token::End { name, pos } => eprintln!("{i} END {name} @{pos}"),
                Token::Text { start, end, .. } => eprintln!("{i} TEXT {start}..{end}"),
            }
        }
    }
}
