//! v0.4：三端点 + PG 落库（webhook / verify / restore）。
//!
//! ## 端点契约
//!
//! | 端点 | 鉴权 | 说明 |
//! |---|---|---|
//! | `POST /v1/payment/apple/webhook` | **无**（Apple 推送，JWS 验签即鉴权） | Server Notification V2 落状态机 |
//! | `POST /v1/payment/apple/verify`  | `AuthMode::Required` | 客户端购买后主动查证，绑定 user_id |
//! | `POST /v1/payment/apple/restore` | `AuthMode::Required` | 跨设备恢复：originalTransactionId 绑当前账号 |
//!
//! ## webhook 的三条铁律
//!
//! 1. **验签失败分两档**：`is_untrusted()`（伪签名/坏链）→ **401**，不落库，Apple
//!    视为永久失败不再重试；其余解码错误 → **200** + error 日志（Apple 重试
//!    也不会自愈，不能让它无限重试）。
//! 2. **幂等先行**：`notification_uuid` 已存在 → 直接 200（Apple 对未确认通知
//!    指数退避重试数天，重放必须无损）。
//! 3. **状态机错误也回 200**：UnknownProduct 等配置错误已随 raw_payload 落
//!    `apple_iap_transactions`，可人工回放；5xx 只会引发重试风暴。
//!
//! ## environment 分流（沙盒污染防线）
//!
//! `data.environment == "Sandbox"` 的通知：`iap_subscriptions` 照记
//! （environment 列隔离），但**绝不触碰 `users.tier`** —— 沙盒购买不得
//! 转正生产权益。Production 才走 `UPDATE users SET tier`（借
//! `resolve_principal` 每请求查库，下一请求实时生效）。
//!
//! ## SUBSCRIBED 先于 verify 到达的缺口
//!
//! webhook 无登录态，`iap_subscriptions` 行不存在时（SUBSCRIBED 抢跑）没有
//! user_id 可绑 —— 只落 raw 留痕，等 verify/restore 到达时建行绑定。已绑定的
//! original_transaction_id 后续通知（续期/退款）都能正确路由到 user。

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, FixedOffset, Utc};
use common::entities::{apple_iap_transactions, iap_subscriptions, users};
use common::looma::{fail, resolve_principal, AppState, AuthMode, Principal};
use sea_orm::entity::prelude::DateTimeWithTimeZone;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, sea_query::OnConflict,
    sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::client::{AppStoreClient, Environment};
use crate::jws::{JwsVerifier, TransactionInfo};
use crate::jwt::AppleJwtConfig;
use crate::state_machine::{self, SubscriptionAction, StateTransition, sub_status};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/payment/apple/webhook", post(webhook))
        .route("/v1/payment/apple/verify", post(verify))
        .route("/v1/payment/apple/restore", post(restore))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

// ============================================================
// 1) webhook — Apple Server Notification V2
// ============================================================

