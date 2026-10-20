-- ============================================================
-- preflight_review_blockers.sql — 生产执行前的只读预检
-- ============================================================
-- 纯 SELECT，不建表、不改数据、不需要先装任何函数。
-- 用法： psql "$GENZLTD_DATABASE_URL" -f shared/preflight_review_blockers.sql
--
-- 期望值写在各段注释里。任何一项不符，先别执行 schema_v0.1.sql / account_delete。
-- ============================================================

\echo '===== 1. IAP 两表是否已存在（期望：0 行 → 需要执行 schema_v0.1.sql）====='
SELECT tablename AS existing_iap_table
FROM pg_tables
WHERE schemaname = 'public'
  AND tablename IN ('iap_subscriptions', 'apple_iap_transactions')
ORDER BY 1;

\echo '===== 2. users 被多少外键引用 / 其中裸 FK 多少（删号难度基线）====='
SELECT count(*) FILTER (WHERE c.confdeltype = 'a') AS bare_fk_no_action,
       count(*) FILTER (WHERE c.confdeltype = 'c') AS cascade,
       count(*) FILTER (WHERE c.confdeltype = 'n') AS set_null,
       count(*)                                   AS total
FROM pg_constraint c
WHERE c.contype = 'f'
  AND c.confrelid = 'public.users'::regclass;

\echo '===== 3. 有 user_id 列但没有外键的表（删号会静默残留，必须进 policy）====='
-- 期望：llm_usage / micro_feedback / product_events / query_logs /
--       iap_subscriptions（iap_* 为交易留痕，只脱敏不删）
SELECT table_name, column_name
FROM information_schema.columns
WHERE table_schema = 'public'
  AND column_name LIKE '%user_id%'
  AND NOT EXISTS (
      SELECT 1 FROM pg_constraint c
      JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
      WHERE c.contype = 'f'
        AND c.confrelid = 'public.users'::regclass
        AND c.conrelid::regclass::text = table_name
        AND a.attname = column_name
  )
ORDER BY 1, 2;

\echo '===== 4. 关键表行数（判断删号影响面）====='
SELECT 'users'            AS t, count(*) FROM public.users
UNION ALL SELECT 'applications',     count(*) FROM public.applications
UNION ALL SELECT 'candidates',       count(*) FROM public.candidates
UNION ALL SELECT 'poems',            count(*) FROM public.poems
UNION ALL SELECT 'orders',           count(*) FROM public.orders
UNION ALL SELECT 'subscriptions',    count(*) FROM public.subscriptions
UNION ALL SELECT 'product_events',   count(*) FROM public.product_events
UNION ALL SELECT 'query_logs',       count(*) FROM public.query_logs
ORDER BY 1;

\echo '===== 5. users.role 的取值分布（对照 Rust 侧 candidate/recruiter/admin）====='
SELECT role, count(*) AS n FROM public.users GROUP BY role ORDER BY 2 DESC;

\echo '===== 6. 当前 RLS 状态（期望：IAP 两表为 t，其余按设计）====='
SELECT relname, relrowsecurity AS rls_enabled, relforcerowsecurity AS forced
FROM pg_class
WHERE relname IN ('iap_subscriptions', 'apple_iap_transactions')
ORDER BY 1;

\echo '===== 7. 索引与约束总量（v0.3 重建会丢的规模）====='
SELECT (SELECT count(*) FROM pg_indexes WHERE schemaname = 'public')          AS indexes,
       (SELECT count(*) FROM pg_constraint WHERE contype IN ('f','c','u','p')) AS constraints;
