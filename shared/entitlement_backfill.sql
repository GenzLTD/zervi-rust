-- =====================================================================
-- entitlement 仲裁层 · 存量回填（第 3 步，可选但强烈建议）
-- =====================================================================
-- 为什么需要
-- ----------------
-- 仲裁层的触发器挂在 subscriptions / iap_subscriptions 上，只在「订阅表发生变化」
-- 时才重算 tier。部署之前**已经错掉的** tier（典型：被 IAP 到期误降成 free，
-- 而 Stripe 订阅还有效）不会自动纠正 ⇒ 必须回填一次。
--
-- 本脚本先出「差异报告」（只读），再执行回填，两阶段都在同一个事务里。
-- 想只看看不落库 ⇒ 把最后的 COMMIT 改成 ROLLBACK。
--
-- 前置：已执行 entitlement_arbitration.sql
-- =====================================================================

\set ON_ERROR_STOP on
BEGIN;

-- ---------------------------------------------------------------------
-- 阶段一：差异报告（谁现在错着）
-- ---------------------------------------------------------------------
\echo '=============== 差异报告：当前 users.tier vs 仲裁结果 ==============='
WITH involved AS (
  SELECT user_id FROM subscriptions
  UNION
  SELECT user_id FROM iap_subscriptions
),
calc AS (
  SELECT i.user_id,
         coalesce((SELECT e.tier FROM v_active_entitlements e
                    WHERE e.user_id = i.user_id
                    ORDER BY tier_rank(e.tier) DESC, e.tier LIMIT 1), 'free') AS 仲裁tier
    FROM involved i
)
SELECT u.id,
       u.email,
       u.tier                      AS 现tier,
       c.仲裁tier                   AS 应tier,
       CASE WHEN u.tier IS DISTINCT FROM c.仲裁tier THEN '❗ 需纠正' ELSE 'OK' END AS 判定
  FROM calc c JOIN users u ON u.id = c.user_id
 ORDER BY 判定 DESC, u.email;

\echo ''
\echo '=============== 将被纠正的行数 ==============='
WITH involved AS (
  SELECT user_id FROM subscriptions
  UNION
  SELECT user_id FROM iap_subscriptions
)
SELECT count(*) FILTER (
         WHERE u.tier IS DISTINCT FROM
               coalesce((SELECT e.tier FROM v_active_entitlements e
                          WHERE e.user_id = i.user_id
                          ORDER BY tier_rank(e.tier) DESC, e.tier LIMIT 1), 'free')
       ) AS 需纠正,
       count(*) AS 涉及用户总数
  FROM involved i JOIN users u ON u.id = i.user_id;

-- ---------------------------------------------------------------------
-- 阶段二：执行回填
-- 只碰「有订阅记录」的用户；没有任何订阅记录的用户不在 involved 里，
-- 不会被误伤（他们的 tier 可能是手工配置的，不该被仲裁动）。
-- ---------------------------------------------------------------------
\echo ''
\echo '=============== 执行回填 ==============='
DO $$
DECLARE
  v_uid text;
  v_new text;
  v_n   int := 0;
BEGIN
  FOR v_uid IN
    SELECT user_id FROM subscriptions
    UNION
    SELECT user_id FROM iap_subscriptions
  LOOP
    SELECT recompute_user_tier(v_uid) INTO v_new;
    v_n := v_n + 1;
  END LOOP;
  RAISE NOTICE '回填完成：共 % 个用户重算 tier', v_n;
END $$;

-- ---------------------------------------------------------------------
-- 阶段三：回填后复查（应全部 OK）
-- ---------------------------------------------------------------------
\echo ''
\echo '=============== 回填后复查 ==============='
WITH involved AS (
  SELECT user_id FROM subscriptions
  UNION
  SELECT user_id FROM iap_subscriptions
)
SELECT u.email, u.tier AS 现tier,
       coalesce((SELECT e.tier FROM v_active_entitlements e
                  WHERE e.user_id = i.user_id
                  ORDER BY tier_rank(e.tier) DESC, e.tier LIMIT 1), 'free') AS 应tier,
       CASE WHEN u.tier IS DISTINCT FROM
                 coalesce((SELECT e.tier FROM v_active_entitlements e
                            WHERE e.user_id = i.user_id
                            ORDER BY tier_rank(e.tier) DESC, e.tier LIMIT 1), 'free')
            THEN '❌ 仍不一致' ELSE 'OK' END AS 判定
  FROM involved i JOIN users u ON u.id = i.user_id
 ORDER BY 判定 DESC, u.email;

COMMIT;
-- ↑ 只想预演不落库的话，把这行改成 ROLLBACK 再跑一遍
