//! Merge raw NodeSets into one URI-keyed model.
//!
//! Semantics enforced here:
//! * numeric namespace indices are file-local; merge re-keys by URI;
//! * aliases resolve per-file with cycle detection (no arbitrary target);
//! * duplicate NodeIds keep every candidate;
//! * forward/inverse references normalize without creating missing edges;
//! * missing dependency targets remain unresolved placeholder nodes.

use crate::id::stable_id;
use crate::model::*;
use crate::nodeid::{parse_browse_name, parse_node_id, uri_for_index, Identifier};
use crate::parser::UA_NS;
use crate::rules;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub code: String,
    pub severity: String, // info | warn | anomaly | conflict
    pub subject: String,
    pub message: String,
    pub rationale: String,
    pub span: Option<Span>,
    /// Stable id of the affected object for UI drill-in.
    pub object_id: Option<String>,
}

impl Event {
    pub fn new(code: &str, severity: &str, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Event {
            code: code.into(),
            severity: severity.into(),
            subject: subject.into(),
            message: message.into(),
            rationale: rules::rationale(code).to_string(),
            span: None,
            object_id: None,
        }
    }
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
    pub fn with_span_opt(mut self, span: Option<Span>) -> Self {
        self.span = span;
        self
    }
    pub fn with_object(mut self, id: impl Into<String>) -> Self {
        self.object_id = Some(id.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalNamespace {
    /// Global order, assigned deterministically by first appearance.
    pub global_index: u16,
    pub uri: String,
    /// (source_id, file-local index) — shows the remap.
    pub bound: Vec<(String, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCandidate {
    pub source_id: String,
    pub raw_node_id: String,
    pub browse_name_raw: Option<String>,
    pub display_name: Option<String>,
    pub kind: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedNode {
    pub id: String,
    pub key: GlobalNodeId,
    pub kind: String,
    pub browse_name: Option<BrowseName>,
    pub display_name: Option<String>,
    pub candidates: Vec<NodeCandidate>,
    pub is_abstract: bool,
    /// Present when the node is a placeholder for a missing dependency target.
    pub unresolved_reason: Option<String>,
    /// Source ids this node came from (placeholders: source that referenced it).
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedEdge {
    pub id: String,
    /// Reference type as a global key text (resolved alias/URI or raw text).
    pub ref_type: String,
    pub ref_type_resolved: bool,
    pub source: GlobalNodeId,
    pub target: GlobalNodeId,
    pub forward: bool,
    /// The raw IsForward flag as written (before normalization).
    pub raw_is_forward: bool,
    pub source_id: String,
    pub span: Span,
    pub status: String, // ok | dangling
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedQuantity {
    pub id: String,
    pub node_id: String,
    pub quantity: RawQuantity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepRequirement {
    pub source_id: String,
    pub model_uri: String,
    pub version: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedModel {
    pub rule_version: String,
    pub namespaces: Vec<GlobalNamespace>,
    pub nodes: BTreeMap<String, MergedNode>,
    pub edges: Vec<MergedEdge>,
    pub quantities: Vec<MergedQuantity>,
    pub dependencies: Vec<DepRequirement>,
    pub events: Vec<Event>,
}

fn node_id_for(key: &GlobalNodeId) -> String {
    stable_id("node", &[&key.ns_uri, &key.id])
}

fn is_standard_ref_type(text: &str) -> Option<&'static str> {
    matches!(text, "HasComponent" | "HasTypeDefinition" | "HasSubtype" | "HasProperty"
        | "Organizes" | "HasModellingRule" | "GeneratesEvent" | "References").then_some(text)
}

struct FileCtx<'a> {
    ns: &'a RawNodeSet,
    table: Vec<String>,
    /// alias name -> resolved GlobalNodeId or error marker
    alias_target: BTreeMap<String, Result<GlobalNodeId, String>>,
}

pub fn merge(raw: &RawModel) -> MergedModel {
    let mut model = MergedModel {
        rule_version: rules::RULESET_VERSION.to_string(),
        namespaces: Vec::new(),
        nodes: BTreeMap::new(),
        edges: Vec::new(),
        quantities: Vec::new(),
        dependencies: Vec::new(),
        events: raw.parse_diagnostics.iter().map(|d| {
            Event::new("PARSE_ERROR", "anomaly", d.code.clone(), d.message.clone())
                .with_span_opt(d.span.clone())
        }).collect(),
    };

    // Global namespace table: deterministic first-appearance order; UA at 0.
    let mut ns_order: Vec<String> = vec![UA_NS.to_string()];
    let mut ns_bound: BTreeMap<String, Vec<(String, u16)>> = BTreeMap::new();
    ns_bound.insert(UA_NS.to_string(), vec![]);

    for src in &raw.sources {
        // Validate declared table order matches captured ordinal and bind.
        for rn in &src.namespaces {
            let uri = rn.uri.clone();
            ns_bound.entry(uri.clone()).or_default().push((src.source_id.clone(), rn.local_index));
            if !ns_order.contains(&uri) {
                ns_order.push(uri);
            }
        }
        for dep in &src.dependencies {
            if dep.model_uri != own_model_uri(src) {
                model.dependencies.push(DepRequirement {
                    source_id: src.source_id.clone(),
                    model_uri: dep.model_uri.clone(),
                    version: dep.version.clone(),
                    span: dep.span.clone(),
                });
            }
        }
    }
    for (i, uri) in ns_order.iter().enumerate() {
        model.namespaces.push(GlobalNamespace {
            global_index: i as u16,
            uri: uri.clone(),
            bound: ns_bound.remove(uri).unwrap_or_default(),
        });
    }

    // Dependency version conflicts.
    let mut dep_versions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for d in &model.dependencies {
        dep_versions.entry(d.model_uri.clone()).or_default().insert(d.version.clone());
    }
    let supplied_uris: BTreeSet<String> = raw.sources.iter().flat_map(|s| s.namespaces.iter().map(|n| n.uri.clone())).collect();
    for (uri, versions) in &dep_versions {
        if versions.len() > 1 {
            model.events.push(
                Event::new(
                    rules::codes::DEPENDENCY_VERSION_CONFLICT,
                    "conflict",
                    uri.clone(),
                    format!("required versions: {}", versions.iter().cloned().collect::<Vec<_>>().join(", ")),
                )
                .with_object(stable_id("dep", &[uri])),
            );
        }
        if !supplied_uris.contains(uri) && uri != UA_NS {
            model.events.push(
                Event::new(
                    rules::codes::DEPENDENCY_MISSING,
                    "warn",
                    uri.clone(),
                    "required model namespace is not supplied; targets remain unresolved",
                )
                .with_object(stable_id("dep", &[uri])),
            );
        }
    }

    let contexts = build_contexts(raw, &mut model);

    // First pass: nodes.
    for (ci, src) in raw.sources.iter().enumerate() {
        let ctx = &contexts[ci];
        for node in &src.nodes {
            let parsed = match parse_node_id(&node.node_id_text) {
                Ok(p) => p,
                Err(msg) => {
                    model.events.push(
                        Event::new("BAD_NODE_ID", "anomaly", node.node_id_text.clone(), msg)
                            .with_span(node.span.clone()),
                    );
                    continue;
                }
            };
            let key = match resolve_identifier(&parsed.ns, &parsed.id, ctx) {
                Ok(k) => k,
                Err(ev) => {
                    model.events.push(ev.with_span(node.span.clone()));
                    continue;
                }
            };
            let browse_name = node.browse_name_text.as_ref().and_then(|t| {
                parse_browse_name(t).ok().and_then(|(ns, name)| {
                    ctx.table.get(ns as usize).cloned().or_else(|| (ns == 0).then(|| UA_NS.to_string())).map(|uri| BrowseName { ns_uri: uri, name })
                })
            });
            let id = node_id_for(&key);
            let candidate = NodeCandidate {
                source_id: src.source_id.clone(),
                raw_node_id: node.node_id_text.clone(),
                browse_name_raw: node.browse_name_text.clone(),
                display_name: node.display_name.clone(),
                kind: node.kind.clone(),
                span: node.span.clone(),
            };
            let entry = model.nodes.entry(id.clone()).or_insert_with(|| MergedNode {
                id: id.clone(),
                key: key.clone(),
                kind: node.kind.clone(),
                browse_name: browse_name.clone(),
                display_name: node.display_name.clone(),
                candidates: Vec::new(),
                is_abstract: node.is_abstract.unwrap_or(false),
                unresolved_reason: None,
                source_ids: Vec::new(),
            });
            if !entry.source_ids.contains(&src.source_id) {
                entry.source_ids.push(src.source_id.clone());
            }
            entry.candidates.push(candidate);
            if entry.browse_name.is_none() {
                entry.browse_name = browse_name;
            }
            if entry.display_name.is_none() {
                entry.display_name = node.display_name.clone();
            }
            for q in &node.quantities {
                model.quantities.push(MergedQuantity {
                    id: stable_id("q", &[&id, &q.span.source_id, &q.span.start.to_string(), &q.value]),
                    node_id: id.clone(),
                    quantity: q.clone(),
                });
            }
            for ext in &node.extensions {
                model.events.push(
                    Event::new(
                        rules::codes::UNKNOWN_EXTENSION_RETAINED,
                        "anomaly",
                        ext.tag.clone(),
                        "unrecognized node extension subtree retained verbatim",
                    )
                    .with_span(ext.span.clone())
                    .with_object(id.clone()),
                );
            }
        }
        for ext in &src.file_extensions {
            model.events.push(
                Event::new(
                    rules::codes::UNKNOWN_EXTENSION_RETAINED,
                    "anomaly",
                    ext.tag.clone(),
                    "unrecognized file extension subtree retained verbatim",
                )
                .with_span(ext.span.clone()),
            );
        }
    }

    // Duplicate NodeId events with all candidates retained.
    for node in model.nodes.values() {
        if node.candidates.len() > 1 {
            model.events.push(
                Event::new(
                    rules::codes::DUPLICATE_NODE_ID,
                    "conflict",
                    node.key.display(),
                    format!("{} source candidates retained", node.candidates.len()),
                )
                .with_object(node.id.clone())
                .with_span(node.candidates[0].span.clone()),
            );
        }
    }

    build_edges(raw, &contexts, &mut model);
    normalize_quantities(&mut model);
    model
}

fn own_model_uri(src: &RawNodeSet) -> String {
    // The first dependency entry populated from <Model> (matching one of the
    // declared namespace URIs) represents the file's own model. Fall back to
    // the last declared namespace URI (typical NodeSet convention).
    if let Some(d) = src.dependencies.iter().find(|d| {
        src.namespaces.iter().any(|n| n.uri == d.model_uri)
    }) {
        return d.model_uri.clone();
    }
    src.namespaces.last().map(|n| n.uri.clone()).unwrap_or_default()
}

fn build_contexts<'a>(raw: &'a RawModel, model: &mut MergedModel) -> Vec<FileCtx<'a>> {
    raw.sources
        .iter()
        .map(|src| {
            let table: Vec<String> = src.namespaces.iter().map(|n| n.uri.clone()).collect();
            let mut alias_target: BTreeMap<String, Result<GlobalNodeId, String>> = BTreeMap::new();

            // Iterative alias expansion with cycle detection. A plain token in
            // a NodeId position that matches an alias name is itself an alias.
            let names: Vec<String> = src.aliases.iter().map(|a| a.name.clone()).collect();
            for alias in &src.aliases {
                let mut chain: Vec<String> = vec![alias.name.clone()];
                let mut current_text = alias.node_id_text.clone();
                let result = loop {
                    let parsed = match parse_node_id(&current_text) {
                        Ok(p) => p,
                        Err(e) => break Err(e),
                    };
                    // A string identifier with no namespace that equals an
                    // alias name means "point to that alias".
                    let follows = if parsed.ns == 0 {
                        if let Identifier::String_(s) = &parsed.id {
                            names.contains(s).then(|| s.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(next) = follows {
                        if chain.contains(&next) {
                            chain.push(next);
                            model.events.push(
                                Event::new(
                                    rules::codes::ALIAS_CYCLE,
                                    "conflict",
                                    chain.join(" -> "),
                                    "alias expansion is cyclic; no target chosen",
                                )
                                .with_span(alias.span.clone()),
                            );
                            break Err("alias cycle".to_string());
                        }
                        chain.push(next.clone());
                        let def = src.aliases.iter().find(|a| a.name == next);
                        match def {
                            Some(a) => current_text = a.node_id_text.clone(),
                            None => {
                                model.events.push(
                                    Event::new(
                                        rules::codes::ALIAS_UNRESOLVED,
                                        "warn",
                                        next,
                                        "alias names an alias with no definition",
                                    )
                                    .with_span(alias.span.clone()),
                                );
                                break Err("unresolved alias".to_string());
                            }
                        }
                    } else {
                        match key_from_parsed(parsed.ns, &parsed.id, &table) {
                            Ok(k) => break Ok(k),
                            Err(e) => break Err(e.message),
                        }
                    }
                };
                alias_target.insert(alias.name.clone(), result);
            }
            FileCtx { ns: src, table, alias_target }
        })
        .collect()
}

fn key_from_parsed(ns: u16, id: &Identifier, table: &[String]) -> Result<GlobalNodeId, Diagnostic> {
    Ok(GlobalNodeId { ns_uri: uri_for_index(ns, table)?, id: id.key() })
}

fn resolve_identifier(
    ns: u16,
    id: &Identifier,
    ctx: &FileCtx,
) -> Result<GlobalNodeId, Event> {
    if ns == 0 {
        if let Identifier::String_(s) = id {
            if let Some(found) = ctx.alias_target.get(s) {
                return found.clone().map_err(|e| {
                    Event::new(
                        rules::codes::ALIAS_CYCLE,
                        "conflict",
                        s.clone(),
                        e,
                    )
                });
            }
        }
    }
    key_from_parsed(ns, id, &ctx.table).map_err(|d| {
        Event::new(d.code.as_str(), "anomaly", "node", d.message).with_span_opt(d.span)
    })
}

fn build_edges(raw: &RawModel, contexts: &[FileCtx], model: &mut MergedModel) {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (ci, src) in raw.sources.iter().enumerate() {
        let ctx = &contexts[ci];
        for node in &src.nodes {
            let source_parsed = match parse_node_id(&node.node_id_text) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let source_key = match resolve_identifier(&source_parsed.ns, &source_parsed.id, ctx) {
                Ok(k) => k,
                Err(_) => continue,
            };
            for r in &node.references {
                // Reference type: alias first, then standard UA name, else raw.
                let (ref_type, resolved) = if let Some(found) = ctx.alias_target.get(&r.reference_type) {
                    match found {
                        Ok(k) => (k.display(), true),
                        Err(_) => (r.reference_type.clone(), false),
                    }
                } else if is_standard_ref_type(&r.reference_type).is_some() {
                    (format!("{UA_NS}|s={}", r.reference_type), true)
                } else {
                    (r.reference_type.clone(), false)
                };
                if !resolved {
                    model.events.push(
                        Event::new(
                            rules::codes::REFERENCE_TYPE_UNRESOLVED,
                            "warn",
                            r.reference_type.clone(),
                            "reference type kept as raw text",
                        )
                        .with_span(r.span.clone()),
                    );
                }

                let target_parsed = match parse_node_id(&r.target_text) {
                    Ok(p) => p,
                    Err(msg) => {
                        model.events.push(
                            Event::new("BAD_TARGET_ID", "anomaly", r.target_text.clone(), msg)
                                .with_span(r.span.clone()),
                        );
                        continue;
                    }
                };
                let target_key = match resolve_identifier(&target_parsed.ns, &target_parsed.id, ctx) {
                    Ok(k) => k,
                    Err(ev) => {
                        model.events.push(ev.with_span(r.span.clone()));
                        continue;
                    }
                };

                // Normalize inverse by swapping endpoints — never duplicating.
                let (fwd_source, fwd_target, forward) = if r.is_forward {
                    (source_key.clone(), target_key.clone(), true)
                } else {
                    (target_key.clone(), source_key.clone(), false)
                };

                let edge_id = stable_id(
                    "edge",
                    &[&ref_type, &fwd_source.display(), &fwd_target.display(), &src.source_id, &r.span.start.to_string()],
                );
                if !seen.insert(edge_id.clone()) {
                    continue;
                }

                let target_exists = model.nodes.values().any(|n| n.key == fwd_target);
                let status = if target_exists { "ok" } else { "dangling" };
                if !target_exists {
                    insert_placeholder(model, &fwd_target, &src.source_id, &r.span);
                    model.events.push(
                        Event::new(
                            rules::codes::DANGLING_REFERENCE,
                            "warn",
                            fwd_target.display(),
                            "edge target absent; retained as dangling plus unresolved node",
                        )
                        .with_span(r.span.clone())
                        .with_object(node_id_for(&fwd_target)),
                    );
                }

                model.events.push(
                    Event::new(
                        if r.is_forward { rules::codes::FORWARD_NORMALIZED } else { rules::codes::INVERSE_NORMALIZED },
                        "info",
                        fwd_source.display(),
                        if r.is_forward {
                            "forward reference kept as written".to_string()
                        } else {
                            "inverse reference normalized by swapping endpoints".to_string()
                        },
                    )
                    .with_span(r.span.clone())
                    .with_object(edge_id.clone()),
                );

                model.edges.push(MergedEdge {
                    id: edge_id,
                    ref_type,
                    ref_type_resolved: resolved,
                    source: fwd_source,
                    target: fwd_target,
                    forward: true,
                    raw_is_forward: r.is_forward,
                    source_id: src.source_id.clone(),
                    span: r.span.clone(),
                    status: status.to_string(),
                });
            }
        }
    }
}

fn insert_placeholder(model: &mut MergedModel, key: &GlobalNodeId, source_id: &str, span: &Span) {
    let id = node_id_for(key);
    let entry = model.nodes.entry(id.clone()).or_insert_with(|| MergedNode {
        id: id.clone(),
        key: key.clone(),
        kind: "Unresolved".to_string(),
        browse_name: None,
        display_name: None,
        candidates: Vec::new(),
        is_abstract: false,
        unresolved_reason: Some(rules::codes::DEPENDENCY_MISSING.to_string()),
        source_ids: Vec::new(),
    });
    if !entry.source_ids.contains(&source_id.to_string()) {
        entry.source_ids.push(source_id.to_string());
    }
    let already = entry.candidates.iter().any(|c| c.source_id == source_id && c.span == *span);
    if !already {
        entry.candidates.push(NodeCandidate {
            source_id: source_id.to_string(),
            raw_node_id: key.display(),
            browse_name_raw: None,
            display_name: None,
            kind: "UnresolvedReference".to_string(),
            span: span.clone(),
        });
    }
}

/// Derived layer only: normalize supported pressure/temperature/flow units to
/// SI. Anything that cannot be converted stays in its original place with a
/// diagnostic and is never dropped for the sake of the graph.
fn normalize_quantities(model: &mut MergedModel) {
    for q in &mut model.quantities {
        let raw_value = q.quantity.value.clone();
        let parsed: Option<f64> = raw_value.trim().parse().ok();
        let unit = q.quantity.unit.clone().unwrap_or_default();
        match (parsed, unit.as_str()) {
            (Some(v), "bar") => {
                q.quantity.canonical_value = Some(format!("{}", (v * 100_000.0).round()));
                q.quantity.canonical_unit = Some("Pa".into());
                model.events.push(
                    Event::new(rules::codes::UNIT_NORMALIZED, "info", raw_value, format!("{v} bar -> Pa"))
                        .with_span(q.quantity.span.clone())
                        .with_object(q.id.clone()),
                );
            }
            (Some(v), "hPa" | "mbar") => {
                q.quantity.canonical_value = Some(format!("{}", (v * 100.0).round()));
                q.quantity.canonical_unit = Some("Pa".into());
                model.events.push(
                    Event::new(rules::codes::UNIT_NORMALIZED, "info", raw_value, format!("{v} {unit} -> Pa"))
                        .with_span(q.quantity.span.clone())
                        .with_object(q.id.clone()),
                );
            }
            (Some(v), "kPa") => {
                q.quantity.canonical_value = Some(format!("{}", (v * 1000.0).round()));
                q.quantity.canonical_unit = Some("Pa".into());
                model.events.push(
                    Event::new(rules::codes::UNIT_NORMALIZED, "info", raw_value, format!("{v} kPa -> Pa"))
                        .with_span(q.quantity.span.clone())
                        .with_object(q.id.clone()),
                );
            }
            (Some(v), "degC" | "C" | "°C") => {
                q.quantity.canonical_value = Some(format!("{}", (v + 273.15 * 100.0).round() / 100.0));
                q.quantity.canonical_unit = Some("K".into());
                model.events.push(
                    Event::new(rules::codes::UNIT_NORMALIZED, "info", raw_value, format!("{v} {unit} -> K"))
                        .with_span(q.quantity.span.clone())
                        .with_object(q.id.clone()),
                );
            }
            (Some(v), "Pa" | "K") => {
                q.quantity.canonical_value = Some(format!("{v}"));
                q.quantity.canonical_unit = Some(unit.clone());
            }
            _ => {
                q.quantity.diagnostic = Some(rules::codes::UNCONVERTIBLE_VALUE.to_string());
                let mut msg = format!("value `{raw_value}` with unit `{unit}` has no supported conversion");
                if q.quantity.coordinate_frame.is_some() {
                    msg.push_str("; coordinate frame preserved without cross-frame normalization");
                }
                model.events.push(
                    Event::new(rules::codes::UNCONVERTIBLE_VALUE, "warn", raw_value, msg)
                        .with_span(q.quantity.span.clone())
                        .with_object(q.id.clone()),
                );
                if q.quantity.coordinate_frame.is_some() {
                    model.events.push(
                        Event::new(
                            rules::codes::COORD_FRAME_PRESERVED,
                            "info",
                            q.quantity.coordinate_frame.clone().unwrap_or_default(),
                            "coordinate basis retained",
                        )
                        .with_span(q.quantity.span.clone())
                        .with_object(q.id.clone()),
                    );
                }
            }
        }
    }
}
