use nscompass::{server, AppState};
use std::env;

fn help() {
    eprintln!(
        "nscompass - 命名空间罗盘: OPC UA NodeSet merge / reference closure / model diff\n\n\
USAGE:\n  nscompass [--addr 127.0.0.1:5383] [--db nscompass.sqlite] [--case fixtures/cases/namespace-compass]\n\n\
Then open http://127.0.0.1:5383 for the 90-second guided walkthrough."
    );
}

#[tokio::main]
async fn main() {
    let mut addr = "127.0.0.1:5383".to_string();
    let mut db = "nscompass.sqlite".to_string();
    let mut case: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--addr" => addr = args.next().unwrap_or_else(|| addr.clone()),
            "--db" => db = args.next().unwrap_or_else(|| db.clone()),
            "--case" => case = args.next(),
            "-h" | "--help" => {
                help();
                return;
            }
            other => eprintln!("ignoring unknown argument {}", other),
        }
    }
    let state = AppState::new(&db);
    *state.case_root.lock().unwrap() = case;
    server::serve(state, &addr).await;
}
