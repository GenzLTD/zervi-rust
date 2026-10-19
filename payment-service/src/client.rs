//! App Store Server API 客户端（v0.3）。
//!
//! 两个底层模块在这里第一次咬合：
//!   `jwt.rs`（ES256 请求令牌）→ Authorization: Bearer 注入
//!   `jws.rs`（验签器）→ 响应体本身就是 JWS，直接吃
//!
//! ## 端点（官方文档 App Store Server API）
//!
//! | 方法 | 路径 | 用途 |
//! |---|---|---|
//! | GET | `/inApps/v1/subscriptions/{originalTransactionId}` | 订阅状态（含 lastTransactions） |
//! | GET | `/inApps/v1/transactions/{transactionId}` | 单笔交易 |
//! | GET | `/inApps/v1/history/{originalTransactionId}?revision=` | 交易历史（分页） |
//!
//! ## Host（environment 必须分流，沙盒购买绝不能落生产权益）
//!
//! - Production: `https://api.storekit.itunes.apple.com`
//! - Sandbox:    `https://api.storekit-sandbox.itunes.apple.com`
//!
//! ## 错误语义（routes 层据此映射）
//!
//! - 401/403 → 令牌或密钥问题（配置错误，重试无意义）
//! - 404 → originalTransactionId 不存在（客户端上报假交易）
//! - 429 → 限流（可重试）
//! - 5xx → Apple 侧故障（可重试）

use crate::jws::{JwsVerifier, TransactionInfo};
use crate::jwt::{AppleJwtConfig, AppleJwtSigner, JwtError};
use serde::Deserialize;
use thiserror::Error;

const PROD_HOST: &str = "https://api.storekit.itunes.apple.com";
const SANDBOX_HOST: &str = "https://api.storekit-sandbox.itunes.apple.com";

/// 订阅状态码（Apple status 值，非 HTTP）。
pub mod status {
    pub const ACTIVE: i32 = 1;
    pub const EXPIRED: i32 = 2;
    pub const BILLING_RETRY: i32 = 3;
    pub const GRACE_PERIOD: i32 = 4;
    pub const REVOKED: i32 = 5;
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("http transport failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("app store responded {status}: {body}")]
    Api { status: u16, body: String },
    #[error("jwt signing failed: {0}")]
    Jwt(#[from] JwtError),
    #[error("response jws decode failed: {0}")]
    Decode(String),
    /// Apple 限流 / 5xx —— routes 层可安全重试。
    #[error("app store transient failure (status {status})")]
    Transient { status: u16 },
}

/// App Store 返回的响应一律 camelCase（与外层通知 JWS 同风格）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionStatus {
    pub environment: String,
    pub bundle_id: String,
    #[serde(default)]
    pub app_apple_id: Option<i64>,
    pub last_transactions: Vec<LastTransaction>,
}

/// `lastTransactions[]`：status + 两段内层 JWS。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastTransaction {
    pub original_transaction_id: String,
    /// 见 [`status`] 模块常量。
    pub status: i32,
    pub signed_transaction_info: String,
    pub signed_renewal_info: String,
}

/// `GET /transactions/{id}` 的响应包装。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransactionResponse {
    signed_transaction_info: String,
}

/// `GET /history/{id}` 的响应包装（分页）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryResponse {
    has_more: bool,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    signed_transactions: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Sandbox,
    Production,
}

impl Environment {
    /// Apple 字符串值（JWS / API 响应里的 `"Sandbox"` / `"Production"`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "Sandbox",
            Self::Production => "Production",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Sandbox" => Some(Self::Sandbox),
            "Production" => Some(Self::Production),
            _ => None,
        }
    }

    fn base_url(self) -> &'static str {
        match self {
            Self::Sandbox => SANDBOX_HOST,
            Self::Production => PROD_HOST,
        }
    }
}

pub struct AppStoreClient {
    http: reqwest::Client,
    env: Environment,
    signer: AppleJwtSigner,
}

