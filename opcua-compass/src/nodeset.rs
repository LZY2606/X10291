//! Raw extraction from an OPC UA NodeSet XML file.
//!
//! Nothing here is merged or normalized. The goal is a faithful, span-backed
//! baseline: original XML nodes, namespace table, NodeId, BrowseName,
//! references, aliases, model dependencies and unrecognized extensions are
//! all retained at their source byte ranges.

use crate::xml::{parse_root, DomElement, Span, XmlError};
use serde::{Deserialize, Serialize};

pub const NODE_TAGS: &[&str] = &[
    "UAObject",
    "UAObjectType",
    "UAVariable",
    "UAVariableType",
    "UAMethod",
    "UADataType",
    "UAReferenceType",
    "UAView",
];

pub const UA_NS: &str = "http://opcfoundation.org/UA/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNodeSet {
    pub file: String,
    pub root_span: Span,
    /// Index 0 is implicitly the OPC UA UA namespace; indices >=1 come from
    /// the file's `NamespaceUris/Uri` table, in declared order.
    pub namespaces: Vec<RawNamespace>,
    pub models: Vec<RawModel>,
    pub aliases: Vec<RawAlias>,
    pub nodes: Vec<RawNode>,
    /// Unknown/non-standard direct children of the document root, retained
    /// verbatim with source spans.
    pub root_unknown: Vec<RawUnknown>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNamespace {
    pub index: u16,
    pub uri: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required: Vec<RawRequiredModel>,
    pub unknown: Vec<RawUnknown>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRequiredModel {
    pub model_uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawAlias {
    pub name: String,
    pub target: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawReference {
    pub text: String,
    pub is_forward: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawEngineeringValue {
    /// Original lexical representation exactly as authored.
    pub raw_text: String,
    pub unit: Option<String>,
    pub coordinate_frame: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawNode {
    pub node_class: String,
    pub node_id: String,
    pub browse_name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub references: Vec<RawReference>,
    /// Preserved non-standard child elements (Value, vendor extensions) as
    /// verbatim source XML with spans. Standard display/description are
    /// covered by their own fields.
    pub other_children: Vec<RawUnknown>,
    pub engineering: Option<RawEngineeringValue>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawUnknown {
    pub qname: String,
    pub xml: String,
    pub span: Span,
}

pub fn parse_nodeset(file: &str, data: &[u8]) -> Result<RawNodeSet, XmlError> {
    let root = parse_root(data)?;
    if root.tag != "UANodeSet" {
        return Err(XmlError::Pos(root.span.start, "root is not UANodeSet".into()));
    }
    let raw = String::from_utf8_lossy(data);

    let mut namespaces = vec![RawNamespace {
        index: 0,
        uri: UA_NS.to_string(),
        span: root.span,
    }];
    if let Some(tbl) = root.first("NamespaceUris") {
        for (i, uri_el) in tbl.all("Uri").enumerate() {
            namespaces.push(RawNamespace {
                index: (i + 1) as u16,
                uri: uri_el.text.trim().to_string(),
                span: uri_el.span,
            });
        }
    }

    let mut models = Vec::new();
    for m in root.all("Models") {
        for model_el in m.all("Model") {
            let mut reqs = Vec::new();
            for rdm in model_el.all("RequiredModel") {
                reqs.push(RawRequiredModel {
                    model_uri: rdm.attr("ModelUri").unwrap_or("").trim().to_string(),
                    version: rdm.attr("Version").map(String::from),
                    publication_date: rdm.attr("PublicationDate").map(String::from),
                    span: rdm.span,
                });
            }
            let known = ["RequiredModel"];
            let unknown = model_el
                .children
                .iter()
                .filter(|c| !known.contains(&c.tag.as_str()))
                .map(|c| dom_to_unknown(c, &raw))
                .collect();
            models.push(RawModel {
                model_uri: model_el.attr("ModelUri").unwrap_or("").trim().to_string(),
                version: model_el.attr("Version").map(String::from),
                publication_date: model_el.attr("PublicationDate").map(String::from),
                required: reqs,
                unknown,
                span: model_el.span,
            });
        }
    }

    let mut aliases = Vec::new();
    for a in root.all("Aliases") {
        for al in a.children.iter().filter(|c| c.tag == "Alias") {
            aliases.push(RawAlias {
                name: al.attr("Alias").unwrap_or("").trim().to_string(),
                target: al.text.trim().to_string(),
                span: al.span,
            });
        }
    }

    let mut nodes = Vec::new();
    for child in &root.children {
        if NODE_TAGS.contains(&child.tag.as_str()) {
            nodes.push(extract_node(child, &raw));
        }
    }

    let known_root = ["NamespaceUris", "Models", "Aliases"];
    let root_unknown = root
        .children
        .iter()
        .filter(|c| {
            !known_root.contains(&c.tag.as_str()) && !NODE_TAGS.contains(&c.tag.as_str())
        })
        .map(|c| dom_to_unknown(c, &raw))
        .collect();

    Ok(RawNodeSet {
        file: file.to_string(),
        root_span: root.span,
        namespaces,
        models,
        aliases,
        nodes,
        root_unknown,
    })
}

fn extract_node(el: &DomElement, raw: &str) -> RawNode {
    let mut references = Vec::new();
    let mut other_children = Vec::new();
    let mut engineering = None;

    let mut refs_container: Option<&DomElement> = None;
    for ch in &el.children {
        if ch.tag == "References" {
            refs_container = Some(ch);
            for r in ch.children.iter().filter(|c| c.tag == "Reference") {
                references.push(RawReference {
                    text: r.text.trim().to_string(),
                    is_forward: r
                        .attr("IsForward")
                        .map(|v| !v.eq_ignore_ascii_case("false"))
                        .unwrap_or(true),
                    span: r.span,
                });
            }
        }
    }

    for ch in &el.children {
        if ch.tag == "References"
            || ch.tag == "DisplayName"
            || ch.tag == "Description"
            || ch.tag == "Category"
        {
            continue;
        }
        if let Some(rc) = refs_container {
            if std::ptr::eq(rc, ch) {
                continue;
            }
        }
        if ch.tag == "EngineeringValue" {
            engineering = Some(RawEngineeringValue {
                raw_text: ch.text.trim().to_string(),
                unit: ch.attr("EngineeringUnits").or_else(|| ch.attr("Unit")).map(String::from),
                coordinate_frame: ch.attr("CoordinateFrame").map(String::from),
                span: ch.span,
            });
        }
        other_children.push(dom_to_unknown(ch, raw));
    }

    RawNode {
        node_class: el.tag.clone(),
        node_id: el.attr("NodeId").unwrap_or("").trim().to_string(),
        browse_name: el.attr("BrowseName").unwrap_or("").trim().to_string(),
        display_name: el.first("DisplayName").map(|d| d.text.trim().to_string()),
        description: el.first("Description").map(|d| d.text.trim().to_string()),
        references,
        other_children,
        engineering,
        span: el.span,
    }
}

fn dom_to_unknown(el: &DomElement, raw: &str) -> RawUnknown {
    let xml = raw
        .get(el.span.start as usize..el.span.end as usize)
        .unwrap_or("")
        .to_string();
    RawUnknown {
        qname: el.qname(),
        xml,
        span: el.span,
    }
}

/// Return the original source slice for a span.
pub fn source_slice<'a>(data: &'a [u8], span: Span) -> &'a str {
    std::str::from_utf8(
        data.get(span.start as usize..span.end as usize)
            .unwrap_or(&[]),
    )
    .unwrap_or("")
}
