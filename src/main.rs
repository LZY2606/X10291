use nodeset_compass::web;

#[tokio::main]
async fn main() {
    let addr = std::env::args().nth(1).map(|_| {
        // 兼容两种调用：cargo run -- --addr HOST:PORT  或  cargo run -- HOST:PORT
        let mut args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(|s| s.as_str()) == Some("--addr") {
            args.remove(0);
        }
        args.into_iter().next().unwrap_or_else(|| "127.0.0.1:5383".to_string())
    }).unwrap_or_else(|| "127.0.0.1:5383".to_string());
    web::serve(&addr).await;
}
