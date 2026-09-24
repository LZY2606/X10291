use anyhow::Result;
use compass::scenario::run_case_pack;
use compass::store::Store;
use compass::web::{router, AppState};
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<()> {
    let mut addr = "127.0.0.1:5383".to_string();
    let mut db = "compass.db".to_string();
    let mut seed_demo = true;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                addr = args.get(i + 1).cloned().unwrap_or(addr);
                i += 1;
            }
            "--db" => {
                db = args.get(i + 1).cloned().unwrap_or(db);
                i += 1;
            }
            "--no-demo" => seed_demo = false,
            _ => {}
        }
        i += 1;
    }

    let store = Store::open(&db)?;
    let recovered = store.recover()?;
    if recovered > 0 {
        eprintln!("recovered: removed {recovered} interrupted version write(s)");
    }
    if seed_demo && store.current_pointer()?.is_none() {
        let result = run_case_pack(&store)?;
        for line in &result.log {
            eprintln!("demo: {line}");
        }
        eprintln!("demo case pack loaded ({} versions)", store.list_versions()?.len());
    }

    let state = Arc::new(AppState {
        store: Mutex::new(store),
    });
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("namespace-compass listening on http://{addr}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
