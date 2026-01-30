use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Internal server error: {0}")]
    InternalServerError(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sea_orm::DbErr),

    #[error("JWT error: {0}")]
    JwtError(#[from] jsonwebtoken::errors::Error),

    #[error("Password hash error: {0}")]
    PasswordError(String),

    #[error("Redis error: {0}")]
    RedisError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Duplicate entry: {0}")]
    DuplicateEntry(String),

    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Token expired")]
    TokenExpired,

    #[error("Invalid verification code")]
    InvalidVerificationCode,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, 40001, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, 40101, msg),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, 40301, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, 40002, msg),
            AppError::ValidationError(msg) => (StatusCode::BAD_REQUEST, 40003, msg),
            AppError::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                40102,
                "Invalid credentials".to_string(),
            ),
            AppError::TokenExpired => {
                (StatusCode::UNAUTHORIZED, 40103, "Token expired".to_string())
            }
            AppError::InvalidVerificationCode => (
                StatusCode::BAD_REQUEST,
                40004,
                "Invalid verification code".to_string(),
            ),
            AppError::DuplicateEntry(msg) => (StatusCode::CONFLICT, 40901, msg),
            AppError::DatabaseError(err) => {
                tracing::error!("Database error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    50001,
                    "Database error occurred".to_string(),
                )
            }
            AppError::JwtError(err) => {
                tracing::error!("JWT error: {}", err);
                (StatusCode::UNAUTHORIZED, 40104, "Invalid token".to_string())
            }
            AppError::PasswordError(msg) => {
                tracing::error!("Password error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, 50002, msg)
            }
            AppError::RedisError(msg) => {
                tracing::error!("Redis error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, 50003, msg)
            }
            AppError::IoError(err) => {
                tracing::error!("IO error: {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    50004,
                    "File operation error".to_string(),
                )
            }
            AppError::InternalServerError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, 50000, msg),
        };

        let body = Json(json!({
            "code": error_code,
            "message": message,
            "success": false,
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
