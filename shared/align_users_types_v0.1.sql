-- ============================================================================
-- users 列类型对齐 v0.1（把生产库拉到 v0.3 / Rust entity 的类型上）
--
-- 为什么必须做：
--   `shared/common/src/entities/users.rs`（a7feb51 按 v0.3 反生成）声明：
--     is_early_adopter: Option<i32>              （SQL INT4）
--     created_at / updated_at: Option<DateTimeWithTimeZone>（SQL TIMESTAMPTZ）
--   而生产库（genzltd_20261020 dump）实际是：
--     is_early_adopter bigint                    （INT8）
--     created_at / updated_at text DEFAULT now()  （TEXT）
--
--   后果不是"排序列不准"，而是**任何一行 users 都读不出来**：
--     Query Error: error occurred while decoding column "is_early_adopter":
--     mismatched types; Rust type `Option<i32>` (as SQL type `INT4`)
--     is not compatible with SQL type `INT8`
--   实测：payment verify 只要命中一行 users 就 500 —— 买了必 500，比 P0-1 更早爆。
--   （P0-1 的 409 分支之所以能跑通，是因为它走的是"0 行"路径，不做解码。）
--
-- 用法（三步）：
--   \i shared/align_users_types_v0.1.sql
--   SELECT * FROM public.align_users_types_preview();          -- ① 只读体检
--   BEGIN; SELECT * FROM public.align_users_types(true); COMMIT;  -- ② 执行（单事务）
--   SELECT * FROM public.align_users_types_preview();          -- ③ 复核
--
-- 安全设计：
--   · 已是目标类型 ⇒ 跳过（可重入）；
--   · 先逐值试转，有**转不了的值**就整体不执行（不猜、不丢数据）；
--     text 里的空串按 NULL 处理（`DEFAULT now()` 时代可能留下空串）；
--   · is_early_adopter 超出 int4 范围 ⇒ 拒绝执行；
--   · 必须在显式事务里调用；失败整体回滚，不会留半套。
--
-- 注意：
--   · ALTER TYPE 会重写表并重建索引；users 只有 10 行，秒级完成；大表请用
--     pg_repack / 在线变更工具。
--   · 列上有视图依赖会直接报错（依赖视图需先删后建），这是有意为之：
--     宁可报错，也不要悄悄改坏下游。
--   · 执行后**必须重启服务**：sqlx 的 prepared statement 已缓存旧结果类型，
--     不重启会报 `cached plan must not change result type`。
-- ============================================================================

-- ----------------------------------------------------------------------------
-- 0) 安全试转工具（转不了返回 NULL，不抛异常）
-- ----------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.identity_try_timestamptz(p text)
RETURNS timestamptz
LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    IF p IS NULL OR btrim(p) = '' THEN
        RETURN NULL;
    END IF;
    BEGIN
        RETURN btrim(p)::timestamptz;
    EXCEPTION WHEN others THEN
        RETURN NULL;
    END;
END $$;

COMMENT ON FUNCTION public.identity_try_timestamptz(text) IS
    'text → timestamptz 的"试转"：空串→NULL，转不了→NULL（用于迁移前的脏值体检）。';

-- ----------------------------------------------------------------------------
-- 1) 体检（只读）
-- ----------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.align_users_types_preview()
RETURNS TABLE (
    column_name   text,
    current_type  text,
    target_type   text,
    action        text,
    bad_rows      bigint,
    detail        text
)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_ts   text;
    v_iea  text;
    v_bad_created  bigint := 0;
    v_bad_updated  bigint := 0;
    v_bad_iea      bigint := 0;
    v_note text;
