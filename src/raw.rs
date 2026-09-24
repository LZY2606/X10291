//! Raw layer: parse NodeSet XML into a lossless baseline.
//!
//! Nothing semantic is decided here. The original element kind, attributes,
//! child XML and byte ranges are retained together with namespace tables,
//! aliases, models and *all* unrecognized extensions.

use crate::identity;
use crate::xml::{self, Element};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: String,
    pub path: String,
    pub content: String,
    pub model: RawModelSet,
}

impl SourceFile {
    pub fn snippet(&self, span: (usize, usize)) -> &str {
        self.content
            .get(span.0..span.1)
            .unwrap_or(&self.content[..0])
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RawModelSet {
    pub namespaces: Vec<RawNamespace>,
    pub models: Vec<RawModel>,
    pub aliases: Vec<RawAlias>,
    pub nodes: Vec<RawNode>,
    /// unknown top-level elements (everything except NamespaceUris/Models/Aliases/nodes)
    pub root_extensions: Vec<RawExtension>,
    pub root_span: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawNamespace {
    pub index: u16,
    pub uri: String,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawModel {
    pub uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub required: Vec<RawRequiredModel>,
    pub unknown_attrs: Vec<(String, String)>,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawRequiredModel {
    pub uri: String,
    pub version: Option<String>,
    pub publication_date: Option<String>,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawAlias {
    pub id: String,
    pub name: String,
    pub target: String,
    pub span: (usize, usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawRef {
    pub reference_type: String,
    pub target: String,
    pub is_forward: bool,
    pub span: (usize, usize),
    pub ordinal: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawNode {
    pub id: String,
    pub kind: String,
    pub node_id: String,
    pub browse_name: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub refs: Vec<RawRef>,
    /// known structured child elements (DisplayName, Description, Value)
    pub children_known: Vec<RawKnownChild>,
    /// every element/attribute the parser does not explicitly model
    pub unknown_extensions: Vec<RawExtension>,
    pub span: (usize, usize),
    pub raw_attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawKnownChild {
    pub local: String,
    pub text: String,
    pub span: (usize, usize),
    pub attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawExtension {
    pub id: String,
    pub container: String,
    pub tag: String,
    pub raw_xml: String,
    pub span: (usize, usize),
}

const NODE_ELEMENTS: &[&str] = &[
    "UAObject",
    "UAObjectType",
    "UAVariable",
    "UAVariableType",
    "UAMethod",
    "UADataType",
    "UReferenceType",
    "UAView",
];

const KNOWN_TOP: &[&str] = &[
    "NamespaceUris",
    "ServerUris",
    "Models",
    "Aliases",
];

const KNOWN_NODE_ATTRS: &[&str] = &[
    "NodeId",
    "BrowseName",
    "DataType",
    "ValueRank",
    "ArrayDimensions",
    "AccessLevel",
    "UserAccessLevel",
    "EventNotifier",
    "IsAbstract",
    "Symmetric",
    "Executable",
    "UserExecutable",
    "MinimumSamplingInterval",
    "Historizing",
    "WriteMask",
    "UserWriteMask",
    "ReleaseStatus",
    "Category",
];

pub fn parse_file(path: &str, content: &str) -> Result<SourceFile, String> {
    let root = xml::parse(content).map_err(|e| format!("{}: {}", path, e))?;
    if root.local != "UANodeSet" {
        return Err(format!("{}: root is not UANodeSet", path));
    }
    let fid = identity::file_id(path);
    let mut model = RawModelSet {
        root_span: (root.start, root.end),
        ..Default::default()
    };

    if let Some(uris) = root.child("NamespaceUris") {
        for (i, u) in uris.children_local("Uri").into_iter().enumerate() {
            model.namespaces.push(RawNamespace {
                index: (i + 1) as u16,
                uri: u.text.clone(),
                span: (u.start, u.end),
            });
        }
    }

    for m in root.children_local("Models") {
        for mm in m.children_local("Model") {
            let mut reqs = Vec::new();
            if let Some(required) = mm.child("RequiredModel") {
                for r in required.children_local("Model") {
                    reqs.push(RawRequiredModel {
                        uri: r.attr("ModelUri").unwrap_or("").to_string(),
                        version: r.attr("Version").map(String::from),
                        publication_date: r.attr("PublicationDate").map(String::from),
                        span: (r.start, r.end),
                    });
                }
            }
            let unknown_attrs = mm
                .attrs
                .iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "ModelUri" | "Version" | "PublicationDate" | "AccessRestrictions"
                    )
                })
                .cloned()
                .collect();
            model.models.push(RawModel {
                uri: mm.attr("ModelUri").unwrap_or("").to_string(),
                version: mm.attr("Version").map(String::from),
                publication_date: mm.attr("PublicationDate").map(String::from),
                required: reqs,
                unknown_attrs,
                span: (mm.start, mm.end),
            });
        }
    }

    for aliases in root.children_local("Aliases") {
        for a in aliases.children_local("Alias") {
            let name = a.attr("Alias").unwrap_or(a.text.trim()).to_string();
            let target = a.text.trim().to_string();
            model.aliases.push(RawAlias {
                id: identity::alias_id(path, &name),
                name,
                target,
                span: (a.start, a.end),
            });
        }
    }

    for child in &root.children {
        if KNOWN_TOP.contains(&child.local.as_str())
            || NODE_ELEMENTS.contains(&child.local.as_str())
        {
            continue;
        }
        model.root_extensions.push(RawExtension {
            id: format!(
                "ext-{}",
                identity::short_hash(&[path, "root", &child.start.to_string(), &child.tag])
            ),
            container: "root".into(),
            tag: child.tag.clone(),
            raw_xml: content[child.start..child.end].to_string(),
            span: (child.start, child.end),
        });
    }

    for el in &root.children {
        if NODE_ELEMENTS.contains(&el.local.as_str()) {
            model.nodes.push(parse_node(path, content, el)?);
        }
    }

    Ok(SourceFile {
        id: fid,
        path: path.to_string(),
        content: content.to_string(),
        model,
    })
}

fn parse_node(path: &str, content: &str, el: &Element) -> Result<RawNode, String> {
    let node_id = el.attr("NodeId").unwrap_or_default().to_string();
    let mut refs = Vec::new();
    let mut children_known = Vec::new();
    let mut unknown_extensions = Vec::new();

    if let Some(references) = el.child("References") {
        for (ordinal, r) in references.children_local("Reference").into_iter().enumerate() {
            refs.push(RawRef {
                reference_type: r.attr("ReferenceType").unwrap_or_default().to_string(),
                target: r.text.trim().to_string(),
                is_forward: r.attr("IsForward").map(|v| v != "false").unwrap_or(true),
                span: (r.start, r.end),
                ordinal,
            });
        }
    }

    for c in &el.children {
        if c.local == "References" {
            continue;
        }
        if matches!(c.local.as_str(), "DisplayName" | "Description" | "Value" | "RolePermissions" | "AccessRestriction") {
            children_known.push(RawKnownChild {
                local: c.local.clone(),
                text: c.text.clone(),
                span: (c.start, c.end),
                attrs: c.attrs.clone(),
            });
            // Value may contain structured children (e.g. engineering units)
            for inner in &c.children {
                unknown_extensions.push(make_ext(
                    path,
                    content,
                    &node_id,
                    inner,
                    &format!("{}:{}", node_id, c.local),
                ));
            }
        } else {
            unknown_extensions.push(make_ext(path, content, &node_id, c, &node_id));
        }
    }

    for (k, v) in &el.attrs {
        if KNOWN_NODE_ATTRS.contains(&k.as_str()) {
            continue;
        }
        unknown_extensions.push(RawExtension {
            id: format!(
                "ext-{}",
                identity::short_hash(&[path, &node_id, "attr", k])
            ),
            container: format!("node-attr:{}", node_id),
            tag: format!("@{}", k),
            raw_xml: format!("{}=\"{}\"", k, v),
            span: el.name_span,
        });
    }

    Ok(RawNode {
        id: identity::raw_node_id(path, &node_id),
        kind: el.local.clone(),
        node_id,
        browse_name: el.attr("BrowseName").map(String::from),
        display_name: el
            .child("DisplayName")
            .map(|d| d.text.clone()),
        description: el.child("Description").map(|d| d.text.clone()),
        refs,
        children_known,
        unknown_extensions,
        span: (el.start, el.end),
        raw_attrs: el.attrs.clone(),
    })
}

fn make_ext(
    path: &str,
    content: &str,
    node_id: &str,
    el: &Element,
    container: &str,
) -> RawExtension {
    RawExtension {
        id: format!(
            "ext-{}",
            identity::short_hash(&[path, container, &el.start.to_string(), &el.tag])
        ),
        container: format!("node:{}", container),
        tag: el.tag.clone(),
        raw_xml: content[el.start..el.end].to_string(),
        span: (el.start, el.end),
    }
}