/// Apple 的 body 就是裸 JWS 字符串（无 JSON 包装）。
async fn webhook(State(state): State<AppState>, body: String) -> Response {
    let verifier = match JwsVerifier::from_env() {
        Ok(v) => v,
        Err(e) => {
            // pinned root 未配置是部署问题，500 让 Apple 稍后重试。
            tracing::error!("webhook: Apple Root CA 未配置: {e}");
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "root_ca_missing",
                "APPLE_ROOT_CA_PEM not configured",
            );
        }
    };

    let notification = match verifier.decode_notification(body.trim()) {
        Ok(n) => n,
        Err(e) if e.is_untrusted() => {
            // 伪签名 / 坏证书链：拒绝且不留痕（Apple 停止重试这类通知）。
            tracing::warn!("webhook: untrusted payload rejected: {e}");
            return fail(StatusCode::UNAUTHORIZED, "untrusted", "JWS verification failed");
        }
        Err(e) => {
            // 结构性坏数据：重试也不会自愈，ACK 掉并留日志。
            tracing::error!("webhook: malformed payload acked: {e}");
            return ok_received();
        }
    };

    // ---- 幂等：同 notification_uuid 的重试直接确认 ----
    let uuid = &notification.notification_uuid;
    let dup = apple_iap_transactions::Entity::find()
        .filter(apple_iap_transactions::Column::NotificationUuid.eq(uuid.as_str()))
        .one(&state.db)
        .await;
    match dup {
        Ok(Some(_)) => return ok_received(),
        Ok(None) => {}
        Err(e) => {
            // 幂等查询失败不能确定是否重放：宁可不处理，让 Apple 重试。
            tracing::error!("webhook: idempotency check failed: {e}");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "db_error", "idempotency check");
        }
    }

    // ---- 内层解码（TEST 等通知可能缺交易段）----
    let inner = decode_inner(&verifier, &notification.data);
    let (tx_opt, renewal_opt) = match &inner {
        Ok(pair) => (Some(pair.0.clone()), pair.1.clone()),
        Err(e) => {
            tracing::error!(
                "webhook: inner decode failed for {} ({}): {e}",
                notification.notification_type,
                uuid
            );
            (None, None)
        }
    };

    // ---- 落原始事件（transaction_id 缺失时用 notification_uuid 兜底占位）----
    let tx_key = tx_opt
        .as_ref()
        .map(|t| t.transaction_id.clone())
        .unwrap_or_else(|| format!("notification:{}", uuid));
    let raw = serde_json::to_value(&notification).unwrap_or(json!({}));
    let now = now_ts();
    let record = apple_iap_transactions::ActiveModel {
        id: Set(uuid::Uuid::new_v4()),
        transaction_id: Set(tx_key),
        original_transaction_id: Set(tx_opt
            .as_ref()
            .map(|t| t.original_transaction_id.clone())
            .unwrap_or_default()),
        notification_uuid: Set(Some(uuid.clone())),
        notification_type: Set(Some(notification.notification_type.clone())),
        subtype: Set(notification.subtype.clone()),
        environment: Set(notification.data.environment.clone()),
        raw_payload: Set(raw),
        processed_at: Set(now),
    };
    // transaction_id 撞车 = 该交易已被 verify/restore 落过 → do_nothing 放行。
    if let Err(e) = apple_iap_transactions::Entity::insert(record)
        .on_conflict(OnConflict::column(apple_iap_transactions::Column::TransactionId).do_nothing().to_owned())
        .exec(&state.db)
        .await
    {
        tracing::error!("webhook: raw event insert failed: {e}");
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "db_error", "raw insert");
    }

    let Some(tx) = tx_opt else {
        // 无交易段（TEST / 异常通知）：raw 已留痕，确认即可。
        return ok_received();
    };

    // ---- 状态机（纯函数）----
    let transition = state_machine::transition(
        &notification.notification_type,
        notification.subtype.as_deref(),
        &tx,
        renewal_opt.as_ref(),
    );
    let transition = match transition {
        Ok(t) => t,
        Err(e) => {
            // fail-closed 的配置错误（未知产品等）：raw 已落，回放可修复，不能 5xx。
            tracing::error!(
                "webhook: state machine rejected {} {} ({}): {e} — raw 已留痕待人工回放",
                notification.notification_type,
                notification.subtype.as_deref().unwrap_or("-"),
                uuid
            );
            return ok_received();
        }
    };

    // ---- 落库 ----
    if let Err(e) = apply_transition(&state.db, &transition, &tx).await {
        tracing::error!(
            "webhook: apply failed for {} ({}): {e} — raw 已留痕待人工回放",
            notification.notification_type,
            uuid
        );
    }
    ok_received()
}

/// 内层双段解码：`signedTransactionInfo`（必需）+ `signedRenewalInfo`（可缺）。
fn decode_inner(
    verifier: &JwsVerifier,
    data: &crate::jws::NotificationData,
) -> Result<(TransactionInfo, Option<crate::jws::RenewalInfo>), crate::jws::JwsError> {
    let tx = verifier.decode_transaction(&data.signed_transaction_info)?;
    let renewal = match &data.signed_renewal_info {
        Some(jws) if !jws.is_empty() => Some(verifier.decode_renewal_info(jws)?),
        _ => None,
    };
    Ok((tx, renewal))
}

