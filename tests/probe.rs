use nscompass::*;

fn load(paths: &[&str]) -> Vec<raw::SourceFile> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/cases/namespace-compass");
    paths
        .iter()
        .map(|p| {
            let full = root.join(p);
            let c = std::fs::read_to_string(&full).unwrap();
            raw::parse_file(p, &c).unwrap()
        })
        .collect()
}

#[test]
fn probe_baseline() {
    let files = load(&[
        "files/base/base-ua.xml",
        "files/boiler/v1.xml",
        "files/skid/v1.xml",
    ]);
    let m = merge::build(files, "main", &Default::default());
    println!("BASELINE events: {:?}", m.events.iter().map(|e| e.kind.clone()).collect::<Vec<_>>());
    println!("unresolved: {:?}", m.unresolved.iter().map(|u| (u.kind.clone(), u.subject.clone())).collect::<Vec<_>>());
    println!("nodes: {}, edges: {}, qty: {}, ext: {}", m.nodes.len(), m.edges.len(), m.quantities.len(), m.extensions.len());
}

#[test]
fn probe_edge() {
    let files = load(&[
        "files/base/base-ua.xml",
        "files/boiler/v2.xml",
        "files/skid/v2.xml",
        "files/optional/edge-pack.xml",
    ]);
    let m = merge::build(files, "boundary", &Default::default());
    println!("EDGE events: {:?}", m.events.iter().map(|e| (e.seq, e.kind.clone(), e.subject.clone())).collect::<Vec<_>>());
    println!("EDGE unresolved: {:?}", m.unresolved.iter().map(|u| (u.kind.clone(), u.subject.clone())).collect::<Vec<_>>());
    for n in m.nodes.iter().filter(|n| n.status != "ok") {
        println!("NODE status={} cid={} sources={} candidates={}", n.status, n.canonical_node_id, n.sources.len(), n.candidates.len());
    }
    for e in m.edges.iter().filter(|e| e.status != "resolved") {
        println!("EDGE status={} type={} inv={} prov={}", e.status, e.reference_type, e.from_inverse, e.provenance.len());
    }
    println!("extensions: {:?}", m.extensions.iter().map(|x| (x.tag.clone(), x.recognized)).collect::<Vec<_>>());
}
