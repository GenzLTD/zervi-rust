-- =====================================================================
-- entitlement 仲裁层（PostgreSQL · 目标库 genzltd）
-- =====================================================================
-- 解决的问题
-- ----------------
-- 两条变现轨各自 `UPDATE users SET tier`，last-write-wins，没有仲裁：
--   * Stripe（Web/海外站）  settlement.py:84   db.update_user_tier(user_id, tier)
--   * Apple IAP（App 内）   routes.rs:326/420  set_user_tier(..., tier)
--   * 降级侧同样粗暴        routes.rs:483/516  set_user_tier(..., 'free')
--                           service.py        _handle_cancellation → 降级 free
--
-- 后果（真丢钱）：用户在 Web 买了 30 天 pro，同一账号 App 内 IAP 订阅一到期，
--                 tier 被打回 free，还有效着的 Stripe 订阅被连带抹掉。反向同理。
--
-- 正确语义（本文件实现）
-- ----------------
--   tier          = max( 两条轨上所有「未过期」订阅的 tier )
--   降级 free     只在「两条轨都没有有效订阅」时发生
--
-- 为什么用触发器而不是改调用点
-- ----------------
-- 两条轨分属 Python / Rust 两个服务、两种语言。把仲裁下沉到数据库触发器后，
-- **任何一条轨只要动了自己的订阅表，tier 就自动重算** —— 服务层不需要记得
-- 调用重算，也不会因为漏改某处而再次出现 last-write-wins。
--
-- ⚠️ 部署前提
-- ----------------
-- Stripe 轨目前写的是 **SQLite**（Vultr-Genzltd backend，DATABASE_PATH=/app/data/looma.db），
-- 本文件的函数/触发器只在 PG 生效。⇒ 海外站接入 PG（单源权威工程的一部分）后，
-- Stripe 轨才纳入仲裁。在此之前见文件末尾「降级保护」一节，先止血。
--
-- 幂等：全部 CREATE OR REPLACE / IF NOT EXISTS，可重复执行。
-- =====================================================================

BEGIN;

-- ---------------------------------------------------------------------
-- 1) tier 序（与 zervi-rust auth.rs 的层级一致）
--    guest(-1) < free(0) < supporter(1) < pro(2) < enterprise(3)
--    未知 tier 给 -99 ⇒ 永不参与取胜，避免脏字符串意外提权
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION tier_rank(p_tier text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
  SELECT CASE lower(coalesce(p_tier, 'free'))
           WHEN 'guest'      THEN -1
           WHEN 'free'       THEN  0
           WHEN 'supporter'  THEN  1
           WHEN 'pro'        THEN  2
           WHEN 'enterprise' THEN  3
           ELSE -99
         END;
$$;

COMMENT ON FUNCTION tier_rank(text) IS
  'tier 序数。未知值返回 -99（永不取胜），防止脏字符串意外提权。';


-- ---------------------------------------------------------------------
-- 2) 安全时间转换
--    subscriptions.expires_at 是 **TEXT**（存 ISO 串，来自 Python isoformat()），
--    而 iap_subscriptions.expires_at 是 timestamptz。两条轨类型不一致，
--    裸 cast 遇到脏数据会直接报错打断结算 ⇒ 转换失败一律当 NULL（= 不生效）。
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION safe_timestamptz(p text)
RETURNS timestamptz LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
  IF p IS NULL OR btrim(p) = '' THEN
    RETURN NULL;
  END IF;
  RETURN p::timestamptz;
EXCEPTION WHEN others THEN
  RETURN NULL;
END;
$$;

COMMENT ON FUNCTION safe_timestamptz(text) IS
  'TEXT→timestamptz 的安全转换；不可解析返回 NULL（视为无有效期限）。';


-- ---------------------------------------------------------------------
-- 3) 沙盒开关
--    原实现把「沙盒不升 tier」硬编码在 Rust 里（routes.rs:325/:419 的 is_prod
--    门禁）。这里把它提升为**可配置项**：默认关闭（保持现有防线），
--    联调时打开即可端到端验证 IAP 轨，无需改代码重新发版。
-- ---------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS entitlement_config (
  key         text PRIMARY KEY,
  value       text NOT NULL,
  description text DEFAULT '',
  updated_at  timestamptz NOT NULL DEFAULT now()
);

INSERT INTO entitlement_config (key, value, description)
VALUES ('include_iap_sandbox', 'false',
        'true = 沙盒 IAP 订阅也计入权益（仅联调用，生产必须 false）')
ON CONFLICT (key) DO NOTHING;

CREATE OR REPLACE FUNCTION entitlement_include_sandbox()
RETURNS boolean LANGUAGE sql STABLE AS $$
  SELECT EXISTS (
    SELECT 1 FROM entitlement_config
     WHERE key = 'include_iap_sandbox'
       AND lower(value) IN ('true','1','yes','on')
  );
$$;

COMMENT ON FUNCTION entitlement_include_sandbox() IS
  '是否把 Sandbox 环境的 IAP 订阅计入权益。默认 false（保持沙盒污染防线）。';