// ============================================================
// 2) verify — 登录用户购买后查证（激活主路径）
// ============================================================

#[derive(Deserialize)]
struct VerifyBody {
    /// StoreKit 交易返回的 `transaction.id`（每次续期不同）。
    transaction_id: String,
}

async fn verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VerifyBody>,
) -> Response {
    let principal = match resolve_principal(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let (client, verifier) = match build_client() {
        Ok(v) => v,
        Err(e) => return fail(StatusCode::SERVICE_UNAVAILABLE, "apple_not_configured", e),
    };

    let tx = match client.transaction(&body.transaction_id, &verifier).await {
        Ok(t) => t,
        Err(crate::client::ClientError::Transient { status }) => {
            return fail(StatusCode::BAD_GATEWAY, "apple_upstream", format!("Apple {status}"))
        }
        Err(e) => {
            return fail(StatusCode::BAD_REQUEST, "apple_rejected", e.to_string());
        }
    };

    activate_for_user(&state, &principal, &tx).await
}

// ============================================================
// 3) restore — 跨设备恢复
// ============================================================

#[derive(Deserialize)]
struct RestoreBody {
    original_transaction_id: String,
}

async fn restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RestoreBody>,
) -> Response {
    let principal = match resolve_principal(&state, &headers, AuthMode::Required).await {
        Ok(p) => p,
        Err(r) => return r,
    };

    let (client, verifier) = match build_client() {
        Ok(v) => v,
        Err(e) => return fail(StatusCode::SERVICE_UNAVAILABLE, "apple_not_configured", e),
    };

    let status = match client.subscription_status(&body.original_transaction_id).await {
        Ok(s) => s,
        Err(crate::client::ClientError::Transient { status }) => {
            return fail(StatusCode::BAD_GATEWAY, "apple_upstream", format!("Apple {status}"))
        }
        Err(e) => return fail(StatusCode::BAD_REQUEST, "apple_rejected", e.to_string()),
    };

    // 取 status=1（active）的分组；全部过期则取最后一组（如实返回 expired）。
    let entry = status
        .last_transactions
        .iter()
        .find(|t| t.status == 1)
        .or_else(|| status.last_transactions.last());
    let Some(entry) = entry else {
        return fail(StatusCode::NOT_FOUND, "no_transactions", "no transactions for this id");
    };
    let tx = match verifier.decode_transaction(&entry.signed_transaction_info) {
        Ok(t) => t,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, "jws_decode", e.to_string()),
    };

    activate_for_user(&state, &principal, &tx).await
}

/// verify/restore 共用落点：以登录态 user_id 绑定订阅并激活。
async fn activate_for_user(state: &AppState, principal: &Principal, tx: &TransactionInfo) -> Response {
    let Some(tier) = state_machine::tier_for_product(&tx.product_id) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "unknown_product",
            format!("product {} not mapped to any tier", tx.product_id),
        );
    };

    let is_active = tx
        .expires_date
        .map(|ms| ms > Utc::now().timestamp_millis())
        .unwrap_or(true);

    // 绑定/刷新订阅行（upsert 以 original_transaction_id 为锚）。
    if let Err(e) = upsert_subscription(&state.db, tx, &principal.user_id, tier, is_active).await {
        tracing::error!("activate: upsert failed: {e}");
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "db_error", "upsert subscription");
    }

    // tier 只在 Production 生效（沙盒污染防线）且订阅当前有效。
    if is_active && tx.environment == "Production" {
        if let Err(e) = set_user_tier(&state.db, &principal.user_id, tier).await {
            tracing::error!("activate: users.tier update failed: {e}");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "db_error", "update tier");
        }
    }

    let expires_at = tx
        .expires_date
        .map(|ms| DateTime::from_timestamp_millis(ms).map(|d| d.to_rfc3339()))
        .flatten()
        .unwrap_or_default();
    Json(json!({
        "tier": tier,
        "active": is_active,
        "expiresAt": expires_at,
        "environment": tx.environment,
        "originalTransactionId": tx.original_transaction_id,
    }))
    .into_response()
}

