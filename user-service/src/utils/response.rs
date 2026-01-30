use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::json;

/// 统一的成功响应格式
#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub code: u16,
    pub message: String,
    pub data: Option<T>,
    pub success: bool,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            code: 200,
            message: "Success".to_string(),
            data: Some(data),
            success: true,
        }
    }

    pub fn success_with_message(data: T, message: impl Into<String>) -> Self {
        Self {
            code: 200,
            message: message.into(),
            data: Some(data),
            success: true,
        }
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

/// 用于只返回消息的成功响应
pub fn success_message(message: impl Into<String>) -> Response {
    let body = Json(json!({
        "code": 200,
        "message": message.into(),
        "success": true,
    }));

    (StatusCode::OK, body).into_response()
}

/// 分页响应
#[derive(Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub code: u16,
    pub message: String,
    pub data: PaginatedData<T>,
    pub success: bool,
}

#[derive(Serialize)]
pub struct PaginatedData<T: Serialize> {
    pub list: Vec<T>,
    pub total: u64,
    pub page: u64,
    pub size: u64,
    pub pages: u64,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn new(list: Vec<T>, total: u64, page: u64, size: u64) -> Self {
        let pages = (total as f64 / size as f64).ceil() as u64;

        Self {
            code: 200,
            message: "Success".to_string(),
            data: PaginatedData {
                list,
                total,
                page,
                size,
                pages,
            },
            success: true,
        }
    }
}

impl<T: Serialize> IntoResponse for PaginatedResponse<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}
