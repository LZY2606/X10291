use nodeset_compass::engine;
use nodeset_compass::parser::parse_nodeset;
use std::fs;

fn load(dir: &str) -> Vec<nodeset_compass::domain::ParsedFile> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let bytes = fs::read(p).unwrap();
            let id = p.file_name().unwrap().to_string_lossy().to_string();
            parse_nodeset(&id, p.to_str().unwrap(), &bytes).unwrap().file
        })
        .collect()
}

#[test]
fn v1_derivation() {
    let files = load("fixtures/compass/v1");
    let g = engine::derive(&files);
    println!("nodes={}", g.nodes.len());
    println!("edges={}", g.edges.len());
    println!("unresolved={}", g.unresolved.len());
    println!("unresolved_nodes={}", g.unresolved_nodes.len());
    println!("deps={:?}", g.dependencies.iter().map(|d|(&d.required_uri,&d.state)).collect::<Vec<_>>());
    println!("duplicates={:?}", g.duplicates.iter().map(|d|d.gid.to_string()).collect::<Vec<_>>());
    println!("alias_cycles={:?}", g.alias_cycles);
    println!("diag codes={:?}", g.diagnostics.iter().map(|d|&d.code).collect::<Vec<_>>());
    println!("quantities={}", g.quantities.len());
    assert!(g.nodes.len() >= 15);
}
