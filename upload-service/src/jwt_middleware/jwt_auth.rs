use axum::{
    body::Body,
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::Response,
};

use crate::{utils::error::AppError, AppState};

/// JWT认证中间件
/// 注意：所有路径都需要JWT认证，不再有公开路径白名单
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let path = req.uri().path();

    // 克隆路径字符串，避免借用冲突
    let path_str = path.to_string();

    // 所有路径都需要JWT认证（已移除公开路径白名单）

    // 从Authorization header提取token
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            tracing::warn!("❌ JWT验证失败: 路径 {} 缺少Authorization header", path_str);
            AppError::Unauthorized("Missing authorization header".to_string())
        })?;

    // 检查Bearer格式
    let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
        tracing::warn!("❌ JWT验证失败: 路径 {} 的Authorization格式无效", path_str);
        AppError::Unauthorized("Invalid authorization format".to_string())
    })?;

    // 验证JWT token
    let claims = state.jwt_service.verify_token(token).map_err(|e| {
        tracing::warn!("❌ JWT验证失败: 路径 {} token无效: {}", path_str, e);
        AppError::Unauthorized(format!("Invalid token: {}", e))
    })?;

    // 将Claims注入到request extensions中
    req.extensions_mut().insert(claims.clone());

    // 记录JWT验证成功
    tracing::info!(
        "🔐 JWT验证成功: 路径={}, 用户ID={}, 角色={}",
        path_str,
        claims.sub,
        claims.role
    );

    // 继续处理请求
    Ok(next.run(req).await)
}
