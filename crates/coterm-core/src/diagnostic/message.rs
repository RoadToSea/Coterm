use super::lifecycle::RuntimeHello;
use crate::{driver::HostCommand, model::RuntimeError};
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum Command {
    Hello(RuntimeHello),
    Driver(HostCommand),
}

#[derive(Debug, Clone)]
pub struct HostMessage {
    pub request_id: String,
    pub command: Command,
}

#[derive(Debug, Clone)]
pub enum DriverEvent {
    Reply {
        request_id: String,
        result: Result<Value, RuntimeError>,
    },
}