impl AppStoreClient {
    pub fn new(env: Environment, config: AppleJwtConfig) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self {
            http,
            env,
            signer: AppleJwtSigner::new(config)?,
        })
    }

    pub fn environment(&self) -> Environment {
        self.env
    }

    /// GET + Bearer 注入 + 状态码分类。成功返回反序列化前的 body 字符串，
    /// 由调用方按各自响应类型解码（响应体都很小，无流式需求）。
    async fn get_raw(&self, path: &str) -> Result<String, ClientError> {
        let token = self.signer.token()?;
        let resp = self
            .http
            .get(format!("{}{path}", self.env.base_url()))
            .bearer_auth(token)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp.text().await?);
        }
        let body = resp.text().await.unwrap_or_default();
        let code = status.as_u16();
        if code == 429 || code >= 500 {
            // 丢弃 body（可能很大），只保留状态码供重试决策。
            Err(ClientError::Transient { status: code })
        } else {
            Err(ClientError::Api { status: code, body })
        }
    }

    /// 单笔交易。响应是 JWS 包装，验签后给出 `TransactionInfo`。
    pub async fn transaction(
        &self,
        transaction_id: &str,
        verifier: &JwsVerifier,
    ) -> Result<TransactionInfo, ClientError> {
        let raw = self
            .get_raw(&format!("/inApps/v1/transactions/{transaction_id}"))
            .await?;
        let wrap: TransactionResponse =
            serde_json::from_str(&raw).map_err(|e| ClientError::Decode(e.to_string()))?;
        verifier
            .decode_transaction(&wrap.signed_transaction_info)
            .map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// 订阅状态（verify / restore 端点的核心调用）。
    /// 返回原始 `SubscriptionStatus`——两段内层 JWS 由调用方按需验签解码，
    /// 因为不同 notification 只关心其中一段。
    pub async fn subscription_status(
        &self,
        original_transaction_id: &str,
    ) -> Result<SubscriptionStatus, ClientError> {
        let raw = self
            .get_raw(&format!(
                "/inApps/v1/subscriptions/{original_transaction_id}"
            ))
            .await?;
        serde_json::from_str(&raw).map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// 交易历史全量拉取（自动翻页直到 `hasMore = false`）。
    /// 注意：Apple 对 history 有速率限制，页与页之间不额外 sleep——
    /// 429 会以 [`ClientError::Transient`] 冒出，由上层重试策略统一处理。
    pub async fn transaction_history(
        &self,
        original_transaction_id: &str,
        verifier: &JwsVerifier,
    ) -> Result<Vec<TransactionInfo>, ClientError> {
        let mut out = Vec::new();
        let mut revision: Option<String> = None;
        loop {
            let path = match &revision {
                Some(r) => format!(
                    "/inApps/v1/history/{original_transaction_id}?revision={r}"
                ),
                None => format!("/inApps/v1/history/{original_transaction_id}"),
            };
            let raw = self.get_raw(&path).await?;
            let page: HistoryResponse =
                serde_json::from_str(&raw).map_err(|e| ClientError::Decode(e.to_string()))?;
            for jws in &page.signed_transactions {
                out.push(
                    verifier
                        .decode_transaction(jws)
                        .map_err(|e| ClientError::Decode(e.to_string()))?,
                );
            }
            if !page.has_more {
                return Ok(out);
            }
            revision = page.revision;
        }
    }
}

// ============================================================
// 测试（纯反序列化 / 映射，零网络 —— HTTP 层留给联调与 wiremock 阶段）
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_roundtrip() {
        for e in [Environment::Sandbox, Environment::Production] {
            assert_eq!(Environment::parse(e.as_str()), Some(e));
        }
        assert_eq!(Environment::parse("sandbox"), None, "大小写敏感：Apple 精确返回 Sandbox/Production");
        assert_eq!(Environment::parse(""), None);
    }

    #[test]
    fn sandbox_and_prod_hosts_differ() {
        assert_ne!(Environment::Sandbox.base_url(), Environment::Production.base_url());
        assert!(Environment::Sandbox.base_url().contains("sandbox"));
    }

    #[test]
    fn subscription_status_decodes_camel_case() {
        let raw = r#"{
            "environment": "Sandbox",
            "bundleId": "com.yedall.planetx",
            "appAppleId": 1234567890,
            "lastTransactions": [{
                "originalTransactionId": "orig-001",
                "status": 1,
                "signedTransactionInfo": "x.y.z",
                "signedRenewalInfo": "a.b.c"
            }]
        }"#;
        let s: SubscriptionStatus = serde_json::from_str(raw).unwrap();
        assert_eq!(s.environment, "Sandbox");
        assert_eq!(s.bundle_id, "com.yedall.planetx");
        assert_eq!(s.app_apple_id, Some(1234567890));
        assert_eq!(s.last_transactions.len(), 1);
        assert_eq!(s.last_transactions[0].original_transaction_id, "orig-001");
        assert_eq!(s.last_transactions[0].status, status::ACTIVE);
        assert_eq!(s.last_transactions[0].signed_transaction_info, "x.y.z");
    }

    #[test]
    fn subscription_status_tolerates_missing_app_apple_id() {
        let raw = r#"{ "environment": "Production", "bundleId": "b", "lastTransactions": [] }"#;
        let s: SubscriptionStatus = serde_json::from_str(raw).unwrap();
        assert_eq!(s.app_apple_id, None);
        assert!(s.last_transactions.is_empty());
    }

    #[test]
    fn transaction_response_decodes() {
        let raw = r#"{ "signedTransactionInfo": "h.p.s" }"#;
        let w: TransactionResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(w.signed_transaction_info, "h.p.s");
    }

    #[test]
    fn history_page_decodes_with_optional_fields() {
        let raw = r#"{ "hasMore": true, "revision": "r2", "signedTransactions": ["t1", "t2"] }"#;
        let h: HistoryResponse = serde_json::from_str(raw).unwrap();
        assert!(h.has_more);
        assert_eq!(h.revision.as_deref(), Some("r2"));
        assert_eq!(h.signed_transactions.len(), 2);

        let raw_end = r#"{ "hasMore": false }"#;
        let h2: HistoryResponse = serde_json::from_str(raw_end).unwrap();
        assert!(!h2.has_more);
        assert!(h2.revision.is_none());
        assert!(h2.signed_transactions.is_empty());
    }

    #[test]
    fn status_constants_match_apple_docs() {
        // Apple Status: 1 active / 2 expired / 3 billing retry / 4 grace period / 5 revoked
        assert_eq!(status::ACTIVE, 1);
        assert_eq!(status::EXPIRED, 2);
        assert_eq!(status::BILLING_RETRY, 3);
        assert_eq!(status::GRACE_PERIOD, 4);
        assert_eq!(status::REVOKED, 5);
    }
}
