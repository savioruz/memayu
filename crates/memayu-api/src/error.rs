use memayu_core::CoreError;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiErrorBody {
    pub error: String,
    pub message: String,
}

/// Transport-agnostic API error — no axum types in the struct.
/// Transport layer is responsible for converting `status` (u16) into
/// the appropriate HTTP response.
pub struct ApiError {
    pub status: u16,
    pub error: String,
    pub message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            error: "bad_request".into(),
            message: message.into(),
        }
    }

    // ── helpers for transport layer ──

    pub fn status_u16(&self) -> u16 {
        self.status
    }

    pub fn body(&self) -> ApiErrorBody {
        ApiErrorBody {
            error: self.error.clone(),
            message: self.message.clone(),
        }
    }
}

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        let status = match &e {
            CoreError::DimensionMismatch { .. } => 422,
            CoreError::InvalidExtraction(_) => 422,
            CoreError::NotFound(_) => 404,
            _ => 500,
        };
        Self {
            status,
            error: "internal_error".into(),
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_request_sets_400() {
        let err = ApiError::bad_request("nope");
        assert_eq!(err.status, 400);
        assert_eq!(err.error, "bad_request");
        assert_eq!(err.message, "nope");
    }

    #[test]
    fn body_returns_cloned_fields() {
        let err = ApiError {
            status: 422,
            error: "foo".into(),
            message: "bar".into(),
        };
        let body = err.body();
        assert_eq!(body.error, "foo");
        assert_eq!(body.message, "bar");
    }

    #[test]
    fn from_core_not_found_is_404() {
        let err: ApiError = CoreError::NotFound("mem not found".into()).into();
        assert_eq!(err.status, 404);
    }

    #[test]
    fn from_core_dimension_mismatch_is_422() {
        let err: ApiError = CoreError::DimensionMismatch {
            expected: 128,
            got: 256,
        }
        .into();
        assert_eq!(err.status, 422);
    }

    #[test]
    fn from_core_invalid_extraction_is_422() {
        let err: ApiError = CoreError::InvalidExtraction("bad".into()).into();
        assert_eq!(err.status, 422);
    }

    #[test]
    fn from_core_storage_is_500() {
        use memayu_core::StorageError;
        let err: ApiError =
            CoreError::Storage(StorageError::Other("some internal failure".into())).into();
        assert_eq!(err.status, 500);
    }
}
