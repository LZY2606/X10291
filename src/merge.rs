//! Merge engine: remap by URI, normalize references, keep uncertainty.

use crate::model::*;
use crate::xml_import::parse_local_node_id;
use std::collections::{BTreeMap, BTreeSet};

pub const CORE_URI: &str = "http://opcfoundation.org/UA/";

pub fn merge(files: &[NodeSetFile]) -> MergedModel {
    let mut m = MergedModel::default();
    // 1. Global namespace table, deterministic first-seen order, core first.
    m.namespace_uris.push(CORE_URI.to_string());
    for f in files {
        for uri in &f.namespaces {
            if !m.namespace_uris.contains(uri) {
                m.namespace_uris.push(uri.clone());
            }
        }
    }
    let uri_of = |f: &NodeSetFile, ns: u16| -> Option<String> {
        if ns == 0 {
            Some(CORE_URI.to_string())
        } else {
            f.namespaces.get(ns as usize).cloned()
        }
    };

    // 2. Alias resolution per file, with cycle detection.
    let mut alias_maps: BTreeMap<&str, BTreeMap<String, Option<LocalNodeId>>> = BTreeMap::new();
    for f in files {
        let raw: BTreeMap<&str, &str> = f
            .aliases
            .iter()
            .map(|a| (a.alias.as_str(), a.target_text.as_str()))
            .collect();
        let mut resolved: BTreeMap<String, Option<LocalNodeId>> = BTreeMap::new();
        for a in &f.aliases {
            let mut chain = vec![a.alias.clone()];
            let mut cur: &str = &a.target_text;
            let mut value = parse_local_node_id(cur);
            while value.is_none() {
                // target text is not a NodeId literal: treat as another alias.
                match raw.get(cur) {
                    Some(next) => {
                        if chain.iter().any(|c| c == cur) {
                            chain.push(cur.to_string());
                            m.diagnostics.push(Diagnostic::AliasCycle {
                                file_id: f.file_id.clone(),
                                chain,
                            });
                            break;
                        }
                        chain.push(cur.to_string());
                        cur = next;
                        value = parse_local_node_id(cur);
                    }
                    None => {
                        m.diagnostics.push(Diagnostic::UnresolvableAlias {
                            file_id: f.file_id.clone(),
                            alias: a.alias.clone(),
                        });
                        break;
                    }
                }
            }
            resolved.insert(a.alias.clone(), value);
        }
        alias_maps.insert(&f.file_id, resolved);
    }

    let resolve_text = |f: &NodeSetFile, text: &str| -> Option<LocalNodeId> {
        if let Some(id) = parse_local_node_id(text) {
            return Some(id);
        }
        alias_maps
            .get(f.file_id.as_str())
            .and_then(|am| am.get(text).cloned())
            .flatten()
    };

    // 3. Nodes: remap to URI identity. Duplicates kept as candidates.
    for f in files {
        for n in &f.nodes {
            let Some(local) = resolve_text(f, &n.node_id_text) else {
                m.diagnostics.push(Diagnostic::UnresolvableAlias {
                    file_id: f.file_id.clone(),
                    alias: n.node_id_text.clone(),
                });
                continue;
            };
            let Some(ns_uri) = uri_of(f, local.ns) else {
                m.diagnostics.push(Diagnostic::UnknownNamespaceIndex {
                    file_id: f.file_id.clone(),
                    ns: local.ns,
                    context: n.node_id_text.clone(),
                });
                continue;
            };
            let gid = GlobalNodeId { ns_uri: ns_uri.clone(), id: local.id.clone() };
            let gbn = GlobalQName {
                ns_uri: uri_of(f, n.browse_name.ns).unwrap_or(CORE_URI.to_string()),
                name: n.browse_name.name.clone(),
            };
            let mn = MergedNode {
                id: gid.clone(),
                node_class: n.node_class,
                browse_name: gbn,
                display_name: n.display_name.clone(),
                attrs: n.attrs.clone(),
                extensions: n.extensions.clone(),
                origin_file: f.file_id.clone(),
                span: n.span.clone(),
            };
            let sid = stable_id(&gid);
            if m.nodes.contains_key(&sid) {
                m.duplicates.entry(sid.clone()).or_default().push(mn);
                let mut files: Vec<String> = vec![m.nodes[&sid].origin_file.clone()];
                files.extend(m.duplicates[&sid].iter().map(|d| d.origin_file.clone()));
                m.diagnostics.push(Diagnostic::DuplicateNodeId { id: gid, files });
            } else {
                m.nodes.insert(sid, mn);
            }
        }
    }

    // 4. Edges: canonical forward orientation, dedupe, no fabricated edges.
    let mut seen: BTreeSet<(StableId, StableId, StableId)> = BTreeSet::new();
    for f in files {
        for n in &f.nodes {
            let Some(src_local) = resolve_text(f, &n.node_id_text) else { continue };
            let Some(src_uri) = uri_of(f, src_local.ns) else { continue };
            let src = GlobalNodeId { ns_uri: src_uri, id: src_local.id };
            for r in &n.references {
                let Some(rt_local) = resolve_text(f, &r.reference_type) else { continue };
                let Some(rt_uri) = uri_of(f, rt_local.ns) else { continue };
                let rt = GlobalNodeId { ns_uri: rt_uri, id: rt_local.id };
                let Some(tgt_local) = resolve_text(f, &r.target) else { continue };
                let Some(tgt_uri) = uri_of(f, tgt_local.ns) else { continue };
                let tgt = GlobalNodeId { ns_uri: tgt_uri, id: tgt_local.id };
                // Canonical orientation: forward direction.
                let (fsrc, ftgt, declared_forward) = if r.is_forward {
                    (src.clone(), tgt, true)
                } else {
                    (tgt, src.clone(), false)
                };
                let key = (
                    stable_id(&fsrc),
                    stable_id(&rt),
                    stable_id(&ftgt),
                );
                if seen.contains(&key) {
                    // Declared from both directions: mark as confirmed forward.
                    if let Some(e) = m.edges.iter_mut().find(|e| {
                        stable_id(&e.source) == key.0
                            && stable_id(&e.reference_type) == key.1
                            && stable_id(&e.target) == key.2
                    }) {
                        e.forward = true;
                    }
                    continue;
                }
                seen.insert(key);
                m.edges.push(Edge {
                    source: fsrc,
                    reference_type: rt,
                    target: ftgt,
                    forward: declared_forward,
                    origin_file: f.file_id.clone(),
                });
            }
        }
    }

    // 5. Dangling references: edge kept, diagnostic recorded, no edge invented.
    for e in &m.edges {
        if !m.nodes.contains_key(&stable_id(&e.target)) {
            m.diagnostics.push(Diagnostic::DanglingReference {
                source: e.source.clone(),
                reference_type: e.reference_type.clone(),
                target: e.target.clone(),
            });
        }
    }

    // 6. Model dependencies: conflicts keep all candidates; missing models
    //    produce unresolved placeholders (never dropped).
    for f in files {
        for me in &f.models {
            m.models.push(me.clone());
        }
    }
    let present_uris: BTreeSet<&str> =
        m.models.iter().map(|me| me.model_uri.as_str()).collect();
    let mut reqs: BTreeMap<String, Vec<VersionCandidate>> = BTreeMap::new();
    for me in &m.models {
        for rm in &me.required_models {
            reqs.entry(rm.model_uri.clone()).or_default().push(VersionCandidate {
                version: rm.version.clone(),
                required_by: me.model_uri.clone(),
            });
        }
    }
    for (uri, cands) in &reqs {
        if uri == CORE_URI || present_uris.contains(uri.as_str()) {
            // version conflict check
            let versions: BTreeSet<_> = cands.iter().map(|c| &c.version).collect();
            if versions.len() > 1 {
                m.diagnostics.push(Diagnostic::DependencyVersionConflict {
                    model_uri: uri.clone(),
                    candidates: cands.clone(),
                });
                m.dependency_candidates.insert(uri.clone(), cands.clone());
            }
        } else {
            m.diagnostics.push(Diagnostic::UnresolvedDependency {
                requiring_model: cands
                    .first()
                    .map(|c| c.required_by.clone())
                    .unwrap_or_default(),
                missing_model_uri: uri.clone(),
            });
            m.dependency_candidates.insert(uri.clone(), cands.clone());
        }
    }

    // 7. Domain quantities: raw strings preserved; conversion failures become
    //    diagnostics and the raw value stays in place.
    for n in m.nodes.values() {
        for key in ["Value", "Minimum", "Maximum"] {
            if let Some(raw) = n.attrs.get(key) {
                if raw.parse::<f64>().is_err() {
                    m.diagnostics.push(Diagnostic::UnconvertibleValue {
                        node: n.id.clone(),
                        attr: key.to_string(),
                        raw: raw.clone(),
                        reason: "not a numeric literal; kept raw with unit basis".into(),
                    });
                }
            }
        }
    }

    m
}

/// Type hierarchy: parent -> children from HasSubtype edges (i=45 in core).
pub fn type_hierarchy(m: &MergedModel) -> BTreeMap<StableId, Vec<StableId>> {
    let has_subtype = GlobalNodeId { ns_uri: CORE_URI.into(), id: NodeIdKind::Numeric(45) };
    let key = stable_id(&has_subtype);
    let mut tree: BTreeMap<StableId, Vec<StableId>> = BTreeMap::new();
    for e in &m.edges {
        if stable_id(&e.reference_type) == key {
            tree.entry(stable_id(&e.source)).or_default().push(stable_id(&e.target));
        }
    }
    for v in tree.values_mut() {
        v.sort();
    }
    tree
}
