mod error;
pub mod extraction;
pub mod fusion;
mod memory;
pub mod pagination;
mod ports;
mod service;

pub use error::CoreError;
pub use memory::Memory;
pub use pagination::{
    decode_cursor, encode_cursor, metadata_matches, MemoryPage, MetadataFilter, MAX_PAGE_SIZE,
};
pub use ports::{
    EmbedError, EmbedderProvider, ExtractionDecision, ExtractionMode, ExtractionResult, LlmError,
    LlmProvider, Message, Metadata, StorageError, StorageProvider,
};
pub use service::{AddMemoryOutcome, MemoryService};
