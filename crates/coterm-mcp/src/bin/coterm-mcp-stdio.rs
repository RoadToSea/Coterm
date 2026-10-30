//! 独立的 MCP Stdio 适配器入口。桌面版通过 `coterm-desktop mcp stdio` 提供同一实现。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    coterm_mcp::stdio::run(&args).await
}
