-- ============================================================================
-- supabase_uid 回填 v0.1（一次性运维脚本，可重入）
--
-- 目的：把老账号的 users.supabase_uid 绑上 Supabase Auth 的 uid，
--       让 `load_user` 的第 ① 条路径（uid 命中）成立，避免老用户分裂成两行。
--
-- 背景：生产库 users 10 / 已绑 supabase_uid 0。未绑定时，Supabase 登录只能靠
--       "email 命中"合并；一旦 Apple「隐藏我的邮箱」把 email 变成
--       xxx@privaterelay.appleid.com，email 也命不中，旧代码就会新建一行孤儿 user，
--       订阅记在孤儿行上（买完不解锁）。本脚本先把这 10 个账号绑掉。
--
-- 安全设计（逐条对应一次真实事故的可能性）：
--   1) 只写 `supabase_uid IS NULL` 的行 —— 永不覆盖已有绑定；
--   2) 目标 uid 已被别的 users 行占用 ⇒ 跳过（conflict_uid_taken），不猜；
--   3) 同一 email 在 auth.users 或本地 users 命中多行 ⇒ 跳过（ambiguous_*），不猜；
--   4) 手工指定的 uid 若 auth.users 里查不到 ⇒ 跳过（uid_not_in_auth），除非显式放行；
--   5) 每次真正写入都落 identity_link_log，可回溯"谁在什么时候绑了谁"；
--   6) 默认 p_apply=false（dry-run），必须显式传 true 才写库。
--
-- 用法（三步，别跳）：
--   \i shared/backfill_supabase_uid_v0.1.sql
--   -- ① 体检：列出每个未绑账号的判定结果，不写库
--   SELECT * FROM public.identity_backfill_preview();
--   -- ② 试运行（等价于 ①，但走 apply 的同一套决策逻辑）
--   SELECT * FROM public.identity_backfill(false);
--   -- ③ 确认 status 全是 ready / already_bound 后，真正写入
--   SELECT * FROM public.identity_backfill(true);
--   -- ④ 复核
--   SELECT * FROM public.identity_backfill_verify();
--
-- 运行位置：
--   · Supabase：SQL Editor（能读 auth.users，可自动按 email 匹配）
--   · 自建 PG：psql（读不到 auth.users，只能用手工映射表 supabase_uid_backfill_map）
--
-- ⚠️ 本脚本会 CREATE TABLE（两张运维台账表），与 v0.3 快照不同源，
--    后续应由正式迁移吸收；不要在业务事务里调用。
-- ============================================================================

-- ----------------------------------------------------------------------------
-- 0) 运维台账表
-- ----------------------------------------------------------------------------

-- 手工映射表（email → supabase_uid）。读不到 auth.users 时靠它。
CREATE TABLE IF NOT EXISTS public.supabase_uid_backfill_map (
    email        text NOT NULL PRIMARY KEY,
    supabase_uid text NOT NULL,
    note         text,
    added_at     timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE public.supabase_uid_backfill_map IS
    'supabase_uid 回填手工映射（email 小写）。identity_backfill 优先用它，其次用 auth.users 自动匹配。';

-- 绑定审计日志：一次写入一行。
CREATE TABLE IF NOT EXISTS public.identity_link_log (
    id           bigserial PRIMARY KEY,
    user_id      text NOT NULL,
    email        text,
    supabase_uid text,
    action       text NOT NULL,   -- link | relink | unlink
    source       text,            -- auth.users | manual
    actor        text NOT NULL DEFAULT current_user,
    applied_at   timestamptz NOT NULL DEFAULT now(),
    note         text
);

COMMENT ON TABLE public.identity_link_log IS
    '身份绑定审计：users.supabase_uid 的每次写入都留痕（含操作者与来源）。';

CREATE INDEX IF NOT EXISTS identity_link_log_user_idx
    ON public.identity_link_log (user_id, applied_at DESC);

-- ----------------------------------------------------------------------------
-- 1) 读 auth.users 的三个小工具（auth.users 不存在时安全退化）
-- ----------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION public.identity_auth_available()
RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT to_regclass('auth.users') IS NOT NULL;
$$;

