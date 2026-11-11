//! Opt-in TCP/Telnet throughput diagnostics; never used by the desktop or shipped as a driver.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("tcp") => {
            coterm_core::diagnostic::transport::serve(
                coterm_driver::tcp::backend::Backend::default(),
            )
            .await?
        }
        Some("telnet") => {
            coterm_core::diagnostic::transport::serve(
                coterm_driver::telnet::backend::Backend::default(),
            )
            .await?
        }
        _ => return Err("Usage: network-diagnostic tcp|telnet".into()),
    }
    Ok(())
}
