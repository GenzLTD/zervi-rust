use axum::{extract::State, Extension, Json};
use validator::Validate;

use crate::{
    dto::{ChangePasswordRequest, UpdateProfileRequest, UploadAvatarRequest, UserProfile},
    services::UserService,
    utils::{
        error::{AppError, Result},
        jwt::Claims,
        response::ApiResponse,
    },
    AppState,
};

/// 获取用户资料
pub async fn get_profile(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>, // JWT中间件会注入
) -> Result<ApiResponse<UserProfile>> {
    let profile = UserService::get_profile(
        &state.db,
        claims.sub.clone(),
        claims.name.clone(),
        claims.phone.clone(),
        claims.role.clone(),
    )
    .await?;

    Ok(ApiResponse::success(profile))
}

/// 更新用户资料
pub async fn update_profile(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<ApiResponse<UserProfile>> {
    payload
        .validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let user_id = claims.sub;
    let profile = UserService::update_profile(&state.db, user_id, payload).await?;

    Ok(ApiResponse::success_with_message(profile, "更新成功"))
}

/// 上传头像
pub async fn upload_avatar(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UploadAvatarRequest>,
) -> Result<ApiResponse<UserProfile>> {
    let user_id = claims.sub;
    let profile = UserService::upload_avatar(&state.db, user_id, payload.avatar_url).await?;

    Ok(ApiResponse::success_with_message(profile, "头像上传成功"))
}

/// 修改密码
pub async fn change_password(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<ApiResponse<String>> {
    payload
        .validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let user_id = claims.sub;
    UserService::change_password(&state.db, user_id, payload).await?;

    Ok(ApiResponse::success_with_message(
        "密码修改成功".to_string(),
        "请使用新密码登录",
    ))
}
