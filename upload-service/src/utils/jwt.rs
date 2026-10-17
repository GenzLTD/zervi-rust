use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use super::error::{AppError, Result};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,   // user_id
    pub role: String,  // 用户角色
    pub exp: i64,      // 过期时间
    pub iat: i64,      // 签发时间
    pub name: String,  // 用户名
    pub phone: String, // 手机号
}

#[derive(Clone)]
pub struct JwtService {
    secret: String,
    expiration: i64, // 秒
}

impl JwtService {
    pub fn new(secret: String, expiration: i64) -> Self {
        Self { secret, expiration }
    }

    /// 生成JWT Token
    pub fn generate_token(
        &self,
        user_id: String,
        role: &str,
        name: &str,
        phone: &str,
    ) -> Result<String> {
        let now = Utc::now();
        let exp = (now + Duration::seconds(self.expiration)).timestamp();

        let claims = Claims {
            sub: user_id,
            role: role.to_string(),
            exp,
            iat: now.timestamp(),
            name: name.to_string(),
            phone: phone.to_string(),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_ref()),
        )
        .map_err(AppError::from)
    }

    /// 验证JWT Token
    pub fn verify_token(&self, token: &str) -> Result<Claims> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_ref()),
            &Validation::default(),
        )
        .map_err(|e| {
            tracing::warn!("Token verification failed: {:?}", e);
            if e.to_string().contains("ExpiredSignature") {
                AppError::TokenExpired
            } else {
                AppError::from(e)
            }
        })?;

        Ok(token_data.claims)
    }

    /// 从Token中提取用户ID（不验证过期时间）
    pub fn decode_token_without_validation(&self, token: &str) -> Result<Claims> {
        let mut validation = Validation::default();
        validation.validate_exp = false;

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_ref()),
            &validation,
        )
        .map_err(AppError::from)?;

        Ok(token_data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_generation_and_verification() {
        let jwt_service = JwtService::new("test_secret".to_string(), 3600);
        // 既有缺陷修复：原来写 `Uuid::new_v4()`，两个问题 ——
        // ① 测试模块里没导入 Uuid（整个 crate 的测试目标编译不过）；
        // ② `generate_token` 要的是 `String`，claims.sub 也是 `String`，
        //    旧写法是早期 API 的残留。这里改成字符串并 clone 一次。
        let user_id = uuid::Uuid::new_v4().to_string();

        let token = jwt_service
            .generate_token(user_id.clone(), "candidate", "测试用户", "13800138000")
            .unwrap();

        let claims = jwt_service.verify_token(&token).unwrap();
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.role, "candidate");
    }
}
