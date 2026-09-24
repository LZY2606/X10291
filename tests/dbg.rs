#[test]
fn dbg_deps() {
    use nodeset_compass::replay::run;
    use nodeset_compass::service::App;
    use nodeset_compass::store::Store;
    let app = App::new(Store::open_memory().unwrap());
    let _ = run(&app);
    let dep = app.store.branch_pointer("dep-revision").unwrap().unwrap();
    println!("dep branch at seq {} ver {}", dep.event_seq, dep.version_id);
    let snap = app.store.snapshot(&dep.version_id).unwrap().unwrap();
    for d in &snap.derived.dependencies {
        println!("DEP {} req={} avail={:?} state={} ({})", d.required_uri, d.required_version, d.available_version, d.state, d.detail);
    }
    for f in &snap.file_order { println!("FILE {f}"); }
}

#[test]
fn dbg_events() {
    use nodeset_compass::replay::run;
    use nodeset_compass::service::App;
    use nodeset_compass::store::Store;
    let app = App::new(Store::open_memory().unwrap());
    let _ = run(&app);
    for e in app.store.all_events().unwrap() {
        println!("EV seq={} branch={} kind={} ver={:?}", e.seq, e.branch, e.kind, e.result_version_id);
    }
    let dep = app.store.branch_pointer("dep-revision").unwrap().unwrap();
    let snap = app.store.snapshot(&dep.version_id).unwrap().unwrap();
    for u in &snap.derived.unresolved_nodes { println!("UNRESOLVED {}", u.gid); }
}

#[test]
fn dbg_roundtrip() {
    use nodeset_compass::replay::{run, export_then_reimport};
    use nodeset_compass::service::App;
    use nodeset_compass::store::Store;
    let app = App::new(Store::open_memory().unwrap());
    let _ = run(&app);
    let (b, app2) = export_then_reimport(&app, "x");
    println!("bundle branches: {:?}", b.branches.iter().map(|x|(&x.branch,&x.version_id,x.event_seq)).collect::<Vec<_>>());
    println!("bundle snapshots: {}", b.snapshots.len());
    println!("bundle events: {}", b.events.len());
    println!("bundle filerows: {}", b.file_rows.len());
    println!("app2 branches: {:?}", app2.store.list_branches().unwrap().iter().map(|x|(&x.branch,&x.version_id)).collect::<Vec<_>>());
}

#[test]
fn dbg_snaps() {
    use nodeset_compass::replay::run;
    use nodeset_compass::service::App;
    use nodeset_compass::store::Store;
    let app = App::new(Store::open_memory().unwrap());
    let _ = run(&app);
    let c = app.store.lock_raw();
    let mut st = c.prepare("SELECT version_id, branch, event_seq FROM snapshots ORDER BY rowid").unwrap();
    let rows = st.query_map([], |r| Ok((r.get::<_,String>(0).unwrap(), r.get::<_,String>(1).unwrap(), r.get::<_,i64>(2).unwrap()))).unwrap();
    for r in rows { let (a,b,cc)=r.unwrap(); println!("SNAP {a} {b} {cc}"); }
}

#[test]
fn dbg_hist() {
    use nodeset_compass::replay::run;
    use nodeset_compass::service::App;
    use nodeset_compass::store::Store;
    let app = App::new(Store::open_memory().unwrap());
    let _ = run(&app);
    for seq in [5i64,6,7] {
        let s = app.store.snapshot_for_event("dep-revision", seq).unwrap();
        println!("HIST seq {seq}: {:?}", s.as_ref().map(|x| x.version_id.clone()));
    }
}