COMMENT ON FUNCTION public.identity_auth_available() IS
    '当前库能否看到 Supabase 的 auth.users（自建 PG 上通常为 false）。';

-- email → uid（同 email 多行时 n > 1，调用方必须跳过，不许猜）
CREATE OR REPLACE FUNCTION public.identity_auth_email_map()
RETURNS TABLE (email text, uid text, auth_rows integer)
LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF to_regclass('auth.users') IS NULL THEN
        RETURN;  -- 空集：自建 PG 的正常情况
    END IF;
    RETURN QUERY EXECUTE $q$
        SELECT lower(btrim(u.email))::text,
               (array_agg(u.id::text ORDER BY u.id::text))[1]::text,
               count(*)::int
          FROM auth.users u
         WHERE u.email IS NOT NULL AND btrim(u.email) <> ''
         GROUP BY 1
    $q$;
END $$;

COMMENT ON FUNCTION public.identity_auth_email_map() IS
    'auth.users 按小写 email 聚合出的 uid 候选；auth.$ 不存在时返回空集。';

-- 指定 uid 是否真的存在于 auth.users；auth.users 不可见时返回 NULL（未知）
CREATE OR REPLACE FUNCTION public.identity_auth_uid_exists(p_uid text)
RETURNS boolean
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_exists boolean;
BEGIN
    IF to_regclass('auth.users') IS NULL THEN
        RETURN NULL;
    END IF;
    EXECUTE 'SELECT EXISTS (SELECT 1 FROM auth.users WHERE id::text = $1)' INTO v_exists USING p_uid;
    RETURN v_exists;
END $$;

COMMENT ON FUNCTION public.identity_auth_uid_exists(text) IS
    '校验手工映射里的 uid 是否真实存在；auth 不可见时返回 NULL 表示"无法验证"。';

