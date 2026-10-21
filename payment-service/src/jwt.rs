//! Apple App Store Server API 请求令牌签发（v0.2）。
//!
//! ## 与仓内既有 JWT 代码的区别（零复用，勿合并）
//!
//! | | auth/user/upload-service 的 jwt.rs | 本模块 |
//! |---|---|---|
//! | 向谁证明身份 | 我们自己的用户 | **Apple**（API 调用方身份） |
//! | 算法 | HS256（对称 HMAC） | **ES256**（EC P-256 非对称） |
//! | 密钥 | `JWT_SECRET` 环境变量 | App Store Connect 的 **`.p8` 私钥** |
//! | Claims | `sub`/`phone`/`exp` | `iss`/`iat`/`exp`/`aud`/`bid` |
//! | Header 特例 | 无 | 必须带 `kid`（Key ID） |
//!
//! ## Apple 约束（官方文档 App Store Server API → Generating JSON Web Tokens）
//!
//! - `aud` 固定 `"appstoreconnect-v1"`；`iss` = Issuer ID（App Store Connect →
//!   Users and Access → Integrations → In-App Purchase）；`bid` = Bundle ID
//! - token 有效期**最长 60 分钟**，本实现取 20 分钟 TTL + 剩余不足 2 分钟重签，
//!   既避免每请求重签的浪费，也远离 60 分钟红线
//! - `.p8` 私钥从 App Store Connect 下载，**仅一次，丢失只能重签作废重来**
//!
//! ## 配置（环境变量，生产由部署注入）
//!
//! - `APPLE_ISSUER_ID`    — Issuer ID（UUID 形态）
//! - `APPLE_KEY_ID`       — Key ID（10 字符，随 .p8 一起给出）
//! - `APPLE_BUNDLE_ID`    — 如 `com.yedall.planetx`
//! - `APPLE_PRIVATE_KEY_PEM` — 私钥 PEM 内容（生产优先）
//! - `APPLE_PRIVATE_KEY_PATH` — `.p8` 文件路径（本地开发回退）

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use std::sync::Mutex;
use thiserror::Error;

const AUD_APPLE: &str = "appstoreconnect-v1";

/// 保守 TTL：远离 Apple 60 分钟上限。
const DEFAULT_TTL_SECS: i64 = 20 * 60;
/// 剩余寿命低于此值即重签（容忍时钟漂移与请求在途耗时）。
const DEFAULT_REFRESH_MARGIN_SECS: i64 = 2 * 60;

#[derive(Debug, Error)]
pub enum JwtError {
    #[error("missing Apple JWT config: set {0}")]
    MissingConfig(&'static str),
    #[error("private key load failed: {0}")]
    KeyLoad(String),
    #[error("token sign failed: {0}")]
    Sign(String),
}

// ============================================================
// 配置
// ============================================================

#[derive(Debug, Clone)]
pub struct AppleJwtConfig {
    pub issuer_id: String,
    pub key_id: String,
    pub bundle_id: String,
    /// PKCS#8（或 SEC1）PEM 私钥内容。
    pub private_key_pem: String,
}

impl AppleJwtConfig {
    /// 按注释表加载：PEM 内容优先，其次 .p8 文件路径。
    pub fn from_env() -> Result<Self, JwtError> {
        let issuer_id = nonempty_env("APPLE_ISSUER_ID", "APPLE_ISSUER_ID")?;
        let key_id = nonempty_env("APPLE_KEY_ID", "APPLE_KEY_ID")?;
        let bundle_id = nonempty_env("APPLE_BUNDLE_ID", "APPLE_BUNDLE_ID")?;

        let private_key_pem = match std::env::var("APPLE_PRIVATE_KEY_PEM") {
            Ok(pem) if !pem.trim().is_empty() => pem,
            _ => {
                let path = std::env::var("APPLE_PRIVATE_KEY_PATH")
                    .map_err(|_| JwtError::MissingConfig("APPLE_PRIVATE_KEY_PEM（或 APPLE_PRIVATE_KEY_PATH）"))?;
                std::fs::read_to_string(&path)
                    .map_err(|e| JwtError::KeyLoad(format!("read {path:?} failed: {e}")))?
            }
        };

        Ok(Self { issuer_id, key_id, bundle_id, private_key_pem })
    }
}

fn nonempty_env(key: &'static str, _label: &'static str) -> Result<String, JwtError> {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or(JwtError::MissingConfig(key))
}

// ============================================================
// Claims
// ============================================================

#[derive(Serialize)]
struct AppleClaims {
    iss: String,
    iat: i64,
    exp: i64,
    aud: String,
    bid: String,
}

// ============================================================
// 签发器（带缓存）
// ============================================================

struct CachedToken {
    token: String,
    /// 过期时刻（Unix 秒）。
    expires_at: i64,
}

pub struct AppleJwtSigner {
    key: EncodingKey,
    issuer_id: String,
    key_id: String,
    bundle_id: String,
    ttl_secs: i64,
    refresh_margin_secs: i64,
    cached: Mutex<Option<CachedToken>>,
}

impl AppleJwtSigner {
    pub fn new(config: AppleJwtConfig) -> Result<Self, JwtError> {
        let key = EncodingKey::from_ec_pem(config.private_key_pem.as_bytes())
            .map_err(|e| JwtError::KeyLoad(e.to_string()))?;
        Ok(Self {
            key,
            issuer_id: config.issuer_id,
            key_id: config.key_id,
            bundle_id: config.bundle_id,
            ttl_secs: DEFAULT_TTL_SECS,
            refresh_margin_secs: DEFAULT_REFRESH_MARGIN_SECS,
            cached: Mutex::new(None),
        })
    }

