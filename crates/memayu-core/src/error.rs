use crate::{EmbedError, LlmError, StorageError};
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("storage error: {0}")]
    Storage(#[source] StorageError),
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),
    #[error("embedding error: {0}")]
    Embed(#[from] EmbedError),
    #[error("dimension mismatch: provider produces {got}-dim, stored data uses {expected}-dim")]
    DimensionMismatch { expected: usize, got: usize },
    #[error("extraction result invalid: {0}")]
    InvalidExtraction(String),
    #[error("not found: {0}")]
    NotFound(String),
}

impl From<StorageError> for CoreError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::DimensionMismatch { expected, got } => {
                CoreError::DimensionMismatch { expected, got }
            }
            other => CoreError::Storage(other),
        }
    }
}
