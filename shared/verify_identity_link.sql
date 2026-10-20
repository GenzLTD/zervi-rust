-- ============================================================================
-- supabase_uid 回填 · 自包含验证（v0.1）
--
-- 在**一次性空库**里把 backfill 的全部分支跑一遍，输出 PASS/FAIL。
-- 不需要生产数据、不需要 Supabase，纯本地可复现。
--
-- 用法：
--   createdb zervi_verify_identity
--   psql -d zervi_verify_identity -v ON_ERROR_STOP=1 -f shared/verify_identity_link.sql
--
-- 覆盖的判定分支（对应 identity_backfill_rows 的 status）：
--   ready / linked              正常绑定
--   already_bound               已绑同一 uid → 不动
--   already_bound_other         已绑不同 uid → 不覆盖
--   ambiguous_email_auth        auth.users 同 email 多行 → 不猜
--   ambiguous_email_local       本地 users 同 email 多行 → 不猜
--   unmatched                   无候选 → 不猜
--   conflict_uid_taken          候选 uid 被别的行占 → 不抢
--   uid_not_in_auth             手工 uid 在 auth.users 查不到 → 不写
--   no_email                    无 email → 不写
--   重入：第二次 apply 必须 0 写入、日志不翻倍
-- ============================================================================

\set ON_ERROR_STOP on

-- ----------------------------------------------------------------------------
-- PART 0 · 空库守卫：绝不在有数据的库上跑
-- ----------------------------------------------------------------------------
DO $$
BEGIN
    IF to_regclass('public.users') IS NOT NULL THEN
        RAISE EXCEPTION '本脚本要建表造数，只允许在空库运行（public.users 已存在）';
    END IF;
END $$;

-- ----------------------------------------------------------------------------
-- PART 1 · 造"生产形状"的 users（时间列是 text、email/supabase_uid 都没 UNIQUE）
-- ----------------------------------------------------------------------------
CREATE TABLE public.users (
    id          text NOT NULL PRIMARY KEY,
    email       text,
    name        text DEFAULT ''::text,
    tier        text DEFAULT 'free'::text,
    role        text DEFAULT 'user'::text,
    created_at  text DEFAULT now(),
    updated_at  text DEFAULT now(),
    email_hash  text,
    supabase_uid text
);

-- 造 auth.users（模拟 Supabase 可见的情况）
CREATE SCHEMA auth;
CREATE TABLE auth.users (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email      text,
    created_at timestamptz DEFAULT now()
);

-- ----------------------------------------------------------------------------
-- PART 2 · 装载被测脚本（\ir = 相对本文件）
-- ----------------------------------------------------------------------------
\ir backfill_supabase_uid_v0.1.sql

-- ----------------------------------------------------------------------------
-- PART 3 · 造数据
-- ----------------------------------------------------------------------------
-- uid 常量：
--   U_READY=$a  U_DUP_AUTH=$b1/$b2  U_MANUAL=$c  U_TAKEN=$d  U_SAME=$e
--   U_OTHER=$f  U_NOT_IN_AUTH=$9
INSERT INTO auth.users (id, email) VALUES
    ('00000000-0000-0000-0000-0000000000a1', 'ready@example.com'),
    ('00000000-0000-0000-0000-0000000000b1', 'dup@example.com'),
    ('00000000-0000-0000-0000-0000000000b2', 'DUP@example.com'),   -- 大小写不同、同 email
    ('00000000-0000-0000-0000-0000000000d1', 'taken@example.com'),
    ('00000000-0000-0000-0000-0000000000e1', 'same@example.com'),
    ('00000000-0000-0000-0000-0000000000f1', 'other@example.com'),
    ('00000000-0000-0000-0000-0000000000c1', 'manual@example.com'),   -- 手工映射的 uid 真实存在
    ('00000000-0000-0000-0000-00000000dead', 'legacy@example.com');   -- u-other 已绑的旧 uid 也真实存在

