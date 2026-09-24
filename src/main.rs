use namespace_compass_opcua::server::serve;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let addr = std::env::args()
        .position(|argument| argument == "--addr")
        .and_then(|index| std::env::args().nth(index + 1))
        .unwrap_or_else(|| "127.0.0.1:5383".to_string())
        .parse::<SocketAddr>()
        .expect("valid --addr socket address");
    serve(addr, "namespace-compass.sqlite3").await.expect("server failed");
}
