use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{utils::error::AppError, AppState};

/// JWT认证中间件
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    // 从Authorization header提取token
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("Missing authorization header".to_string()))?;

    // 检查Bearer格式
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized("Invalid authorization format".to_string()))?;

    // 验证JWT token
    let claims = state
        .jwt_service
        .verify_token(token)
        .map_err(|e| AppError::Unauthorized(format!("Invalid token: {}", e)))?;

    // 将Claims注入到request extensions中
    req.extensions_mut().insert(claims);

    // 继续处理请求
    Ok(next.run(req).await)
}