-- ----------------------------------------------------------------------------
-- 2) 决策表：preview 与 apply 共用同一套判定（避免"预览说行、执行说不行"）
-- ----------------------------------------------------------------------------
--
-- status 取值：
--   already_bound         已绑定，且与候选一致      → 不动
--   already_bound_other   已绑定，但候选不同        → 不动（人工判断是否要重绑）
--   no_email              本地行没有 email          → 无法匹配
--   ambiguous_email_local 本地 users 里同 email 多行 → 人工裁决
--   ambiguous_email_auth  auth.users 里同 email 多行 → 人工裁决
--   unmatched             没有任何候选 uid          → 手工补映射表
--   conflict_uid_taken    候选 uid 已被别的行占用    → 人工裁决
--   uid_not_in_auth       手工 uid 在 auth.users 查不到 → 核对后再放行
--   ready                 可写入
CREATE OR REPLACE FUNCTION public.identity_backfill_rows()
RETURNS TABLE (
    user_id       text,
    email         text,
    status        text,
    candidate_uid text,
    source        text,
    detail        text
)
LANGUAGE sql STABLE AS $$
WITH u AS (
    SELECT us.id::text                                  AS rid,
           lower(btrim(coalesce(us.email, '')))         AS mail,
           nullif(btrim(coalesce(us.supabase_uid, '')), '') AS bound
      FROM public.users us
),
man AS (
    SELECT lower(btrim(mm.email)) AS mail, btrim(mm.supabase_uid) AS uid
      FROM public.supabase_uid_backfill_map mm
),
auth AS (SELECT * FROM public.identity_auth_email_map()),
mail_dupes AS (
    SELECT u.mail FROM u WHERE u.mail <> '' GROUP BY u.mail HAVING count(*) > 1
),
uid_used AS (
    SELECT u.bound AS uid FROM u WHERE u.bound IS NOT NULL
),
pick AS (
    SELECT u.rid, u.mail, u.bound,
           coalesce(man.uid, auth.uid)                             AS cand,
           CASE WHEN man.uid IS NOT NULL THEN 'manual'
                WHEN auth.uid IS NOT NULL THEN 'auth.users'
                ELSE null END                                     AS cand_src,
           coalesce(auth.auth_rows, 0)                            AS auth_n
      FROM u
      LEFT JOIN man  ON man.mail  = u.mail
      LEFT JOIN auth ON auth.email = u.mail
)
SELECT p.rid,
       p.mail,
       CASE
         WHEN p.bound IS NOT NULL AND p.cand IS NOT NULL AND p.bound = p.cand
              THEN 'already_bound'
         WHEN p.bound IS NOT NULL
              THEN 'already_bound_other'
         WHEN p.mail = ''
              THEN 'no_email'
         WHEN EXISTS (SELECT 1 FROM mail_dupes d WHERE d.mail = p.mail)
              THEN 'ambiguous_email_local'
         WHEN p.cand_src = 'auth.users' AND p.auth_n > 1
              THEN 'ambiguous_email_auth'
         WHEN p.cand IS NULL
              THEN 'unmatched'
         WHEN p.cand_src = 'manual' AND public.identity_auth_uid_exists(p.cand) IS FALSE
              THEN 'uid_not_in_auth'
         WHEN EXISTS (SELECT 1 FROM uid_used x WHERE x.uid = p.cand)
              THEN 'conflict_uid_taken'
         ELSE 'ready'
       END                                     AS status,
       p.cand                                  AS candidate_uid,
       p.cand_src                              AS source,
       CASE
         WHEN p.bound IS NOT NULL AND p.cand IS NOT NULL AND p.bound = p.cand
              THEN '已绑定到同一 uid，无需处理'
         WHEN p.bound IS NOT NULL
              THEN format('已绑定到 %s，与候选 %s 不同；如确认要换绑请人工处理（脚本不覆盖）', p.bound, coalesce(p.cand, '(无候选)'))
         WHEN p.mail = ''
              THEN '该行没有 email，只能手工映射'
         WHEN EXISTS (SELECT 1 FROM mail_dupes d WHERE d.mail = p.mail)
              THEN '本地 users 里同一 email 有多行（生产库 email 无 UNIQUE 约束），需人工合并'
         WHEN p.cand_src = 'auth.users' AND p.auth_n > 1
              THEN format('auth.users 里同一 email 有 %s 个账号，需人工确认用哪个', p.auth_n)
         WHEN p.cand IS NULL
              THEN 'auth.users 与手工映射表都没有该 email；可在 supabase_uid_backfill_map 里补一行'
         WHEN p.cand_src = 'manual' AND public.identity_auth_uid_exists(p.cand) IS FALSE
              THEN format('手工指定的 uid %s 在 auth.users 里不存在，核对后再放行', p.cand)
         WHEN EXISTS (SELECT 1 FROM uid_used x WHERE x.uid = p.cand)
              THEN format('候选 uid %s 已被其它 users 行占用', p.cand)
         ELSE format('将写入 supabase_uid=%s（来源 %s）', p.cand, p.cand_src)
       END                                     AS detail
  FROM pick p
 ORDER BY 3, 2, 1;
$$;

COMMENT ON FUNCTION public.identity_backfill_rows() IS
    '回填决策表（只读）：一行一个 users 记录，给出 status/candidate_uid/source。preview 与 apply 共用。';

-- ----------------------------------------------------------------------------
-- 3) 体检（只读，推荐先看这个）
-- ----------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION public.identity_backfill_preview()
RETURNS TABLE (
    user_id       text,
    email         text,
    status        text,
    candidate_uid text,
    source        text,
    detail        text
)
LANGUAGE sql STABLE AS $$
    SELECT * FROM public.identity_backfill_rows();
$$;

