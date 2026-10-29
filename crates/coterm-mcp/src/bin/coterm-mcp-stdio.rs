//! 独立的 MCP Stdio 适配器入口。桌面版通过 `coterm-desktop mcp stdio` 提供同一实现。
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    coterm_mcp::stdio::run_from_env().await
}
