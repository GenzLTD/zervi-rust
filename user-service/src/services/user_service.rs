use chrono::Utc;
use sea_orm::*;

use crate::{
    dto::{ChangePasswordRequest, UpdateProfileRequest, UserProfile},
    models::user::{self, Entity as User, UserRole, UserStatus},
    utils::{
        error::{AppError, Result},
        password,
    },
};

pub struct UserService;

impl UserService {
    /// 获取用户资料（按 sub 建档）
    ///
    /// - 已存在（以 id = sub 命中）：直接返回，不新增行。
    /// - 不存在（新 sub）：插入前做两道前置校验，任一不满足都**不插入**、两个库行数都不变：
    ///   1. role 必须是 candidate/recruiter/admin，否则返回 400（未知 role 含 observer 不落成 candidate）；
    ///   2. phone 不得已被其它 sub 占用，否则返回 409（不再折成 500）。
    /// - 通过校验才插入一条默认行（status=active / password_hash 空串代表「未设置」），再返回。
    /// - 绝不触碰 adirp_auth。
    pub async fn get_profile(
        db: &DatabaseConnection,
        user_id: String,
        name: String,
        phone: String,
        role: String,
    ) -> Result<UserProfile> {
        // 已存在（以 id = sub 命中）：直接返回，不新增行。
        if let Some(user) = User::find_by_id(user_id.clone()).one(db).await? {
            return Ok(Self::to_profile(&user));
        }

        // 未知 role（含 observer）在插入前拒绝，不落成 candidate，两个库行数都不变。
        let role_enum = match role.as_str() {
            "candidate" => UserRole::Candidate,
            "recruiter" => UserRole::Recruiter,
            "admin" => UserRole::Admin,
            other => {
                return Err(AppError::BadRequest(format!("不支持的 role: {}", other)));
            }
        };

        // 另一个 sub 带上已存在的 phone：返回 409，不插入（不再折成 500）。
        if User::find()
            .filter(user::Column::Phone.eq(&phone))
            .one(db)
            .await?
            .is_some()
        {
            return Err(AppError::DuplicateEntry(format!("手机号 {} 已被占用", phone)));
        }

        let now = Utc::now().naive_utc();
        let active = user::ActiveModel {
            id: Set(user_id.clone()),
            phone: Set(phone),
            email: Set(None),
            password_hash: Set(String::new()), // 空 password_hash 当未设置
            name: Set(name),
            avatar: Set(None),
            role: Set(role_enum),
            status: Set(UserStatus::Active),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let inserted = active.insert(db).await?;
        Ok(Self::to_profile(&inserted))
    }

    /// 把 user::Model 映射为响应 DTO
    fn to_profile(user: &user::Model) -> UserProfile {
        UserProfile {
            id: user.id.clone(),
            phone: user.phone.clone(),
            email: user.email.clone(),
            name: user.name.clone(),
            avatar: user.avatar.clone(),
            role: user.role.to_string(),
            status: user.status.to_string(),
            created_at: user.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        }
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

        // 建档时 password_hash 是空串。空串不是「空密码」，不能拿空口令比对通过。
        if password::password_is_unset(&user.password_hash) {
            return Err(AppError::BadRequest("尚未设置密码".to_string()));
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ConnectionTrait, Database, DatabaseBackend, EntityTrait, PaginatorTrait, Statement,
    };

    async fn empty_users_db() -> DatabaseConnection {
        let mut opt = sea_orm::ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1);
        let db = Database::connect(opt).await.unwrap();
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            r#"CREATE TABLE users (
                id TEXT PRIMARY KEY NOT NULL,
                phone TEXT NOT NULL UNIQUE,
                email TEXT,
                password_hash TEXT NOT NULL,
                name TEXT NOT NULL,
                avatar TEXT,
                role TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )"#,
        ))
        .await
        .unwrap();
        db
    }

    async fn row_count(db: &DatabaseConnection) -> u64 {
        User::find().count(db).await.unwrap()
    }

    #[tokio::test]
    async fn profile_first_call_returns_id_eq_sub() {
        let db = empty_users_db().await;
        let profile = UserService::get_profile(
            &db,
            "sub-win-1".into(),
            "测试".into(),
            "13800138000".into(),
            "candidate".into(),
        )
        .await
        .unwrap();
        assert_eq!(profile.id, "sub-win-1");
        assert_eq!(row_count(&db).await, 1);
        let row = User::find_by_id("sub-win-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(password::password_is_unset(&row.password_hash));
        assert_eq!(row.role, UserRole::Candidate);
    }

    #[tokio::test]
    async fn profile_second_call_does_not_insert() {
        let db = empty_users_db().await;
        UserService::get_profile(
            &db,
            "sub-win-1".into(),
            "测试".into(),
            "13800138000".into(),
            "candidate".into(),
        )
        .await
        .unwrap();
        let profile = UserService::get_profile(
            &db,
            "sub-win-1".into(),
            "测试".into(),
            "13800138000".into(),
            "candidate".into(),
        )
        .await
        .unwrap();
        assert_eq!(profile.id, "sub-win-1");
        assert_eq!(row_count(&db).await, 1);
    }

    #[tokio::test]
    async fn unknown_role_is_400_and_inserts_nothing() {
        let db = empty_users_db().await;
        let err = UserService::get_profile(
            &db,
            "sub-win-1".into(),
            "测试".into(),
            "13800138000".into(),
            "observer".into(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        assert_eq!(row_count(&db).await, 0);
    }

    #[tokio::test]
    async fn phone_conflict_is_409_and_inserts_nothing() {
        let db = empty_users_db().await;
        UserService::get_profile(
            &db,
            "sub-win-1".into(),
            "测试".into(),
            "13800138000".into(),
            "candidate".into(),
        )
        .await
        .unwrap();
        let err = UserService::get_profile(
            &db,
            "sub-win-2".into(),
            "另一个".into(),
            "13800138000".into(),
            "candidate".into(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::DuplicateEntry(_)));
        assert_eq!(row_count(&db).await, 1);
    }

    #[tokio::test]
    async fn change_password_rejects_unset_hash() {
        let db = empty_users_db().await;
        UserService::get_profile(
            &db,
            "sub-win-1".into(),
            "测试".into(),
            "13800138000".into(),
            "candidate".into(),
        )
        .await
        .unwrap();
        let err = UserService::change_password(
            &db,
            "sub-win-1".into(),
            crate::dto::ChangePasswordRequest {
                old_password: "old-pass".into(),
                new_password: "new-pass".into(),
            },
        )
        .await
        .unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert_eq!(msg, "尚未设置密码"),
            other => panic!("应返回尚未设置密码，实际 {other:?}"),
        }
        let row = User::find_by_id("sub-win-1")
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(password::password_is_unset(&row.password_hash));
    }
}
