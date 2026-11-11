//! Shared TCP lifecycle; Telnet framing remains in its own codec.
mod backend;
mod catalog;
mod commands;
mod config;
mod proxy;
mod session;

pub use backend::Backend;

#[cfg(test)]
mod tests;
