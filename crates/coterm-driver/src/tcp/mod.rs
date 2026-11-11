//! Raw TCP byte streams and optional observed Shell interpretation.
pub mod backend {
    /// In-process Raw TCP backend. Uses the existing driver contract and bounded notifications.
    pub type Backend = crate::network::Backend<false>;
}
