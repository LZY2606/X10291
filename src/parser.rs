use crate::model::{
    AliasDef, ModelDependency, ModelInfo, NamespaceDecl, NodeDef, ParsedSource, Quantity,
    RawReference, SourceRange, UnknownExtension, XmlAttr, XmlNode,
};
use quick_xml::events::Event;
use quick_xml::Reader;

const NODE_KINDS: &[&str] = &[
    "UAObject",
    "UAVariable",
    "UAMethod",
    "UAObjectType",
    "UAVariableType",
    "UAReferenceType",
    "UADataType",
    "UAView",
];

pub fn parse_source(id: &str, path: &str, label: &str, content: &str) -> Result<ParsedSource, String> {
    let document = parse_xml(content)?;
    let root = document
        .children
        .iter()
        .find(|node| node.kind == "element" && local_name(&node.tag) == "UANodeSet")
        .cloned()
        .ok_or_else(|| format!("{} does not contain a UANodeSet root", path))?;

    let mut namespaces = Vec::new();
    let mut aliases = Vec::new();
    let mut models = Vec::new();
    let mut required_models = Vec::new();
    let mut nodes = Vec::new();
    let mut document_extensions = Vec::new();

    for child in &root.children {
        if child.kind != "element" {
            continue;
        }
        match local_name(&child.tag) {
            "NamespaceUris" => {
                for (offset, uri_node) in child.children.iter().filter(|n| n.kind == "element").enumerate() {
                    namespaces.push(NamespaceDecl {
                        index: (offset + 1) as u16,
                        uri: uri_node.text.trim().to_string(),
                        raw: uri_node.text.trim().to_string(),
                        range: uri_node.range.clone(),
                    });
                }
            }
            "Aliases" => {
                for alias_node in child.children.iter().filter(|n| n.kind == "element") {
                    let name = attr(alias_node, "Alias").unwrap_or_default();
                    if !name.is_empty() {
                        aliases.push(AliasDef {
                            name,
                            target: alias_node.text.trim().to_string(),
                            range: alias_node.range.clone(),
                        });
                    }
                }
            }
            "Models" => {
                for model_node in child.children.iter().filter(|n| n.kind == "element") {
                    let model = ModelInfo {
                        uri: attr(model_node, "ModelUri").unwrap_or_default(),
                        version: optional_attr(model_node, "Version"),
                        publication_date: optional_attr(model_node, "PublicationDate"),
                        range: model_node.range.clone(),
                    };
                    for required in model_node.children.iter().filter(|n| {
                        n.kind == "element" && local_name(&n.tag) == "RequiredModel"
                    }) {
                        required_models.push(ModelDependency {
                            uri: attr(required, "ModelUri").unwrap_or_default(),
                            version: optional_attr(required, "Version"),
                            publication_date: optional_attr(required, "PublicationDate"),
                            source_id: id.to_string(),
                            range: required.range.clone(),
                        });
                    }
                    models.push(model);
                }
            }
            tag if NODE_KINDS.contains(&tag) => nodes.push(parse_node(child, tag)),
            _ => document_extensions.push(UnknownExtension {
                path: format!("UANodeSet/{}", local_name(&child.tag)),
                reason: "unrecognized document-level extension retained".to_string(),
                xml: child.clone(),
            }),
        }
    }

    Ok(ParsedSource {
        id: id.to_string(),
        path: path.to_string(),
        label: label.to_string(),
        content: content.to_string(),
        namespaces,
        aliases,
        models,
        required_models,
        nodes,
        document_extensions,
    })
}

fn parse_node(xml: &XmlNode, kind: &str) -> NodeDef {
    let raw_node_id = attr(xml, "NodeId").unwrap_or_default();
    let (namespace_index, identifier) = split_node_id(&raw_node_id);
    let mut display_name = String::new();
    let mut description = String::new();
    let mut references = Vec::new();
    let mut quantities = Vec::new();
    let mut unknown_extensions = Vec::new();

    for child in xml.children.iter().filter(|n| n.kind == "element") {
        match local_name(&child.tag) {
            "DisplayName" => display_name = child.text.trim().to_string(),
            "Description" => description = child.text.trim().to_string(),
            "References" => {
                for reference in child.children.iter().filter(|n| {
                    n.kind == "element" && local_name(&n.tag) == "Reference"
                }) {
                    references.push(RawReference {
                        text: reference.text.trim().to_string(),
                        ref_type: attr(reference, "ReferenceType").unwrap_or_else(|| "References".to_string()),
                        is_forward: attr(reference, "IsForward")
                            .map(|value| value != "false")
                            .unwrap_or(true),
                        target: reference.text.trim().to_string(),
                        range: reference.range.clone(),
                    });
                }
            }
            "Measurement" | "Setpoint" | "Position" => {
                quantities.push(parse_quantity(child));
            }
            _ => unknown_extensions.push(UnknownExtension {
                path: format!("{}/{}", local_name(&xml.tag), local_name(&child.tag)),
                reason: "unrecognized node extension retained".to_string(),
                xml: child.clone(),
            }),
        }
    }

    NodeDef {
        kind: kind.to_string(),
        raw_node_id,
        namespace_index,
        identifier,
        browse_name: attr(xml, "BrowseName").unwrap_or_default(),
        display_name,
        description,
        references,
        quantities,
        unknown_extensions,
        xml: xml.clone(),
        range: xml.range.clone(),
    }
}