INSERT INTO public.users (id, email, tier, supabase_uid) VALUES
    ('u-ready',    'ready@example.com',  'free', NULL),                 -- ready
    ('u-dup',      'dup@example.com',    'free', NULL),                 -- ambiguous_email_auth
    ('u-manual',   'manual@example.com', 'pro',  NULL),                 -- ready（手工映射）
    ('u-holder',   'holder@example.com', 'free', '00000000-0000-0000-0000-0000000000d1'), -- 占住 U_TAKEN
    ('u-taken',    'taken@example.com',  'free', NULL),                 -- conflict_uid_taken
    ('u-same',     'same@example.com',   'pro',  '00000000-0000-0000-0000-0000000000e1'), -- already_bound
    ('u-other',    'other@example.com',  'pro',  '00000000-0000-0000-0000-00000000dead'), -- already_bound_other
    ('u-nomail',   NULL,                 'free', NULL),                 -- no_email
    ('u-localdup1','localdup@example.com','free', NULL),                -- ambiguous_email_local
    ('u-localdup2','LOCALdup@example.com','free', NULL),                --  （同上，大小写不同）
    ('u-unmatched','nobody@example.com', 'free', NULL),                 -- unmatched
    ('u-notauth',  'notauth@example.com','free', NULL);                 -- uid_not_in_auth（手工给个假 uid）

-- 手工映射：u-manual 正常；u-notauth 的 uid 在 auth.users 里不存在
INSERT INTO public.supabase_uid_backfill_map (email, supabase_uid, note) VALUES
    ('manual@example.com',  '00000000-0000-0000-0000-0000000000c1', '手工映射用例'),
    ('notauth@example.com', '00000000-0000-0000-0000-000000000999', '故意给一个不存在的 uid');

-- ----------------------------------------------------------------------------
-- PART 4 · 断言工具
-- ----------------------------------------------------------------------------
CREATE TABLE verify_results (seq serial, check_name text, expected text, actual text, ok boolean);

CREATE OR REPLACE FUNCTION pg_temp.expect_status(p_email text, p_expected text)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_actual text;
BEGIN
    SELECT status INTO v_actual
      FROM public.identity_backfill_rows()
     WHERE email = lower(p_email);
    INSERT INTO verify_results (check_name, expected, actual, ok)
    VALUES (format('status(%s)', p_email), p_expected, coalesce(v_actual, '(缺行)'),
            coalesce(v_actual, '') = p_expected);
END $$;

-- ----------------------------------------------------------------------------
-- PART 5 · 决策表断言（不写库）
-- ----------------------------------------------------------------------------
SELECT pg_temp.expect_status('ready@example.com',    'ready');
SELECT pg_temp.expect_status('dup@example.com',      'ambiguous_email_auth');
SELECT pg_temp.expect_status('manual@example.com',   'ready');
SELECT pg_temp.expect_status('taken@example.com',    'conflict_uid_taken');
SELECT pg_temp.expect_status('same@example.com',     'already_bound');
SELECT pg_temp.expect_status('other@example.com',    'already_bound_other');
SELECT pg_temp.expect_status('localdup@example.com', 'ambiguous_email_local');
SELECT pg_temp.expect_status('nobody@example.com',   'unmatched');
SELECT pg_temp.expect_status('notauth@example.com',  'uid_not_in_auth');

-- no_email：email 为 NULL 时 identity_backfill_rows 返回的是空串
DO $$
DECLARE v text;
BEGIN
    SELECT status INTO v FROM public.identity_backfill_rows() WHERE user_id = 'u-nomail';
    INSERT INTO verify_results (check_name, expected, actual, ok)
    VALUES ('status(no_email)', 'no_email', coalesce(v, '(缺行)'), coalesce(v, '') = 'no_email');
END $$;

-- ----------------------------------------------------------------------------
-- PART 6 · dry-run 不写库
-- ----------------------------------------------------------------------------
DO $$
DECLARE v_changed int;
BEGIN
    PERFORM public.identity_backfill(false);
    SELECT count(*) INTO v_changed FROM public.users
     WHERE supabase_uid IS NOT NULL AND id IN ('u-ready', 'u-manual');
    INSERT INTO verify_results (check_name, expected, actual, ok)
    VALUES ('dry_run_writes_nothing', '0', v_changed::text, v_changed = 0);
END $$;

-- ----------------------------------------------------------------------------
-- PART 7 · 真正写入
-- ----------------------------------------------------------------------------
CREATE TEMP TABLE applied AS SELECT * FROM public.identity_backfill(true);

DO $$
DECLARE
    v_ready   text;
    v_manual  text;
    v_log     int;
    v_ts_null int;
