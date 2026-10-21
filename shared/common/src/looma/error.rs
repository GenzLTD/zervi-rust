use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

/// Looma 错误体：`{"error","message"}`，无 `code` / `success` 包裹。
pub struct LoomaError {
    pub status: StatusCode,
    pub error: &'static str,
    pub message: String,
    pub extra: Value,
}

impl LoomaError {
    pub fn new(status: StatusCode, error: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            error,
            message: message.into(),
            extra: json!({}),
        }
    }
}

impl IntoResponse for LoomaError {
    fn into_response(self) -> Response {
        let mut body = json!({
            "error": self.error,
            "message": self.message,
        });
        if let (Some(obj), Some(extra)) = (body.as_object_mut(), self.extra.as_object()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
        (self.status, Json(body)).into_response()
    }
}

pub fn fail(status: StatusCode, error: &'static str, message: impl Into<String>) -> Response {
    LoomaError::new(status, error, message).into_response()
}

pub fn fail_extra(
    status: StatusCode,
    error: &'static str,
    message: impl Into<String>,
    extra: Value,
) -> Response {
    LoomaError {
        status,
        error,
        message: message.into(),
        extra,
    }
    .into_response()
}
