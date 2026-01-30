use axum::{extract::State, Json};
use validator::Validate;

use crate::{
    dto::{
        LoginByCodeRequest, LoginByPasswordRequest, LoginResponse, RegisterRequest,
        SendCodeRequest, SendCodeResponse,
    },
    services::AuthService,
    utils::{
        error::{AppError, Result},
        response::ApiResponse,
    },
    AppState,
};

/// 发送验证码
pub async fn send_code(
    State(state): State<AppState>,
    Json(payload): Json<SendCodeRequest>,
) -> Result<ApiResponse<SendCodeResponse>> {
    payload
        .validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let mut redis = state.redis.clone();
    let response = AuthService::send_code(&mut redis, payload).await?;

    Ok(ApiResponse::success(response))
}

/// 验证码登录
pub async fn login_by_code(
    State(state): State<AppState>,
    Json(payload): Json<LoginByCodeRequest>,
) -> Result<ApiResponse<LoginResponse>> {
    payload
        .validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let mut redis = state.redis.clone();
    let response =
        AuthService::login_by_code(&state.db, &mut redis, &state.jwt_service, payload).await?;

    Ok(ApiResponse::success(response))
}

/// 密码登录
pub async fn login_by_password(
    State(state): State<AppState>,
    Json(payload): Json<LoginByPasswordRequest>,
) -> Result<ApiResponse<LoginResponse>> {
    payload
        .validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let response = AuthService::login_by_password(&state.db, &state.jwt_service, payload).await?;

    Ok(ApiResponse::success(response))
}

/// 注册
pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<ApiResponse<LoginResponse>> {
    payload
        .validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let mut redis = state.redis.clone();
    let response =
        AuthService::register(&state.db, &mut redis, &state.jwt_service, payload).await?;

    Ok(ApiResponse::success_with_message(response, "注册成功"))
}