    /// 供测试注入短 TTL / 窄余量。
    #[cfg(test)]
    fn with_ttl(mut self, ttl_secs: i64, refresh_margin_secs: i64) -> Self {
        self.ttl_secs = ttl_secs;
        self.refresh_margin_secs = refresh_margin_secs;
        self
    }

    /// 取有效 token。缓存命中且剩余寿命充足 ⇒ 直接复用；否则重签。
    ///
    /// 同步锁即可：签发是纯 CPU（µs 级），无 await 点，不值得上 tokio 锁。
    pub fn token(&self) -> Result<String, JwtError> {
        let mut guard = self.cached.lock().expect("AppleJwtSigner mutex poisoned");
        let now = unix_now();
        if let Some(c) = guard.as_ref() {
            if now < c.expires_at - self.refresh_margin_secs {
                return Ok(c.token.clone());
            }
        }
        let claims = AppleClaims {
            iss: self.issuer_id.clone(),
            iat: now,
            exp: now + self.ttl_secs,
            aud: AUD_APPLE.to_string(),
            bid: self.bundle_id.clone(),
        };
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        let token = encode(&header, &claims, &self.key)
            .map_err(|e| JwtError::Sign(e.to_string()))?;
        *guard = Some(CachedToken { token: token.clone(), expires_at: claims.exp });
        Ok(token)
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::{STANDARD as B64_STD, URL_SAFE_NO_PAD as B64_URL};
    use base64::Engine as _;
    use serde_json::Value;

    /// 用 ring 现场生成一对 P-256 密钥，拼成 PKCS#8 PEM —— 不落盘、不依赖外部文件。
    /// （ring 已是 jws.rs 的依赖，测试零新增。）
    fn test_p8_pem() -> String {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &rng,
        )
        .expect("generate P-256 keypair");
        let b64 = B64_STD.encode(pkcs8.as_ref());
        format!("-----BEGIN PRIVATE KEY-----\n{b64}\n-----END PRIVATE KEY-----\n")
    }

    fn test_config() -> AppleJwtConfig {
        AppleJwtConfig {
            issuer_id: "99b16627-4b6f-4f4d-a1f8-2b6a1d8a7e5f".into(),
            key_id: "TESTKEY123".into(),
            bundle_id: "com.yedall.planetx.test".into(),
            private_key_pem: test_p8_pem(),
        }
    }

    /// 三段 JWS 结构拆解（base64url 解码，不验签——签发正确性由 jsonwebtoken 保证，
    /// 验签路径在 jws.rs 已覆盖）。
    fn decode_segments(token: &str) -> (Value, Value) {
        let mut parts = token.split('.');
        let h = parts.next().unwrap();
        let p = parts.next().unwrap();
        assert_eq!(parts.next().is_some(), true, "signature segment present");
        assert_eq!(parts.next(), None, "exactly 3 segments");
        let h: Value = serde_json::from_slice(&B64_URL.decode(h).unwrap()).unwrap();
        let p: Value = serde_json::from_slice(&B64_URL.decode(p).unwrap()).unwrap();
        (h, p)
    }

    #[test]
    fn token_shape_matches_apple_contract() {
        let signer = AppleJwtSigner::new(test_config()).unwrap();
        let token = signer.token().unwrap();
        let (header, claims) = decode_segments(&token);

        // header：alg ES256 + kid
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "TESTKEY123");
        // claims：iss / aud / bid + TTL 口径
        assert_eq!(claims["aud"], "appstoreconnect-v1");
        assert_eq!(claims["iss"], "99b16627-4b6f-4f4d-a1f8-2b6a1d8a7e5f");
        assert_eq!(claims["bid"], "com.yedall.planetx.test");
        let (iat, exp) = (claims["iat"].as_i64().unwrap(), claims["exp"].as_i64().unwrap());
        assert_eq!(exp - iat, DEFAULT_TTL_SECS);
        assert!(exp - iat <= 60 * 60, "必须远离 Apple 60 分钟上限");
    }

    #[test]
    fn cached_token_reused_until_margin() {
        let signer = AppleJwtSigner::new(test_config()).unwrap();
        let t1 = signer.token().unwrap();
        let t2 = signer.token().unwrap();
        assert_eq!(t1, t2, "寿命充足时必须复用缓存，不能每请求重签");
    }

    #[test]
    fn refresh_when_within_margin() {
        // TTL=60s、margin=300s ⇒ 首次签发后剩余寿命(≤60s)必然 < margin ⇒ 必重签。
        let signer = AppleJwtSigner::new(test_config())
            .unwrap()
            .with_ttl(60, 300);
        let t1 = signer.token().unwrap();
        let t2 = signer.token().unwrap();
        assert_ne!(t1, t2, "剩余寿命不足 margin 时必须重签");
    }

    #[test]
    fn bad_pem_rejected() {
        let mut cfg = test_config();
        cfg.private_key_pem = "not a pem at all".into();
        assert!(matches!(
            AppleJwtSigner::new(cfg),
            Err(JwtError::KeyLoad(_))
        ));
    }
}