// ============================================================
// 状态转移落库
// ============================================================

/// 把状态机决策写入 `iap_subscriptions`，Production 时联动 `users.tier`。
async fn apply_transition(
    db: &sea_orm::DatabaseConnection,
    t: &StateTransition,
    tx: &TransactionInfo,
) -> Result<(), sea_orm::DbErr> {
    let is_prod = t.environment == "Production";
    let otid = tx.original_transaction_id.as_str();

    let existing = iap_subscriptions::Entity::find()
        .filter(iap_subscriptions::Column::OriginalTransactionId.eq(otid))
        .one(db)
        .await?;

    match &t.action {
        SubscriptionAction::AckOnly => Ok(()),

        SubscriptionAction::UpdateRenewal { auto_renew } => {
            // 只更新已存在的行（行都没有时无需动作）。
            if let Some(_) = existing {
                iap_subscriptions::Entity::update_many()
                    .col_expr(
                        iap_subscriptions::Column::AutoRenewStatus,
                        Expr::value(*auto_renew),
                    )
                    .filter(iap_subscriptions::Column::OriginalTransactionId.eq(otid))
                    .exec(db)
                    .await?;
            }
            Ok(())
        }

        SubscriptionAction::Activate {
            tier,
            expires_at_ms,
            auto_renew,
            in_app_ownership_type,
        } => {
            match existing {
                Some(row) => {
                    iap_subscriptions::Entity::update_many()
                        .col_expr(iap_subscriptions::Column::Tier, Expr::value(tier.clone()))
                        .col_expr(
                            iap_subscriptions::Column::Status,
                            Expr::value(sub_status::ACTIVE),
                        )
                        .col_expr(
                            iap_subscriptions::Column::ExpiresAt,
                            Expr::value(ms_to_ts(*expires_at_ms)),
                        )
                        .col_expr(
                            iap_subscriptions::Column::AutoRenewStatus,
                            Expr::value(*auto_renew),
                        )
                        .col_expr(
                            iap_subscriptions::Column::InAppOwnershipType,
                            Expr::value(in_app_ownership_type
                                .clone()
                                .unwrap_or_else(|| "PURCHASED".into())),
                        )
                        .col_expr(
                            iap_subscriptions::Column::UpdatedAt,
                            Expr::value(now_ts()),
                        )
                        .filter(iap_subscriptions::Column::OriginalTransactionId.eq(otid))
                        .exec(db)
                        .await?;
                    // 实时升降级（借每请求查库的 resolve_principal 生效）。
                    if is_prod {
                        set_user_tier(db, &row.user_id, tier).await?;
                    }
                }
                None => {
                    // SUBSCRIBED/DID_RENEW 抢在 verify 之前到达：无 user_id 可绑，
                    // raw 已落 transactions 表，等 verify/restore 建行。
                    tracing::info!(
                        "webhook: {} arrived before verify (otid={otid}) — raw kept, no row yet",
                        t.notification_type
                    );
                }
            }
            Ok(())
        }

        SubscriptionAction::EnterGrace {
            tier,
            expires_at_ms,
            grace_period_expires_at_ms,
        } => {
            if let Some(row) = existing {
                iap_subscriptions::Entity::update_many()
                    .col_expr(iap_subscriptions::Column::Tier, Expr::value(tier.clone()))
                    .col_expr(
                        iap_subscriptions::Column::Status,
                        Expr::value(sub_status::GRACE),
                    )
                    .col_expr(
                        iap_subscriptions::Column::ExpiresAt,
                        Expr::value(ms_to_ts(*expires_at_ms)),
                    )
                    .col_expr(
                        iap_subscriptions::Column::GracePeriodExpiresAt,
                        Expr::value(ms_to_ts(*grace_period_expires_at_ms)),
                    )
                    .col_expr(
                        iap_subscriptions::Column::UpdatedAt,
                        Expr::value(now_ts()),
                    )
                    .filter(iap_subscriptions::Column::OriginalTransactionId.eq(otid))
                    .exec(db)
                    .await?;
                // 宽限期内权益保留：tier 不动。
                let _ = row;
            }
            Ok(())
        }

        SubscriptionAction::Expire { .. } => {
            if let Some(row) = existing {
                iap_subscriptions::Entity::update_many()
                    .col_expr(
                        iap_subscriptions::Column::Status,
                        Expr::value(sub_status::EXPIRED),
                    )
                    .col_expr(
                        iap_subscriptions::Column::UpdatedAt,
                        Expr::value(now_ts()),
                    )
                    .filter(iap_subscriptions::Column::OriginalTransactionId.eq(otid))
                    .exec(db)
                    .await?;
                if is_prod {
                    set_user_tier(db, &row.user_id, "free").await?;
                }
            }
            Ok(())
        }

        SubscriptionAction::Revoke {
            revocation_reason,
            revocation_date_ms,
        } => {
            if let Some(row) = existing {
                iap_subscriptions::Entity::update_many()
                    .col_expr(
                        iap_subscriptions::Column::Status,
                        Expr::value(sub_status::REVOKED),
                    )
                    .col_expr(
                        iap_subscriptions::Column::RevocationReason,
                        Expr::value(revocation_reason.clone()),
                    )
                    .col_expr(
                        iap_subscriptions::Column::RevocationDate,
                        Expr::value(revocation_date_ms.map(ms_to_ts)),
                    )
                    .col_expr(
                        iap_subscriptions::Column::UpdatedAt,
                        Expr::value(now_ts()),
                    )
                    .filter(iap_subscriptions::Column::OriginalTransactionId.eq(otid))
                    .exec(db)
                    .await?;
                if is_prod {
                    // 退款/家庭共享撤销：立即降级，防薅关键路径。
                    set_user_tier(db, &row.user_id, "free").await?;
                }
            }
            Ok(())
        }
    }
}

