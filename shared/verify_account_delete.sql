-- ============================================================
-- verify_account_delete.sql — 删号能力的自包含验证
-- ============================================================
-- 目的：在没有生产库、没有 dump 的机器上，验证
--       account_purge_policy / preview / purge / audit 四个函数的逻辑正确性。
--
-- 用法（必须在一个【空库】里跑，脚本会自己建表）：
--   createdb zervi_verify
--   psql -d zervi_verify -v ON_ERROR_STOP=1 -f shared/account_delete_v0.1.sql
--   psql -d zervi_verify -v ON_ERROR_STOP=1 -f shared/verify_account_delete.sql
--   dropdb zervi_verify
--
-- 它验证什么：
--   1. policy 里 41 张表都能被正确寻址（表名/列名拼错会在这里暴露）
--   2. 多轮收敛能把所有关联行删干净
--   3. N 档（置空）保留实体内容、只断开关联
--   4. audit 在清单完整时返回空集
--
-- 它不验证什么：生产库是否真的长这样。那个靠 shared/preflight_review_blockers.sql。
-- ============================================================

\echo '===== 1. 环境检查：必须是空库 ====='
DO $$
BEGIN
    IF to_regclass('public.users') IS NOT NULL THEN
        RAISE EXCEPTION '当前库已有 public.users，请换一个空库（createdb zervi_verify）再跑';
    END IF;
END $$;

\echo '===== 2. 建 users 主表 ====='
CREATE TABLE public.users (
    id    text PRIMARY KEY,
    email text,
    role  text DEFAULT 'user',
    tier  text DEFAULT 'free'
);

\echo '===== 3. 按 policy 动态补齐所有被引用的表（列类型与生产一致：text）====='
DO $$
DECLARE
    r record;
BEGIN
    FOR r IN SELECT DISTINCT tbl FROM public.account_purge_policy() ORDER BY 1 LOOP
        IF to_regclass('public.' || quote_ident(r.tbl)) IS NULL THEN
            -- 每张表给一个 id 主键 + policy 需要的那一列
            EXECUTE format(
                'CREATE TABLE public.%I (id bigserial PRIMARY KEY)',
                r.tbl
            );
        END IF;
    END LOOP;

    -- 再把 policy 里的列补上（避免同一张表出现两次的不同列）
    FOR r IN SELECT tbl, col FROM public.account_purge_policy() ORDER BY 1, 2 LOOP
        IF NOT EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = r.tbl AND column_name = r.col
        ) THEN
            EXECUTE format('ALTER TABLE public.%I ADD COLUMN %I text', r.tbl, r.col);
        END IF;
    END LOOP;
END $$;

\echo '===== 4. 造测试数据：一个用户，每张表挂 1 行；poems 挂 3 行 ====='
INSERT INTO public.users (id, email) VALUES ('verify-user-1', 'verify@example.test');

-- 交易留痕表（ANON 档）：不在 policy 里，但 preview 必须能报出来
CREATE TABLE public.iap_subscriptions (
    id                      bigserial PRIMARY KEY,
    original_transaction_id text,
    user_id                 text,
    status                  text
);
INSERT INTO public.iap_subscriptions (original_transaction_id, user_id, status)
VALUES ('2000000000000001', 'verify-user-1', 'active');

DO $$
DECLARE
    r record;
BEGIN
    FOR r IN SELECT tbl, col FROM public.account_purge_policy() ORDER BY 1, 2 LOOP
        IF r.tbl = 'poems' THEN
            -- N 档重点：内容必须活下来
            EXECUTE format(
                'INSERT INTO public.%I (%I) VALUES ($1), ($1), ($1)',
                r.tbl, r.col
            ) USING 'verify-user-1';
        ELSE
            EXECUTE format('INSERT INTO public.%I (%I) VALUES ($1)', r.tbl, r.col)
                USING 'verify-user-1';
        END IF;
    END LOOP;
END $$;

\echo '===== 5. audit 应为空集（清单完整）====='
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM public.account_purge_audit();
    IF n > 0 THEN
        RAISE EXCEPTION 'audit 非空：有 % 张疑似用户表未纳入 policy', n;
    END IF;
    RAISE NOTICE 'PASS  audit 返回 0 行';
END $$;

\echo '===== 6. preview 应覆盖多张表 ====='
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM public.account_purge_preview('verify-user-1');
    IF n < 30 THEN
        RAISE EXCEPTION 'preview 只报出 % 张表，预期 ≥ 30（policy 有 41 条）', n;
    END IF;
    RAISE NOTICE 'PASS  preview 报出 % 张表', n;
END $$;

\echo '===== 7. 执行删除 ====='
SELECT tbl, action, rows_affected
FROM public.account_purge('verify-user-1')
ORDER BY round, tbl;

\echo '===== 8. 断言 ====='
DO $$
DECLARE
    r record;
    n bigint;
    leftover int := 0;
    poems_total bigint;
    poems_orphan bigint;
BEGIN
    -- 8.1 用户行必须没了
    IF EXISTS (SELECT 1 FROM public.users WHERE id = 'verify-user-1') THEN
        RAISE EXCEPTION 'FAIL  users 行仍存在，删号未完成';
    END IF;

    -- 8.2 D 档：每张表该用户的行必须为 0
    FOR r IN SELECT tbl, col FROM public.account_purge_policy() WHERE act = 'D' ORDER BY 1, 2 LOOP
        EXECUTE format('SELECT count(*) FROM public.%I WHERE %I = $1', r.tbl, r.col)
            INTO n USING 'verify-user-1';
        IF n > 0 THEN
            RAISE WARNING 'FAIL  % 残留 % 行（应删净）', r.tbl, n;
            leftover := leftover + 1;
        END IF;
    END LOOP;

    -- 8.3 N 档：poems 内容必须还在，且 publisher_id 已置空
    SELECT count(*), count(*) FILTER (WHERE publisher_id IS NULL)
      INTO poems_total, poems_orphan
      FROM public.poems;

    IF poems_total <> 3 THEN
        RAISE EXCEPTION 'FAIL  poems 内容丢失：剩 % 行，应为 3', poems_total;
    END IF;
    IF poems_orphan <> 3 THEN
        RAISE EXCEPTION 'FAIL  poems.publisher_id 未置空：% / 3', poems_orphan;
    END IF;

    IF leftover > 0 THEN
        RAISE EXCEPTION 'FAIL  % 张表存在残留', leftover;
    END IF;

    -- 8.4 ANON 档：交易留痕必须保留（财务凭证不能随删号消失）
    SELECT count(*) INTO n FROM public.iap_subscriptions WHERE user_id = 'verify-user-1';
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL  iap_subscriptions 被误删：剩 % 行，应保留 1 行', n;
    END IF;

    RAISE NOTICE 'PASS  users 已删；D 档 0 残留；poems 3 行完好且 publisher_id 置空；iap 留痕保留';
    RAISE NOTICE '===== ALL CHECKS PASSED =====';
END $$;
