/*!
 * Rust微服务集群 - 服务间认证库
 *
 * 用于Rust服务调用Python服务时的X-Service-Token生成和验证
 */

use anyhow::Result;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// 服务Token Claims
#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceTokenClaims {
    /// 调用方服务ID
    pub service_id: String,
    /// 目标服务名
    pub target: String,
    /// 过期时间（Unix时间戳）
    pub exp: i64,
    /// 签发时间（Unix时间戳）
    pub iat: i64,
}

/// 生成服务间Token
///
/// # Arguments
/// * `service_id` - 调用方服务ID（如："upload-service"）
/// * `target` - 目标服务名（如："pyresparser", "paddleocr"）
/// * `secret` - 密钥（应与Python服务共享）
///
/// # Returns
/// * `Result<String>` - JWT Token字符串
///
/// # Example
/// ```
/// let token = generate_service_token("upload-service", "pyresparser", "secret")?;
/// ```
pub fn generate_service_token(service_id: &str, target: &str, secret: &str) -> Result<String> {
    let now = Utc::now();
    let exp = now + Duration::minutes(5); // 5分钟有效期

    let claims = ServiceTokenClaims {
        service_id: service_id.to_string(),
        target: target.to_string(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok(token)
}

/// 验证服务Token（如果需要在Rust端验证）
pub fn verify_service_token(
    token: &str,
    secret: &str,
    expected_target: &str,
) -> Result<ServiceTokenClaims> {
    let validation = Validation::new(Algorithm::HS256);

    let token_data = decode::<ServiceTokenClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    // 验证目标服务
    if token_data.claims.target != expected_target {
        anyhow::bail!("Target service mismatch");
    }

    Ok(token_data.claims)
}

/// HTTP客户端扩展：自动添加服务Token
pub mod http {
    use super::*;
    use anyhow::Result;
    use reqwest::{Client, Response};

    /// 带服务Token的HTTP POST请求
    pub async fn post_with_service_token(
        client: &Client,
        url: &str,
        service_id: &str,
        target: &str,
        secret: &str,
        json_body: &serde_json::Value,
    ) -> Result<Response> {
        let token = generate_service_token(service_id, target, secret)?;

        let response = client
            .post(url)
            .header("X-Service-Token", token)
            .header("Content-Type", "application/json")
            .json(json_body)
            .send()
            .await?;

        Ok(response)
    }

    /// 带服务Token的multipart请求
    pub async fn post_multipart_with_service_token(
        client: &Client,
        url: &str,
        service_id: &str,
        target: &str,
        secret: &str,
        form: reqwest::multipart::Form,
    ) -> Result<Response> {
        let token = generate_service_token(service_id, target, secret)?;

        let response = client
            .post(url)
            .header("X-Service-Token", token)
            .multipart(form)
            .send()
            .await?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token() {
        let token = generate_service_token("upload-service", "pyresparser", "test-secret").unwrap();
        assert!(!token.is_empty());
        println!("Generated token: {}", token);
    }

    #[test]
    fn test_verify_token() {
        let secret = "test-secret";
        let token = generate_service_token("upload-service", "pyresparser", secret).unwrap();
        let claims = verify_service_token(&token, secret, "pyresparser").unwrap();

        assert_eq!(claims.service_id, "upload-service");
        assert_eq!(claims.target, "pyresparser");
    }
}
