-- ============================================================
-- resume_blacklist_v0.1.sql — 用户维屏蔽企业（简历域 · 非简历资源）
-- ============================================================
-- 契约：looma-zervi backend/contracts/resume.v1.json v0.1.0
--       · blacklist_list   GET    /v1/resume/blacklist
--       · blacklist_add    POST   /v1/resume/blacklist
--       · blacklist_remove DELETE /v1/resume/blacklist/:item_id
-- 实现：zervi-rust resume-service（routes.rs · blacklist_* 三个 handler）
-- 实体：shared/common/src/entities/resume_blacklist.rs
--
-- 为什么单独建表，不塞 documents：
--   documents 是"简历文档"（doc_type=resume，owner 藏在 metadata->>'user_id'），
--   生命周期绑文件；屏蔽列表是**按用户维的匹配规则集**（企业名 / 信用代码 / 企业 ID
--   三态，命中即过滤推荐与投递），与文档无关。塞进 documents 会让 list/analysis
--   的 owner 过滤、以及按 doc_type 的配额统计一起漂。
--
-- 幂等：本文件可重复执行（IF NOT EXISTS 全量），与 payment-service/schema_v0.1.sql 同约定。
-- 执行：psql "$GENZLTD_DATABASE_URL" -v ON_ERROR_STOP=1 -f shared/resume_blacklist_v0.1.sql
--       （本地：psql postgres://postgres:postgres@127.0.0.1:5433/genzltd -f ...）
--
-- 三条约束口径：
--   1. 分情况必填（契约 body）：
--        match=exact_name    → company_name 非空
--        match=credit_code   → credit_code  非空
--        match=enterprise_id → enterprise_id 非空
--      应用层先校验返回 400 bad_request；DB 的 CHECK 是第二道闸，防绕过 API 直写脏行。
--   2. 去重：同一用户 + 同一 match 维度 + 同一取值只允许一条。
--      用三条**部分唯一索引**而不是单列 match_value：
--        · 不必新增派生列，entity 与 DDL 不会因生成列而漂移
--        · 不同维度互不干扰（同名企业 + 同信用代码可并存）
--        · 取值归一（名称大小写/首尾空格、信用代码大小写）落在索引表达式里
--   3. 不启 RLS：与存量 50 张业务表一致（见 genzltd_schema_v0.3.sql 口径），
--      服务用库 owner 连接。IAP 两表启 RLS 是因为它们只由支付服务后端写入、
--      不允许客户端直连；本表同样只走 resume-service，暂无直连面。
-- ============================================================

CREATE TABLE IF NOT EXISTS resume_blacklist (
    id            TEXT        NOT NULL PRIMARY KEY,
    user_id       TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- 列名用 match_kind：match 在多种方言里是关键字，避开可省掉一整类引号问题。
    -- JSON 面仍为 "match"（实体 #[sea_orm(column_name = "match_kind")] 之外无二次映射，
    -- 序列化在 handler 里显式拼 JSON，见 routes.rs::blacklist_item_json）。
    match_kind    TEXT        NOT NULL,
    company_name  TEXT        NOT NULL DEFAULT '',
    credit_code   TEXT,
    enterprise_id TEXT,
    note          TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT ck_resume_blacklist_match_kind
        CHECK (match_kind IN ('exact_name', 'credit_code', 'enterprise_id')),

    CONSTRAINT ck_resume_blacklist_required_by_match
        CHECK (
               (match_kind = 'exact_name'    AND btrim(company_name) <> '')
            OR (match_kind = 'credit_code'   AND credit_code   IS NOT NULL AND btrim(credit_code)   <> '')
            OR (match_kind = 'enterprise_id' AND enterprise_id IS NOT NULL AND btrim(enterprise_id) <> '')
        )
);

-- 去重（部分唯一索引）
CREATE UNIQUE INDEX IF NOT EXISTS uq_resume_blacklist_name
    ON resume_blacklist (user_id, lower(btrim(company_name)))
    WHERE match_kind = 'exact_name';

CREATE UNIQUE INDEX IF NOT EXISTS uq_resume_blacklist_credit
    ON resume_blacklist (user_id, upper(btrim(credit_code)))
    WHERE match_kind = 'credit_code';

CREATE UNIQUE INDEX IF NOT EXISTS uq_resume_blacklist_ent
    ON resume_blacklist (user_id, btrim(enterprise_id))
    WHERE match_kind = 'enterprise_id';

-- 列表查询：按用户取、按创建时间倒序
CREATE INDEX IF NOT EXISTS idx_resume_blacklist_user
    ON resume_blacklist (user_id, created_at DESC);

COMMENT ON TABLE resume_blacklist IS
'用户维屏蔽企业列表（简历域）。match_kind 对应契约 match 枚举；命中即从推荐/投递侧过滤。删除账号时随 users 级联。';
COMMENT ON COLUMN resume_blacklist.match_kind IS
'契约字段 match（exact_name|credit_code|enterprise_id）；改名是为避开 SQL 关键字。';
COMMENT ON COLUMN resume_blacklist.company_name IS
'展示名。match=exact_name 时即匹配键且必填；其余两态可为空串（客户端只传匹配键），由服务侧 best-effort 从 enterprises 补名。';
