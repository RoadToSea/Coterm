//! 显式串口测速使用的 stdio 适配器；不参与桌面构建和发行。
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    coterm_proto::runtime::transport::serve(coterm_driver_serial::backend::Backend::default())
        .await?;
    Ok(())
}
