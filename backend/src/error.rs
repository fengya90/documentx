use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// 把应用错误统一转成 JSON 错误响应。
pub struct AppError {
    status: StatusCode,
    error: anyhow::Error,
}

impl AppError {
    pub fn bad_request(error: impl Into<anyhow::Error>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: error.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!("request failed: {:#}", self.error);
        } else {
            tracing::warn!("request rejected: {:#}", self.error);
        }
        (
            self.status,
            Json(json!({ "error": self.error.to_string() })),
        )
            .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        AppError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: err.into(),
        }
    }
}
