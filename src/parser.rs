//! NodeSet XML -> NodeSetFile. Everything recognized is structured; everything
//! else is preserved raw (extensions, unknown root children, extra attributes).

use crate::model::*;
use crate::xmlmodel::{parse_raw, RawXml};
use anyhow::Result;

const NODE_CLASSES: &[&str] = &[
    "UAObject",
    "UAVariable",
    "UAMethod",
    "UAObjectType",
    "UAVariableType",
    "UAReferenceType",
    "UADataType",
    "UAView",
];

pub fn parse_nodeset(source: SourceFile) -> Result<NodeSetFile> {
    let root = parse_raw(&source.content)?;
    let mut file = NodeSetFile {
        source: source.clone(),
        namespace_uris: Vec::new(),
        models: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        extra_root: Vec::new(),
    };
    for child in &root.children {
        match child.local() {
            "NamespaceUris" => {
                for uri in child.children_named("Uri") {
                    file.namespace_uris
                        .push(uri.text.clone().unwrap_or_default().trim().to_string());
                }
            }
            "Models" => {
                for m in child.children_named("Model") {
                    let mut required = Vec::new();
                    for r in m.children_named("RequiredModel") {
                        required.push(ModelDep {
                            model_uri: r.attr("ModelUri").unwrap_or_default().to_string(),
                            version: r.attr("Version").unwrap_or_default().to_string(),
                            publication_date: r
                                .attr("PublicationDate")
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                    file.models.push(ModelInfo {
                        model_uri: m.attr("ModelUri").unwrap_or_default().to_string(),
                        version: m.attr("Version").unwrap_or_default().to_string(),
                        publication_date: m.attr("PublicationDate").unwrap_or_default().to_string(),
                        required,
                    });
                }
            }
            "Aliases" => {
                for a in child.children_named("Alias") {
                    file.aliases.push(Alias {
                        name: a.attr("Alias").unwrap_or_default().to_string(),
                        target_text: a.text.clone().unwrap_or_default().trim().to_string(),
                        span: a.span,
                    });
                }
            }
            local if NODE_CLASSES.contains(&local) => {
                file.nodes.push(parse_node(child));
            }
            // NamespaceUris/Models/Aliases handled; anything else is preserved raw.
            _ => file.extra_root.push(child.clone()),
        }
    }
    Ok(file)
}

fn parse_node(el: &RawXml) -> ParsedNode {
    let mut node = ParsedNode {
        node_class: el.local().to_string(),
        node_id_text: el.attr("NodeId").unwrap_or_default().to_string(),
        browse_name: el.attr("BrowseName").unwrap_or_default().to_string(),
        display_name: None,
        references: Vec::new(),
        value_text: None,
        extensions_raw: Vec::new(),
        extra_attrs: Vec::new(),
        extra_children: Vec::new(),
        span: el.span,
    };
    for (k, v) in &el.attrs {
        if k != "NodeId" && k != "BrowseName" {
            node.extra_attrs.push((k.clone(), v.clone()));
        }
    }
    for child in &el.children {
        match child.local() {
            "DisplayName" => {
                node.display_name = child.text.clone().map(|t| t.trim().to_string());
            }
            "References" => {
                for r in child.children_named("Reference") {
                    node.references.push(Reference {
                        ref_type_text: r.attr("ReferenceType").unwrap_or_default().to_string(),
                        is_forward: r.attr("IsForward").map(|v| v != "false").unwrap_or(true),
                        target_text: r.text.clone().unwrap_or_default().trim().to_string(),
                        span: r.span,
                    });
                }
            }
            "Value" => {
                node.value_text = child.text.clone().map(|t| t.trim().to_string());
            }
            "Extensions" => {
                for ext in &child.children {
                    node.extensions_raw.push(ext.clone());
                }
            }
            "Description" => node.extra_children.push(child.clone()),
            _ => node.extra_children.push(child.clone()),
        }
    }
    node
}

/// Serialize a RawXml subtree back to XML text (for display / export of
/// preserved unknown extensions).
pub fn raw_to_string(raw: &RawXml) -> String {
    let mut out = String::new();
    write_raw(raw, &mut out);
    out
}

fn write_raw(raw: &RawXml, out: &mut String) {
    out.push('<');
    out.push_str(&raw.tag);
    for (k, v) in &raw.attrs {
        out.push_str(&format!(" {}=\"{}\"", k, v));
    }
    if raw.children.is_empty() && raw.text.is_none() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    if let Some(t) = &raw.text {
        out.push_str(t);
    }
    for c in &raw.children {
        write_raw(c, out);
    }
    out.push_str(&format!("</{}>", raw.tag));
}
