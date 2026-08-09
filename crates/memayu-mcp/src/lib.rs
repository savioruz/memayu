//! MCP stdio server for memayu — trait-based so backends live in the binary crate.

mod memory;
mod tools;
mod transport;
mod types;

pub use memory::{Backend, McpError, MemoryBackend};
pub use transport::run;
pub use types::*;