COMMENT ON FUNCTION public.identity_backfill_preview() IS
    '回填体检（只读）：不写库，只告诉你每个账号会被判成 ready 还是被拦。';

-- ----------------------------------------------------------------------------
-- 4) 执行（默认 dry-run）
-- ----------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION public.identity_backfill(
    p_apply                  boolean DEFAULT false,
    p_allow_unverified_uid   boolean DEFAULT false
)
RETURNS TABLE (
    user_id       text,
    email         text,
    status        text,
    candidate_uid text,
    source        text,
    applied       boolean,
    detail        text
)
LANGUAGE plpgsql AS $$
DECLARE
    r          record;
    v_ts_expr  text;
    v_rows     int;
    v_holder   text;
BEGIN
    -- updated_at 类型对齐：生产库是 text（DEFAULT now() 的历史遗留），v0.3 是 timestamptz。
    -- 直接 SET updated_at = now() 在 text 列上会报类型错，所以按实际类型拼表达式。
    SELECT CASE WHEN c.data_type IN ('text', 'character varying') THEN 'now()::text'
                ELSE 'now()' END
      INTO v_ts_expr
      FROM information_schema.columns c
     WHERE c.table_schema = 'public' AND c.table_name = 'users' AND c.column_name = 'updated_at';
    v_ts_expr := coalesce(v_ts_expr, 'now()');

    FOR r IN SELECT * FROM public.identity_backfill_rows() ORDER BY 1
    LOOP
        user_id       := r.user_id;
        email         := r.email;
        status        := r.status;
        candidate_uid := r.candidate_uid;
        source        := r.source;
        applied       := false;
        detail        := r.detail;

        IF r.status = 'ready' THEN
            IF NOT p_apply THEN
                detail := format('dry-run：将写入 supabase_uid=%s（来源 %s），未写库', r.candidate_uid, r.source);
                RETURN NEXT;
                CONTINUE;
            END IF;

            -- 手工 uid 的可验证性（auth 不可见时 identity_auth_uid_exists 返回 NULL，不拦）
            IF r.source = 'manual'
               AND NOT p_allow_unverified_uid
               AND public.identity_auth_uid_exists(r.candidate_uid) IS FALSE THEN
                status := 'uid_not_in_auth';
                detail := format('手工 uid %s 在 auth.users 不存在，已跳过（如需强制，传 p_allow_unverified_uid=true）', r.candidate_uid);
                RETURN NEXT;
                CONTINUE;
            END IF;

            -- 抢锁式复查：uid 是否已被别的行占用（并行/重入时兜底）
            EXECUTE 'SELECT id::text FROM public.users WHERE supabase_uid = $1 AND id::text <> $2 LIMIT 1'
               INTO v_holder USING r.candidate_uid, r.user_id;
            IF v_holder IS NOT NULL THEN
                status := 'conflict_uid_taken';
                detail := format('候选 uid %s 已被 users.id=%s 占用，已跳过', r.candidate_uid, v_holder);
                RETURN NEXT;
                CONTINUE;
            END IF;

            -- 只写 NULL 行：永不覆盖已有绑定（重入安全）
            EXECUTE format(
                'UPDATE public.users SET supabase_uid = $1, updated_at = %s
                  WHERE id::text = $2 AND (supabase_uid IS NULL OR btrim(supabase_uid) = '''')',
                v_ts_expr
            ) USING r.candidate_uid, r.user_id;
            GET DIAGNOSTICS v_rows = ROW_COUNT;

            IF v_rows = 1 THEN
                applied := true;
                status  := 'linked';
                detail  := format('已写入 supabase_uid=%s（来源 %s）', r.candidate_uid, r.source);
                INSERT INTO public.identity_link_log
                    (user_id, email, supabase_uid, action, source, note)
                VALUES
                    (r.user_id, nullif(r.email, ''), r.candidate_uid, 'link', r.source,
                     'identity_backfill v0.1');
            ELSE
                applied := false;
                status  := 'skipped_concurrent';
                detail  := '更新 0 行：该行已被并发绑定或已不是 NULL，未写入';
            END IF;
        END IF;

        RETURN NEXT;
    END LOOP;
END $$;

COMMENT ON FUNCTION public.identity_backfill(boolean, boolean) IS
    '执行回填：默认 dry-run（p_apply=false）；只写 supabase_uid IS NULL 的行，每次写入落 identity_link_log。';

-- ----------------------------------------------------------------------------
-- 5) 复核（跑完必看）
-- ----------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION public.identity_backfill_verify()
RETURNS TABLE (check_name text, value bigint, verdict text, detail text)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_auth     boolean := public.identity_auth_available();
    v_total    bigint;
    v_bound    bigint;
    v_dup_uid  bigint;
    v_orphan   bigint;
    v_no_mail  bigint;
BEGIN
    SELECT count(*),
           count(*) FILTER (WHERE nullif(btrim(coalesce(supabase_uid, '')), '') IS NOT NULL),
           count(*) FILTER (WHERE btrim(coalesce(email, '')) = '')
      INTO v_total, v_bound, v_no_mail
      FROM public.users;

    SELECT coalesce(sum(n - 1), 0) INTO v_dup_uid
      FROM (SELECT supabase_uid, count(*) n FROM public.users
             WHERE nullif(btrim(coalesce(supabase_uid, '')), '') IS NOT NULL
             GROUP BY 1) t;

    -- 已绑但 uid 在 auth.users 里查不到（仅 auth 可见时有意义）
    IF v_auth THEN
        EXECUTE $q$
            SELECT count(*) FROM public.users us
             WHERE nullif(btrim(coalesce(us.supabase_uid, '')), '') IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM auth.users a WHERE a.id::text = btrim(us.supabase_uid))
        $q$ INTO v_orphan;
    ELSE
        v_orphan := NULL;
    END IF;

    check_name := 'users_total';        value := v_total;   verdict := 'INFO';
    detail := '总账号数';                                            RETURN NEXT;
    check_name := 'supabase_uid_bound'; value := v_bound;   verdict := 'INFO';
    detail := format('已绑定 %s / %s', v_bound, v_total);            RETURN NEXT;
    check_name := 'supabase_uid_unbound'; value := v_total - v_bound;
    verdict := CASE WHEN v_total - v_bound = 0 THEN 'PASS' ELSE 'WARN' END;
    detail := '未绑定的账号仍会在 Supabase 登录时走 email 合并';      RETURN NEXT;
    check_name := 'duplicate_supabase_uid'; value := v_dup_uid;
    verdict := CASE WHEN v_dup_uid = 0 THEN 'PASS' ELSE 'FAIL' END;
    detail := '同一 uid 被多行占用会导致登录随机命中';                RETURN NEXT;
    check_name := 'bound_uid_not_in_auth'; value := v_orphan;
    verdict := CASE WHEN v_orphan IS NULL THEN 'SKIP'
                    WHEN v_orphan = 0 THEN 'PASS' ELSE 'FAIL' END;
    detail := CASE WHEN v_auth THEN '已绑 uid 与 auth.users 的对照'
                   ELSE 'auth.users 不可见（自建 PG），此项跳过' END;  RETURN NEXT;
    check_name := 'users_without_email'; value := v_no_mail;
    verdict := CASE WHEN v_no_mail = 0 THEN 'PASS' ELSE 'WARN' END;
    detail := '无 email 的账号只能靠手工映射绑定';                    RETURN NEXT;
END $$;

COMMENT ON FUNCTION public.identity_backfill_verify() IS
    '回填复核：绑定覆盖率、重复 uid、uid 是否真实存在于 auth.users。';

-- ============================================================================
-- 装好了。先跑：
--   SELECT * FROM public.identity_backfill_preview();
--   SELECT * FROM public.identity_backfill(true);   -- 确认后
--   SELECT * FROM public.identity_backfill_verify();
-- ============================================================================
