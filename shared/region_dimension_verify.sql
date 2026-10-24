-- =====================================================================
-- 区域维度 对拍脚本（只读） —— 给双端各自执行后逐行比对
-- =====================================================================
-- 用途
--   同一份 add_region_dimension.sql 在两地分别执行后，环境不同会造成
--   **静默漂移**。本脚本输出确定性结果，两边对齐 diffs 即可确认是否同构。
--
-- 为什么需要它（实测发现，2026-10-24）
--   迁移脚本对「类型不符」是 fail-closed 的（列类型冲突会报错回滚，✅ 安全）；
--   但对「同名对象已存在且定义不同」是 **静默跳过**：
--     模拟 Win 侧已有 users_region_check CHECK (region IN ('CN','US','EU','SG'))
--     再跑迁移脚本 ⇒ EXIT=0 且无任何提示，CHECK 仍多一个 'SG'
--   ⇒ 两边都以为装的是同一套区域维度，实际取值集合不同，
--     将来 FDW 联邦与按区路由会分歧，而且不报错。
--
-- 用法
--   psql "$CONN" -f region_dimension_verify.sql
--   把输出整段发给对端 diff 即可。只读，不写任何东西。
--
-- 判据
--   所有检查项 = PASS，且最后一行 DIGEST 两端完全一致 ⇒ 同构。
-- =====================================================================

\set ON_ERROR_STOP off

-- ---------------------------------------------------------------
-- 0. 环境指纹
-- ---------------------------------------------------------------
SELECT current_database()           AS database,
       current_user                 AS role,
       regexp_replace(version(), '^PostgreSQL ([0-9]+\\.[0-9]+).*$', '\\1') AS pg,
       (SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname IN ('public','vec') AND c.relkind='r')             AS total_tables;