fn parse_quantity(xml: &XmlNode) -> Quantity {
    let tag = local_name(xml.tag.as_str());
    let key = attr(xml, "key").unwrap_or_else(|| tag.to_ascii_lowercase());
    let raw_value = attr(xml, "value").unwrap_or_default();
    let raw_unit = attr(xml, "unit").unwrap_or_default();
    let coordinate_basis = optional_attr(xml, "coordinateBasis");
    let (normalized_value, normalized_unit, status, diagnostic) = normalize_quantity(&raw_value, &raw_unit);
    Quantity {
        key,
        raw_value,
        raw_unit,
        coordinate_basis,
        normalized_value,
        normalized_unit,
        status,
        diagnostic,
        range: xml.range.clone(),
    }
}

fn normalize_quantity(raw_value: &str, raw_unit: &str) -> (Option<f64>, String, String, String) {
    let value = match raw_value.parse::<f64>() {
        Ok(value) => value,
        Err(_) => {
            return (
                None,
                String::new(),
                "invalid_number".to_string(),
                format!("value '{}' cannot be converted", raw_value),
            )
        }
    };
    match raw_unit {
        "m" => (Some(value), "m".to_string(), "normalized".to_string(), String::new()),
        "mm" => (Some(value / 1000.0), "m".to_string(), "normalized".to_string(), String::new()),
        "rpm" => (Some(value), "rpm".to_string(), "normalized".to_string(), String::new()),
        "K" => (Some(value), "K".to_string(), "normalized".to_string(), String::new()),
        "degC" | "°C" => (Some(value + 273.15), "K".to_string(), "normalized".to_string(), String::new()),
        "bar" => (Some(value * 100000.0), "Pa".to_string(), "normalized".to_string(), String::new()),
        "" => (None, String::new(), "missing_unit".to_string(), "quantity has no unit".to_string()),
        unit => (
            None,
            String::new(),
            "unresolved_unit".to_string(),
            format!("unit '{}' is not in the conversion table", unit),
        ),
    }
}

fn parse_xml(content: &str) -> Result<XmlNode, String> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut root = XmlNode {
        kind: "document".to_string(),
        tag: "#document".to_string(),
        attrs: Vec::new(),
        text: String::new(),
        children: Vec::new(),
        range: SourceRange { start: 0, end: content.len() },
    };
    let mut stack: Vec<XmlNode> = Vec::new();

    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let tag = decode(element.name().as_ref());
                stack.push(XmlNode {
                    kind: "element".to_string(),
                    tag,
                    attrs: parse_attrs(&element, start),
                    text: String::new(),
                    children: Vec::new(),
                    range: SourceRange { start, end: start },
                });
            }
            Ok(Event::Empty(element)) => {
                let end = reader.buffer_position() as usize;
                append_node(
                    &mut root,
                    &mut stack,
                    XmlNode {
                        kind: "element".to_string(),
                        tag: decode(element.name().as_ref()),
                        attrs: parse_attrs(&element, start),
                        text: String::new(),
                        children: Vec::new(),
                        range: SourceRange { start, end },
                    },
                );
            }
            Ok(Event::End(_)) => {
                let end = reader.buffer_position() as usize;
                let mut node = stack.pop().ok_or_else(|| "unexpected closing tag".to_string())?;
                node.range.end = end;
                append_node(&mut root, &mut stack, node);
            }
            Ok(Event::Text(text)) => {
                let decoded = text.unescape().map_err(|err| err.to_string())?.to_string();
                append_text(&mut root, &mut stack, &decoded);
            }
            Ok(Event::CData(text)) => {
                let decoded = String::from_utf8_lossy(text.as_ref()).to_string();
                append_text(&mut root, &mut stack, &decoded);
            }
            Ok(Event::Decl(_)) | Ok(Event::Comment(_)) | Ok(Event::PI(_)) | Ok(Event::DocType(_)) => {}
            Ok(Event::Eof) => break,
            Err(err) => return Err(format!("XML parse error at byte {}: {}", reader.buffer_position(), err)),
        }
    }
    if !stack.is_empty() {
        return Err("XML document ended before all elements were closed".to_string());
    }
    Ok(root)
}

fn append_node(root: &mut XmlNode, stack: &mut Vec<XmlNode>, node: XmlNode) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        root.children.push(node);
    }
}

fn append_text(root: &mut XmlNode, stack: &mut Vec<XmlNode>, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let target = stack.last_mut().unwrap_or(root);
    if !target.text.is_empty() {
        target.text.push(' ');
    }
    target.text.push_str(trimmed);
}

fn parse_attrs(element: &quick_xml::events::BytesStart<'_>, element_start: usize) -> Vec<XmlAttr> {
    element
        .attributes()
        .with_checks(false)
        .flatten()
        .map(|attribute| XmlAttr {
            name: decode(attribute.key.as_ref()),
            value: String::from_utf8_lossy(attribute.value.as_ref()).to_string(),
            range: SourceRange { start: element_start, end: element_start },
        })
        .collect()
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

fn local_name(tag: &str) -> &str {
    tag.rsplit(':').next().unwrap_or(tag)
}

fn attr(xml: &XmlNode, name: &str) -> Option<String> {
    xml.attrs
        .iter()
        .find(|attribute| local_name(&attribute.name) == name)
        .map(|attribute| attribute.value.clone())
}

fn optional_attr(xml: &XmlNode, name: &str) -> Option<String> {
    attr(xml, name).filter(|value| !value.is_empty())
}

pub fn split_node_id(raw: &str) -> (u16, String) {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("ns=") {
        let mut parts = rest.splitn(2, ';');
        let namespace = parts.next().and_then(|value| value.parse::<u16>().ok()).unwrap_or(0);
        let identifier = parts.next().unwrap_or_default().to_string();
        (namespace, identifier)
    } else {
        (0, trimmed.to_string())
    }
}
