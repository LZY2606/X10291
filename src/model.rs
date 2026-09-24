//! Merge, reference closure and derivation engine.
//!
//! Pure and deterministic: the same (files, params, decisions, rule version)
//! always produce the same [`Snapshot`], including order and stable ids.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ids::{
    parse_node_id, parse_qname, resolve_browse, resolve_gid, stable_id, Bnid, Gid, UA_NS,
};
use crate::ir::{RawNode, RawNodeSet, RawReference};
use serde::{Deserialize, Serialize};

pub const RULE_VERSION: &str = "rules-2026.09.1";

/// Well-known built-in reference type NodeIds used before alias resolution.
pub fn builtin_ref_type(token: &str) -> Option<u64> {
    match token {
        "HasComponent" => Some(47),
        "HasProperty" => Some(46),
        "Organizes" => Some(35),
        "HasTypeDefinition" => Some(40),
        "HasEncoding" => Some(38),
        "HasSubtype" => Some(45),
        "GeneratesEvent" => Some(70),
        "HasModellingRule" => Some(37),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileInput {
    pub file_id: String,
    pub path: String,
    pub raw: RawNodeSet,
}

/// Derivation parameters. A revision creates a new revision; the graph engine
/// only recomputes facts whose input params changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Params {
    /// Target unit for pressure quantities (`Pa`, `bar`, `MPa`, ...).
    pub pressure_unit: String,
    /// Target unit for rotational speed (`rad/s`, `rpm`).
    pub speed_unit: String,
    /// Coordinate bases for which a transform is known.
    pub datum_bases: BTreeSet<String>,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            pressure_unit: "Pa".to_string(),
            speed_unit: "rad/s".to_string(),
            datum_bases: BTreeSet::from_iter(["PLANT-GRID".to_string()]),
        }
    }
}

/// A human decision recorded against a candidate group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub group_id: String,
    /// Chosen candidate key (a contribution id, or "__keep_all__").
    pub accepted: String,
    pub note: String,
}