BEGIN
    SELECT supabase_uid INTO v_ready FROM public.users WHERE id = 'u-ready';
    SELECT supabase_uid INTO v_manual FROM public.users WHERE id = 'u-manual';
    SELECT count(*) INTO v_log FROM public.identity_link_log WHERE action = 'link';
    SELECT count(*) INTO v_ts_null FROM public.users
     WHERE id IN ('u-ready', 'u-manual') AND updated_at IS NULL;

    INSERT INTO verify_results (check_name, expected, actual, ok) VALUES
      ('linked(auth.users 自动匹配)', '00000000-0000-0000-0000-0000000000a1',
       coalesce(v_ready, '(NULL)'), v_ready = '00000000-0000-0000-0000-0000000000a1'),
      ('linked(手工映射)', '00000000-0000-0000-0000-0000000000c1',
       coalesce(v_manual, '(NULL)'), v_manual = '00000000-0000-0000-0000-0000000000c1'),
      ('identity_link_log 行数', '2', v_log::text, v_log = 2),
      ('updated_at(text 列) 未被写坏', '0', v_ts_null::text, v_ts_null = 0);
END $$;

-- 被拦下的行必须仍是 NULL（不许半途生效）
DO $$
DECLARE v_bad int;
BEGIN
    SELECT count(*) INTO v_bad FROM public.users
     WHERE id IN ('u-dup', 'u-taken', 'u-localdup1', 'u-localdup2', 'u-unmatched', 'u-notauth', 'u-nomail')
       AND supabase_uid IS NOT NULL;
    INSERT INTO verify_results (check_name, expected, actual, ok)
    VALUES ('blocked_rows_untouched', '0', v_bad::text, v_bad = 0);
END $$;

-- 已绑不同 uid 的行决不能被覆盖
DO $$
DECLARE v text;
BEGIN
    SELECT supabase_uid INTO v FROM public.users WHERE id = 'u-other';
    INSERT INTO verify_results (check_name, expected, actual, ok)
    VALUES ('already_bound_other_kept', '00000000-0000-0000-0000-00000000dead',
            coalesce(v, '(NULL)'), v = '00000000-0000-0000-0000-00000000dead');
END $$;

-- ----------------------------------------------------------------------------
-- PART 8 · 重入：再跑一次必须 0 写入、日志不翻倍
-- ----------------------------------------------------------------------------
DO $$
DECLARE
    v_applied int;
    v_log     int;
BEGIN
    SELECT count(*) INTO v_applied FROM public.identity_backfill(true) WHERE applied;
    SELECT count(*) INTO v_log FROM public.identity_link_log WHERE action = 'link';
    INSERT INTO verify_results (check_name, expected, actual, ok) VALUES
      ('rerun_applies_nothing', '0', v_applied::text, v_applied = 0),
      ('rerun_log_not_duplicated', '2', v_log::text, v_log = 2);
END $$;

-- ----------------------------------------------------------------------------
-- PART 9 · 复核函数
-- ----------------------------------------------------------------------------
DO $$
DECLARE r record; v_dup text; v_orphan text;
BEGIN
    SELECT value::text INTO v_dup    FROM public.identity_backfill_verify() WHERE check_name = 'duplicate_supabase_uid';
    SELECT value::text INTO v_orphan FROM public.identity_backfill_verify() WHERE check_name = 'bound_uid_not_in_auth';
    INSERT INTO verify_results (check_name, expected, actual, ok) VALUES
      ('verify(duplicate_supabase_uid)', '0', coalesce(v_dup, '(缺项)'), v_dup = '0'),
      ('verify(bound_uid_not_in_auth)',  '0', coalesce(v_orphan, '(缺项)'), v_orphan = '0');
END $$;

-- ----------------------------------------------------------------------------
-- 输出
-- ----------------------------------------------------------------------------
\echo ''
\echo '=== 明细 ==='
SELECT seq, check_name, expected, actual, CASE WHEN ok THEN 'PASS' ELSE 'FAIL' END AS result
  FROM verify_results ORDER BY seq;

\echo ''
SELECT count(*) AS total_checks,
       count(*) FILTER (WHERE ok) AS passed,
       count(*) FILTER (WHERE NOT ok) AS failed,
       CASE WHEN count(*) FILTER (WHERE NOT ok) = 0 THEN 'ALL CHECKS PASSED' ELSE 'HAS FAILURES' END AS verdict
  FROM verify_results;

DO $$
DECLARE v_failed int;
BEGIN
    SELECT count(*) INTO v_failed FROM verify_results WHERE NOT ok;
    IF v_failed > 0 THEN
        RAISE EXCEPTION '验证失败 % 项', v_failed;
    END IF;
END $$;
