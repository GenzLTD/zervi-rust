//! payment-service — Apple IAP（自研，海外区，Capacitor 壳）
//!
//! 分层与推进顺序（见 docs 评审结论 2026-10-19）：
//!   1. `jws`  — Server Notification V2 / App Store Server API 的 JWS 验签解码（离线可测，最先落地）
//!   2. `jwt`  — App Store Server API 的 ES256 请求令牌签发（.p8 + kid + issuer_id，缓存 20 分钟）
//!   3. `client`  — App Store Server API 封装（subscriptions / transactions / history，Sandbox/Prod 双 host）
//!   4. `state_machine` — 15 种 notificationType → 8 种状态转移（DID_RENEW/EXPIRED/REFUND/REVOKE...）
//!   5. `routes` — /v1/payment/apple/{webhook,verify,restore}
//!
//! 关键设计约束：
//!   · tier 真源在 `users.tier`（PG），`resolve_principal` 每请求查库
//!     ⇒ REFUND/REVOKE 降级只需 UPDATE users，下一请求实时生效。
//!   · 幂等以 apple_iap_transactions 的唯一约束兜底，webhook 必须无条件 200。

pub mod jws;
pub mod jwt;
