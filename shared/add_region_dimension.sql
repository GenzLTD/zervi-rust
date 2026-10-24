-- =====================================================================
-- 给 users 补上「区域维度」—— 一套代码 / 多地账本 的地基层
-- =====================================================================
-- 背景（2026-10-24 实测）
--   全部 61 表、包括 users 的 12 个字段，**没有任何区域/法域字段**。
--   ⇒ 无从判断某个用户属于 CN 还是 US 区，「账本不能合」在语义上无法实施，
--     只能退化为「靠物理哪台机」去推断 —— 这正是两个仓库互相看不见的根源。
--
-- 设计
--   users.region           NOT NULL + CHECK，取值 CN / US / EU
--   users.home_instance    归属实例标识（供将来 FDW 联邦定位远端)
--   users.tenant_id        → tenants.id，复用「法律主体」作区域锚
--                            tenants 已有 icp_no（ICP 备案号，中国大陆特有），
--                            说明它本就在建模带地域属性的经营主体，无需新造表
--
-- ⚠️ DEFAULT 'CN' 的取舍
--   给默认值是为了不阻塞任何写入路径（注册/灌历史数据都会自动填值）。
--   代价：**默认值会掩盖「未分类」的用户**。
--   ⇒ 灌完数据后必须做一次人工校准/views 审计，确认 CN 默认是否与实际相符。
--   若希望「强制显式」，去掉 DEFAULT 即可（本表当前 0 行，加 NOT NULL 不需 default）。
--
-- ⚠️ 前置 1：目标库必须是 **61 表全集**（尤其含 `public.tenants`）
--   2026-10-24 实测：存在第三种叫 genzltd 的环境——「旧 genzltd 空壳」，
--   约 50 表、**缺 public.tenants**。在上面跑本脚本会卡在
--   fk_users_tenant_id（FOREIGN KEY REFERENCES public.tenants）→ relation does not exist。
--   该失败是 **fail-closed**（整事务回滚，不留半成品），属于好性质，
--   但容易被误读成「脚本有 bug」。⇒ 跑之前先自检：
--       SELECT count(*) FROM pg_tables WHERE schemaname='public';   -- 全集 59；部署仲裁层后 60
--       SELECT count(*) FROM pg_tables WHERE schemaname='vec';      -- 另有 2 张，别漏数也别算进 public
--       SELECT to_regclass('public.tenants') IS NOT NULL;           -- 期望 t
--
--   （2026-10-24 华为实测更正：这里原写「public 期望 61」是**错的** ——
--     61 是 public 59 + vec 2 的**合计**，不是 public 单 schema 的数。
--     写错的代价：对端照着数会得到 59/60，反过来怀疑自己的库不完整。
--     这是「文档写错 → 对端误判」的实例，记下来。）
--
-- ⚠️ 前置 2：迁移「跑完没报错」不能证明两端同构
--   同名 CHECK 定义不同时，本脚本 EXIT=0 且零提示地跳过（静默漂移）。
--   ⇒ 迁移后必须再跑 region_dimension_verify.sql 比对 DIGEST。
--
-- ⚠️ 前置 3：PG >= 11（依赖 fast default；低于此版本带数据加 NOT NULL DEFAULT 会重写全表）
--
-- 幂等：可重复执行
-- =====================================================================

BEGIN;

ALTER TABLE public.users
    ADD COLUMN IF NOT EXISTS region text NOT NULL DEFAULT 'CN',
    ADD COLUMN IF NOT EXISTS home_instance text,
    ADD COLUMN IF NOT EXISTS tenant_id text;

-- tenant_id 外键：单独加，避免 ALTER TABLE 一次性加多个约束时互相依赖
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'public.users'::regclass AND conname = 'fk_users_tenant_id'
    ) THEN
        ALTER TABLE public.users
            ADD CONSTRAINT fk_users_tenant_id
            FOREIGN KEY (tenant_id) REFERENCES public.tenants(id);
    END IF;
END $$;

-- region 取值约束
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'public.users'::regclass AND conname = 'users_region_check'
    ) THEN
        ALTER TABLE public.users
            ADD CONSTRAINT users_region_check CHECK (region IN ('CN','US','EU'));
    END IF;
END $$;

COMMENT ON COLUMN public.users.region IS
    '计费/数据驻留区域：CN=境内(深伯乐) US=海外(YEDALL LIMITED) EU=欧洲。'
    '一套代码按此字段决定可用支付渠道与数据落点；NOT NULL + CHECK，'
    'DEFAULT ''CN'' 仅为不阻塞写入，**灌完数据需人工校准**。';

COMMENT ON COLUMN public.users.home_instance IS
    '归属实例标识（如 "tencent-cloud-1" / "frankfurt-stage" / "vultr-overseas"）。'
    '将来 FDW 联邦按此定位 foreign server；当前为软约定，无外键约束。';

COMMENT ON COLUMN public.users.tenant_id IS
    '→ tenants.id。法律主体（含 icp_no），复用为区域锚而非新造维度。';

COMMIT;

-- =====================================================================
-- 验收
-- =====================================================================
SELECT
    count(*) FILTER (WHERE attname = 'region')         AS has_region,
    count(*) FILTER (WHERE attname = 'home_instance')  AS has_home_instance,
    count(*) FILTER (WHERE attname = 'tenant_id')      AS has_tenant_id
FROM pg_attribute a
WHERE a.attrelid = 'public.users'::regclass AND a.attnum > 0 AND NOT a.attisdropped;

SELECT conname, pg_get_constraintdef(oid) AS def
FROM pg_constraint
WHERE conrelid = 'public.users'::regclass AND conname IN ('users_region_check','fk_users_tenant_id');
