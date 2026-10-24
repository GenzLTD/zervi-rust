-- =====================================================================
-- entitlement 仲裁层 · 部署前置检查（只读，不写任何东西）
-- 用途：在法兰克福预发机（45.76.84.90 · genzltd）部署之前先跑一遍，
--       确认本机结构与仲裁层的假设一致。任一项 FAIL 都要先解释清楚再部署。
-- 用法：psql "postgresql://...genzltd" -f entitlement_deploy_precheck.sql
-- =====================================================================
\set ON_ERROR_STOP on
\pset pager off

\echo '=============== 0. 环境与身份 ==============='
SELECT version() AS pg_version;
SELECT current_database() AS db, current_user AS usr, inet_server_addr() AS server;
-- ⚠️ 若 current_user 不是 users/subscriptions/iap_subscriptions 的 owner，
--    且这三张表开着 RLS，则仲裁函数里的 UPDATE 会被 RLS 拦成 0 行（静默失效）。
SELECT current_user,
       (SELECT rolsuper FROM pg_roles WHERE rolname = current_user) AS is_super,
       (SELECT rolbypassrls FROM pg_roles WHERE rolname = current_user) AS bypass_rls;

\echo '=============== 1. 三张表是否存在 ==============='
SELECT table_name,
       CASE WHEN table_name IS NULL THEN 'FAIL 缺表' ELSE 'OK' END AS 判定
  FROM (VALUES ('users'),('subscriptions'),('iap_subscriptions')) AS v(table_name)
 WHERE table_name NOT IN (
   SELECT table_name FROM information_schema.tables WHERE table_schema='public');

\echo '=============== 2. 仲裁层依赖的列（缺任何一列都会部署失败/语义错）==============='
WITH need(schema_t, tbl, col, typ) AS (
  VALUES ('users','users','id','text'),
         ('users','users','tier','text'),
         ('users','users','updated_at','text'),
         ('subs','subscriptions','user_id','text'),
         ('subs','subscriptions','tier','text'),
         ('subs','subscriptions','status','text'),
         ('subs','subscriptions','expires_at','text'),
         ('iap','iap_subscriptions','user_id','text'),
         ('iap','iap_subscriptions','tier','text'),
         ('iap','iap_subscriptions','status','text'),
         ('iap','iap_subscriptions','environment','text'),
         ('iap','iap_subscriptions','original_transaction_id','text'),
         ('iap','iap_subscriptions','expires_at','timestamp with time zone'),
         ('iap','iap_subscriptions','grace_period_expires_at','timestamp with time zone'),
         ('iap','iap_subscriptions','revocation_date','timestamp with time zone')
)
SELECT n.tbl, n.col, n.typ AS 期望类型,
       coalesce(format_type(a.atttypid, a.atttypmod), '❌ 列不存在') AS 实际类型,
       CASE WHEN format_type(a.atttypid,a.atttypmod) = n.typ THEN 'OK'
            ELSE '❌ FAIL' END AS 判定
  FROM need n
  LEFT JOIN pg_attribute a
    ON a.attrelid = ('public.'||n.tbl)::regclass
   AND a.attname = n.col AND a.attnum > 0 AND NOT a.attisdropped
 ORDER BY n.tbl, n.col;

\echo '=============== 3. subscriptions.user_id 是否唯一（影响测试用例 6 写法）==============='
SELECT indexname,
       CASE WHEN indexname IS NOT NULL THEN '唯一 ⇒ 测试按 user_id upsert（当前脚本已这么写）'
            ELSE '非唯一 ⇒ 用例 6 会插第二行，结论不变（max 仍是 pro）' END AS 说明
  FROM pg_indexes
 WHERE tablename='subscriptions' AND indexdef LIKE '%UNIQUE%(user_id)%';

\echo '=============== 4. RLS 状态（决定仲裁函数能否UPDATE成功）==============='
SELECT c.relname,
       c.relrowsecurity    AS rls开,
       c.relforcerowsecurity AS rls强制,
       pg_get_userbyid(c.relowner) AS owner,
       CASE WHEN pg_get_userbyid(c.relowner) = current_user AND NOT c.relforcerowsecurity
            THEN 'OK 当前用户=owner且未强制 ⇒ 不受 RLS 限'
            WHEN NOT c.relrowsecurity THEN 'OK 未开 RLS'
            ELSE '⚠️ 需确认：非 owner 或已强制 RLS ⇒ 仲裁 UPDATE 可能被拦成 0 行'
       END AS 判定
  FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='public' AND c.relname IN ('users','subscriptions','iap_subscriptions');

\echo '=============== 5. 是否已被别的迁移占用同名对象（重名会冲突）==============='
SELECT 'v_active_entitlements' AS 对象,
       CASE WHEN EXISTS (SELECT 1 FROM pg_views WHERE schemaname='public' AND viewname='v_active_entitlements')
            THEN '⚠️ 已存在（CREATE OR REPLACE 会覆盖，确认无其它依赖）' ELSE 'OK 不存在' END AS 判定;
SELECT proname AS 函数,
       CASE WHEN proname IS NOT NULL THEN '⚠️ 已存在（幂等覆盖）' ELSE 'OK' END AS 判定
  FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname='public'
   AND proname IN ('tier_rank','safe_timestamptz','entitlement_include_sandbox',
                   'recompute_user_tier','explain_user_tier','trg_recompute_user_tier');

\echo '=============== 6. 存量数据规模（决定要不要做一次回填）==============='
SELECT (SELECT count(*) FROM users)                       AS 用户数,
       (SELECT count(*) FROM subscriptions)                AS stripe订阅数,
       (SELECT count(*) FROM iap_subscriptions)            AS iap订阅数,
       (SELECT count(*) FROM subscriptions WHERE lower(coalesce(status,'active'))='active')    AS stripe_active,
       (SELECT count(*) FROM iap_subscriptions WHERE lower(coalesce(status,'')) IN ('active','grace')) AS iap_active;
-- ⚠️ 触发器只在「订阅表发生变化」时重算。存量用户若 tier 已经错（例如被 IAP
--    到期误降过 free），部署后不会自动纠正 ⇒ 必须跑一次回填（见部署脚本第 8 节）。

\echo '=============== 7. users.updated_at 的类型（仲裁函数按 text 写）==============='
SELECT attname, format_type(atttypid, atttypmod) AS 实际类型,
       CASE WHEN format_type(atttypid,atttypmod) = 'text'
            THEN 'OK 与仲裁函数 now()::text 一致'
            ELSE '❌ 仲裁函数里 now()::text 会写崩 ⇒ 需先改脚本' END AS 判定
  FROM pg_attribute
 WHERE attrelid='public.users'::regclass AND attname='updated_at';

\echo ''
\echo '=============== 结论 ==============='
\echo '全部 OK ⇒ 可部署：psql -v ON_ERROR_STOP=1 -f entitlement_arbitration.sql'
\echo '任一项 FAIL ⇒ 先停，不要部署。'
