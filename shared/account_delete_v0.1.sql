-- ============================================================
-- account_delete_v0.1.sql — 账号删除（Apple 5.1.1(v) 的库层能力）
-- ============================================================
-- 背景：public.users 被 41 个外键引用，其中 37 个是裸 FK（NO ACTION）。
--       直接 DELETE FROM users 必然外键违例，所以线上只能软删 ——
--       而 Apple 明确不接受"仅停用"。本文件提供硬删除能力。
--
-- 设计：
--   1. account_purge_policy()   —— 单一声明式清单（表/列/动作 D=删 N=置空）
--   2. account_purge_preview()  —— 只读预演，逐表报出会影响多少行
--   3. account_purge()          —— 多轮收敛删除 + 最后删 users 行
--
-- 为什么是"多轮"而不是固定顺序：
--   表与表之间还有 115 条 FK（例如 match_consensus → candidates），
--   写死顺序在表结构演进后必然失效。收敛式删除对任意拓扑都成立，
--   且新增表只要补一行 policy 即可。
--
-- 不在本文件内、但删号必须一起做的两件事：
--   · Supabase Auth 侧删除 auth.users（调 Admin API，SQL 做不到）
--   · 客户端 token 吊销（否则已签发的 JWT 在过期前仍可用）
--
-- 前置：policy 里的表名必须都真实存在（EXECUTE 不支持缺表），
--       所以 resume_blacklist_v0.1.sql 这类新增表 DDL 要先于本文件执行。
-- ============================================================

-- ------------------------------------------------------------
-- 1. 政策清单
--    D = 行属于该用户，直接删除
--    N = 业务实体要保留（如诗词正文、邀请码），只断开个人关联
--    未列入的表 = 不受影响（3 条 CASCADE / 1 条 SET NULL 由外键自动处理）
-- ------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.account_purge_policy()
RETURNS TABLE (tbl text, col text, act text)
LANGUAGE sql
IMMUTABLE
AS $fn$
    VALUES
    -- 求职侧：投递、档案、匹配、报告
    ('applications',            'seeker_user_id', 'D'),
    ('candidates',              'user_id',        'D'),
    ('match_consensus',         'candidate_id',   'D'),
    ('match_consensus',         'initiator_id',   'D'),
    ('match_reports',           'user_id',        'D'),
    ('timeline_events',         'user_id',        'D'),
    ('narrative_sessions',      'user_id',        'D'),
    ('navigator_memories',      'user_id',        'D'),
    ('trust_attestations',      'candidate_id',   'D'),
    ('trust_memories',          'user_id',        'D'),
    ('value_imprints',          'user_id',        'D'),
    ('report_sharing',          'user_id',        'D'),
    ('share_codes',             'owner_id',       'D'),
    ('quiz_sessions',           'user_id',        'D'),
    ('challenge_entries',       'user_id',        'D'),
    ('mission_completions',     'user_id',        'D'),
    ('game_profiles',           'user_id',        'D'),
    -- 简历域屏蔽企业（shared/resume_blacklist_v0.1.sql）。该表对 users 是 ON DELETE CASCADE，
    -- 即便漏跑本行，删 users 时也不会报外键违例；列入 policy 是为了 preview 能看见行数、
    -- 且 behavior 与其他用户数据一致（先删行，再删 users）。
    ('resume_blacklist',        'user_id',        'D'),

    -- 企业侧
    ('job_posts',               'user_id',        'D'),
    ('enterprise_users',        'user_id',        'D'),
    ('sales_inquiries',         'user_id',        'D'),
    ('fleets',                  'captain_id',     'D'),   -- 0 行；有成员时需先转移队长
    ('fleet_members',           'user_id',        'D'),

    -- 身份与同意
    ('user_identities',         'user_id',        'D'),
    ('consents',                'user_id',        'D'),
    ('domain_visits',           'user_id',        'D'),
    ('emergent_strategy_log',   'user_id',        'D'),
    ('usage_logs',              'user_id',        'D'),
    ('quota_records',           'user_id',        'D'),
    ('huawei_push_tokens',      'user_id',        'D'),
    ('verification_audit_log',  'owner_id',       'D'),

    -- 行为日志：有 user_id 但没有外键指向 users，FK 挡不住、只能靠清单
    -- （这四张是靠 account_purge_audit() 自检发现的漏网表）
    ('llm_usage',               'user_id',        'D'),
    ('micro_feedback',          'user_id',        'D'),
    ('product_events',          'user_id',        'D'),
    ('query_logs',              'user_id',        'D'),

    -- 交易（当前 orders/subscriptions/boost_* 全 0 行）
    ('orders',                  'user_id',        'D'),
    ('subscriptions',           'user_id',        'D'),
    ('boost_packs',             'user_id',        'D'),
    ('boost_consumptions',      'user_id',        'D'),

    -- 保留实体、断开个人：诗词正文与邀请码不属于"个人数据"
    ('poems',                   'publisher_id',   'N'),
    ('invite_codes',            'created_by',     'N'),
    ('invite_codes',            'used_by',        'N')
