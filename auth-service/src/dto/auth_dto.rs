use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::models::user::UserRole;

/// 发送验证码请求
#[derive(Debug, Deserialize, Validate)]
pub struct SendCodeRequest {
    #[validate(length(equal = 11, message = "手机号必须是11位"))]
    pub phone: String,

    #[serde(rename = "type")]
    pub code_type: String, // "login" or "register"
}

/// 验证码登录请求
#[derive(Debug, Deserialize, Validate)]
pub struct LoginByCodeRequest {
    #[validate(length(equal = 11))]
    pub phone: String,

    #[validate(length(equal = 6, message = "验证码必须是6位"))]
    pub code: String,
}

/// 密码登录请求
#[derive(Debug, Deserialize, Validate)]
pub struct LoginByPasswordRequest {
    pub account: String, // 手机号/邮箱/用户名

    #[validate(length(min = 6, message = "密码至少6位"))]
    pub password: String,
}

/// 注册请求
#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(length(equal = 11))]
    pub phone: String,

    #[validate(length(equal = 6))]
    pub code: String,

    #[validate(length(min = 6))]
    pub password: String,

    #[validate(length(min = 2, max = 50, message = "用户名长度必须在2-50之间"))]
    pub name: String,

    pub role: UserRole,
}

/// 登录/注册响应
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_info: UserInfo,
}

/// 用户信息
#[derive(Debug, Serialize, Clone)]
pub struct UserInfo {
    pub id: String,
    pub name: String,
    pub phone: String,
    pub avatar: Option<String>,
    pub role: String,
}

/// 发送验证码响应
#[derive(Debug, Serialize)]
pub struct SendCodeResponse {
    pub message: String,
    pub expire: i64, // 过期时间（秒）
}
