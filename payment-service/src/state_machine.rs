//! Server Notification V2 状态机（v0.4 前置，纯函数层）。
//!
//! ## 15 种 notificationType 的处置口径
//!
//! **落状态机（8 种，产出 [`SubscriptionAction`]）：**
//! | 类型 | 动作 |
//! |---|---|
//! | `DID_RENEW` | Activate：延长 expires，刷新 tier |
//! | `DID_FAIL_TO_RENEW`(subtype `GRACE_PERIOD`) | EnterGrace：保持权益进宽限期 |
//! | `DID_FAIL_TO_RENEW`(无 subtype) | AckOnly：计费重试期，订阅仍有效，等后续事件 |
//! | `DID_CHANGE_RENEWAL_STATUS` | UpdateRenewal：仅更新 auto_renew |
//! | `DID_CHANGE_RENEWAL_PREF`(subtype `UPGRADE`) | Activate：升级立即生效（tx.product_id 已是新档） |
//! | `DID_CHANGE_RENEWAL_PREF`(其余) | AckOnly：降级下期生效，不动当前 |
//! | `GRACE_PERIOD_EXPIRED` | Expire：宽限期结束，降级 |
//! | `EXPIRED` | Expire：subtype（VOLUNTARY/BILLING_RETRY/PRICE_INCREASE）留痕 |
//! | `REFUND` / `REVOKE` | Revoke：立即撤权益（防薅关键路径） |
//!
//! **仅 ACK 200（7 种）：** `RENEWAL` / `CONSUMPTION_REQUEST` / `OFFER_REDEEMED` /
//! `PRICE_INCREASE` / `REFUND_DECLINED` / `REFUND_REVERSED` / `TEST`
//!
//! ## 设计原则
//!
//! - **fail-open for 未知类型**：Apple 未来会新增 notificationType，未知类型返回
//!   AckOnly + `unknown_type = true` 标志——webhook 必须 200，绝不能因新类型报错
//!   触发 Apple 重试风暴
//! - **fail-closed for 未知产品**：`tier_for_product` 映射缺失是配置错误，购买已
//!   发生但无法定档 ⇒ 返回 Err，由 webhook 层落 raw_payload 供人工回放
//! - 时间戳保持 ms（i64），时区/chrono 转换推迟到 v0.4 落库层
//! - `environment` 一路透传：Sandbox 通知绝不许碰生产权益（v0.4 落库层强制分流）

use crate::jws::{RenewalInfo, TransactionInfo};
use thiserror::Error;

/// 15 种 notificationType 全集（Apple 官方）。
pub mod notification_type {
    // —— 落状态机 ——
    pub const SUBSCRIBED: &str = "SUBSCRIBED";
    pub const DID_RENEW: &str = "DID_RENEW";
    pub const DID_FAIL_TO_RENEW: &str = "DID_FAIL_TO_RENEW";
    pub const DID_CHANGE_RENEWAL_STATUS: &str = "DID_CHANGE_RENEWAL_STATUS";
    pub const DID_CHANGE_RENEWAL_PREF: &str = "DID_CHANGE_RENEWAL_PREF";
    pub const GRACE_PERIOD_EXPIRED: &str = "GRACE_PERIOD_EXPIRED";
    pub const EXPIRED: &str = "EXPIRED";
    pub const REFUND: &str = "REFUND";
    pub const REVOKE: &str = "REVOKE";
    // —— 仅 ACK ——
    pub const RENEWAL: &str = "RENEWAL";
    pub const CONSUMPTION_REQUEST: &str = "CONSUMPTION_REQUEST";
    pub const OFFER_REDEEMED: &str = "OFFER_REDEEMED";
    pub const PRICE_INCREASE: &str = "PRICE_INCREASE";
    pub const REFUND_DECLINED: &str = "REFUND_DECLINED";
    pub const REFUND_REVERSED: &str = "REFUND_REVERSED";
    pub const TEST: &str = "TEST";
}

