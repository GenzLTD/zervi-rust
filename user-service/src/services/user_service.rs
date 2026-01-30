use chrono::Utc;
use sea_orm::*;

use crate::{
    dto::{ChangePasswordRequest, UpdateProfileRequest, UserProfile},
    models::user::{self, Entity as User},
    utils::{
        error::{AppError, Result},
        password,
    },
};

pub struct UserService;

impl UserService {
    /// 获取用户资料
    pub async fn get_profile(db: &DatabaseConnection, user_id: String) -> Result<UserProfile> {
        let user = User::find_by_id(user_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("用户不存在".to_string()))?;

        Ok(UserProfile {
            id: user.id.clone(),
            phone: user.phone,
            email: user.email,
            name: user.name,
            avatar: user.avatar,
            role: user.role.to_string(),
            status: user.status.to_string(),
            created_at: user.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
    }

    /// 更新用户资料
    pub async fn update_profile(
        db: &DatabaseConnection,
        user_id: String,
        request: UpdateProfileRequest,
    ) -> Result<UserProfile> {
        let user = User::find_by_id(user_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("用户不存在".to_string()))?;

        let mut user: user::ActiveModel = user.into();

        if let Some(name) = request.name {
            user.name = Set(name);
        }

        if let Some(email) = request.email {
            user.email = Set(Some(email));
        }

        if let Some(avatar) = request.avatar {
            user.avatar = Set(Some(avatar));
        }

        user.updated_at = Set(Utc::now().naive_utc());

        let updated_user = user.update(db).await?;

        Ok(UserProfile {
            id: updated_user.id.clone(),
            phone: updated_user.phone,
            email: updated_user.email,
            name: updated_user.name,
            avatar: updated_user.avatar,
            role: updated_user.role.to_string(),
            status: updated_user.status.to_string(),
            created_at: updated_user
                .created_at
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        })
    }

    /// 上传头像
    pub async fn upload_avatar(
        db: &DatabaseConnection,
        user_id: String,
        avatar_url: String,
    ) -> Result<UserProfile> {
        let user = User::find_by_id(user_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("用户不存在".to_string()))?;

        let mut user: user::ActiveModel = user.into();
        user.avatar = Set(Some(avatar_url));
        user.updated_at = Set(Utc::now().naive_utc());

        let updated_user = user.update(db).await?;

        Ok(UserProfile {
            id: updated_user.id.clone(),
            phone: updated_user.phone,
            email: updated_user.email,
            name: updated_user.name,
            avatar: updated_user.avatar,
            role: updated_user.role.to_string(),
            status: updated_user.status.to_string(),
            created_at: updated_user
                .created_at
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        })
    }

    /// 修改密码
    pub async fn change_password(
        db: &DatabaseConnection,
        user_id: String,
        request: ChangePasswordRequest,
    ) -> Result<()> {
        let user = User::find_by_id(user_id)
            .one(db)
            .await?
            .ok_or(AppError::NotFound("用户不存在".to_string()))?;

        // 验证旧密码
        if !password::verify_password(&request.old_password, &user.password_hash)? {
            return Err(AppError::BadRequest("原密码错误".to_string()));
        }

        // 哈希新密码
        let new_password_hash = password::hash_password(&request.new_password)?;

        // 更新密码
        let mut user: user::ActiveModel = user.into();
        user.password_hash = Set(new_password_hash);
        user.updated_at = Set(Utc::now().naive_utc());

        user.update(db).await?;

        Ok(())
    }
}