/// 以 original_transaction_id 为锚 upsert 订阅行（verify/restore 路径）。
async fn upsert_subscription(
    db: &sea_orm::DatabaseConnection,
    tx: &TransactionInfo,
    user_id: &str,
    tier: &str,
    is_active: bool,
) -> Result<(), sea_orm::DbErr> {
    let now = now_ts();
    let expires = ms_to_ts(tx.expires_date.unwrap_or(tx.signed_date));
    let model = iap_subscriptions::ActiveModel {
        id: Set(uuid::Uuid::new_v4()),
        original_transaction_id: Set(tx.original_transaction_id.clone()),
        user_id: Set(user_id.to_string()),
        app_account_token: Set(tx
            .app_account_token
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())),
        provider: Set("apple_iap".to_string()),
        environment: Set(tx.environment.clone()),
        product_id: Set(tx.product_id.clone()),
        tier: Set(tier.to_string()),
        status: Set(if is_active {
            sub_status::ACTIVE.to_string()
        } else {
            sub_status::EXPIRED.to_string()
        }),
        expires_at: Set(expires),
        grace_period_expires_at: Set(None),
        auto_renew_status: Set(true),
        in_app_ownership_type: Set(tx
            .in_app_ownership_type
            .clone()
            .unwrap_or_else(|| "PURCHASED".to_string())),
        revocation_date: Set(None),
        revocation_reason: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    iap_subscriptions::Entity::insert(model)
        .on_conflict(
            OnConflict::column(iap_subscriptions::Column::OriginalTransactionId)
                .update_columns([
                    iap_subscriptions::Column::UserId, // 跨设备恢复 = 重新绑定到当前账号
                    iap_subscriptions::Column::ProductId,
                    iap_subscriptions::Column::Tier,
                    iap_subscriptions::Column::Status,
                    iap_subscriptions::Column::ExpiresAt,
                    iap_subscriptions::Column::InAppOwnershipType,
                    iap_subscriptions::Column::AppAccountToken,
                    iap_subscriptions::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// `UPDATE users SET tier = ? WHERE id = ?`（每请求查库 ⇒ 下一请求即生效）。
///
/// P1-1：必须校验 `rows_affected == 1`。0 行意味着 `user_id` 在 `users` 里不存在
/// （例如误传了 `sha256(sub)` 而不是 `users.id`），旧代码照样回 200 + `"tier":"pro"`，
/// 于是"买完没生效、日志还干净"。现在 0 行 ⇒ 记 error 并按 DB 错误上抛（→ 500）。
async fn set_user_tier(
    db: &sea_orm::DatabaseConnection,
    user_id: &str,
    tier: &str,
) -> Result<(), sea_orm::DbErr> {
    let outcome = users::Entity::update_many()
        .col_expr(users::Column::Tier, Expr::value(tier))
        .filter(users::Column::Id.eq(user_id))
        .exec(db)
        .await?;

    if outcome.rows_affected != 1 {
        tracing::error!(
            user_id = %user_id,
            tier = %tier,
            rows_affected = outcome.rows_affected,
            "set_user_tier: 期望更新 1 行，实际 {} 行 —— user_id 与 users.id 不匹配（孤儿 user_id 或已删号），拒绝按成功返回",
            outcome.rows_affected
        );
        return Err(sea_orm::DbErr::RecordNotUpdated);
    }
    Ok(())
}

// ============================================================
// 工具
// ============================================================

/// Apple 配置齐全时构造客户端；`APPLE_ENV` 控制 Sandbox（默认）/ Production。
fn build_client() -> Result<(AppStoreClient, JwsVerifier), String> {
    let config = AppleJwtConfig::from_env().map_err(|e| e.to_string())?;
    let verifier = JwsVerifier::from_env().map_err(|e| e.to_string())?;
    let env = std::env::var("APPLE_ENV")
        .ok()
        .and_then(|s| Environment::parse(&s))
        .unwrap_or(Environment::Sandbox);
    let client = AppStoreClient::new(env, config).map_err(|e| e.to_string())?;
    Ok((client, verifier))
}

fn ok_received() -> Response {
    Json(json!({ "received": true })).into_response()
}

fn now_ts() -> DateTimeWithTimeZone {
    Utc::now().fixed_offset()
}

fn ms_to_ts(ms: i64) -> DateTimeWithTimeZone {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .with_timezone(&FixedOffset::east_opt(0).expect("valid offset"))
}

// ============================================================
// 测试（纯函数层；DB 链路依赖本地 genzltd，联调阶段手工验证）
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_to_ts_epoch_and_now() {
        assert_eq!(ms_to_ts(0).to_rfc3339(), "1970-01-01T00:00:00+00:00");
        assert_eq!(
            ms_to_ts(1_900_000_000_000).to_rfc3339(),
            "2030-03-17T17:46:40+00:00"
        );
    }

    #[test]
    fn verify_body_shape() {
        let b: VerifyBody =
            serde_json::from_str(r#"{"transaction_id":"2000000123456789"}"#).unwrap();
        assert_eq!(b.transaction_id, "2000000123456789");
    }

    #[test]
    fn restore_body_shape() {
        let b: RestoreBody = serde_json::from_str(
            r#"{"original_transaction_id":"2000000123456789"}"#,
        )
        .unwrap();
        assert_eq!(b.original_transaction_id, "2000000123456789");
    }

    #[test]
    fn sandbox_is_default_env() {
        // 无 APPLE_ENV 时 build_client 用 Sandbox —— 但配置缺失时先报错，
        // 所以这里只验证 Environment 解析语义（Sandbox 大小写敏感）。
        assert_eq!(Environment::parse("Sandbox"), Some(Environment::Sandbox));
        assert_eq!(Environment::parse("Production"), Some(Environment::Production));
        assert_eq!(Environment::parse("sandbox"), None);
    }
}