$fn$;

COMMENT ON FUNCTION public.account_purge_policy() IS
'账号删除政策清单。D=删除该用户的行；N=保留实体、仅置空关联列。新增引用 users 的表必须在此补一行，否则 account_purge 会在末步报外键违例。';

-- ------------------------------------------------------------
-- 2. 预演（只读，可随时对真实用户跑）
-- ------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.account_purge_preview(p_uid text)
RETURNS TABLE (tbl text, col text, act text, rows_affected bigint)
LANGUAGE plpgsql
STABLE
AS $fn$
DECLARE
    r record;
    n bigint;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.users WHERE id = p_uid) THEN
        RAISE EXCEPTION 'account_purge_preview: users 中不存在 %', p_uid;
    END IF;

    FOR r IN SELECT * FROM public.account_purge_policy() ORDER BY 3 DESC, 1, 2 LOOP
        EXECUTE format('SELECT count(*) FROM public.%I WHERE %I = $1', r.tbl, r.col)
            INTO n USING p_uid;
        IF n > 0 THEN
            tbl := r.tbl; col := r.col; act := r.act; rows_affected := n;
            RETURN NEXT;
        END IF;
    END LOOP;

    -- 交易留痕表（无 FK，不阻塞删号，但含个人标识，需单独脱敏）
    -- 用 to_regclass 判存在：该表由 payment-service/schema_v0.1.sql 单独创建，
    -- 没跑过那份 DDL 的环境里不该让 preview 直接炸。
    IF to_regclass('public.iap_subscriptions') IS NOT NULL THEN
        EXECUTE 'SELECT count(*) FROM public.iap_subscriptions WHERE user_id = $1'
            INTO n USING p_uid;
        IF n > 0 THEN
            tbl := 'iap_subscriptions'; col := 'user_id'; act := 'ANON'; rows_affected := n;
            RETURN NEXT;
        END IF;
    END IF;
END
$fn$;

COMMENT ON FUNCTION public.account_purge_preview(text) IS
'账号删除预演：只统计不修改。act=ANON 表示该表不受外键阻塞，但含个人标识，需业务侧脱敏（保留交易金额与 Apple 标识用于对账）。';