/// `iap_subscriptions.status` 值域（与 schema_v0.1.sql 的 CHECK 对齐）。
pub mod sub_status {
    pub const ACTIVE: &str = "active";
    pub const GRACE: &str = "grace";
    pub const EXPIRED: &str = "expired";
    pub const REVOKED: &str = "revoked";
}

/// 产品 → tier 映射（与 backend/contracts/payment.v1.json 的 plan 命名对齐）。
/// Apple 后台配置 IAP 商品时应使用同名 product_id。
pub fn tier_for_product(product_id: &str) -> Option<&'static str> {
    match product_id {
        "supporter_monthly" => Some("supporter"),
        "pro_monthly" => Some("pro"),
        _ => None,
    }
}

#[derive(Debug, Error)]
pub enum StateMachineError {
    #[error("transaction missing expires_date for activation")]
    MissingExpires,
    #[error("GRACE_PERIOD subtype but renewal info lacks grace_period_expires_date")]
    MissingGracePeriod,
    #[error("unknown product_id (tier mapping missing): {0}")]
    UnknownProduct(String),
}

/// 状态机产出的动作。v0.4 落库层据此生成 SQL（含 `UPDATE users SET tier`）。
#[derive(Debug, Clone, PartialEq)]
pub enum SubscriptionAction {
    /// 进入/保持 active。DID_RENEW、UPGRADE 走这条。
    Activate {
        tier: String,
        expires_at_ms: i64,
        auto_renew: bool,
        /// PURCHASED / FAMILY_SHARED（家庭共享下 user_id 多映射由落库层处理）
        in_app_ownership_type: Option<String>,
    },
    /// 进入宽限期，权益保持（Apple 仍在尝试扣款）。
    EnterGrace {
        tier: String,
        expires_at_ms: i64,
        grace_period_expires_at_ms: i64,
    },
    /// 过期降级。reason 为 Apple subtype 留痕。
    Expire { reason: Option<String> },
    /// 退款/撤销，立即降级（`UPDATE users SET tier='free'`）。
    Revoke {
        revocation_reason: Option<String>,
        revocation_date_ms: Option<i64>,
    },
    /// 仅元数据变更（auto_renew 开关），订阅状态与 tier 不动。
    UpdateRenewal { auto_renew: bool },
    /// 无需处理，回 200 即可。
    AckOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateTransition {
    pub notification_type: String,
    pub subtype: Option<String>,
    pub environment: String,
    pub action: SubscriptionAction,
    /// Apple 未来新增类型时为 true：仍 ACK，但日志需告警人工跟进。
    pub unknown_type: bool,
}

/// 状态转移入口（纯函数，无 IO）。
///
/// 参数：外层通知的 type/subtype + 验签后的内层交易/续期信息。
/// `renewal` 在 TEST 等通知下可能为 None。
pub fn transition(
    notification_type: &str,
    subtype: Option<&str>,
    tx: &TransactionInfo,
    renewal: Option<&RenewalInfo>,
) -> Result<StateTransition, StateMachineError> {
    let env = tx.environment.clone();
    let wrap = |action| StateTransition {
        notification_type: notification_type.to_string(),
        subtype: subtype.map(str::to_string),
        environment: env.clone(),
        action,
        unknown_type: false,
    };

    let action = match notification_type {
        // SUBSCRIBED（首次订阅/重新订阅）：signedTransactionInfo 已是完整购买。
        // routes 层注意：webhook 无 user_id 上下文，行不存在时只落 raw 留痕
        //（等 verify/restore 绑定），行存在则刷新 expires/tier —— apply 层处理。
        notification_type::SUBSCRIBED => activate(tx, renewal, true)?,

        t if t == notification_type::DID_RENEW => activate(tx, renewal, true)?,

        notification_type::DID_FAIL_TO_RENEW => match subtype {
            Some(s) if s == "GRACE_PERIOD" => {
                let grace = renewal
                    .and_then(|r| r.grace_period_expires_date)
                    .ok_or(StateMachineError::MissingGracePeriod)?;
                let tier = require_tier(tx)?;
                let expires = require_expires(tx)?;
                SubscriptionAction::EnterGrace {
                    tier: tier.to_string(),
                    expires_at_ms: expires,
                    grace_period_expires_at_ms: grace,
                }
            }
            // 无 subtype：计费重试期，订阅仍有效——不动状态，等 DID_RENEW 或 EXPIRED。
            _ => SubscriptionAction::AckOnly,
        },

        notification_type::DID_CHANGE_RENEWAL_STATUS => {
            let auto = renewal.map(|r| r.auto_renew_status == 1).unwrap_or(true);
            SubscriptionAction::UpdateRenewal { auto_renew: auto }
        }

        notification_type::DID_CHANGE_RENEWAL_PREF => match subtype {
            // 升级立即生效：signedTransactionInfo 已是新档产品。
            Some(s) if s == "UPGRADE" => activate(tx, renewal, true)?,
            // 降级下期生效；AUTO_RENEW_* 语义同 renewal status，但保险起见同步开关。
            _ => {
                let auto = renewal.map(|r| r.auto_renew_status == 1).unwrap_or(true);
                SubscriptionAction::UpdateRenewal { auto_renew: auto }
            }
        },

        notification_type::GRACE_PERIOD_EXPIRED => SubscriptionAction::Expire {
            reason: Some("GRACE_PERIOD_EXPIRED".to_string()),
        },

        notification_type::EXPIRED => SubscriptionAction::Expire {
            reason: subtype.map(str::to_string),
        },

        notification_type::REFUND | notification_type::REVOKE => SubscriptionAction::Revoke {
            revocation_reason: tx.revocation_reason.clone(),
            revocation_date_ms: tx.revocation_date,
        },

        // 7 种 ACK-only
        notification_type::RENEWAL
        | notification_type::CONSUMPTION_REQUEST
        | notification_type::OFFER_REDEEMED
        | notification_type::PRICE_INCREASE
        | notification_type::REFUND_DECLINED
        | notification_type::REFUND_REVERSED
        | notification_type::TEST => SubscriptionAction::AckOnly,

        // 未知类型：fail-open，仍 ACK。
        _ => {
            return Ok(StateTransition {
                notification_type: notification_type.to_string(),
                subtype: subtype.map(str::to_string),
                environment: env,
                action: SubscriptionAction::AckOnly,
                unknown_type: true,
            })
        }
    };

    Ok(wrap(action))
}

fn activate(
    tx: &TransactionInfo,
    renewal: Option<&RenewalInfo>,
    default_auto_renew: bool,
) -> Result<SubscriptionAction, StateMachineError> {
    Ok(SubscriptionAction::Activate {
        tier: require_tier(tx)?.to_string(),
        expires_at_ms: require_expires(tx)?,
        auto_renew: renewal
            .map(|r| r.auto_renew_status == 1)
            .unwrap_or(default_auto_renew),
        in_app_ownership_type: tx.in_app_ownership_type.clone(),
    })
}

fn require_tier(tx: &TransactionInfo) -> Result<&'static str, StateMachineError> {
    tier_for_product(&tx.product_id).ok_or_else(|| StateMachineError::UnknownProduct(tx.product_id.clone()))
}

