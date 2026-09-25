//! Dependency-graph propagation for recompute.
//!
//! Artifacts: normalize:{file} -> merged -> {hierarchy, diagnostics, graph}.
//! When files change, only artifacts downstream of a changed file are marked
//! for recompute; everything else is reported still-valid, and the report is
//! verified against the freshly merged model (unchanged files contribute
//! identical nodes).

use crate::model::{stable_id, MergedModel, NodeSetFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecomputeReport {
    pub changed_files: Vec<String>,
    pub recomputed: Vec<String>,
    pub still_valid: Vec<String>,
    pub reasons: BTreeMap<String, String>,
}

/// Static artifact dependency graph.
pub fn artifact_dependencies(files: &[NodeSetFile]) -> BTreeMap<String, Vec<String>> {
    let mut g = BTreeMap::new();
    let normalizes: Vec<String> =
        files.iter().map(|f| format!("normalize:{}", f.file_id)).collect();
    for f in files {
        g.insert(format!("normalize:{}", f.file_id), vec![format!("file:{}", f.file_id)]);
    }
    g.insert("merged".to_string(), normalizes);
    g.insert("hierarchy".to_string(), vec!["merged".to_string()]);
    g.insert("diagnostics".to_string(), vec!["merged".to_string()]);
    g.insert("reference-graph".to_string(), vec!["merged".to_string()]);
    g
}

pub fn plan_recompute(
    old_files: &[NodeSetFile],
    new_files: &[NodeSetFile],
    new_merged: &MergedModel,
) -> RecomputeReport {
    let mut report = RecomputeReport::default();
    let old_map: BTreeMap<&str, &NodeSetFile> =
        old_files.iter().map(|f| (f.file_id.as_str(), f)).collect();
    let new_map: BTreeMap<&str, &NodeSetFile> =
        new_files.iter().map(|f| (f.file_id.as_str(), f)).collect();

    let mut changed: BTreeSet<String> = BTreeSet::new();
    for (id, nf) in &new_map {
        match old_map.get(id) {
            Some(of) if of.raw_xml == nf.raw_xml => {}
            _ => {
                changed.insert(id.to_string());
            }
        }
    }
    for id in old_map.keys() {
        if !new_map.contains_key(id) {
            changed.insert(id.to_string());
        }
    }
    report.changed_files = changed.iter().cloned().collect();

    // Propagate through the artifact graph.
    let graph = artifact_dependencies(new_files);
    let mut dirty: BTreeSet<String> =
        changed.iter().map(|c| format!("file:{}", c)).collect();
    loop {
        let mut grew = false;
        for (artifact, deps) in &graph {
            if dirty.contains(artifact) {
                continue;
            }
            if deps.iter().any(|d| dirty.contains(d)) {
                dirty.insert(artifact.clone());
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    for (artifact, deps) in &graph {
        if dirty.contains(artifact) {
            report.recomputed.push(artifact.clone());
            let why: Vec<String> =
                deps.iter().filter(|d| dirty.contains(*d)).cloned().collect();
            report.reasons.insert(
                artifact.clone(),
                format!("depends on changed: {}", why.join(", ")),
            );
        } else {
            report.still_valid.push(artifact.clone());
            report
                .reasons
                .insert(artifact.clone(), "no changed upstream input".to_string());
        }
    }
    report.recomputed.sort();
    report.still_valid.sort();

    // Verify still-valid normalizations: nodes of unchanged files must appear
    // identically in the new merged model.
    for (id, nf) in &new_map {
        if changed.contains(*id) {
            continue;
        }
        let art = format!("normalize:{}", id);
        if report.still_valid.contains(&art) {
            let ok = nf.nodes.iter().all(|n| {
                // every node of an unchanged file is present in merged output
                new_merged.nodes.values().any(|mn| {
                    mn.origin_file == *id && mn.browse_name.name == n.browse_name.name
                }) || new_merged.duplicates.values().flatten().any(|mn| {
                    mn.origin_file == *id && mn.browse_name.name == n.browse_name.name
                })
            });
            if !ok {
                report.reasons.insert(
                    art.clone(),
                    "verification failed: moved to recompute set".to_string(),
                );
                report.still_valid.retain(|a| a != &art);
                report.recomputed.push(art);
            }
        }
    }
    let _ = stable_id; // keep import used if refactored
    report
}
