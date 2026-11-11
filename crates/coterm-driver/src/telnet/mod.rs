//! Telnet NVT framing, negotiation and terminal connections.
pub(crate) mod codec;
pub mod backend {
    /// In-process Telnet backend. Credentials are entered interactively, not saved in configuration.
    pub type Backend = crate::network::Backend<true>;
}
