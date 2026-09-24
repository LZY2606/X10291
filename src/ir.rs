//! Raw Information Records extracted 1:1 from one NodeSet XML file.
//!
//! Nothing here is merged or normalized. Numeric namespace indices are kept
//! exactly as authored; merging by URI happens in [`crate::model`].

use crate::xml::{Element, Span, XmlDoc};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawNsTable {
    pub uri: String,
    pub index: u32,
    pub uri_span: Span,
    pub element_span: Span,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawModel {
    pub model_uri: String,
    pub version: String,
    pub publication_date: Option<String>,
    pub requires: Vec<RawRequiredModel>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawRequiredModel {
    pub model_uri: String,
    pub version: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawAlias {
    pub alias: String,
    pub node_id: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawReference {
    pub reference_type: String,
    pub is_forward: bool,
    pub target: String,
    pub span: Span,
    pub ref_type_span: Option<Span>,
    pub target_span: Span,
}

/// Value element text plus its wrapper tag, preserved verbatim.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawValue {
    pub wrapper_qname: String,
    pub text: String,
    pub span: Span,
}

/// Any child not part of the recognized NodeSet vocabulary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawUnknown {
    pub qname: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub span: Span,
    pub xml: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawNode {
    /// UAObject, UAVariable, UAObjectType, UAVariableType, UAMethod, UADataType,
    /// UAReferenceType, UAView, ...
    pub class: String,
    pub node_id: String,
    pub browse_name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub data_type: Option<String>,
    pub value: Option<RawValue>,
    pub references: Vec<RawReference>,
    pub unknown: Vec<RawUnknown>,
    pub span: Span,
    pub node_id_span: Option<Span>,
    pub browse_name_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawNodeSet {
    pub namespaces: Vec<RawNsTable>,
    pub models: Vec<RawModel>,
    pub aliases: Vec<RawAlias>,
    pub nodes: Vec<RawNode>,
    /// Unrecognized top-level children of the document root.
    pub unknown_toplevel: Vec<RawUnknown>,
    pub root_span: Span,
}

const NODE_CLASSES: &[&str] = &[
    "UAObject",
    "UAVariable",
    "UAObjectType",
    "UAVariableType",
    "UAMethod",
    "UADataType",
    "UAReferenceType",
    "UAView",
];

/// Children understood on a node element besides `<References>` and `<Value>`.
fn recognized_node_child(name: &str) -> bool {
    matches!(
        name,
        "DisplayName" | "Description" | "References" | "Value" | "Category" | "WriteMask"
            | "UserWriteMask" | "RolePermissions" | "AccessRestrictions" | "ReferencesSpecified"
    )
}

fn unknown_from(el: &Element, source: &str) -> RawUnknown {
    RawUnknown {
        qname: el.qname.clone(),
        attrs: el
            .attrs
            .iter()
            .map(|a| (a.name.clone(), a.value.clone()))
            .collect(),
        text: el.text.clone(),
        span: el.span,
        xml: source
            .get(el.span.start..el.span.end)
            .unwrap_or_default()
            .to_string(),
    }
}


fn collect_unknown(el: &Element, source: &str) -> Vec<RawUnknown> {
    let mut out = Vec::new();
    for c in &el.children {
        if !recognized_node_child(&c.name) {
            out.push(unknown_from(c, source));
        }
    }
    out
}

pub fn extract(doc: &XmlDoc, source: &str) -> Result<RawNodeSet, String> {
    let root = &doc.root;
    if root.name != "UANodeSet" {
        return Err(format!(
            "root element is <{}>, expected <UANodeSet>",
            root.qname
        ));
    }

    let mut namespaces = Vec::new();
    if let Some(ns_el) = root.child("NamespaceUris") {
        for (i, uri_el) in ns_el.children_of("Uri").into_iter().enumerate() {
            namespaces.push(RawNsTable {
                uri: uri_el.text_trim(),
                index: i as u32 + 1,
                uri_span: Span::new(ns_el.span.start, ns_el.span.end),
                element_span: uri_el.span,
            });
        }
    }

    let mut models = Vec::new();
    if let Some(models_el) = root.child("Models") {
        for m in models_el.children_of("Model") {
            let mut requires = Vec::new();
            if let Some(req_wrap) = m.child("RequiredModel") {
                let _ = req_wrap;
            }
            for r in m.children_of("RequiredModel") {
                requires.push(RawRequiredModel {
                    model_uri: r
                        .attr("ModelUri")
                        .map(|a| a.value.clone())
                        .unwrap_or_default(),
                    version: r
                        .attr("Version")
                        .map(|a| a.value.clone())
                        .unwrap_or_default(),
                    span: r.span,
                });
            }
            models.push(RawModel {
                model_uri: m
                    .attr("ModelUri")
                    .map(|a| a.value.clone())
                    .unwrap_or_default(),
                version: m
                    .attr("Version")
                    .map(|a| a.value.clone())
                    .unwrap_or_default(),
                publication_date: m.attr("PublicationDate").map(|a| a.value.clone()),
                requires,
                span: m.span,
            });
        }
    }

    let mut aliases = Vec::new();
    if let Some(aliases_el) = root.child("Aliases") {
        for a in aliases_el.children.iter().filter(|c| c.name == "Alias") {
            aliases.push(RawAlias {
                alias: a
                    .attr("Alias")
                    .map(|x| x.value.clone())
                    .unwrap_or_default(),
                node_id: a.text_trim(),
                span: a.span,
            });
        }
    }

    let mut nodes = Vec::new();
    let mut unknown_toplevel = Vec::new();
    for child in &root.children {
        if NODE_CLASSES.contains(&child.name.as_str()) {
            nodes.push(extract_node(child, source));
        } else if matches!(child.name.as_str(), "NamespaceUris" | "Models" | "Aliases" | "Extensions")
        {
            // standard wrappers; children inside <Extensions> are preserved as
            // unknown because the schema does not constrain them.
            if child.name == "Extensions" {
                for c in &child.children {
                    unknown_toplevel.push(unknown_from(c, source));
                }
            }
        } else {
            unknown_toplevel.push(unknown_from(child, source));
        }
    }

    Ok(RawNodeSet {
        namespaces,
        models,
        aliases,
        nodes,
        unknown_toplevel,
        root_span: root.span,
    })
}

fn extract_node(el: &Element, source: &str) -> RawNode {
    let mut references = Vec::new();
    if let Some(refs) = el.child("References") {
        for r in refs.children_of("Reference") {
            let is_forward = r
                .attr("IsForward")
                .map(|a| !a.value.eq_ignore_ascii_case("false"))
                .unwrap_or(true);
            references.push(RawReference {
                reference_type: r
                    .attr("ReferenceType")
                    .map(|a| a.value.clone())
                    .unwrap_or_default(),
                is_forward,
                target: r.text_trim(),
                span: r.span,
                ref_type_span: r.attr("ReferenceType").map(|a| a.value_span),
                target_span: r.span,
            });
        }
    }
    let value = el.child("Value").and_then(|v| v.children.first()).map(|w| RawValue {
        wrapper_qname: w.qname.clone(),
        text: w.text.clone(),
        span: w.span,
    });
    RawNode {
        class: el.name.clone(),
        node_id: el
            .attr("NodeId")
            .map(|a| a.value.clone())
            .unwrap_or_default(),
        browse_name: el
            .attr("BrowseName")
            .map(|a| a.value.clone())
            .unwrap_or_default(),
        display_name: el.child("DisplayName").map(|d| d.text_trim()),
        description: el.child("Description").map(|d| d.text_trim()),
        data_type: el.attr("DataType").map(|a| a.value.clone()),
        value,
        references,
        unknown: collect_unknown(el, source),
        span: el.span,
        node_id_span: el.attr("NodeId").map(|a| a.value_span),
        browse_name_span: el.attr("BrowseName").map(|a| a.value_span),
    }
}
