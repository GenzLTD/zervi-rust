-- =====================================================================
-- entitlement 仲裁层 · 验收测试（全部在事务里，跑完 ROLLBACK，不留痕）
-- 用法：psql -f entitlement_arbitration_test.sql
-- =====================================================================
\set ON_ERROR_STOP on
BEGIN;

-- 建一个测试用户（不碰已灌的 347399@qq.com）
INSERT INTO users (id, email, tier, role) VALUES ('ent-test-001','ent-test@local.test','free','user');

CREATE OR REPLACE FUNCTION _chk(p_case text, p_expect text)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE v_actual text;
BEGIN
  SELECT tier INTO v_actual FROM users WHERE id = 'ent-test-001';
  IF v_actual IS DISTINCT FROM p_expect THEN
    RAISE EXCEPTION '❌ %  期望=%  实际=%', p_case, p_expect, v_actual;
  END IF;
  RAISE NOTICE '✅ %  → tier=%', p_case, v_actual;
  RETURN v_actual;
END $$;

-- ── 用例 1：两轨都无订阅 ⇒ free ───────────────────────────────────────
SELECT _chk('1. 空态', 'free');

-- ── 用例 2：只有 Stripe（pro，30 天）⇒ pro ────────────────────────────
INSERT INTO subscriptions (id, user_id, tier, plan_id, status, expires_at)
VALUES ('sub-stripe-1','ent-test-001','pro','pro_monthly','active',
        (now() + interval '30 days')::text);
SELECT _chk('2. 仅 Stripe pro', 'pro');

-- ── 用例 3：IAP 叠加 supporter（较低档）⇒ 仍 pro（取 max）─────────────
INSERT INTO iap_subscriptions (original_transaction_id, user_id, environment, product_id,
                               tier, status, expires_at, auto_renew_status, in_app_ownership_type)
VALUES ('1000000000001','ent-test-001','Production','supporter_monthly','supporter','active',
        now() + interval '30 days', true, 'PURCHASED');
SELECT _chk('3. Stripe pro + IAP supporter（应取 max=pro）', 'pro');

-- ── 用例 4【核心】IAP 到期/退款 ⇒ 不能抹掉还有效的 Stripe pro ─────────
UPDATE iap_subscriptions SET status='expired', expires_at = now() - interval '1 day'
 WHERE original_transaction_id='1000000000001';
SELECT _chk('4. IAP 到期后 Stripe 仍有效（旧实现会误降 free）', 'pro');

-- ── 用例 5：Stripe 也到期 ⇒ 这时才降级 free ───────────────────────────
UPDATE subscriptions SET status='expired', expires_at=(now() - interval '1 day')::text
 WHERE id='sub-stripe-1';
SELECT _chk('5. 两轨皆失效 ⇒ 才降 free', 'free');

-- ── 用例 6：反向 —— IAP pro + Stripe supporter，Stripe 先到期 ⇒ 仍 pro ─
INSERT INTO iap_subscriptions (original_transaction_id, user_id, environment, product_id,
                               tier, status, expires_at, auto_renew_status, in_app_ownership_type)
VALUES ('1000000000002','ent-test-001','Production','pro_monthly','pro','active',
        now() + interval '30 days', true, 'PURCHASED');
-- 注意：PG 里 subscriptions.user_id 有 UNIQUE 约束（一个用户一行 Stripe 订阅），
--       所以是 upsert 而不是新增第二行。
INSERT INTO subscriptions (id, user_id, tier, plan_id, status, expires_at)
VALUES ('sub-stripe-2','ent-test-001','supporter','sup_monthly','active',
        (now() + interval '30 days')::text)
ON CONFLICT (user_id) DO UPDATE SET status='active', tier='supporter',
  expires_at=(now() + interval '30 days')::text;
SELECT _chk('6. IAP pro + Stripe supporter', 'pro');
-- 按 user_id 更新（上面 upsert 命中的是 user_id 唯一键，行 id 仍是 sub-stripe-1）
UPDATE subscriptions SET status='cancelled' WHERE user_id='ent-test-001';
SELECT _chk('7. Stripe 取消后 IAP 仍有效 ⇒ 保持 pro', 'pro');

-- ── 用例 8：grace 宽限期内保留权益 ────────────────────────────────────
UPDATE iap_subscriptions
   SET status='grace', expires_at = now() - interval '1 day',
       grace_period_expires_at = now() + interval '10 days'
 WHERE original_transaction_id='1000000000002';
SELECT _chk('8. 宽限期（grace）内仍 pro', 'pro');

-- ── 用例 9：退款（revocation_date 非空）⇒ 立刻失效 ────────────────────
UPDATE iap_subscriptions SET status='revoked', revocation_date = now(),
       grace_period_expires_at = NULL
 WHERE original_transaction_id='1000000000002';
SELECT _chk('9. 退款撤销 ⇒ 降 free', 'free');

-- ── 用例 10：沙盒默认不生效（保持沙盒污染防线）────────────────────────
INSERT INTO iap_subscriptions (original_transaction_id, user_id, environment, product_id,
                               tier, status, expires_at, auto_renew_status, in_app_ownership_type)
VALUES ('1000000000003','ent-test-001','Sandbox','pro_monthly','pro','active',
        now() + interval '30 days', true, 'PURCHASED');
SELECT _chk('10. 沙盒订阅默认不计入（默认 false）', 'free');

-- ── 用例 11：打开开关后沙盒可测（联调 IAP 轨用）──────────────────────
UPDATE entitlement_config SET value='true' WHERE key='include_iap_sandbox';
-- 开关变更不触及订阅表 ⇒ 触发器不会自动跑，这里显式重算一次
SELECT recompute_user_tier('ent-test-001');
SELECT _chk('11. 打开 include_iap_sandbox 后沙盒生效', 'pro');
UPDATE entitlement_config SET value='false' WHERE key='include_iap_sandbox';

-- ── 用例 12：诊断函数 ─────────────────────────────────────────────────
SELECT '12. explain_user_tier' AS 用例;
SELECT * FROM explain_user_tier('ent-test-001');

-- ── 清理 ──────────────────────────────────────────────────────────────
DROP FUNCTION _chk(text, text);
ROLLBACK;

-- 确认回滚干净
SELECT count(*) AS 残留测试用户 FROM users WHERE id='ent-test-001';
SELECT count(*) AS 残留测试订阅 FROM iap_subscriptions WHERE user_id='ent-test-001';
