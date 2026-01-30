use serde::{Deserialize, Serialize};
use validator::Validate;

/// 获取用户资料响应
#[derive(Debug, Serialize)]
pub struct UserProfile {
    pub id: String,
    pub phone: String,
    pub email: Option<String>,
    pub name: String,
    pub avatar: Option<String>,
    pub role: String,
    pub status: String,
    pub created_at: String,
}

/// 更新用户资料请求
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateProfileRequest {
    #[validate(length(min = 2, max = 50))]
    pub name: Option<String>,

    #[validate(email)]
    pub email: Option<String>,

    pub avatar: Option<String>,
}

/// 上传头像请求
#[derive(Debug, Deserialize)]
pub struct UploadAvatarRequest {
    pub avatar_url: String,
}

/// 修改密码请求
#[derive(Debug, Deserialize, Validate)]
pub struct ChangePasswordRequest {
    #[validate(length(min = 6))]
    pub old_password: String,

    #[validate(length(min = 6))]
    pub new_password: String,
}
