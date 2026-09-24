use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum XmlError {
    #[error("XML 解析失败: {0}")]
    Parse(String),
    #[error("XML 结构不平衡")]
    Unbalanced,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct XmlAttr {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct XmlEl {
    pub name: String,
    pub attrs: Vec<XmlAttr>,
    pub text: String,
    pub children: Vec<XmlEl>,
    pub span: (usize, usize),
}

impl XmlEl {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }
    pub fn child(&self, name: &str) -> Option<&XmlEl> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a XmlEl> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    pub fn local_name(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }
}

pub fn parse(xml: &str) -> Result<XmlEl, XmlError> {
    let mut reader = Reader::from_str(xml);
    let mut root = XmlEl {
        name: "#document".to_string(),
        attrs: vec![],
        text: String::new(),
        children: vec![],
        span: (0, xml.len()),
    };
    let mut stack: Vec<XmlEl> = Vec::new();
    loop {
        let pos = reader.buffer_position();
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let el = XmlEl {
                    name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    attrs: read_attrs(&reader, &e)?,
                    text: String::new(),
                    children: vec![],
                    span: (pos, pos),
                };
                stack.push(el);
            }
            Ok(Event::Empty(e)) => {
                let el = XmlEl {
                    name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    attrs: read_attrs(&reader, &e)?,
                    text: String::new(),
                    children: vec![],
                    span: (pos, reader.buffer_position()),
                };
                push_child(&mut stack, &mut root, el);
            }
            Ok(Event::End(_)) => {
                let mut el = stack.pop().ok_or(XmlError::Unbalanced)?;
                el.span.1 = reader.buffer_position();
                push_child(&mut stack, &mut root, el);
            }
            Ok(Event::Text(e)) => {
                let t = e
                    .decode()
                    .map_err(|err| XmlError::Parse(err.to_string()))?;
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                } else {
                    root.text.push_str(&t);
                }
            }
            Ok(Event::CData(e)) => {
                let t = String::from_utf8_lossy(e.as_ref()).into_owned();
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(err) => return Err(XmlError::Parse(format!("{} @{}", err, pos))),
        }
    }
    if !stack.is_empty() {
        return Err(XmlError::Unbalanced);
    }
    Ok(root)
}

fn push_child(stack: &mut Vec<XmlEl>, root: &mut XmlEl, el: XmlEl) {
    if let Some(top) = stack.last_mut() {
        top.children.push(el);
    } else {
        root.children.push(el);
    }
}

fn read_attrs(
    reader: &Reader<&[u8]>,
    e: &quick_xml::events::BytesStart,
) -> Result<Vec<XmlAttr>, XmlError> {
    let mut out = vec![];
    for a in e.attributes().with_checks(false) {
        let a = a.map_err(|err| XmlError::Parse(err.to_string()))?;
        let val = a
            .decode_and_unescape_value(reader.decoder())
            .map_err(|err| XmlError::Parse(err.to_string()))?
            .into_owned();
        out.push(XmlAttr {
            name: String::from_utf8_lossy(a.key.as_ref()).into_owned(),
            value: val,
        });
    }
    Ok(out)
}
