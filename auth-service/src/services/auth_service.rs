use chrono::Utc;
use redis::AsyncCommands;
use sea_orm::*;
use uuid::Uuid;

use crate::{
    dto::{
        LoginByCodeRequest, LoginByPasswordRequest, LoginResponse, RegisterRequest,
        SendCodeRequest, SendCodeResponse, UserInfo,
    },
    models::{
        user::{self, Entity as User, UserRole, UserStatus},
        UserModel,
    },
    utils::{
        error::{AppError, Result},
        jwt::JwtService,
        password,
    },
};

pub struct AuthService;

impl AuthService {
    /// 发送验证码
    pub async fn send_code(
        redis: &mut redis::aio::ConnectionManager,
        request: SendCodeRequest,
    ) -> Result<SendCodeResponse> {
        // 生成6位随机验证码
        let code = format!("{:06}", rand::random::<u32>() % 1000000);

        // 存储到Redis，过期时间5分钟
        let key = format!("sms:{}:{}", request.code_type, request.phone);
        redis.set_ex(&key, &code, 300).await?;

        // TODO: 这里调用实际的短信服务商API发送验证码
        tracing::info!("发送验证码到 {}: {}", request.phone, code);

        Ok(SendCodeResponse {
            message: "验证码已发送".to_string(),
            expire: 300,
        })
    }

    /// 验证码登录
    pub async fn login_by_code(
        db: &DatabaseConnection,
        redis: &mut redis::aio::ConnectionManager,
        jwt_service: &JwtService,
        request: LoginByCodeRequest,
    ) -> Result<LoginResponse> {
        // 验证验证码
        let key = format!("sms:login:{}", request.phone);
        let stored_code: Option<String> = redis.get(&key).await?;

        match stored_code {
            None => return Err(AppError::InvalidVerificationCode),
            Some(code) if code != request.code => return Err(AppError::InvalidVerificationCode),
            _ => {}
        }

        // 验证码正确，删除
        redis.del(&key).await?;

        // 查找或创建用户
        let user = User::find()
            .filter(user::Column::Phone.eq(&request.phone))
            .one(db)
            .await?;

        let user = match user {
            Some(u) => u,
            None => {
                // 用户不存在，自动注册
                let new_user = user::ActiveModel {
                    id: Set(Uuid::new_v4().to_string()),
                    phone: Set(request.phone.clone()),
                    password_hash: Set(String::new()), // 验证码登录不需要密码
                    name: Set(format!("用户{}", &request.phone[7..])),
                    role: Set(UserRole::Candidate),
                    status: Set(UserStatus::Active),
                    created_at: Set(Utc::now().naive_utc()),
                    updated_at: Set(Utc::now().naive_utc()),
                    ..Default::default()
                };

                new_user.insert(db).await?
            }
        };

        // 生成Token
        let token = jwt_service.generate_token(
            user.id.clone(),
            &user.role.to_string(),
            &user.name,
            &user.phone,
        )?;

        Ok(LoginResponse {
            token,
            user_info: UserInfo {
                id: user.id.clone(),
                name: user.name,
                phone: user.phone,
                avatar: user.avatar,
                role: user.role.to_string(),
            },
        })
    }

    /// 密码登录
    pub async fn login_by_password(
        db: &DatabaseConnection,
        jwt_service: &JwtService,
        request: LoginByPasswordRequest,
    ) -> Result<LoginResponse> {
        // 查找用户（支持手机号或邮箱登录）
        let user = User::find()
            .filter(
                Condition::any()
                    .add(user::Column::Phone.eq(&request.account))
                    .add(user::Column::Email.eq(&request.account)),
            )
            .one(db)
            .await?
            .ok_or(AppError::InvalidCredentials)?;

        // 验证密码
        if !password::verify_password(&request.password, &user.password_hash)? {
            return Err(AppError::InvalidCredentials);
        }

        // 检查账号状态
        if user.status != UserStatus::Active {
            return Err(AppError::Forbidden);
        }

        // 生成Token
        let token = jwt_service.generate_token(
            user.id.clone(),
            &user.role.to_string(),
            &user.name,
            &user.phone,
        )?;

        Ok(LoginResponse {
            token,
            user_info: UserInfo {
                id: user.id.clone(),
                name: user.name,
                phone: user.phone,
                avatar: user.avatar,
                role: user.role.to_string(),
            },
        })
    }

    /// 注册
    pub async fn register(
        db: &DatabaseConnection,
        redis: &mut redis::aio::ConnectionManager,
        jwt_service: &JwtService,
        request: RegisterRequest,
    ) -> Result<LoginResponse> {
        // 验证验证码
        let key = format!("sms:register:{}", request.phone);
        let stored_code: Option<String> = redis.get(&key).await?;

        match stored_code {
            None => return Err(AppError::InvalidVerificationCode),
            Some(code) if code != request.code => return Err(AppError::InvalidVerificationCode),
            _ => {}
        }

        // 验证码正确，删除
        redis.del(&key).await?;

        // 检查手机号是否已存在
        let existing_user = User::find()
            .filter(user::Column::Phone.eq(&request.phone))
            .one(db)
            .await?;

        if existing_user.is_some() {
            return Err(AppError::DuplicateEntry("该手机号已注册".to_string()));
        }

        // 哈希密码
        let password_hash = password::hash_password(&request.password)?;

        // 创建用户
        let new_user = user::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            phone: Set(request.phone),
            password_hash: Set(password_hash),
            name: Set(request.name),
            role: Set(request.role.clone()),
            status: Set(UserStatus::Active),
            created_at: Set(Utc::now().naive_utc()),
            updated_at: Set(Utc::now().naive_utc()),
            ..Default::default()
        };

        let user = new_user.insert(db).await?;

        // 生成Token
        let token = jwt_service.generate_token(
            user.id.clone(),
            &user.role.to_string(),
            &user.name,
            &user.phone,
        )?;

        Ok(LoginResponse {
            token,
            user_info: UserInfo {
                id: user.id.clone(),
                name: user.name,
                phone: user.phone,
                avatar: user.avatar,
                role: user.role.to_string(),
            },
        })
    }
}
