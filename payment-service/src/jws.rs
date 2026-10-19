//! Apple JWS 验签与三层解码 —— payment-service 的信任根基。
//!
//! ## 三层 JWS 结构（App Store Server API / Server Notification V2）
//!
//! ```text
//! signedPayload (外层, 字段 camelCase)
//! ├── notificationType / subtype / notificationUUID / version / signedDate
//! └── data (NotificationData)
//!     ├── signedTransactionInfo → 内层 JWS (TransactionInfo, snake_case)
//!     └── signedRenewalInfo     → 内层 JWS (RenewalInfo,   snake_case)
//! ```
//!
//! ## 验签口径（不满足任何一条即拒）
//!
//! 1. `alg` 固定 `ES256`，其余算法直接拒（防算法混淆攻击）；
//! 2. `header.x5c` 至少含 `[leaf, WWDR intermediate]` 两级证书；
//! 3. 根证书【不信任网络下发】：从本地 pin（`APPLE_ROOT_CA_PEM` 环境变量或
//!    `certs/AppleRootCA-G3.crt` 文件），并比对 SHA-256 指纹；
//! 4. 链验证：`leaf ← intermediate ← pinned root`，逐级验签；
//! 5. 若 `x5c` 自带第三级（root），其 DER 指纹必须与 pin 一致；
//! 6. 内层 JWS（transaction / renewal）与外层**各自独立验链**
//!    （Apple 用不同签名实例签发，不能复用外层验签结果）；
//! 7. 时间戳一律 ms since epoch（`i64`），时区换算交给 state_machine。
//!
//! ## 证书获取
//!
//! Apple Root CA - G3（及 WWDR 中间证书）从官方下载后放入 `certs/`：
//!   https://www.apple.com/certificateauthority/
//! 生产以环境变量 `APPLE_ROOT_CA_PEM` 优先。

use base64::engine::general_purpose::{STANDARD as B64_STD, URL_SAFE_NO_PAD as B64_URL};
use base64::Engine as _;
use ring::digest::{digest, SHA256};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED as ES256_ALG};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use x509_parser::certificate::X509Certificate;
use x509_parser::prelude::FromDer;

// ============================================================
// 错误
// ============================================================

#[derive(Debug, Error)]
pub enum JwsError {
    #[error("malformed JWS: expected exactly 3 dot-separated segments")]
    Malformed,
    #[error("header alg must be ES256, got {0:?}")]
    UnsupportedAlg(String),
    #[error("x5c chain must contain at least [leaf, intermediate]")]
    MissingChain,
    #[error("pinned root CA required: set APPLE_ROOT_CA_PEM or provide certs/AppleRootCA-G3.crt")]
    RootNotConfigured,
    #[error("x5c embedded root fingerprint mismatch: pinned={pinned}, got={embedded}")]
    RootFingerprintMismatch { pinned: String, embedded: String },
    #[error("certificate chain signature invalid: {0}")]
    ChainInvalid(String),
    #[error("ES256 signature verification failed")]
    BadSignature,
    #[error("leaf public key is not an ECDSA P-256 key (65-byte uncompressed point)")]
    NotP256,
    #[error("payload decode failed: {0}")]
    PayloadDecode(String),
    #[error("base64 decode failed: {0}")]
    Base64(String),
    #[error("x509 parse failed: {0}")]
    X509(String),
}

impl JwsError {
    /// 是否属于"签名/证书不可信"类——webhook 层据此区分
    /// 401（不可信，不落库）与 500（可信但内部处理失败，仍回 200）。
    pub fn is_untrusted(&self) -> bool {
        matches!(
            self,
            JwsError::UnsupportedAlg(_)
                | JwsError::MissingChain
                | JwsError::RootNotConfigured
                | JwsError::RootFingerprintMismatch { .. }
                | JwsError::ChainInvalid(_)
                | JwsError::BadSignature
                | JwsError::NotP256
        )
    }
}

// ============================================================
// JWS header
// ============================================================

#[derive(Debug, Deserialize)]
struct JwsHeader {
    alg: String,
    #[serde(default)]
    x5c: Vec<String>,
}

// ============================================================
// 外层：Server Notification V2（字段 camelCase）
// ============================================================

