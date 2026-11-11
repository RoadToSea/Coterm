//! 内置通信驱动；协议实现和实例数据分别由对应模块管理。

pub mod local;
mod network;
pub mod serial;
pub mod ssh;
mod stream;
pub mod tcp;
pub mod telnet;
