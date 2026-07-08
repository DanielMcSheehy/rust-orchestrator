use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Unified API error: carries the HTTP status the client should see.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<cortex_store::StoreError> for ApiError {
    fn from(e: cortex_store::StoreError) -> Self {
        let status = match &e {
            cortex_store::StoreError::NotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError {
            status,
            message: e.to_string(),
        }
    }
}

impl From<cortex_core::DagError> for ApiError {
    fn from(e: cortex_core::DagError) -> Self {
        ApiError::bad_request(format!("invalid workflow DAG: {e}"))
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::internal(e.to_string())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use cortex_core::DagError;
    use cortex_store::StoreError;
    use serde_json::Value;

    #[test]
    fn constructors_set_status_and_message() {
        let bad = ApiError::bad_request("nope");
        assert_eq!(bad.status, StatusCode::BAD_REQUEST);
        assert_eq!(bad.message, "nope");

        let internal = ApiError::internal(String::from("boom"));
        assert_eq!(internal.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(internal.message, "boom");
    }

    #[test]
    fn store_not_found_maps_to_404() {
        let err: ApiError = StoreError::NotFound("run 1".into()).into();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.message, "not found: run 1");
    }

    #[test]
    fn other_store_errors_map_to_500() {
        let json_err = serde_json::from_str::<Value>("{bad").unwrap_err();
        let err: ApiError = StoreError::Json(json_err).into();
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn dag_errors_map_to_400_with_prefix() {
        let err: ApiError = DagError::Empty.into();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(
            err.message.starts_with("invalid workflow DAG:"),
            "got {}",
            err.message
        );
    }

    #[test]
    fn io_errors_map_to_500() {
        let io = std::io::Error::other("disk gone");
        let err: ApiError = io.into();
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.message, "disk gone");
    }

    // The documented API error shape: non-2xx bodies are `{"error": "message"}`.
    #[tokio::test]
    async fn into_response_emits_error_json_shape() {
        let resp = ApiError::bad_request("bad input").into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body, serde_json::json!({ "error": "bad input" }));
    }
}
