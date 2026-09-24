use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut addr = "127.0.0.1:5383".to_string();
    let mut db = "opc-compass.sqlite".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--addr" => addr = args.next().expect("--addr needs value"),
            "--db" => db = args.next().expect("--db needs value"),
            "--help" | "-h" => {
                println!("usage: opc-compass [--addr 127.0.0.1:5383] [--db opc-compass.sqlite]");
                return;
            }
            other => eprintln!("ignoring unknown arg {other}"),
        }
    }
    let store = Arc::new(opccompass::store::Store::open(&db).expect("open store"));
    println!("OPC UA 命名空间罗盘 listening on http://{addr} (db: {db})");
    println!("页面内置 90 秒短样例路径：打开首页点击“播放离线案例”。");
    opccompass::web::serve(store, &addr).await;
}
