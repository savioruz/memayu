mod error;
pub mod extraction;
mod memory;
mod ports;
mod service;

pub use error::CoreError;
pub use memory::Memory;
pub use ports::{
    EmbedError, EmbedderProvider, ExtractionDecision, ExtractionResult, LlmError, LlmProvider,
    Message, Metadata, StorageError, StorageProvider,
};
pub use service::MemoryService;