BEGIN
    SELECT c.data_type INTO v_ts FROM information_schema.columns c
     WHERE c.table_schema='public' AND c.table_name='users' AND c.column_name='created_at';
    SELECT c.data_type INTO v_iea FROM information_schema.columns c
     WHERE c.table_schema='public' AND c.table_name='users' AND c.column_name='is_early_adopter';

    IF v_ts IS NULL OR v_iea IS NULL THEN
        RETURN;  -- 没有 users 表
    END IF;

    IF v_ts = 'text' OR v_ts = 'character varying' THEN
        EXECUTE 'SELECT count(*) FROM public.users WHERE created_at IS NOT NULL AND btrim(created_at) <> ''''
                  AND public.identity_try_timestamptz(created_at) IS NULL' INTO v_bad_created;
        EXECUTE 'SELECT count(*) FROM public.users WHERE updated_at IS NOT NULL AND btrim(updated_at) <> ''''
                  AND public.identity_try_timestamptz(updated_at) IS NULL' INTO v_bad_updated;
    END IF;

    IF v_iea = 'bigint' THEN
        EXECUTE 'SELECT count(*) FROM public.users WHERE is_early_adopter IS NOT NULL
                  AND (is_early_adopter > 2147483647 OR is_early_adopter < -2147483648)' INTO v_bad_iea;
    END IF;

    -- created_at
    column_name := 'created_at'; current_type := v_ts; target_type := 'timestamp with time zone';
    action := CASE WHEN v_ts IN ('text','character varying') THEN 'ALTER' ELSE 'SKIP' END;
    bad_rows := v_bad_created;
    detail := CASE WHEN v_ts NOT IN ('text','character varying') THEN '已是目标类型'
                   WHEN v_bad_created > 0 THEN format('%s 行无法转换为时间戳，必须先人工清洗', v_bad_created)
                   ELSE '可安全转换（空串按 NULL 处理）' END;
    RETURN NEXT;

    -- updated_at（同构，直接复用判断）
    column_name := 'updated_at'; current_type := v_ts; target_type := 'timestamp with time zone';
    action := CASE WHEN v_ts IN ('text','character varying') THEN 'ALTER' ELSE 'SKIP' END;
    bad_rows := v_bad_updated;
    detail := CASE WHEN v_ts NOT IN ('text','character varying') THEN '已是目标类型'
                   WHEN v_bad_updated > 0 THEN format('%s 行无法转换为时间戳，必须先人工清洗', v_bad_updated)
                   ELSE '可安全转换（空串按 NULL 处理）' END;
    RETURN NEXT;

    -- is_early_adopter
    column_name := 'is_early_adopter'; current_type := v_iea; target_type := 'integer';
    action := CASE WHEN v_iea = 'bigint' THEN 'ALTER' ELSE 'SKIP' END;
    bad_rows := v_bad_iea;
    detail := CASE WHEN v_iea <> 'bigint' THEN '已是目标类型'
                   WHEN v_bad_iea > 0 THEN format('%s 行超出 int4 范围，必须先人工处理', v_bad_iea)
                   ELSE '可安全收窄' END;
    RETURN NEXT;

    RETURN;
END $$;

COMMENT ON FUNCTION public.align_users_types_preview() IS
    'users 三列类型对齐的只读体检：给出 action=SZALTER/SKIP 与脏值行数。';

-- ----------------------------------------------------------------------------
-- 2) 执行（必须包在显式事务里）
-- ----------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION public.align_users_types(p_apply boolean DEFAULT false)
RETURNS TABLE (column_name text, action text, detail text)
LANGUAGE plpgsql AS $$
DECLARE
    r        record;
    v_bad    bigint;
BEGIN
    FOR r IN SELECT * FROM public.align_users_types_preview() LOOP
        column_name := r.column_name;
        action      := r.action;

        IF r.action <> 'ALTER' THEN
            detail := '已是目标类型，跳过';
            RETURN NEXT;
            CONTINUE;
        END IF;

        IF r.bad_rows > 0 THEN
            detail := format('拒绝执行：%s', r.detail);
            RETURN NEXT;
            CONTINUE;
        END IF;

        IF NOT p_apply THEN
            detail := format('dry-run：将执行 ALTER COLUMN %s TYPE %s', r.column_name, r.target_type);
            RETURN NEXT;
            CONTINUE;
        END IF;

        IF r.column_name = 'is_early_adopter' THEN
            EXECUTE 'ALTER TABLE public.users ALTER COLUMN is_early_adopter TYPE integer
                       USING is_early_adopter::integer';
        ELSE
            EXECUTE format(
                'ALTER TABLE public.users ALTER COLUMN %I TYPE timestamptz
                   USING public.identity_try_timestamptz(%I)',
                r.column_name, r.column_name
            );
        END IF;

        action := 'DONE';
        detail := format('已改为 %s', r.target_type);
        RETURN NEXT;
    END LOOP;
END $$;

COMMENT ON FUNCTION public.align_users_types(boolean) IS
    'users 三列类型对齐：默认 dry-run；脏值存在时整列跳过。必须用 BEGIN;…COMMIT; 包住。';

-- ============================================================================
-- 装好了。照上面"用法"三步走；执行后记得重启服务（清 prepared statement 缓存）。
-- ============================================================================
