use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Resource not found")]
    NotFound,

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Forbidden")]
    Forbidden,

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Internal server error")]
    InternalServerError,

    #[error("Database error: {0}")]
    DatabaseError(#[from] sea_orm::DbErr),

    #[error("JWT error: {0}")]
    JwtError(#[from] jsonwebtoken::errors::Error),

    #[error("Password hash error: {0}")]
    PasswordError(String),

    #[error("Redis error: {0}")]
    RedisError(#[from] redis::RedisError),

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
            AppError::NotFound => (StatusCode::NOT_FOUND, 40001, self.to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, 40101, self.to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, 40301, self.to_string()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, 40002, msg),
            AppError::ValidationError(msg) => (StatusCode::BAD_REQUEST, 40003, msg),
            AppError::InvalidCredentials => (StatusCode::UNAUTHORIZED, 40102, self.to_string()),
            AppError::TokenExpired => (StatusCode::UNAUTHORIZED, 40103, self.to_string()),
            AppError::InvalidVerificationCode => (StatusCode::BAD_REQUEST, 40004, self.to_string()),
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
                tracing::error!("JWT error: {:?}", err);
                (StatusCode::UNAUTHORIZED, 40104, "Invalid token".to_string())
            }
            AppError::PasswordError(msg) => {
                tracing::error!("Password error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    50002,
                    "Password processing error".to_string(),
                )
            }
            AppError::RedisError(err) => {
                tracing::error!("Redis error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    50003,
                    "Cache error occurred".to_string(),
                )
            }
            AppError::IoError(err) => {
                tracing::error!("IO error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    50004,
                    "File operation error".to_string(),
                )
            }
            AppError::InternalServerError => {
                (StatusCode::INTERNAL_SERVER_ERROR, 50000, self.to_string())
            }
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