/// 15 种 notificationType 中必须落状态机的 8 种之外的常量也照单全收，
/// 是否处理由 state_machine 决定，本层只负责"可信地解码"。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodedNotification {
    pub notification_type: String,
    #[serde(default)]
    pub subtype: Option<String>,
    pub notification_uuid: String,
    pub version: String, // "2.0"
    pub signed_date: i64,
    pub data: NotificationData,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationData {
    pub app_apple_id: i64,
    pub bundle_id: String,
    pub bundle_version: String,
    pub environment: String, // "Sandbox" | "Production"
    pub signed_transaction_info: String,
    /// 部分通知（如 TEST）可能缺失，解码方需判空。
    #[serde(default)]
    pub signed_renewal_info: Option<String>,
}

// ============================================================
// 内层：JWSTransactionInfoDecodedPayload（字段 snake_case）
// ============================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransactionInfo {
    pub transaction_id: String,
    pub original_transaction_id: String,
    #[serde(default)]
    pub web_order_line_item_id: Option<String>,
    pub bundle_id: String,
    pub product_id: String,
    #[serde(default)]
    pub subscription_group_identifier: Option<String>,
    pub purchase_date: i64,
    pub original_purchase_date: i64,
    #[serde(default)]
    pub expires_date: Option<i64>,
    #[serde(default)]
    pub quantity: Option<i32>,
    /// "Auto-Renewable Subscription" | "Consumable" | "Non-Consumable" | ...
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// 购买前由我们生成并下发的 UUID —— 跨设备恢复/防 A买B用的关键。
    #[serde(default)]
    pub app_account_token: Option<String>,
    /// "PURCHASED" | "FAMILY_SHARED"
    #[serde(default)]
    pub in_app_ownership_type: Option<String>,
    pub signed_date: i64,
    pub environment: String,
    #[serde(default)]
    pub revocation_date: Option<i64>,
    /// Apple revocationReason 原值（"REFUNDED_DUE_TO_ISSUE" 等）留痕。
    #[serde(default)]
    pub revocation_reason: Option<String>,
    #[serde(default)]
    pub offer_type: Option<i32>,
    #[serde(default)]
    pub offer_identifier: Option<String>,
    /// milliunits（千分之一货币单位），如 $5.99 = 5990。
    #[serde(default)]
    pub price: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
}

// ============================================================
// 内层：JWSRenewalInfoDecodedPayload（字段 snake_case）
// ============================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RenewalInfo {
    pub original_transaction_id: String,
    #[serde(default)]
    pub auto_renew_product_id: Option<String>,
    pub product_id: String,
    /// 1 = 自动续期开启, 0 = 用户已关闭
    pub auto_renew_status: i32,
    #[serde(default)]
    pub expiration_intent: Option<i32>,
    #[serde(default)]
    pub is_in_billing_retry_period: Option<bool>,
    /// 宽限期截止（DID_FAIL_TO_RENEW + GRACE_PERIOD 的核心字段）。
    #[serde(default)]
    pub grace_period_expires_date: Option<i64>,
    pub signed_date: i64,
    pub environment: String,
    #[serde(default)]
    pub price_increase_status: Option<i32>,
}

// ============================================================
// 验证器
// ============================================================

/// pinned Apple Root CA 验证器。`Clone` 廉价（DER 字节共享）。
#[derive(Clone)]
pub struct JwsVerifier {
    root_der: Vec<u8>,
    root_fp_hex: String,
}

impl JwsVerifier {
    /// 从 PEM 加载 pinned Apple Root CA（生产：`APPLE_ROOT_CA_PEM` 环境变量）。
    pub fn from_pem(pem: &str) -> Result<Self, JwsError> {
        let der = pem_to_der(pem)?;
        if der.is_empty() {
            return Err(JwsError::RootNotConfigured);
        }
        // 提前解析一次，坏证书当场暴露而不是等到第一笔 webhook。
        parse_cert(&der)?;
        Ok(Self {
            root_fp_hex: sha256_hex(&der),
            root_der: der,
        })
    }

    /// 从 `APPLE_ROOT_CA_PEM` 环境变量或 `certs/AppleRootCA-G3.crt` 文件加载。
    pub fn from_env() -> Result<Self, JwsError> {
        if let Ok(pem) = std::env::var("APPLE_ROOT_CA_PEM") {
            if !pem.trim().is_empty() {
                return Self::from_pem(&pem);
            }
        }
        match std::fs::read_to_string("certs/AppleRootCA-G3.crt") {
            Ok(pem) => Self::from_pem(&pem),
            Err(_) => Err(JwsError::RootNotConfigured),
        }
    }

