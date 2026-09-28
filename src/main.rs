use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "compass", about = "命名空间罗盘：OPC UA NodeSet 合并、引用闭包与模型差异")]
struct Args {
    /// SQLite 数据库路径（默认内存外的文件库）
    #[arg(long, default_value = "compass.db")]
    db: String,
    /// 监听地址
    #[arg(long, default_value = "127.0.0.1:5383")]
    addr: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    compass::web::run(&args.db, &args.addr).await
}
