use compass::app::Orchestrator;
use compass::store::Store;
use compass::web::server::router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut addr = "127.0.0.1:5383".to_string();
    let mut db_path = "compass.sqlite3".to_string();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                addr = args[i + 1].clone();
                i += 2;
            }
            "--db" => {
                db_path = args[i + 1].clone();
                i += 2;
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let store = Store::open(&db_path)?;
    let orch = Orchestrator::with_store(store);
    if let Some(summary) = orch.bootstrap_if_empty()? {
        println!(
            "Bundled case replayed: {} versions, current {}",
            summary.steps, summary.current
        );
    }

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("Namespace compass listening on http://{addr}");
    println!("Open the page and click \"开始 90 秒样例路径\".");
    axum::serve(listener, router(Store::open(&db_path)?)).await?;
    Ok(())
}