-- ---------------------------------------------------------------------
-- 4) 统一权益视图 —— 「一个账本」的入口
--    两轨并集，各自只贡献「当前有效」的那些行。
-- ---------------------------------------------------------------------
CREATE OR REPLACE VIEW v_active_entitlements AS
  -- Stripe 轨（Web / 海外站）：subscriptions，expires_at 为 TEXT
  SELECT s.user_id,
         s.tier,
         'stripe'::text AS rail,
         s.id::text     AS source_id,
         safe_timestamptz(s.expires_at) AS expires_at
    FROM subscriptions s
   WHERE lower(coalesce(s.status, 'active')) = 'active'
     AND safe_timestamptz(s.expires_at) > now()

  UNION ALL

  -- Apple IAP 轨（App 内）：iap_subscriptions，expires_at 为 timestamptz
  -- status ∈ {active, grace, expired, revoked}（state_machine.rs:58-61）
  -- grace = 宽限期，苹果语义下应保留权益 ⇒ 以 grace_period_expires_at 为准
  SELECT i.user_id,
         i.tier,
         'apple_iap'::text AS rail,
         i.original_transaction_id AS source_id,
         GREATEST(i.expires_at, i.grace_period_expires_at) AS expires_at
    FROM iap_subscriptions i
   WHERE lower(coalesce(i.status, '')) IN ('active', 'grace')
     AND i.revocation_date IS NULL
     AND (lower(i.environment) = 'production' OR entitlement_include_sandbox())
     AND COALESCE(GREATEST(i.expires_at, i.grace_period_expires_at), i.expires_at) > now();

COMMENT ON VIEW v_active_entitlements IS
  '两条变现轨的「未过期订阅」并集。tier 仲裁的唯一事实源。';


-- ---------------------------------------------------------------------
-- 5) 仲裁函数 —— 取代所有 `UPDATE users SET tier`
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION recompute_user_tier(p_user_id text)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
  v_tier text;
BEGIN
  SELECT e.tier
    INTO v_tier
    FROM v_active_entitlements e
   WHERE e.user_id = p_user_id
   ORDER BY tier_rank(e.tier) DESC, e.tier
   LIMIT 1;

  v_tier := COALESCE(v_tier, 'free');   -- 两轨皆无有效订阅 ⇒ 才降级

  UPDATE users
     SET tier = v_tier,
         updated_at = now()::text
   WHERE id = p_user_id;

  RETURN v_tier;
END;
$$;

COMMENT ON FUNCTION recompute_user_tier(text) IS
  'tier = max(两轨未过期订阅的 tier)；无有效订阅时降级 free。'
  '所有 rail 都应调用它，而不是各自 UPDATE users SET tier。';


-- ---------------------------------------------------------------------
-- 6) 诊断函数（联调用：看清 tier 是怎么算出来的）
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION explain_user_tier(p_user_id text)
RETURNS TABLE (rail text, source_id text, tier text, expires_at timestamptz)
LANGUAGE sql STABLE AS $$
  SELECT e.rail, e.source_id, e.tier, e.expires_at
    FROM v_active_entitlements e
   WHERE e.user_id = p_user_id
   ORDER BY tier_rank(e.tier) DESC;
$$;

COMMENT ON FUNCTION explain_user_tier(text) IS
  '列出某用户当前生效的所有权益来源，用于排查 tier 争议。';


-- ---------------------------------------------------------------------
-- 7) 触发器 —— 订阅表一有变化就自动重算
--    这是「服务层不需要记得调用」的关键：任何语言写的任何写入路径都覆盖到。
-- ---------------------------------------------------------------------
CREATE OR REPLACE FUNCTION trg_recompute_user_tier()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    PERFORM recompute_user_tier(OLD.user_id);
  ELSE
    PERFORM recompute_user_tier(NEW.user_id);
  END IF;
  RETURN NULL;          -- AFTER 触发器，返回值被忽略
END;
$$;

DROP TRIGGER IF EXISTS trg_subscriptions_recompute_tier ON subscriptions;
CREATE TRIGGER trg_subscriptions_recompute_tier
  AFTER INSERT OR UPDATE OR DELETE ON subscriptions
  FOR EACH ROW EXECUTE FUNCTION trg_recompute_user_tier();

DROP TRIGGER IF EXISTS trg_iap_subscriptions_recompute_tier ON iap_subscriptions;
CREATE TRIGGER trg_iap_subscriptions_recompute_tier
  AFTER INSERT OR UPDATE OR DELETE ON iap_subscriptions
  FOR EACH ROW EXECUTE FUNCTION trg_recompute_user_tier();

COMMIT;


-- =====================================================================
-- 【可选】补齐索引（订阅表变大后再考虑，小表不必）
-- =====================================================================
-- CREATE INDEX IF NOT EXISTS idx_iap_subs_user_status
--   ON iap_subscriptions (user_id, status);
-- CREATE INDEX IF NOT EXISTS idx_subs_user_status
--   ON subscriptions (user_id, status);


-- =====================================================================
-- 【止血】Stripe 轨尚在 SQLite，未纳入仲裁前先别再丢钱
-- =====================================================================
-- 海外站接入 PG 之前，Python 侧的 `db.update_user_tier()` 仍然直写 SQLite，
-- 触发器管不到它。最低限度先做一件事：**把「无条件降级 free」改成
-- 「降级前先确认本地确实没有有效订阅」**，避免继续抹掉已付费权益。
--
-- backend/src/services/membership/service.py → _handle_cancellation
-- backend/src/payment/settlement.py         → settle_payment
--
-- 见配套补丁说明。真正的收敛仍以本文件的 PG 仲裁层为准。