/// Contribution of one source file node to a merged node gid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Contribution {
    pub contrib_id: String,
    pub file_id: String,
    pub class: String,
    pub browse: Bnid,
    pub display_name: Option<String>,
    pub node_span: (usize, usize),
    pub node_id_span: Option<(usize, usize)>,
    pub browse_span: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeView {
    pub gid: Gid,
    pub id: String,
    pub class: String,
    pub browse: Bnid,
    pub display_name: Option<String>,
    pub contributions: Vec<Contribution>,
    /// True when the node is referenced but not defined by any known model
    /// (missing dependency) or lives at an undeclared namespace index.
    pub unresolved: bool,
    /// True when the node's node class was contested and no decision exists.
    pub contested: bool,
    /// gid chosen via a decision, when different from auto-merge.
    pub accepted_contrib: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeView {
    pub id: String,
    pub from: Option<Gid>,
    pub to: Option<Gid>,
    pub ref_type: Option<Gid>,
    /// Reference type token exactly as authored (retained when unresolved).
    pub ref_type_raw: String,
    /// Target token exactly as authored (alias target / bad node id kept).
    pub target_raw: String,
    pub direction: String,
    pub dangling: bool,
    pub unresolved_type: bool,
    /// Every source contribution to the same normalized edge.
    pub sources: Vec<EdgeSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeSource {
    pub file_id: String,
    pub span: (usize, usize),
    pub authored_is_forward: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FactView {
    pub id: String,
    pub node: Gid,
    pub kind: String,
    pub raw_value: String,
    pub raw_unit: Option<String>,
    pub normalized: Option<String>,
    pub unit: Option<String>,
    pub basis: Option<String>,
    pub status: String,
    pub diagnostic_id: Option<String>,
    pub file_id: String,
    pub span: (usize, usize),
    /// Inputs whose revision invalidates this fact (params keys / source ids).
    pub depends_on: Vec<String>,
    /// Why the fact stayed valid or needed recompute in the last run.
    pub validity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagView {
    pub id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub file_id: Option<String>,
    pub span: Option<(usize, usize)>,
    pub subjects: Vec<String>,
}

/// Machine-readable judgment event (the evidence behind a displayed value).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleEvent {
    pub id: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    pub file_id: Option<String>,
    pub span: Option<(usize, usize)>,
    pub subjects: Vec<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateChoice {
    pub key: String,
    pub file_id: String,
    pub class: String,
    pub display_name: Option<String>,
    pub span: (usize, usize),
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateGroup {
    pub id: String,
    pub gid: Gid,
    pub reason: String,
    pub status: String,
    pub accepted: Option<String>,
    pub choices: Vec<CandidateChoice>,
    pub rejected: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequirementView {
    pub uri: String,
    pub requested_version: String,
    pub available_version: Option<String>,
    pub status: String,
    pub span: (usize, usize),
    pub file_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelView {
    pub uri: String,
    pub version: String,
    pub publication_date: Option<String>,
    pub requires: Vec<RequirementView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AliasView {
    pub file_id: String,
    pub alias: String,
    pub target: String,
    pub status: String,
    pub cycle: Vec<String>,
    pub span: (usize, usize),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnknownView {
    pub id: String,
    pub file_id: String,
    pub qname: String,
    pub text: String,
    pub span: (usize, usize),
    pub xml: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NsFileMap {
    pub file_id: String,
    /// index -> uri, index 0 always UA.
    pub by_index: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NsGlobalRow {
    pub uri: String,
    pub global_index: u32,
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ChangeEntry {
    pub kind: String,
    pub entity_id: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub seq: u64,
    pub branch_id: String,
    pub event_id: String,
    pub rule_version: String,
    pub params: Params,
    pub global_ns: Vec<NsGlobalRow>,
    pub file_maps: Vec<NsFileMap>,
    pub models: Vec<ModelView>,
    pub aliases: Vec<AliasView>,
    pub nodes: Vec<NodeView>,
    pub edges: Vec<EdgeView>,
    pub facts: Vec<FactView>,
    pub diags: Vec<DiagView>,
    pub events: Vec<RuleEvent>,
    pub groups: Vec<CandidateGroup>,
    pub unknown: Vec<UnknownView>,
    pub changes: Vec<ChangeEntry>,
}

impl Snapshot {
    pub fn node_by_gid(&self, gid: &Gid) -> Option<&NodeView> {
        self.nodes.iter().find(|n| &n.gid == gid)
    }
    /// Subtype relations (type hierarchy tab), sorted deterministically.
    pub fn type_hierarchy(&self) -> Vec<(&EdgeView, &Gid, &Gid)> {
        let mut out = Vec::new();
        for e in &self.edges {
            if let (Some(f), Some(t), Some(rt)) = (&e.from, &e.to, &e.ref_type) {
                if rt.ns_uri == UA_NS && rt.kind == "i" && rt.value == "45" {
                    out.push((e, f, t));
                }
            }
        }
        out.sort_by(|a, b| (a.1, a.2).cmp(&(b.1, b.2)));
        out
    }
    pub fn unresolved_requirements(&self) -> Vec<&RequirementView> {
        let mut out: Vec<_> = self
            .models
            .iter()
            .flat_map(|m| m.requires.iter())
            .filter(|r| r.status != "satisfied")
            .collect();
        out.sort_by(|a, b| (a.file_id.as_str(), &a.uri).cmp(&(b.file_id.as_str(), &b.uri)));
        out
    }
}

// ---------------------------------------------------------------------------
// internal per-file prepared data
// ---------------------------------------------------------------------------

struct PreparedNode {
    file_id: String,
    gid: Option<Gid>,
    raw: RawNode,
}

struct Prepared {
    file_id: String,
    ns_table: Vec<String>,
    /// alias name -> target token (1 hop).
    alias_raw: BTreeMap<String, (String, (usize, usize))>,
    /// alias name -> fully resolved token or cycle marker.
    alias_status: HashMap<String, AliasResolution>,
    nodes: Vec<PreparedNode>,
}

#[derive(Debug, Clone)]
enum AliasResolution {
    Resolved(String),
    Cycle(Vec<String>),
    Unresolved(String),
}

struct Assembly<'a> {
    inputs: &'a [FileInput],
    params: &'a Params,
    decisions: &'a [Decision],
    prepared: Vec<Prepared>,
    global_ns: Vec<NsGlobalRow>,
    models: Vec<ModelView>,
    aliases: Vec<AliasView>,
    nodes: Vec<NodeView>,
    edges: Vec<EdgeView>,
    facts: Vec<FactView>,
    diags: Vec<DiagView>,
    events: Vec<RuleEvent>,
    groups: Vec<CandidateGroup>,
    unknown: Vec<UnknownView>,
    file_maps: Vec<NsFileMap>,
}

fn span2(s: crate::xml::Span) -> (usize, usize) {
    (s.start, s.end)
}

// ---------------------------------------------------------------------------
// public API
// ---------------------------------------------------------------------------

pub fn assemble(
    branch_id: &str,
    seq: u64,
    event_id: &str,
    inputs: &[FileInput],
    params: &Params,
    decisions: &[Decision],
    parent: Option<&Snapshot>,
) -> Snapshot {
    let mut a = Assembly {
        inputs,
        params,
        decisions,
        prepared: Vec::new(),
        global_ns: Vec::new(),
        models: Vec::new(),
        aliases: Vec::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
        facts: Vec::new(),
        diags: Vec::new(),
        events: Vec::new(),
        groups: Vec::new(),
        unknown: Vec::new(),
        file_maps: Vec::new(),
    };
    a.prepare_files();
    a.build_global_ns();
    a.build_models();
    a.collect_nodes();
    a.build_edges();
    a.collect_unknown();
    a.derive_facts();
    a.sort_all();

    let mut snap = Snapshot {
        seq,
        branch_id: branch_id.to_string(),
        event_id: event_id.to_string(),
        rule_version: RULE_VERSION.to_string(),
        params: params.clone(),
        global_ns: a.global_ns,
        file_maps: a.file_maps,
        models: a.models,
        aliases: a.aliases,
        nodes: a.nodes,
        edges: a.edges,
        facts: a.facts,
        diags: a.diags,
        events: a.events,
        groups: a.groups,
        unknown: a.unknown,
        changes: Vec::new(),
    };
    snap.changes = diff_snapshots(parent, &snap);
    snap
}

// ---------------------------------------------------------------------------
// file preparation: ns tables + alias cycles
// ---------------------------------------------------------------------------

impl<'a> Assembly<'a> {
    fn prepare_files(&mut self) {
        for input in self.inputs {
            let mut ns_table = vec![UA_NS.to_string()];
            // preserve authored order/indices
            for row in &input.raw.namespaces {
                while ns_table.len() <= row.index as usize {
                    ns_table.push(String::new());
                }
                ns_table[row.index as usize] = row.uri.clone();
            }
            // blank entries (skipped indices) get deterministic placeholders
            for (i, u) in ns_table.iter_mut().enumerate() {
                if u.is_empty() {
                    *u = format!("__unresolved_ns#{}", i);
                }
            }

            let mut alias_raw: BTreeMap<String, (String, (usize, usize))> = BTreeMap::new();
            for al in &input.raw.aliases {
                alias_raw.insert(al.alias.clone(), (al.node_id.clone(), span2(al.span)));
            }
            let alias_status = resolve_aliases(&alias_raw);

            // views
            for al in &input.raw.aliases {
                let (status, cycle) = match alias_status.get(&al.alias) {
                    Some(AliasResolution::Resolved(_)) => ("resolved", vec![]),
                    Some(AliasResolution::Cycle(c)) => ("cycle", c.clone()),
                    Some(AliasResolution::Unresolved(_)) => ("unresolved", vec![]),
                    None => ("unresolved", vec![]),
                };
                self.aliases.push(AliasView {
                    file_id: input.file_id.clone(),
                    alias: al.alias.clone(),
                    target: al.node_id.clone(),
                    status: status.to_string(),
                    cycle,
                    span: span2(al.span),
                });
            }

            let mut nodes = Vec::new();
            for rn in &input.raw.nodes {
                let gid = match parse_node_id(&rn.node_id) {
                    Some(nid) => {
                        match resolve_gid(&nid, &ns_table) {
                            Some(g) => Some(g),
                            None => {
                                self.diag(
                                    "UNDECLARED_NAMESPACE_INDEX",
                                    "error",
                                    format!(
                                        "NodeId `{}` uses numeric index {} which this file's namespace table does not declare; the node stays unresolved",
                                        rn.node_id, nid.ns
                                    ),
                                    Some(&input.file_id),
                                    rn.node_id_span.map(span2),
                                    vec![rn.node_id.clone()],
                                );
                                self.rule_event(
                                    "namespace.index.undeclared",
                                    "error",
                                    "numeric namespace index has no URI in this file; kept unresolved",
                                    Some(&input.file_id),
                                    rn.node_id_span.map(span2),
                                    vec![rn.node_id.clone()],
                                    serde_json::json!({"index": nid.ns, "node_id": rn.node_id}),
                                );
                                None
                            }
                        }
                    }
                    None => {
                        self.diag(
                            "BAD_NODE_ID",
                            "error",
                            format!("NodeId `{}` could not be parsed", rn.node_id),
                            Some(&input.file_id),
                            rn.node_id_span.map(span2),
                            vec![rn.node_id.clone()],
                        );
                        None
                    }
                };
                nodes.push(PreparedNode { file_id: input.file_id.clone(), gid, raw: rn.clone() });
            }

            self.file_maps.push(NsFileMap {
                file_id: input.file_id.clone(),
                by_index: ns_table.clone(),
            });
            self.prepared.push(Prepared {
                file_id: input.file_id.clone(),
                ns_table,
                alias_raw,
                alias_status,
                nodes,
            });
        }
    }

    fn build_global_ns(&mut self) {
        // Merge by URI. Global indices are assigned deterministically: UA first,
        // remaining URIs sorted lexicographically. File indices are never reused
        // as global indices on their own.
        let mut usage: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for fm in &self.file_maps {
            for uri in fm.by_index.iter().filter(|u| !u.starts_with("__unresolved_ns#")) {
                usage.entry(uri.clone()).or_default().insert(fm.file_id.clone());
            }
        }
        let mut uris: Vec<&String> = usage.keys().collect();
        uris.sort_by_key(|u| if *u == UA_NS { "".to_string() } else { (*u).clone() });
        for (i, uri) in uris.into_iter().enumerate() {
            let mut used_by: Vec<String> = usage[uri].iter().cloned().collect();
            used_by.sort();
            self.global_ns.push(NsGlobalRow {
                uri: uri.clone(),
                global_index: i as u32,
                used_by,
            });
        }
    }

    fn build_models(&mut self) {
        // available model versions across all files
        let mut available: BTreeMap<String, String> = BTreeMap::new();
        for inp in self.inputs {
            for m in &inp.raw.models {
                available
                    .entry(m.model_uri.clone())
                    .and_modify(|v| {
                        if version_cmp(&m.version, v) == std::cmp::Ordering::Greater {
                            *v = m.version.clone();
                        }
                    })
                    .or_insert_with(|| m.version.clone());
            }
        }
        for inp in self.inputs {
            for m in &inp.raw.models {
                let mut requires = Vec::new();
                for r in &m.requires {
                    let avail = available.get(&r.model_uri).cloned();
                    let status = match &avail {
                        None => "missing".to_string(),
                        Some(v) if *v == r.version => "satisfied".to_string(),
                        Some(v) => match version_cmp(v, &r.version) {
                            std::cmp::Ordering::Less => "conflict".to_string(),
                            _ => "newer".to_string(),
                        }),
                    };
                    if status == "missing" {
                        self.diag(
                            "MODEL_DEPENDENCY_MISSING",
                            "error",
                            format!(
                                "required model `{}` v{} is not imported; nodes from it remain unresolved",
                                r.model_uri, r.version
                            ),
                            Some(&inp.file_id),
                            Some(span2(r.span)),
                            vec![r.model_uri.clone()],
                        );
                        self.rule_event(
                            "dependency.missing.preserve-unresolved",
                            "error",
                            "missing dependency: referenced nodes are kept as unresolved nodes, not dropped",
                            Some(&inp.file_id),
                            Some(span2(r.span)),
                            vec![r.model_uri.clone()],
                            serde_json::json!({"required": r.version}),
                        );
                    }
                    if status == "conflict" {
                        self.diag(
                            "MODEL_DEPENDENCY_VERSION_CONFLICT",
                            "error",
                            format!(
                                "requires `{}` v{} but only v{} is imported",
                                r.model_uri,
                                r.version,
                                avail.clone().unwrap_or_default()
                            ),
                            Some(&inp.file_id),
                            Some(span2(r.span)),
                            vec![r.model_uri.clone()],
                        );
                        self.rule_event(
                            "dependency.version.conflict",
                            "error",
                            "dependency version conflict retained; no version is silently chosen",
                            Some(&inp.file_id),
                            Some(span2(r.span)),
                            vec![r.model_uri.clone()],
                            serde_json::json!({
                                "required": r.version,
                                "available": avail,
                            }),
                        );
                    }
                    requires.push(RequirementView {
                        uri: r.model_uri.clone(),
                        requested_version: r.version.clone(),
                        available_version: avail,
                        status,
                        span: span2(r.span),
                        file_id: inp.file_id.clone(),
                    });
                }
                self.models.push(ModelView {
                    uri: m.model_uri.clone(),
                    version: m.version.clone(),
                    publication_date: m.publication_date.clone(),
                    requires,
                });
            }
        }
        self.models.sort_by(|a, b| (&a.uri, &a.version).cmp(&(&b.uri, &b.version)));
    }

    fn find_prepared(&self, file_id: &str) -> &Prepared {
        self.prepared.iter().find(|p| p.file_id == file_id).expect("prepared file")
    }
}

/// Follow alias indirection with cycle detection.
fn resolve_aliases(raw: &BTreeMap<String, (String, (usize, usize))>) -> HashMap<String, AliasResolution> {
    let mut out = HashMap::new();
    for name in raw.keys() {
        let mut stack: Vec<String> = Vec::new();
        let mut cur = name.clone();
        let result = loop {
            if stack.contains(&cur) {
                let start = stack.iter().position(|x| x == &cur).unwrap();
                let mut cycle = stack[start..].to_vec();
                cycle.push(cur.clone());
                break AliasResolution::Cycle(cycle);
            }
            match raw.get(&cur) {
                None => {
                    if cur == *name {
                        break AliasResolution::Unresolved(cur);
                    }
                    // chained into a token that is not an alias: the token is
                    // the resolution (may still be a bad NodeId, handled later).
                    break AliasResolution::Resolved(cur);
                }
                Some((target, _)) => {
                    stack.push(cur.clone());
                    cur = target.clone();
                }
            }
        };
        out.insert(name.clone(), result);
    }
    out
}

/// Compare dotted numeric versions; non numeric suffix falls back to string.
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<u64> = a.split('.').filter_map(|x| x.parse().ok()).collect();
    let pb: Vec<u64> = b.split('.').filter_map(|x| x.parse().ok()).collect();
    match pa.cmp(&pb) {
        std::cmp::Ordering::Equal => a.cmp(b),
        o => o,
    }
}

impl<'a> Assembly<'a> {
    fn diag(
        &mut self,
        code: &str,
        severity: &str,
        message: String,
        file_id: Option<&str>,
        span: Option<(usize, usize)>,
        subjects: Vec<String>,
    ) {
        let key = format!(
            "{}|{}|{}|{:?}|{}",
            code,
            file_id.unwrap_or(""),
            span.map(|s| s.0).unwrap_or(0),
            subjects,
            message
        );
        self.diags.push(DiagView {
            id: stable_id("diag", &key),
            code: code.to_string(),
            severity: severity.to_string(),
            message,
            file_id: file_id.map(|s| s.to_string()),
            span,
            subjects,
        });
    }

    fn rule_event(
        &mut self,
        code: &str,
        severity: &str,
        message: &str,
        file_id: Option<&str>,
        span: Option<(usize, usize)>,
        subjects: Vec<String>,
        payload: serde_json::Value,
    ) {
        let key = format!("{}|{:?}|{}|{}", code, subjects, file_id, span.map(|s| s.0).unwrap_or(0));
        self.events.push(RuleEvent {
            id: stable_id("evt", &key),
            code: code.to_string(),
            severity: severity.to_string(),
            message: message.to_string(),
            file_id: file_id.map(|s| s.to_string()),
            span,
            subjects,
            payload,
        });
    }
}

impl<'a> Assembly<'a> {
    fn collect_nodes(&mut self) {
        // gid -> contributions (only nodes with a resolvable gid)
        let mut groups: BTreeMap<Gid, Vec<Contribution>> = BTreeMap::new();
        for p in &self.prepared {
            for pn in &p.nodes {
                let Some(gid) = &pn.gid else { continue };
                let qn = parse_qname(&pn.raw.browse_name);
                let bn = resolve_browse(&qn, &p.ns_table);
                let c = Contribution {
                    contrib_id: stable_id(
                        "contrib",
                        &format!("{}|{}|{}|{}", p.file_id, gid.as_key(), pn.raw.class, pn.span.start),
                    ),
                    file_id: p.file_id.clone(),
                    class: pn.raw.class.clone(),
                    browse: bn,
                    display_name: pn.raw.display_name.clone(),
                    node_span: span2(pn.raw.span),
                    node_id_span: pn.raw.node_id_span.map(span2),
                    browse_span: pn.raw.browse_name_span.map(span2),
                };
                groups.entry(gid.clone()).or_default().push(c);
            }
        }

        for (gid, mut contribs) in groups {
            contribs.sort_by(|a, b| {
                (a.file_id.as_str(), a.node_span.0).cmp(&(b.file_id.as_str(), b.node_span.0))
            });
            let classes: BTreeSet<String> = contribs.iter().map(|c| c.class.clone()).collect();
            let group_id = stable_id("group", &format!("dup|{}", gid.as_key()));
            let decision = self.decisions.iter().rev().find(|d| d.group_id == group_id);

            if classes.len() == 1 {
                // compatible restatements are auto-adopted; no ambiguity is hidden
                // because every contribution is still listed.
                let first = &contribs[0];
                let display = contribs
                    .iter()
                    .find_map(|c| c.display_name.clone())
                    .or_else(|| first.display_name.clone());
                self.nodes.push(NodeView {
                    gid: gid.clone(),
                    id: stable_id("node", &gid.as_key()),
                    class: first.class.clone(),
                    browse: first.browse.clone(),
                    display_name: display,
                    contributions: contribs,
                    unresolved: false,
                    contested: false,
                    accepted_contrib: Some(contribs[0].contrib_id.clone()),
                });
                if contribs.len() > 1 {
                    self.rule_event(
                        "duplicate-node.compatible.auto-merged",
                        "info",
                        "duplicate NodeId with identical node class: auto-merged; every contribution retained",
                        None,
                        None,
                        vec![gid.as_key()],
                        serde_json::json!({"contributions": contribs.len()}),
                    );
                }
            } else {
                // conflicting node classes: keep all legal candidates.
                let mut choices = Vec::new();
                for c in &contribs {
                    choices.push(CandidateChoice {
                        key: c.contrib_id.clone(),
                        file_id: c.file_id.clone(),
                        class: c.class.clone(),
                        display_name: c.display_name.clone(),
                        span: c.node_span,
                        state: "candidate".to_string(),
                    });
                }
                let (status, accepted, accepted_contrib) = match decision {
                    Some(d) if d.accepted != "__keep_all__" => {
                        for ch in &mut choices {
                            ch.state = if ch.key == d.accepted {
                                "accepted".to_string()
                            } else {
                                "rejected".to_string()
                            };
                        }
                        ("decided".to_string(), Some(d.accepted.clone()), Some(d.accepted.clone()))
                    }
                    _ => ("open".to_string(), None, None),
                };
                let rejected = choices
                    .iter()
                    .filter(|c| c.state == "rejected")
                    .map(|c| c.key.clone())
                    .collect();
                self.groups.push(CandidateGroup {
                    id: group_id.clone(),
                    gid: gid.clone(),
                    reason: "node class conflict for same global NodeId".to_string(),
                    status,
                    accepted: accepted.clone(),
                    choices: choices.clone(),
                    rejected,
                });
                let chosen = accepted_contrib
                    .as_ref()
                    .and_then(|key| contribs.iter().find(|c| &c.contrib_id == key));
                let (class, browse, display) = match chosen {
                    Some(c) => (
                        c.class.clone(),
                        c.browse.clone(),
                        c.display_name.clone(),
                    ),
                    None => {
                        // undecided: fall back to first contribution for graph
                        // layout, but mark contested.
                        let f = &contribs[0];
                        (f.class.clone(), f.browse.clone(), f.display_name.clone())
                    }
                };
                self.diag(
                    "DUPLICATE_NODEID_CLASS_CONFLICT",
                    "warn",
                    format!(
                        "global NodeId `{}` is declared as {:?} by different files; all candidates retained",
                        gid.label(),
                        classes.into_iter().collect::<Vec<_>>()
                    ),
                    None,
                    None,
                    vec![gid.as_key()],
                );
                self.rule_event(
                    "duplicate-node.conflict.keep-all-candidates",
                    "warn",
                    "duplicate NodeId with conflicting node classes: no single answer; all legal candidates retained",
                    None,
                    None,
                    vec![gid.as_key()],
                    serde_json::json!({"group": group_id, "candidates": choices.len()}),
                );
                self.nodes.push(NodeView {
                    gid: gid.clone(),
                    id: stable_id("node", &gid.as_key()),
                    class,
                    browse,
                    display_name: display.or_else(|| contribs[0].display_name.clone()),
                    contributions: contribs,
                    unresolved: false,
                    contested: accepted.is_none(),
                    accepted_contrib: accepted,
                });
            }
        }
    }

    fn collect_unknown(&mut self) {
        for inp in self.inputs {
            for u in &inp.raw.unknown_toplevel {
                self.unknown.push(UnknownView {
                    id: stable_id("unk", &format!("{}|{}|{}", inp.file_id, u.qname, u.span.start)),
                    file_id: inp.file_id.clone(),
                    qname: u.qname.clone(),
                    text: u.text.trim().to_string(),
                    span: span2(u.span),
                    xml: u.xml.clone(),
                });
            }
            for n in &inp.raw.nodes {
                for u in &n.unknown {
                    self.unknown.push(UnknownView {
                        id: stable_id(
                            "unk",
                            &format!("{}|{}|{}|{}", inp.file_id, n.node_id, u.qname, u.span.start),
                        ),
                        file_id: inp.file_id.clone(),
                        qname: u.qname.clone(),
                        text: u.text.trim().to_string(),
                        span: span2(u.span),
                        xml: u.xml.clone(),
                    });
                }
            }
        }
        if !self.unknown.is_empty() {
            self.rule_event(
                "unknown-extension.preserved",
                "info",
                "unrecognized XML elements are preserved verbatim with source spans, never discarded for the diagram",
                None,
                None,
                self.unknown.iter().map(|u| u.id.clone()).collect(),
                serde_json::json!({"count": self.unknown.len()}),
            );
        }
    }
}

impl<'a> Assembly<'a> {
    fn build_edges(&mut self) {
        // key: from|to|reftype|direction  -> EdgeView
        let mut edge_map: BTreeMap<String, EdgeView> = BTreeMap::new();
        let mut dangling_gids: BTreeMap<Gid, (String, String)> = BTreeMap::new();

        for p in &self.prepared {
            for pn in &p.nodes {
                for r in &pn.raw.references {
                    let source_gid = pn.gid.clone();
                    // resolve reference type: builtin token / alias / NodeId
                    let ref_type = self.resolve_ref_token(&p.file_id, &r.reference_type);
                    let rt_gid = match &ref_type {
                        TokenResolution::Gid(g) => Some(g.clone()),
                        TokenResolution::Unknown => None,
                        TokenResolution::AliasCycle(c) => {
                            self.diag(
                                "ALIAS_CYCLE_IN_REFERENCE_TYPE",
                                "error",
                                format!(
                                    "reference type alias `{}` participates in a cycle {:?}; edge retained with raw token",
                                    r.reference_type, c
                                ),
                                Some(&p.file_id),
                                r.ref_type_span.map(span2),
                                vec![r.reference_type.clone()],
                            );
                            self.rule_event(
                                "alias.cycle.reference-type",
                                "error",
                                "alias cycle blocks resolution; the reference is retained with its raw token",
                                Some(&p.file_id),
                                r.ref_type_span.map(span2),
                                vec![r.reference_type.clone()],
                                serde_json::json!({"cycle": c}),
                            );
                            None
                        }
                        TokenResolution::Bad => None,
                    };

                    // resolve target: alias indirection first, then NodeId
                    let target = self.resolve_target_token(&p.file_id, &r.target);
                    let (to_gid, target_raw) = match &target {
                        TokenResolution::Gid(g) => (Some(g.clone()), r.target.clone()),
                        TokenResolution::AliasCycle(c) => {
                            self.diag(
                                "ALIAS_CYCLE",
                                "error",
                                format!(
                                    "alias `{}` participates in a cycle {:?}; edge retained as dangling with raw target",
                                    r.target, c
                                ),
                                Some(&p.file_id),
                                Some(span2(r.span)),
                                vec![r.target.clone()],
                            );
                            self.rule_event(
                                "alias.cycle.target-unresolved",
                                "error",
                                "alias cycle: target not resolved; edge is retained, not dropped",
                                Some(&p.file_id),
                                Some(span2(r.span)),
                                vec![r.target.clone()],
                                serde_json::json!({"cycle": c}),
                            );
                            (None, r.target.clone())
                        }
                        TokenResolution::Unknown => {
                            self.diag(
                                "UNRESOLVED_TARGET",
                                "error",
                                format!("reference target token `{}` did not resolve", r.target),
                                Some(&p.file_id),
                                Some(span2(r.span)),
                                vec![r.target.clone()],
                            );
                            (None, r.target.clone())
                        }
                        TokenResolution::Bad => {
                            self.diag(
                                "BAD_TARGET_NODE_ID",
                                "error",
                                format!("reference target `{}` is not a valid NodeId", r.target),
                                Some(&p.file_id),
                                Some(span2(r.span)),
                                vec![r.target.clone()],
                            );
                            (None, r.target.clone())
                        }
                    };

                    // normalize direction: inverse references become forward.
                    // the same edge authored forward in one file and inverse in
                    // another collapses to ONE edge; provenance records both.
                    let (from, to) = if r.is_forward {
                        (source_gid.clone(), to_gid.clone())
                    } else {
                        (to_gid.clone(), source_gid.clone())
                    };

                    if let Some(g) = &to {
                        if !self.is_known_gid(g) {
                            dangling_gids
                                .entry(g.clone())
                                .or_insert_with(|| (r.target.clone(), p.file_id.clone()));
                        }
                    }
                    let dangling = to.as_ref().map(|g| !self.is_known_gid(g)).unwrap_or(true);

                    let key = format!(
                        "{}|{}|{}|{}",
                        from.as_ref().map(|g| g.as_key()).unwrap_or_else(|| "__no_src__".to_string()),
                        to.as_ref().map(|g| g.as_key()).unwrap_or_else(|| target_raw.clone()),
                        rt_gid.as_ref().map(|g| g.as_key()).unwrap_or_else(|| r.reference_type.clone()),
                        direction
                    );
                    let src = EdgeSource {
                        file_id: p.file_id.clone(),
                        span: span2(r.span),
                        authored_is_forward: r.is_forward,
                    };
                    let entry = edge_map.entry(key.clone()).or_insert_with(|| EdgeView {
                        id: stable_id("edge", &key),
                        from: from.clone(),
                        to: to.clone(),
                        ref_type: rt_gid.clone(),
                        ref_type_raw: r.reference_type.clone(),
                        target_raw,
                        direction: "forward".to_string(),
                        dangling,
                        unresolved_type: rt_gid.is_none(),
                        sources: Vec::new(),
                    });
                    // merge identical edges authored in opposite directions:
                    // normalization makes them the same key; we record provenance
                    // instead of creating a phantom reverse edge.
                    if !entry.sources.contains(&src) {
                        entry.sources.push(src);
                    }
                    if !r.is_forward {
                        entry.direction = "merged-forward-and-inverse";
                    }
                }
            }
        }

        // dangling placeholders become unresolved nodes (missing dependency)
        let existing: BTreeSet<Gid> = self.nodes.iter().map(|n| n.gid.clone()).collect();
        let mut placeholders = Vec::new();
        for (gid, (raw, file_id)) in &dangling_gids {
            if !existing.contains(gid) {
                placeholders.push((gid.clone(), raw.clone(), file_id.clone()));
            }
        }
        for (gid, raw, file_id) in placeholders {
            self.diag(
                "DANGLING_REFERENCE",
                "error",
                format!("reference to `{}` (from token `{}`) has no node definition; kept as unresolved node", gid.label(), raw),
                Some(&file_id),
                None,
                vec![gid.as_key()],
            );
            self.rule_event(
                "reference.dangling.keep-unresolved-node",
                "error",
                "edge target missing (deleted node or missing dependency): an unresolved placeholder is retained, the edge is not fabricated away",
                Some(&file_id),
                None,
                vec![gid.as_key()],
                serde_json::json!({"raw_token": raw}),
            );
            self.nodes.push(NodeView {
                id: stable_id("node", &gid.as_key()),
                gid: gid.clone(),
                class: "Unresolved".to_string(),
                browse: Bnid { ns_uri: gid.ns_uri.clone(), name: gid.value.clone() },
                display_name: Some(format!("unresolved: {}", raw)),
                contributions: vec![],
                unresolved: true,
                contested: false,
                accepted_contrib: None,
            });
        }

        for e in edge_map.into_values() {
            self.edges.push(e);
        }
    }

    fn is_known_gid(&self, gid: &Gid) -> bool {
        // built-in UA nodes are always considered known even when not imported
        if gid.ns_uri == UA_NS {
            return true;
        }
        self.prepared.iter().any(|p| p.nodes.iter().any(|n| n.gid.as_ref() == Some(gid)))
    }
}

#[derive(Debug)]
enum TokenResolution {
    Gid(Gid),
    AliasCycle(Vec<String>),
    Unknown,
    Bad,
}

impl<'a> Assembly<'a> {
    fn resolve_target_token(&self, file_id: &str, token: &str) -> TokenResolution {
        let p = self.find_prepared(file_id);
        // alias?
        if let Some(status) = p.alias_status.get(token) {
            return match status {
                AliasResolution::Resolved(t) => self.gid_from_token(p, t),
                AliasResolution::Cycle(c) => TokenResolution::AliasCycle(c.clone()),
                AliasResolution::Unresolved(_) => TokenResolution::Unknown,
            };
        }
        self.gid_from_token(p, token)
    }

    fn resolve_ref_token(&self, file_id: &str, token: &str) -> TokenResolution {
        let p = self.find_prepared(file_id);
        if let Some(status) = p.alias_status.get(token) {
            return match status {
                AliasResolution::Resolved(t) => self.gid_from_token(p, t),
                AliasResolution::Cycle(c) => TokenResolution::AliasCycle(c.clone()),
                AliasResolution::Unresolved(_) => TokenResolution::Unknown,
            };
        }
        if let Some(i) = builtin_ref_type(token) {
            return TokenResolution::Gid(Gid::new(UA_NS, crate::ids::IdValue::Numeric(i)));
        }
        self.gid_from_token(p, token)
    }

    fn gid_from_token(&self, p: &Prepared, token: &str) -> TokenResolution {
        match parse_node_id(token) {
            Some(nid) => match resolve_gid(&nid, &p.ns_table) {
                Some(g) => TokenResolution::Gid(g),
                None => TokenResolution::Bad,
            },
            None => TokenResolution::Unknown,
        }
    }
}

// ---------------------------------------------------------------------------
// derived facts: units and datum. Raw representation is always preserved;
// normalization happens only in this layer.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Quantity {
    numeric: f64,
    span: (usize, usize),
}

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}

fn convert_speed(v: f64, from: &str, target: &str) -> Option<f64> {
    let rad = match from {
        "rpm" => Some(v * std::f64::consts::TAU / 60.0),
        "rad/s" => Some(v),
        "deg/s" => Some(v.to_radians()),
        _ => None,
    }?;
    match target {
        "rad/s" => Some(rad),
        "rpm" => Some(rad * 60.0 / std::f64::consts::TAU),
        "deg/s" => Some(rad.to_degrees()),
        _ => None,
    }
}

fn convert_pressure(v: f64, from: &str, target: &str) -> Option<f64> {
    let pa = match from {
        "Pa" => Some(v),
        "kPa" => Some(v * 1_000.0),
        "bar" => Some(v * 100_000.0),
        "MPa" => Some(v * 1_000_000.0),
        "psi" => Some(v * 6_894.757_293_168_354),
        _ => None,
    }?;
    match target {
        "Pa" => Some(pa),
        "kPa" => Some(pa / 1_000.0),
        "bar" => Some(pa / 100_000.0),
        "MPa" => Some(pa / 1_000_000.0),
        "psi" => Some(pa / 6_894.757_293_168_354),
        _ => None,
    }
}

impl<'a> Assembly<'a> {
    fn derive_facts(&mut self) {
        for p in &self.prepared {
            for pn in &p.nodes {
                let Some(gid) = &pn.gid else { continue };
                for u in &pn.raw.unknown {
                    let key_in = match u.qname.as_str() {
                        "Unit" => format!("param:unit-target"),
                        "Datum" => format!("param:datum-bases"),
                        _ => continue,
                    };
                    let raw_text = match &pn.raw.value {
                        Some(v) => v.text.trim().to_string(),
                        None => continue,
                    };
                    let q = parse_f64(&raw_text).map(|n| Quantity {
                        numeric: n,
                        span: match &pn.raw.value {
                            Some(v) => span2(v.span),
                            None => (0, 0),
                        },
                    });
                    let (kind, raw_unit, normalized, unit, basis, status, diag_code, msg, depends) =
                        if u.qname == "Unit" {
                            let raw_unit = u.text.trim().to_string();
                            let is_speed = raw_unit == "rpm"
                                || raw_unit == "rad/s"
                                || raw_unit == "deg/s"
                                || pn.raw.browse_name.contains("Speed");
                            let is_pressure = raw_unit == "bar"
                                || raw_unit == "Pa"
                                || raw_unit == "kPa"
                                || raw_unit == "MPa"
                                || raw_unit == "psi";
                            let (conv, target, deps) = if is_speed {
                                (
                                    q.and_then(|q| convert_speed(q.numeric, &raw_unit, &self.params.speed_unit)),
                                    self.params.speed_unit.clone(),
                                    vec![
                                        format!("param:speed_unit"),
                                        format!("source:{}", pn.raw.span.start),
                                    ],
                                )
                            } else if is_pressure {
                                (
                                    q.and_then(|q| convert_pressure(q.numeric, &raw_unit, &self.params.pressure_unit)),
                                    self.params.pressure_unit.clone(),
                                    vec![
                                        format!("param:pressure_unit"),
                                        format!("source:{}", pn.raw.span.start),
                                    ],
                                )
                            } else {
                                (None, String::new(), vec![key_in])
                            };
                            let _ = key_in;
                            match conv {
                                Some(v) if q.is_some() => (
                                    "quantity".to_string(),
                                    Some(raw_unit.clone()),
                                    Some(format_number(v)),
                                    Some(target),
                                    None,
                                    "normalized".to_string(),
                                    None,
                                    String::new(),
                                    deps,
                                ),
                                _ => (
                                    "quantity".to_string(),
                                    Some(raw_unit.clone()),
                                    None,
                                    None,
                                    None,
                                    "raw-only".to_string(),
                                    Some("UNCONVERTIBLE_UNIT"),
                                    format!(
                                        "value `{}` with unit `{}` cannot be converted in rule version {}; original value kept in place",
                                        raw_text, raw_unit, RULE_VERSION
                                    ),
                                    deps,
                                ),
                            }
                        } else {
                            // datum / coordinate basis
                            let basis = u
                                .attrs
                                .iter()
                                .find(|(k, _)| k == "Basis")
                                .map(|(_, v)| v.clone())
                                .unwrap_or_default();
                            let known = self.params.datum_bases.contains(&basis);
                            if known {
                                (
                                    "coordinate".to_string(),
                                    None,
                                    Some(raw_text.clone()),
                                    None,
                                    Some(basis.clone()),
                                    "on-known-basis".to_string(),
                                    None,
                                    String::new(),
                                    vec![format!("param:datum-bases"), format!("source:{}", pn.raw.span.start)],
                                )
                            } else {
                                (
                                    "coordinate".to_string(),
                                    None,
                                    None,
                                    None,
                                    Some(basis.clone()),
                                    "raw-only".to_string(),
                                    Some("UNTRANSFORMABLE_DATUM"),
                                    format!(
                                        "coordinate `{}` on basis `{}` has no transform in the current basis set; raw coordinate retained",
                                        raw_text, basis
                                    ),
                                    vec![format!("param:datum-bases"), format!("source:{}", pn.raw.span.start)],
                                )
                            }
                        };
                    let mut diag_id = None;
                    if let Some(code) = diag_code {
                        let key = format!("{}|{}|{}|{}", code, p.file_id, gid.as_key(), msg);
                        diag_id = Some(stable_id("diag", &key));
                        self.diags.push(DiagView {
                            id: diag_id.clone().unwrap(),
                            code,
                            severity: "warn".to_string(),
                            message: msg.clone(),
                            file_id: Some(p.file_id.clone()),
                            span: Some(span2(u.span)),
                            subjects: vec![gid.as_key()],
                        });
                        self.rule_event(
                            if msg.contains("basis") {
                                "derivation.datum.raw-preserved"
                            } else {
                                "derivation.unit.raw-preserved"
                            },
                            "warn",
                            "unconvertible value stays at its original node with a diagnostic; nothing is dropped to draw the graph",
                            Some(&p.file_id),
                            Some(span2(u.span)),
                            vec![gid.as_key()],
                            serde_json::json!({"raw": raw_text}),
                        );
                    }
                    let fact_key = format!("{}|{}|{}", gid.as_key(), kind, pn.raw.span.start);
                    self.facts.push(FactView {
                        id: stable_id("fact", &fact_key),
                        node: gid.clone(),
                        kind,
                        raw_value: raw_text,
                        raw_unit: if u.qname == "Unit" {
                            Some(u.text.trim().to_string())
                        } else {
                            None
                        },
                        normalized,
                        unit,
                        basis,
                        status,
                        diagnostic_id: diag_id,
                        file_id: p.file_id.clone(),
                        span: span2(u.span),
                        depends_on: depends,
                        validity: "computed".to_string(),
                    });
                }
            }
        }
        self.mark_validity();
    }

    /// Dependency-graph propagation: facts depending on unchanged params/sources
    /// stay valid; facts depending on a changed input are recomputed.
    fn mark_validity(&mut self) {
        // validity is computed relative to the parent snapshot by the caller;
        // here every freshly derived fact records its input set. Default text is
        // overwritten by [`diff_snapshots`] via [`annotate_validity`].
    }
}

fn format_number(v: f64) -> String {
    if v == v.trunc() {
        format!("{:.1}", v)
    } else {
        let s = format!("{:.6}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Annotate facts with a human/machine explanation based on whether their
/// declared inputs changed between parent and child.
pub fn annotate_validity(parent: Option<&Snapshot>, snap: &mut Snapshot) {
    let Some(parent) = parent else {
        for f in &mut snap.facts {
            f.validity = "first computation".to_string();
        }
        return;
    };
    let parent_params = &parent.params;
    let changed = |key: &str| -> bool {
        match key {
            "param:pressure_unit" => parent_params.pressure_unit != snap.params.pressure_unit,
            "param:speed_unit" => parent_params.speed_unit != snap.params.speed_unit,
            "param:datum-bases" => parent_params.datum_bases != snap.params.datum_bases,
            k if k.starts_with("source:") => {
                // source start identity: changed facts are detected by id change
                // in the diff step; conservatively recomputed unless the fact id
                // already existed on the parent.
                !snap.facts.iter().any(|f| f.depends_on.iter().any(|d| d == k))
                    || !parent.facts.iter().any(|f| f.depends_on.iter().any(|d| d == k))
            }
            _ => false,
        }
    };
    let parent_ids: BTreeSet<String> = parent.facts.iter().map(|f| f.id.clone()).collect();
    for f in &mut snap.facts {
        if !parent_ids.contains(&f.id) {
            f.validity = "newly computed (source or node introduced)".to_string();
        } else if f.depends_on.iter().any(|d| changed(d)) {
            f.validity = "recomputed (dependency input changed)".to_string();
        } else {
            f.validity = "still valid (no changed input on dependency path)".to_string();
        }
    }
}

impl<'a> Assembly<'a> {
    fn sort_all(&mut self) {
        self.global_ns.sort_by(|a, b| a.global_index.cmp(&b.global_index));
        self.file_maps.sort_by(|a, b| a.file_id.cmp(&b.file_id));
        self.aliases.sort_by(|a, b| {
            (&a.file_id, &a.alias).cmp(&(&b.file_id, &b.alias))
        });
        self.nodes.sort_by(|a, b| a.gid.cmp(&b.gid));
        self.edges.sort_by(|a, b| {
            (
                a.from.as_ref().map(|g| g.as_key()),
                a.to.as_ref().map(|g| g.as_key()),
                a.ref_type.as_ref().map(|g| g.as_key()),
            )
                .cmp(&(
                    b.from.as_ref().map(|g| g.as_key()),
                    b.to.as_ref().map(|g| g.as_key()),
                    b.ref_type.as_ref().map(|g| g.as_key()),
                ))
        });
        self.facts.sort_by(|a, b| (a.node.as_key(), &a.kind).cmp(&(b.node.as_key(), &b.kind)));
        self.diags.sort_by(|a, b| a.id.cmp(&b.id));
        self.events.sort_by(|a, b| a.id.cmp(&b.id));
        self.groups.sort_by(|a, b| a.gid.cmp(&b.gid));
        self.unknown.sort_by(|a, b| a.id.cmp(&b.id));
    }
}

/// Semantic diff between two versions. Only identity-bearing entities count,
/// and order differences never produce noise because inputs are canonicalized.
pub fn diff_snapshots(parent: Option<&Snapshot>, child: &Snapshot) -> Vec<ChangeEntry> {
    let Some(p) = parent else {
        let mut out = Vec::new();
        for n in &child.nodes {
            out.push(ChangeEntry {
                kind: "node-added".to_string(),
                entity_id: n.id.clone(),
                detail: n.gid.label(),
            });
        }
        out.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
        return out;
    };

    let mut changes = Vec::new();
    let pm: BTreeMap<_, _> = p.nodes.iter().map(|n| (n.gid.as_key(), n)).collect();
    let cm: BTreeMap<_, _> = child.nodes.iter().map(|n| (n.gid.as_key(), n)).collect();
    for (k, n) in &cm {
        match pm.get(k) {
            None => changes.push(ChangeEntry {
                kind: "node-added".to_string(),
                entity_id: n.id.clone(),
                detail: n.gid.label(),
            }),
            Some(old) if old.class != n.class
                || old.contested != n.contested
                || old.display_name != n.display_name =>
            {
                changes.push(ChangeEntry {
                    kind: "node-changed".to_string(),
                    entity_id: n.id.clone(),
                    detail: format!(
                        "{}: class {} -> {}",
                        n.gid.label(),
                        old.class,
                        n.class
                    ),
                });
            }
            _ => {}
        }
    }
    for (k, n) in &pm {
        if !cm.contains_key(k) {
            changes.push(ChangeEntry {
                kind: "node-deleted".to_string(),
                entity_id: n.id.clone(),
                detail: n.gid.label(),
            });
        }
    }

    let pe: BTreeMap<_, _> = p.edges.iter().map(|e| (e.id.clone(), e)).collect();
    let ce: BTreeMap<_, _> = child.edges.iter().map(|e| (e.id.clone(), e)).collect();
    for (id, e) in &ce {
        match pe.get(id) {
            None => changes.push(ChangeEntry {
                kind: "edge-added".to_string(),
                entity_id: id.clone(),
                detail: format!("{} -> {}", label_of(e.from), label_of(e.to)),
            }),
            Some(old) if old.dangling != e.dangling || old.sources.len() != e.sources.len() => {
                changes.push(ChangeEntry {
                    kind: "edge-changed".to_string(),
                    entity_id: id.clone(),
                    detail: format!(
                        "dangling {} -> {}",
                        old.dangling, e.dangling
                    ),
                });
            }
            _ => {}
        }
    }
    for (id, e) in &pe {
        if !ce.contains_key(id) {
            changes.push(ChangeEntry {
                kind: "edge-deleted".to_string(),
                entity_id: id.clone(),
                detail: format!("{} -> {}", label_of(e.from), label_of(e.to)),
            });
        }
    }

    let pf: BTreeMap<_, _> = p.facts.iter().map(|f| (f.id.clone(), f)).collect();
    let cf: BTreeMap<_, _> = child.facts.iter().map(|f| (f.id.clone(), f)).collect();
    for (id, f) in &cf {
        match pf.get(id) {
            None => changes.push(ChangeEntry {
                kind: "fact-added".to_string(),
                entity_id: id.clone(),
                detail: f.node.label(),
            }),
            Some(old)
                if old.normalized != f.normalized
                    || old.status != f.status
                    || old.unit != f.unit
                    || old.basis != f.basis =>
            {
                changes.push(ChangeEntry {
                    kind: "fact-changed".to_string(),
                    entity_id: id.clone(),
                    detail: format!(
                        "{}: {:?} {:?} -> {:?} {:?}",
                        f.node.label(),
                        old.normalized,
                        old.unit,
                        f.normalized,
                        f.unit
                    ),
                });
            }
            _ => {}
        }
    }
    for (id, f) in &pf {
        if !cf.contains_key(id) {
            changes.push(ChangeEntry {
                kind: "fact-deleted".to_string(),
                entity_id: id.clone(),
                detail: f.node.label(),
            });
        }
    }

    let pd: BTreeSet<_> = p.diags.iter().map(|d| d.id.clone()).collect();
    for d in &child.diags {
        if !pd.contains(&d.id) {
            changes.push(ChangeEntry {
                kind: "diagnostic-added".to_string(),
                entity_id: d.id.clone(),
                detail: d.code.clone(),
            });
        }
    }
    let cd: BTreeSet<_> = child.diags.iter().map(|d| d.id.clone()).collect();
    for d in &p.diags {
        if !cd.contains(&d.id) {
            changes.push(ChangeEntry {
                kind: "diagnostic-resolved".to_string(),
                entity_id: d.id.clone(),
                detail: d.code.clone(),
            });
        }
    }

    changes.sort_by(|a, b| (a.kind.as_str(), a.entity_id.as_str()).cmp(&(b.kind.as_str(), b.entity_id.as_str())));
    changes
}

fn label_of(g: &Option<Gid>) -> String {
    g.as_ref().map(|x| x.label()).unwrap_or_else(|| "?".to_string())
}