-- ------------------------------------------------------------
-- 3. 执行（幂等 + 收敛；建议外层包事务）
-- ------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.account_purge(p_uid text)
RETURNS TABLE (round int, action text, tbl text, rows_affected bigint)
LANGUAGE plpgsql
AS $fn$
DECLARE
    r record;
    n bigint;
    rnd int := 0;
    moved boolean;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.users WHERE id = p_uid) THEN
        RAISE EXCEPTION 'account_purge: users 中不存在 %', p_uid;
    END IF;

    LOOP
        rnd := rnd + 1;
        moved := false;

        FOR r IN SELECT * FROM public.account_purge_policy() ORDER BY 1, 2 LOOP
            BEGIN
                IF r.act = 'D' THEN
                    EXECUTE format('DELETE FROM public.%I WHERE %I = $1', r.tbl, r.col)
                        USING p_uid;
                ELSE
                    EXECUTE format('UPDATE public.%I SET %I = NULL WHERE %I = $1', r.tbl, r.col, r.col)
                        USING p_uid;
                END IF;

                GET DIAGNOSTICS n = ROW_COUNT;
                IF n > 0 THEN
                    moved := true;
                    round := rnd; action := r.act; tbl := r.tbl; rows_affected := n;
                    RETURN NEXT;
                END IF;
            EXCEPTION WHEN foreign_key_violation THEN
                -- 该表仍被本轮尚未清理的行引用，留到下一轮
                NULL;
            END;
        END LOOP;

        EXIT WHEN NOT moved;

        IF rnd >= 10 THEN
            RAISE EXCEPTION 'account_purge: 10 轮仍未收敛，存在与用户无关的外键环，需补 policy 或先转移资产';
        END IF;
    END LOOP;

    DELETE FROM public.users WHERE id = p_uid;
    GET DIAGNOSTICS n = ROW_COUNT;
    round := rnd; action := 'DEL_USER'; tbl := 'users'; rows_affected := n;
    RETURN NEXT;
END
$fn$;

COMMENT ON FUNCTION public.account_purge(text) IS
'硬删除账号：按 policy 多轮收敛清理，最后删 users 行。建议在事务内调用并校验结果；删除后需另行调 Supabase Admin API 删 auth.users。';

-- ------------------------------------------------------------
-- 4. 自检：找出"像用户数据、却没被 policy 覆盖"的表
--    新增表时跑一次；返回空集才算清单完整
-- ------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.account_purge_audit()
RETURNS TABLE (table_name text, column_name text, reason text)
LANGUAGE sql
STABLE
AS $fn$
    WITH pol AS (
        SELECT p.tbl, p.col FROM public.account_purge_policy() p
    ),
    fk AS (
        SELECT c.conrelid::regclass::text AS tbl, a.attname AS col,
               (c.confdeltype <> 'a') AS auto   -- CASCADE / SET NULL 由外键自动处理
        FROM pg_constraint c
        JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
        WHERE c.contype = 'f'
          -- 用 to_regclass 而非 'public.users'::regclass：
          -- 后者在建表前会直接抛 "relation does not exist"，导致函数无法先于表装载。
          AND c.confrelid = to_regclass('public.users')
    ),
    anon AS (
        -- 无外键、含个人标识，但按合规需保留交易留痕的表（只脱敏不删）
        SELECT unnest(ARRAY['iap_subscriptions', 'apple_iap_transactions']) AS t
    )
    SELECT ic.table_name::text, ic.column_name::text,
           CASE WHEN NOT EXISTS (SELECT 1 FROM fk WHERE fk.tbl = ic.table_name AND fk.col = ic.column_name)
                THEN '有用户列但无外键：FK 挡不住，遗漏即残留'
                ELSE '有外键但不在 policy：删除会被阻塞'
           END
    FROM information_schema.columns ic
    WHERE ic.table_schema = 'public'
      AND (ic.column_name LIKE '%user_id%'
           OR ic.column_name IN ('owner_id','created_by','publisher_id','candidate_id',
                                 'driver_id','captain_id','seeker_id','sender_id','recipient_id'))
      AND NOT EXISTS (SELECT 1 FROM pol WHERE pol.tbl = ic.table_name AND pol.col = ic.column_name)
      AND NOT EXISTS (SELECT 1 FROM fk WHERE fk.tbl = ic.table_name AND fk.col = ic.column_name AND fk.auto)
      AND NOT EXISTS (SELECT 1 FROM anon WHERE anon.t = ic.table_name)
    ORDER BY 1, 2
$fn$;

COMMENT ON FUNCTION public.account_purge_audit() IS
'账号删除清单自检：返回所有疑似用户数据列但未被 policy 覆盖的表。空集 = 清单完整。上线后每次加表都跑一次。';

-- ============================================================
-- 回滚
-- ============================================================
-- DROP FUNCTION IF EXISTS public.account_purge_audit();
-- DROP FUNCTION IF EXISTS public.account_purge(text);
-- DROP FUNCTION IF EXISTS public.account_purge_preview(text);
-- DROP FUNCTION IF EXISTS public.account_purge_policy();
