use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("Please sign in again.")]
    Unauthorized,
    #[error("This request is not allowed.")]
    Forbidden,
    #[error("{0}")]
    Conflict(String),
    #[error("Too many attempts. Please wait a minute.")]
    TooManyRequests,
    #[error("{0}")]
    Spotify(String),
    #[error("The local database operation failed.")]
    Database(#[from] sqlx::Error),
    #[error("The local database operation failed.")]
    BatchDatabase(#[from] rusqlite::Error),
    #[error("An internal operation failed.")]
    Internal,
}
pub type Result<T> = std::result::Result<T, AppError>;
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::Spotify(_) => StatusCode::BAD_GATEWAY,
            Self::Database(_) | Self::BatchDatabase(_) | Self::Internal => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            eprintln!("A local operation failed ({}).", status.as_u16());
        }
        (status, Json(json!({"error": self.to_string()}))).into_response()
    }
}
