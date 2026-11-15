//! 独立的 MCP Stdio 适配器入口。桌面版通过 `coterm-desktop mcp stdio` 提供同一实现。
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(coterm_mcp::stdio::run(&args));
    // The client may keep stdin open after the bridge ends; its blocking read
    // must not hold this short-lived adapter process open during runtime drop.
    runtime.shutdown_background();
    result
}
