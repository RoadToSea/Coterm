//! 基座拥有的单条 stdio 连接；信封与读行交给共享传输层。
use anyhow::Result;
use coterm_plugin_local::backend::Backend;
use coterm_proto::runtime::transport::serve;

#[tokio::main]
async fn main() -> Result<()> {
    serve(Backend::default()).await?;
    Ok(())
}
