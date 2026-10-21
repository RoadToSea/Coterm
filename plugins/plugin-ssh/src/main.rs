//! Single host-owned stdio connection: the shared transport owns framing and envelopes.
use anyhow::Result;
use coterm_plugin_ssh::backend::Backend;
use coterm_proto::runtime::transport::serve;

#[tokio::main]
async fn main() -> Result<()> {
    serve(Backend::default()).await?;
    Ok(())
}