-- ---------------------------------------------------------------
-- 1. 三列是否存在 + 类型 + 可空性 + 默认值
--    期望：region/text/NO/'CN'  home_instance/text/YES  tenant_id/text/YES
-- ---------------------------------------------------------------
SELECT e.col                                             AS col,
       coalesce(format_type(a.atttypid, a.atttypmod), '❌ 缺失') AS type,
       CASE WHEN a.attnum IS NULL THEN '❌ 缺失'
            WHEN a.attnotnull THEN 'NO' ELSE 'YES' END    AS nullable,
       coalesce(pg_get_expr(d.adbin, d.adrelid), '-')     AS default,
       CASE
         WHEN a.attnum IS NULL THEN 'FAIL'
         WHEN e.col='region' AND format_type(a.atttypid,a.atttypmod)='text'
              AND a.attnotnull AND pg_get_expr(d.adbin,d.adrelid)='''CN''::text' THEN 'PASS'
         WHEN e.col IN ('home_instance','tenant_id')
              AND format_type(a.atttypid,a.atttypmod)='text' AND NOT a.attnotnull
              AND d.adbin IS NULL THEN 'PASS'
         ELSE 'FAIL'
       END                                               AS verdict
FROM (VALUES ('region'),('home_instance'),('tenant_id')) AS e(col)
LEFT JOIN pg_attribute a
       ON a.attrelid='public.users'::regclass AND a.attname=e.col
      AND a.attnum > 0 AND NOT a.attisdropped
LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
ORDER BY e.col;

-- ---------------------------------------------------------------
-- 2. 🔴 关键项：CHECK 逐字符比对（此项最容易被静默漂移）
--    期望 definition =
--      CHECK ((region = ANY (ARRAY['CN'::text, 'US'::text, 'EU'::text])))
-- ---------------------------------------------------------------
SELECT 'users_region_check'                              AS obj,
       coalesce(pg_get_constraintdef(c.oid), '❌ 缺失')   AS definition,
       CASE WHEN pg_get_constraintdef(c.oid)
                 = 'CHECK ((region = ANY (ARRAY[''CN''::text, ''US''::text, ''EU''::text])))'
            THEN 'PASS' ELSE 'FAIL' END                   AS verdict
FROM (SELECT 1) z LEFT JOIN pg_constraint c
  ON c.conrelid='public.users'::regclass AND c.conname='users_region_check';

-- ---------------------------------------------------------------
-- 3. 外键：tenant_id → public.tenants(id)
-- ---------------------------------------------------------------
SELECT 'fk_users_tenant_id'                              AS obj,
       coalesce(pg_get_constraintdef(c.oid), '❌ 缺失')   AS definition,
       CASE WHEN pg_get_constraintdef(c.oid)
                 = 'FOREIGN KEY (tenant_id) REFERENCES tenants(id)'
            THEN 'PASS' ELSE 'FAIL' END                   AS verdict
FROM (SELECT 1) z LEFT JOIN pg_constraint c
  ON c.conrelid='public.users'::regclass AND c.conname='fk_users_tenant_id';

-- ---------------------------------------------------------------
-- 4. 列注释是否在位（迁移脚本带的说明；缺失说明可能只加列没跑注释）
-- ---------------------------------------------------------------
SELECT e.col AS col,
       CASE WHEN col_description('public.users'::regclass, a.attnum) IS NOT NULL
            THEN 'PASS' ELSE 'FAIL' END AS verdict
FROM (VALUES ('region'),('home_instance'),('tenant_id')) AS e(col)
LEFT JOIN pg_attribute a
       ON a.attrelid='public.users'::regclass AND a.attname=e.col
      AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY e.col;

-- ---------------------------------------------------------------
-- 5. 依赖前提：tenants 表与 icp_no（区域锚）
--    region 的语义锚点在 tenants.icp_no；若锚不在，region 沦为孤立枚举
-- ---------------------------------------------------------------
SELECT 'tenants 表' AS obj,
       CASE WHEN to_regclass('public.tenants') IS NOT NULL THEN 'PASS' ELSE 'FAIL' END AS verdict
UNION ALL
SELECT 'tenants.icp_no',
       CASE WHEN EXISTS (SELECT 1 FROM pg_attribute a
                         WHERE a.attrelid='public.tenants'::regclass
                           AND a.attname='icp_no' AND NOT a.attisdropped)
            THEN 'PASS' ELSE 'FAIL' END;

-- ---------------------------------------------------------------
-- 6. users 现有行数与 region 分布（两侧数据量不同没关系，
--    但 region 分布若一端全 NULL/一端有值，说明一边没回填）
-- ---------------------------------------------------------------
--    注意：用 to_jsonb(u)->>'region' 而非裸列名 —— 列不存在时裸列名会在
--    **计划阶段** 就报 "column region does not exist"，污染 diff 输出。
SELECT count(*) AS users_rows,
       count(*) FILTER (WHERE to_jsonb(u)->>'region' IS NULL) AS region_null,
       count(*) FILTER (WHERE to_jsonb(u)->>'region' = 'CN')  AS cn,
       count(*) FILTER (WHERE to_jsonb(u)->>'region' = 'US')  AS us,
       count(*) FILTER (WHERE to_jsonb(u)->>'region' = 'EU')  AS eu,
       count(*) FILTER (WHERE to_jsonb(u)->>'region' NOT IN ('CN','US','EU')
                          AND to_jsonb(u)->>'region' IS NOT NULL) AS out_of_domain
FROM public.users u;

-- ---------------------------------------------------------------
-- DIGEST：把第 1~4 项压成一行指纹，两端 diff 时只需比这一行
-- ---------------------------------------------------------------
SELECT md5(string_agg(v, ';' ORDER BY v)) AS digest FROM (
  SELECT e.col||'='||coalesce(format_type(a.atttypid,a.atttypmod),'MISSING')||'|'
         ||coalesce(a.attnotnull::text,'MISSING')||'|'
         ||coalesce(pg_get_expr(d.adbin,d.adrelid),'-') AS v
  FROM (VALUES ('region'),('home_instance'),('tenant_id')) AS e(col)
  LEFT JOIN pg_attribute a
         ON a.attrelid='public.users'::regclass AND a.attname=e.col
        AND a.attnum>0 AND NOT a.attisdropped
  LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
  UNION ALL
  SELECT 'users_region_check='||coalesce(pg_get_constraintdef(c.oid),'MISSING')
  FROM (SELECT 1) z LEFT JOIN pg_constraint c
    ON c.conrelid='public.users'::regclass AND c.conname='users_region_check'
  UNION ALL
  SELECT 'fk_users_tenant_id='||coalesce(pg_get_constraintdef(c.oid),'MISSING')
  FROM (SELECT 1) z LEFT JOIN pg_constraint c
    ON c.conrelid='public.users'::regclass AND c.conname='fk_users_tenant_id'
  UNION ALL
  SELECT 'comment_'||e.col||'='||
         CASE WHEN col_description('public.users'::regclass,a.attnum) IS NOT NULL
              THEN 'Y' ELSE 'N' END
  FROM (VALUES ('region'),('home_instance'),('tenant_id')) AS e(col)
  LEFT JOIN pg_attribute a
         ON a.attrelid='public.users'::regclass AND a.attname=e.col AND a.attnum>0 AND NOT a.attisdropped
) s;