    /// 验签 + 解码外层通知（webhook 入口的第一步）。
    pub fn decode_notification(&self, signed_payload: &str) -> Result<DecodedNotification, JwsError> {
        self.verify_and_decode(signed_payload)
    }

    /// 验签 + 解码内层 `signedTransactionInfo`。
    pub fn decode_transaction(&self, signed_info: &str) -> Result<TransactionInfo, JwsError> {
        self.verify_and_decode(signed_info)
    }

    /// 验签 + 解码内层 `signedRenewalInfo`。
    pub fn decode_renewal_info(&self, signed_info: &str) -> Result<RenewalInfo, JwsError> {
        self.verify_and_decode(signed_info)
    }

    // --------------------------------------------------------
    // 核心通用路径
    // --------------------------------------------------------

    fn verify_and_decode<T: DeserializeOwned>(&self, jws: &str) -> Result<T, JwsError> {
        // 1) 结构：恰好 3 段
        let mut parts = jws.split('.');
        let (h, p, s) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(h), Some(p), Some(s), None) => (h, p, s),
            _ => return Err(JwsError::Malformed),
        };

        // 2) header：alg 白名单
        let header: JwsHeader = serde_json::from_slice(&b64_url(h)?)
            .map_err(|e| JwsError::PayloadDecode(format!("header: {e}")))?;
        if header.alg != "ES256" {
            return Err(JwsError::UnsupportedAlg(header.alg));
        }

        // 3) 证书链：至少 leaf + intermediate
        if header.x5c.len() < 2 {
            return Err(JwsError::MissingChain);
        }
        let leaf_der = b64_std(&header.x5c[0])?;
        let intermediate_der = b64_std(&header.x5c[1])?;
        let leaf = parse_cert(&leaf_der)?;
        let intermediate = parse_cert(&intermediate_der)?;

        // 4) 若 x5c 自带 root（第 3 张），指纹必须与 pin 一致；
        //    无论是否自带，链的最后一级都由 pinned root 验签。
        let root = parse_cert(&self.root_der)?;
        if let Some(embedded) = header.x5c.get(2) {
            let embedded_der = b64_std(embedded)?;
            let embedded_fp = sha256_hex(&embedded_der);
            if embedded_fp != self.root_fp_hex {
                return Err(JwsError::RootFingerprintMismatch {
                    pinned: self.root_fp_hex.clone(),
                    embedded: embedded_fp,
                });
            }
        }
        // x509-parser 0.16 真实 API：verify_signature(签发者公钥) 验证本证书签名。
        // 语义即"leaf ← intermediate"与"intermediate ← pinned root"两级链验证。
        leaf.verify_signature(Some(intermediate.public_key()))
            .map_err(|e| JwsError::ChainInvalid(format!("leaf←intermediate: {e}")))?;
        intermediate
            .verify_signature(Some(root.public_key()))
            .map_err(|e| JwsError::ChainInvalid(format!("intermediate←root: {e}")))?;

        // 5) ES256 签名验证（leaf 公钥，签名输入 = "header.payload"）
        let key_bytes = extract_p256_pubkey(&leaf)?;
        // ring 0.17：UnparsedPublicKey（0.16 的 UnparsedKey 已改名）。
        let key = UnparsedPublicKey::new(&ES256_ALG, key_bytes);
        let signature = b64_url(s)?;
        key.verify(format!("{h}.{p}").as_bytes(), &signature)
            .map_err(|_| JwsError::BadSignature)?;

        // 6) payload 解码
        let payload = b64_url(p)?;
        serde_json::from_slice(&payload).map_err(|e| JwsError::PayloadDecode(e.to_string()))
    }
}

// ============================================================
// 内部工具
// ============================================================

fn b64_url(s: &str) -> Result<Vec<u8>, JwsError> {
    B64_URL.decode(s).map_err(|e| JwsError::Base64(e.to_string()))
}

fn b64_std(s: &str) -> Result<Vec<u8>, JwsError> {
    B64_STD.decode(s).map_err(|e| JwsError::Base64(e.to_string()))
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, JwsError> {
    let b64: String = pem
        .lines()
        .filter(|l| !l.trim_start().starts_with("-----"))
        .collect::<String>();
    if b64.trim().is_empty() {
        return Err(JwsError::RootNotConfigured);
    }
    B64_STD.decode(b64.trim()).map_err(|e| JwsError::Base64(e.to_string()))
}

fn parse_cert(der: &[u8]) -> Result<X509Certificate<'_>, JwsError> {
    let (_, cert) = X509Certificate::from_der(der).map_err(|e| JwsError::X509(e.to_string()))?;
    Ok(cert)
}

