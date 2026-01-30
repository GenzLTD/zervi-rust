use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Internal server error: {0}")]
    InternalServerError(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sea_orm::DbErr),

    #[error("Redis error: {0}")]
    RedisError(String),

    #[error("JWT error: {0}")]
    JwtError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, 404, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, 401, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, 400, msg),
            AppError::ValidationError(msg) => (StatusCode::BAD_REQUEST, 400, msg),
            AppError::DatabaseError(err) => {
                tracing::error!("Database error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    500,
                    "Database error".to_string(),
                )
            }
            AppError::RedisError(msg) => {
                tracing::error!("Redis error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, 500, msg)
            }
            AppError::JwtError(msg) => (StatusCode::UNAUTHORIZED, 401, msg),
            AppError::InternalServerError(msg) => {
                tracing::error!("Internal server error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, 500, msg)
            }
        };

        let body = Json(json!({
            "code": code,
            "message": message,
            "success": false,
        }));

        (status, body).into_response()
    }
}
