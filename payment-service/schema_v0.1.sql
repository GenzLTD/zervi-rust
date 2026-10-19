-- ============================================================
-- payment-service schema v0.1 — Apple IAP (Server Notification V2)
-- ============================================================
-- 目标库:  genzltd (PostgreSQL 17 / Supabase)
-- 执行位置（两处）:
--   · 本地开发:  psql postgres://postgres:postgres@127.0.0.1:5433/genzltd -f schema_v0.1.sql
--   · 生产:      Supabase SQL Editor / supabase db push（与本地保持同版本）
-- 基准管理:
--   本文件为增量。按 Cargo.toml 注释口径（"以 genzltd_schema_v0.2.sql 为基准"），
--   合入主干时应升级基准至 v0.3 并把以下两张表并入，避免下次 sea-orm-codegen 漂移。
-- 安全红线:
--   两张表含交易 PII（raw_payload 有金额、转发邮箱、交易标识）。
--   ENABLE ROW LEVEL SECURITY 且【不加任何 policy】
--   = 对 anon / authenticated 全拒绝；zervi-rust 走 DATABASE_URL 直连
--   （postgres / service 角色 bypass RLS），读写不受影响。
--   严禁在 Supabase 后台为这两张表创建 SELECT policy。
-- ============================================================

-- ------------------------------------------------------------
-- iap_subscriptions — 订阅真源
--   锚点 = original_transaction_id（跨设备恢复 / 家庭共享的关联键）。
--   与旧 subscriptions 表的关系：并存，不迁移。旧表继续服务
--   Stripe / 微信 / 华为 IAP（:5200 Flask 侧）；Apple IAP 只写本表。
--   user_id 刻意【不设 UNIQUE】：家庭共享（in_app_ownership_type =
--   FAMILY_SHARED）下一个 original_transaction_id 允许多个 user_id 共享。
-- ------------------------------------------------------------
CREATE TABLE IF NOT EXISTS iap_subscriptions (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Apple 交易锚点（对 Google Play 即 order_id，对华为即 purchaseToken）
    original_transaction_id TEXT NOT NULL UNIQUE,
    user_id                 TEXT NOT NULL,
    -- 购买前由服务端生成并下发给客户端 StoreKit 的 appAccountToken，
    -- 用于"谁买的 = 谁享受"，防 A 账号购买 B 账号薅权益
    app_account_token       UUID,
    provider                TEXT NOT NULL DEFAULT 'apple_iap',   -- apple_iap | google_play | huawei_iap (预留)
    environment             TEXT NOT NULL CHECK (environment IN ('Sandbox', 'Production')),
    product_id              TEXT NOT NULL,          -- supporter_monthly | pro_monthly
    tier                    TEXT NOT NULL,          -- supporter | pro
    status                  TEXT NOT NULL CHECK (status IN ('active', 'grace', 'expired', 'revoked')),
    expires_at              TIMESTAMPTZ NOT NULL,
    grace_period_expires_at TIMESTAMPTZ,            -- 宽限期截止（DID_FAIL_TO_RENEW/GRACE_PERIOD 写入）
    auto_renew_status       BOOLEAN NOT NULL DEFAULT TRUE,
    in_app_ownership_type   TEXT NOT NULL DEFAULT 'PURCHASED'
                            CHECK (in_app_ownership_type IN ('PURCHASED', 'FAMILY_SHARED')),
    revocation_date         TIMESTAMPTZ,            -- REFUND / REVOKE 时间
    revocation_reason       TEXT,                   -- Apple revocationReason 原值留痕
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_iap_subs_user    ON iap_subscriptions(user_id);
CREATE INDEX IF NOT EXISTS idx_iap_subs_env     ON iap_subscriptions(environment, status);
CREATE INDEX IF NOT EXISTS idx_iap_subs_expires ON iap_subscriptions(expires_at);

ALTER TABLE iap_subscriptions ENABLE ROW LEVEL SECURITY;

-- ------------------------------------------------------------
-- apple_iap_transactions — 幂等核心（Apple 重试风暴的挡板）
--   Apple 对未收到 200 的通知会指数退避重试数天；同一 original_transaction_id
--   的续期也会产生多笔 transaction。写入一律 ON CONFLICT DO NOTHING，
--   以 transaction_id / notification_uuid 唯一约束做天然去重。
-- ------------------------------------------------------------
CREATE TABLE IF NOT EXISTS apple_iap_transactions (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    transaction_id          TEXT NOT NULL UNIQUE,   -- 每笔交易唯一（含每次续期）
    original_transaction_id TEXT NOT NULL,
    notification_uuid       TEXT UNIQUE,            -- 同一通知重试去重键（TEST 通知可为 NULL）
    notification_type       TEXT,                   -- DID_RENEW / REFUND / REVOKE / ...
    subtype                 TEXT,                   -- GRACE_PERIOD / AUTO_RENEW_DISABLED / ...
    environment             TEXT NOT NULL CHECK (environment IN ('Sandbox', 'Production')),
    raw_payload             JSONB NOT NULL,         -- 原始事件，出问题可回放重算
    processed_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_iap_tx_orig ON apple_iap_transactions(original_transaction_id);
CREATE INDEX IF NOT EXISTS idx_iap_tx_type ON apple_iap_transactions(notification_type, subtype);

ALTER TABLE apple_iap_transactions ENABLE ROW LEVEL SECURITY;

-- ============================================================
-- 回滚（开发期应急，生产禁用）
-- ============================================================
-- DROP TABLE IF EXISTS apple_iap_transactions;
-- DROP TABLE IF EXISTS iap_subscriptions;