fn require_expires(tx: &TransactionInfo) -> Result<i64, StateMachineError> {
    tx.expires_date.ok_or(StateMachineError::MissingExpires)
}

// ============================================================
// 测试（纯函数，16 项）
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(product: &str) -> TransactionInfo {
        TransactionInfo {
            transaction_id: "t-1".into(),
            original_transaction_id: "orig-1".into(),
            web_order_line_item_id: None,
            bundle_id: "com.yedall.planetx".into(),
            product_id: product.into(),
            subscription_group_identifier: Some("g1".into()),
            purchase_date: 1_000,
            original_purchase_date: 1_000,
            expires_date: Some(1_900_000_000_000),
            quantity: Some(1),
            kind: Some("Auto-Renewable Subscription".into()),
            app_account_token: None,
            in_app_ownership_type: Some("PURCHASED".into()),
            signed_date: 1_500,
            environment: "Sandbox".into(),
            revocation_date: None,
            revocation_reason: None,
            offer_type: None,
            offer_identifier: None,
            price: None,
            currency: None,
        }
    }

    fn renewal(auto_renew_status: i32, grace: Option<i64>) -> RenewalInfo {
        RenewalInfo {
            original_transaction_id: "orig-1".into(),
            auto_renew_product_id: None,
            product_id: "supporter_monthly".into(),
            auto_renew_status,
            expiration_intent: None,
            is_in_billing_retry_period: None,
            grace_period_expires_date: grace,
            signed_date: 1_500,
            environment: "Sandbox".into(),
            price_increase_status: None,
        }
    }

    fn act(t: &str, sub: Option<&str>, tx: &TransactionInfo, r: Option<&RenewalInfo>) -> StateTransition {
        transition(t, sub, tx, r).unwrap()
    }

    #[test]
    fn did_renew_activates() {
        let t = act(
            notification_type::DID_RENEW,
            Some("BILLING_RECOVERY"),
            &tx("pro_monthly"),
            Some(&renewal(1, None)),
        );
        assert_eq!(
            t.action,
            SubscriptionAction::Activate {
                tier: "pro".into(),
                expires_at_ms: 1_900_000_000_000,
                auto_renew: true,
                in_app_ownership_type: Some("PURCHASED".into()),
            }
        );
        assert_eq!(t.environment, "Sandbox");
        assert!(!t.unknown_type);
    }

    #[test]
    fn subscribed_activates() {
        // 首次订阅 / 重新订阅：与 DID_RENEW 同为 Activate 语义。
        for sub in [None, Some("INITIAL_BUY"), Some("RESUBSCRIBE")] {
            let t = act(notification_type::SUBSCRIBED, sub, &tx("supporter_monthly"), None);
            assert!(
                matches!(&t.action, SubscriptionAction::Activate { tier, .. } if tier == "supporter"),
                "SUBSCRIBED {sub:?} 应产出 Activate"
            );
            assert!(!t.unknown_type);
        }
    }

    #[test]
    fn did_renew_without_expires_errors() {
        let mut x = tx("pro_monthly");
        x.expires_date = None;
        assert!(matches!(
            transition(notification_type::DID_RENEW, None, &x, None),
            Err(StateMachineError::MissingExpires)
        ));
    }

    #[test]
    fn did_fail_to_renew_grace_enters_grace() {
        let t = act(
            notification_type::DID_FAIL_TO_RENEW,
            Some("GRACE_PERIOD"),
            &tx("supporter_monthly"),
            Some(&renewal(1, Some(1_950_000_000_000))),
        );
        assert_eq!(
            t.action,
            SubscriptionAction::EnterGrace {
                tier: "supporter".into(),
                expires_at_ms: 1_900_000_000_000,
                grace_period_expires_at_ms: 1_950_000_000_000,
            }
        );
    }

    #[test]
    fn did_fail_to_renew_grace_without_date_errors() {
        assert!(matches!(
            transition(
                notification_type::DID_FAIL_TO_RENEW,
                Some("GRACE_PERIOD"),
                &tx("supporter_monthly"),
                Some(&renewal(1, None)),
            ),
            Err(StateMachineError::MissingGracePeriod)
        ));
    }

    #[test]
    fn did_fail_to_renew_without_subtype_stays_active() {
        // 计费重试期：订阅仍有效，不动状态。
        let t = act(
            notification_type::DID_FAIL_TO_RENEW,
            None,
            &tx("supporter_monthly"),
            Some(&renewal(1, None)),
        );
        assert_eq!(t.action, SubscriptionAction::AckOnly);
    }

    #[test]
    fn renewal_status_toggles_auto_renew() {
        let on = act(
            notification_type::DID_CHANGE_RENEWAL_STATUS,
            Some("AUTO_RENEW_ENABLED"),
            &tx("supporter_monthly"),
            Some(&renewal(1, None)),
        );
        let off = act(
            notification_type::DID_CHANGE_RENEWAL_STATUS,
            Some("AUTO_RENEW_DISABLED"),
            &tx("supporter_monthly"),
            Some(&renewal(0, None)),
        );
        assert_eq!(on.action, SubscriptionAction::UpdateRenewal { auto_renew: true });
        assert_eq!(off.action, SubscriptionAction::UpdateRenewal { auto_renew: false });
    }

    #[test]
    fn upgrade_takes_effect_immediately() {
        // 升级时 signedTransactionInfo 的 product_id 已是新档。
        let t = act(
            notification_type::DID_CHANGE_RENEWAL_PREF,
            Some("UPGRADE"),
            &tx("pro_monthly"),
            Some(&renewal(1, None)),
        );
        assert!(matches!(t.action, SubscriptionAction::Activate { tier, .. } if tier == "pro"));
    }

    #[test]
    fn downgrade_deferred_to_next_period() {
        let t = act(
            notification_type::DID_CHANGE_RENEWAL_PREF,
            Some("DOWNGRADE"),
            &tx("supporter_monthly"),
            Some(&renewal(1, None)),
        );
        assert_eq!(t.action, SubscriptionAction::UpdateRenewal { auto_renew: true });
    }

    #[test]
    fn grace_period_expired_downgrades() {
        let t = act(
            notification_type::GRACE_PERIOD_EXPIRED,
            None,
            &tx("supporter_monthly"),
            None,
        );
        assert_eq!(
            t.action,
            SubscriptionAction::Expire { reason: Some("GRACE_PERIOD_EXPIRED".into()) }
        );
    }

    #[test]
    fn expired_carries_apple_subtype() {
        for sub in ["VOLUNTARY", "BILLING_RETRY", "PRICE_INCREASE"] {
            let t = act(notification_type::EXPIRED, Some(sub), &tx("pro_monthly"), None);
            assert_eq!(
                t.action,
                SubscriptionAction::Expire { reason: Some(sub.to_string()) }
            );
        }
    }

    #[test]
    fn refund_revokes_immediately() {
        let mut x = tx("pro_monthly");
        x.revocation_date = Some(1_800_000_000_000);
        x.revocation_reason = Some("REFUNDED_DUE_TO_ISSUE".into());
        let t = act(notification_type::REFUND, None, &x, None);
        assert_eq!(
            t.action,
            SubscriptionAction::Revoke {
                revocation_reason: Some("REFUNDED_DUE_TO_ISSUE".into()),
                revocation_date_ms: Some(1_800_000_000_000),
            }
        );
    }

    #[test]
    fn revoke_family_sharing() {
        let mut x = tx("pro_monthly");
        x.in_app_ownership_type = Some("FAMILY_SHARED".into());
        let t = act(notification_type::REVOKE, None, &x, None);
        assert!(matches!(t.action, SubscriptionAction::Revoke { .. }));
    }

    #[test]
    fn seven_types_are_ack_only() {
        let ack = [
            notification_type::RENEWAL,
            notification_type::CONSUMPTION_REQUEST,
            notification_type::OFFER_REDEEMED,
            notification_type::PRICE_INCREASE,
            notification_type::REFUND_DECLINED,
            notification_type::REFUND_REVERSED,
            notification_type::TEST,
        ];
        for t in ack {
            let r = act(t, None, &tx("supporter_monthly"), None);
            assert_eq!(r.action, SubscriptionAction::AckOnly, "{t}");
            assert!(!r.unknown_type);
        }
    }

    #[test]
    fn unknown_type_fails_open_but_flags() {
        let r = act("SOME_FUTURE_TYPE", None, &tx("supporter_monthly"), None);
        assert_eq!(r.action, SubscriptionAction::AckOnly);
        assert!(r.unknown_type, "未知类型必须打标志供告警");
    }

    #[test]
    fn tier_mapping_matches_contract() {
        assert_eq!(tier_for_product("supporter_monthly"), Some("supporter"));
        assert_eq!(tier_for_product("pro_monthly"), Some("pro"));
        assert_eq!(tier_for_product("random_thing"), None);
    }

    #[test]
    fn unknown_product_rejected_on_activation() {
        assert!(matches!(
            transition(notification_type::DID_RENEW, None, &tx("mystery_pack"), None),
            Err(StateMachineError::UnknownProduct(p)) if p == "mystery_pack"
        ));
    }
}