/// 提取 ECDSA P-256 公钥（65 字节未压缩点）。
/// ring 的 `UnparsedKey` 直接吃原始密钥字节。
fn extract_p256_pubkey<'a>(cert: &'a X509Certificate<'_>) -> Result<&'a [u8], JwsError> {
    let raw = cert.public_key().subject_public_key.data.as_ref();
    if raw.len() != 65 {
        // Apple 签名证书应为 EC P-256；长度不符即拒绝（防算法降级）。
        return Err(JwsError::NotP256);
    }
    Ok(raw)
}

fn sha256_hex(data: &[u8]) -> String {
    let fp = digest(&SHA256, data);
    let mut out = String::with_capacity(64);
    for b in fp.as_ref() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 前置校验（结构 / alg / 链长度）不依赖 pinned root，
    /// 用空 pin 即可覆盖到对应错误分支。
    fn bare_verifier() -> JwsVerifier {
        JwsVerifier {
            root_der: Vec::new(),
            root_fp_hex: String::new(),
        }
    }

    fn b64url(s: &[u8]) -> String {
        B64_URL.encode(s)
    }

    #[test]
    fn malformed_jws_rejected() {
        let v = bare_verifier();
        for bad in ["", "abc", "a.b", "a.b.c.d"] {
            let r: Result<serde_json::Value, _> = v.verify_and_decode(bad);
            assert!(matches!(r, Err(JwsError::Malformed)), "input: {bad:?}");
        }
    }

    #[test]
    fn non_es256_alg_rejected() {
        let v = bare_verifier();
        let header = br#"{"alg":"RS256","x5c":["x","y"]}"#;
        let jws = format!("{}.{}.{}", b64url(header), b64url(b"{}"), b64url(&[0u8; 64]));
        let r: Result<serde_json::Value, _> = v.verify_and_decode(&jws);
        assert!(matches!(r, Err(JwsError::UnsupportedAlg(a)) if a == "RS256"));
    }

    #[test]
    fn empty_x5c_rejected() {
        let v = bare_verifier();
        let header = br#"{"alg":"ES256","x5c":[]}"#;
        let jws = format!("{}.{}.{}", b64url(header), b64url(b"{}"), b64url(&[0u8; 64]));
        let r: Result<serde_json::Value, _> = v.verify_and_decode(&jws);
        assert!(matches!(r, Err(JwsError::MissingChain)));
    }

    #[test]
    fn garbage_x5c_rejected_as_x509_error() {
        let v = bare_verifier();
        let header = format!(
            r#"{{"alg":"ES256","x5c":["{}","{}"]}}"#,
            B64_STD.encode("not-a-cert-0"),
            B64_STD.encode("not-a-cert-1")
        );
        let jws = format!("{}.{}.{}", b64url(header.as_bytes()), b64url(b"{}"), b64url(&[0u8; 64]));
        let r: Result<serde_json::Value, _> = v.verify_and_decode(&jws);
        assert!(matches!(r, Err(JwsError::X509(_))), "got: {r:?}");
    }

    #[test]
    fn from_pem_rejects_empty() {
        assert!(matches!(JwsVerifier::from_pem(""), Err(JwsError::RootNotConfigured)));
        assert!(matches!(
            JwsVerifier::from_pem("-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----\n"),
            Err(JwsError::RootNotConfigured)
        ));
    }

    // --------------------------------------------------------
    // 真实样例占位（联调时启用）
    // --------------------------------------------------------
    // 步骤：App Store Connect → App → App Store Server Notifications
    //   → 配置 webhook URL（生产/沙盒）→ "Send Test Notification"，
    //   从 payment-service webhook 日志抓下 signedPayload 填到下面常量，
    //   并把 certs/AppleRootCA-G3.crt 放好，即可验证完整三层链路。
    //
    // #[test]
    // fn real_test_notification_full_chain() {
    //     let v = JwsVerifier::from_env().expect("pin root CA first");
    //     let n = v.decode_notification(REAL_TEST_SIGNED_PAYLOAD).unwrap();
    //     assert_eq!(n.notification_type, "TEST");
    //     let tx = v.decode_transaction(&n.data.signed_transaction_info).unwrap();
    //     assert!(!tx.original_transaction_id.is_empty());
    // }
    //
    // const REAL_TEST_SIGNED_PAYLOAD: &str = "";
}
