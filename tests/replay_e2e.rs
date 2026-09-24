use nodeset_compass::replay::{export_then_reimport, run};
use nodeset_compass::service::DEFAULT_BRANCH;
use nodeset_compass::store::Store;
use nodeset_compass::{replay, service::App};

fn fresh() -> App {
    App::new(Store::open_memory().unwrap())
}

#[test]
fn full_compass_replay() {
    let app = fresh();
    let report = run(&app);
    for s in &report.steps {
        println!("{} {}: {:?}", s.clock, s.action, s.findings);
    }
    assert!(report.branches.contains(&"dep-revision".to_string()));

    // baseline diagnostics include alias cycle + missing dependency
    let main = app
        .store
        .branch_pointer(DEFAULT_BRANCH)
        .unwrap()
        .unwrap();
    let snap = app.store.snapshot(&main.version_id).unwrap().unwrap();
    let codes: Vec<_> = snap.derived.diagnostics.iter().map(|d| d.code.clone()).collect();
    assert!(codes.contains(&"ALIAS_CYCLE".to_string()), "codes: {codes:?}");

    let dep = app.store.branch_pointer("dep-revision").unwrap().unwrap();
    let dep_snap = app.store.snapshot(&dep.version_id).unwrap().unwrap();
    assert!(dep_snap
        .derived
        .dependencies
        .iter()
        .any(|d| d.required_uri == "http://test.org/CompassUnits/" && d.state == "conflict"));
    assert!(dep_snap
        .derived
        .unresolved_nodes
        .iter()
        .any(|u| u.gid.uri == "http://test.org/CompassUnits/" && u.gid.value == "101"));

    // decisions retain rejected candidates
    let decs = app.store.list_decisions("dep-revision").unwrap();
    assert!(decs
        .iter()
        .flat_map(|d| &d.rejected)
        .any(|r| r.contains("0.90")));

    // export -> delete -> reimport identity/order
    let (_bundle, app2) = export_then_reimport(&app, "clock-export");
    let bp1 = app.store.branch_pointer(DEFAULT_BRANCH).unwrap().unwrap();
    let bp2 = app2.store.branch_pointer(DEFAULT_BRANCH).unwrap().unwrap();
    assert_eq!(bp1.version_id, bp2.version_id);
    assert_eq!(bp1.event_seq, bp2.event_seq);
    let s1 = app.store.snapshot(&bp1.version_id).unwrap().unwrap();
    let s2 = app2.store.snapshot(&bp2.version_id).unwrap().unwrap();
    assert_eq!(s1.digest, s2.digest);
    assert_eq!(
        s1.derived.nodes.len(),
        s2.derived.nodes.len()
    );
    let ids1: Vec<_> = s1.derived.nodes.iter().map(|n| n.gid.to_string()).collect();
    let ids2: Vec<_> = s2.derived.nodes.iter().map(|n| n.gid.to_string()).collect();
    assert_eq!(ids1, ids2, "node order/identity preserved after reimport");
}

#[test]
fn history_view_restores_past_conclusions() {
    let app = fresh();
    let _ = replay::run(&app);
    // view at seq 3 (plant v1 baseline) should show v1 setpoint, not latest
    // dep-revision seq 5 == copied v1 baseline snapshot; seq 6 == v2 conflict.
    let hv = app.history_view("dep-revision", 5).unwrap();
    let q = hv
        .snapshot
        .derived
        .quantities
        .iter()
        .find(|q| q.gid.value == "6001")
        .unwrap();
    assert_eq!(q.value_text, "294.65");
    let latest = app.history_view("dep-revision", 6).unwrap();
    // companion v2 removes nothing from plant quantity; setpoint still v1 value here
    let q2 = latest
        .snapshot
        .derived
        .quantities
        .iter()
        .find(|q| q.gid.value == "6001")
        .unwrap();
    assert_eq!(q2.value_text, "294.65");
    // main branch carries the parameter revision 295.15
    let main_latest = app.history_view("main", 999).unwrap();
    let qm = main_latest
        .snapshot
        .derived
        .quantities
        .iter()
        .find(|q| q.gid.value == "6001")
        .unwrap();
    assert_eq!(qm.value_text, "295.15");
}
