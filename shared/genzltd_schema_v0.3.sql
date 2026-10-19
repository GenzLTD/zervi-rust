-- ============================================================
-- genzltd_schema_v0.3 — 全库基准（2026-10-20 重建入库）
-- ============================================================
-- 背景: genzltd_schema_v0.2.sql 从未入库（仅根 Cargo.toml 注释引用），
--       全库 DDL 基准处于"失踪"状态。本文件以
--       shared/common/src/entities 的 50 张存量 entity（sea-orm 0.12
--       codegen 产物，经 gap-close 线整理）反向重建，并并入
--       payment-service/schema_v0.1.sql 的两张 IAP 表，升级为 v0.3。
-- 生成: cargo run -p payment-service --bin gen_schema
--       （scripts/gen_schema_v03.ps1 一条龙：生成+拓扑排序+拼装）
-- 口径限制（entity 重建的已知差异）:
--   · 存量 50 表不带 DEFAULT / CHECK / 业务索引 —— 原库即由应用层
--     （sea-orm ActiveModel 全字段写入）显式赋值，行为一致
--   · FK 约束来自 entity Relation，拓扑排序后按依赖顺序建表
--   · IAP 两表保留 schema_v0.1.sql 原文（含 DEFAULT/CHECK/索引/RLS）
-- 执行: psql postgres://postgres:postgres@127.0.0.1:5433/genzltd
--       -f shared/genzltd_schema_v0.3.sql（生产 Supabase 先在测试项目验证）
-- ============================================================

-- ============================================================
-- 第一部分: 50 张存量表（entity 反向重建，FK 拓扑序）
-- ============================================================
-- [1/50] audit_logs
CREATE TABLE "audit_logs" ( "id" text NOT NULL PRIMARY KEY, "actor" text NOT NULL, "action" text NOT NULL, "resource_type" text NOT NULL, "resource_id" text, "consent_id" text, "metadata" text, "ip" text, "user_agent" text, "created_at" timestamp with time zone );

-- [2/50] documents
CREATE TABLE "documents" ( "id" bigserial NOT NULL PRIMARY KEY, "title" text NOT NULL, "file_path" text NOT NULL UNIQUE, "doc_type" text NOT NULL, "file_size" integer, "metadata" text, "status" text, "chunk_count" integer, "created_at" timestamp with time zone NOT NULL, "processed_at" timestamp with time zone );

-- [3/50] enterprises
CREATE TABLE "enterprises" ( "id" text NOT NULL PRIMARY KEY, "name" text NOT NULL, "domain" text, "plan" text, "created_at" timestamp with time zone );

-- [4/50] llm_usage
CREATE TABLE "llm_usage" ( "id" text NOT NULL PRIMARY KEY, "user_id" text, "provider" text NOT NULL, "model" text NOT NULL, "tokens_in" integer, "tokens_out" integer, "cost_yuan" double precision, "latency_ms" integer, "status" text, "error" text, "created_at" timestamp with time zone );

-- [5/50] micro_feedback
CREATE TABLE "micro_feedback" ( "id" bigserial NOT NULL PRIMARY KEY, "context" text NOT NULL, "score" integer NOT NULL, "optional_text" text, "user_id" text, "session_id" text, "platform" text, "share_code" text, "created_at" timestamp with time zone NOT NULL );

-- [6/50] poems
CREATE TABLE "poems" ( "id" bigserial NOT NULL PRIMARY KEY, "title" text NOT NULL, "author" text NOT NULL, "dynasty" text NOT NULL, "theme" text NOT NULL, "content" text NOT NULL, "tags" text, "source" text, "created_at" timestamp with time zone );

-- [7/50] product_events
CREATE TABLE "product_events" ( "id" bigserial NOT NULL PRIMARY KEY, "event_name" text NOT NULL, "user_id" text, "session_id" text, "platform" text, "share_code" text, "source" text, "success" integer NOT NULL, "properties_json" text, "created_at" timestamp with time zone NOT NULL );

-- [8/50] query_logs
CREATE TABLE "query_logs" ( "id" bigserial NOT NULL PRIMARY KEY, "query_text" text NOT NULL, "provider" text NOT NULL, "response_time_ms" double precision, "chunk_count" integer, "rating" integer, "intent_label" text, "user_id" text, "created_at" timestamp with time zone NOT NULL );

-- [9/50] signal_ingest
CREATE TABLE "signal_ingest" ( "signal_type" text NOT NULL, "user_email_hash" text NOT NULL, "nonce" text NOT NULL, "payload_json" text, "created_at" timestamp with time zone, CONSTRAINT "pk-signal_ingest" PRIMARY KEY ("signal_type", "user_email_hash", "nonce") );

-- [10/50] users
CREATE TABLE "users" ( "id" text NOT NULL PRIMARY KEY, "email" text UNIQUE, "email_hash" text, "password_hash" text, "wechat_openid" text UNIQUE, "supabase_uid" text UNIQUE, "name" text, "tier" text, "role" text, "is_early_adopter" integer, "created_at" timestamp with time zone, "updated_at" timestamp with time zone );

-- [11/50] applications
CREATE TABLE "applications" ( "id" text NOT NULL PRIMARY KEY, "seeker_user_id" text NOT NULL, "resume_id" bigint NOT NULL, "job_id" text NOT NULL, "enterprise_id" text, "status" text, "metadata" text, "created_at" timestamp with time zone NOT NULL, "updated_at" timestamp with time zone NOT NULL, "withdrawn_at" timestamp with time zone, CONSTRAINT "fk-applications-resume_id" FOREIGN KEY ("resume_id") REFERENCES "documents" ("id") ON DELETE RESTRICT ON UPDATE NO ACTION, CONSTRAINT "fk-applications-enterprise_id" FOREIGN KEY ("enterprise_id") REFERENCES "enterprises" ("id") ON DELETE SET NULL ON UPDATE NO ACTION, CONSTRAINT "fk-applications-seeker_user_id" FOREIGN KEY ("seeker_user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [12/50] boost_packs
CREATE TABLE "boost_packs" ( "id" bigserial NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "pack_name" text NOT NULL, "credits" integer NOT NULL, "credits_used" integer, "price_yuan" integer, "purchased_at" timestamp with time zone, "expires_at" timestamp with time zone NOT NULL, "status" text, CONSTRAINT "fk-boost_packs-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [13/50] candidates
CREATE TABLE "candidates" ( "id" text NOT NULL PRIMARY KEY, "enterprise_id" text NOT NULL, "user_id" text, "name" text NOT NULL, "email" text, "phone" text, "status" text, "profile_data" text, "created_at" timestamp with time zone, CONSTRAINT "fk-candidates-enterprise_id" FOREIGN KEY ("enterprise_id") REFERENCES "enterprises" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION, CONSTRAINT "fk-candidates-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [14/50] challenge_rounds
CREATE TABLE "challenge_rounds" ( "id" bigserial NOT NULL PRIMARY KEY, "week_key" text NOT NULL UNIQUE, "poem_id" integer NOT NULL, "title" text NOT NULL, "status" text NOT NULL, "starts_at" timestamp with time zone NOT NULL, "ends_at" timestamp with time zone NOT NULL, "created_at" timestamp with time zone, CONSTRAINT "fk-challenge_rounds-poem_id" FOREIGN KEY ("poem_id") REFERENCES "poems" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [15/50] chunks
CREATE TABLE "chunks" ( "id" bigserial NOT NULL PRIMARY KEY, "document_id" integer NOT NULL, "chunk_index" integer NOT NULL, "content" text NOT NULL, "char_count" integer, "metadata" text, "created_at" timestamp with time zone, CONSTRAINT "fk-chunks-document_id" FOREIGN KEY ("document_id") REFERENCES "documents" ("id") ON DELETE CASCADE ON UPDATE NO ACTION );

-- [16/50] consents
CREATE TABLE "consents" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "scope" text NOT NULL, "purpose" text, "status" text, "ip" text, "user_agent" text, "granted_at" timestamp with time zone, "revoked_at" timestamp with time zone, "created_at" timestamp with time zone, "updated_at" timestamp with time zone, CONSTRAINT "fk-consents-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [17/50] emergent_strategy_log
CREATE TABLE "emergent_strategy_log" ( "id" bigserial NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "strategy_type" text NOT NULL, "triggered_at" timestamp with time zone, CONSTRAINT "fk-emergent_strategy_log-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [18/50] enterprise_users
CREATE TABLE "enterprise_users" ( "id" text NOT NULL PRIMARY KEY, "enterprise_id" text NOT NULL, "user_id" text NOT NULL, "role" text, "created_at" timestamp with time zone, CONSTRAINT "fk-enterprise_users-enterprise_id" FOREIGN KEY ("enterprise_id") REFERENCES "enterprises" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION, CONSTRAINT "fk-enterprise_users-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [19/50] fleets
CREATE TABLE "fleets" ( "id" text NOT NULL PRIMARY KEY, "name" text NOT NULL, "captain_id" text NOT NULL, "description" text, "invite_code" text UNIQUE, "created_at" timestamp with time zone, CONSTRAINT "fk-fleets-captain_id" FOREIGN KEY ("captain_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [20/50] game_profiles
CREATE TABLE "game_profiles" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL UNIQUE, "identity" text, "personality_type" text, "personality_detail" text, "xp" integer, "level" integer, "updated_at" timestamp with time zone, CONSTRAINT "fk-game_profiles-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [21/50] huawei_push_tokens
CREATE TABLE "huawei_push_tokens" ( "id" bigserial NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "push_token" text NOT NULL, "created_at" bigint, CONSTRAINT "fk-huawei_push_tokens-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [22/50] invite_codes
CREATE TABLE "invite_codes" ( "id" text NOT NULL PRIMARY KEY, "code" text NOT NULL UNIQUE, "created_by" text, "used_by" text, "tier_grant" text, "expires_at" timestamp with time zone, "used_at" timestamp with time zone, "created_at" timestamp with time zone, CONSTRAINT "fk-invite_codes-created_by" FOREIGN KEY ("created_by") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION, CONSTRAINT "fk-invite_codes-used_by" FOREIGN KEY ("used_by") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [23/50] job_posts
CREATE TABLE "job_posts" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "title" text NOT NULL, "company" text, "description" text, "requirements" text, "status" text, "created_at" timestamp with time zone, "updated_at" timestamp with time zone, CONSTRAINT "fk-job_posts-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [24/50] match_consensus
CREATE TABLE "match_consensus" ( "id" text NOT NULL PRIMARY KEY, "fleet_id" text NOT NULL, "initiator_id" text NOT NULL, "candidate_id" text NOT NULL, "match_score" integer NOT NULL, "status" text, "reason" text, "created_at" timestamp with time zone, "verified_at" timestamp with time zone, "expires_at" timestamp with time zone NOT NULL, CONSTRAINT "fk-match_consensus-candidate_id" FOREIGN KEY ("candidate_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION, CONSTRAINT "fk-match_consensus-initiator_id" FOREIGN KEY ("initiator_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [25/50] match_reports
CREATE TABLE "match_reports" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "resume_id" text, "resume_snapshot" text, "title" text, "status" text, "summary" text, "metadata" text, "created_at" timestamp with time zone, "updated_at" timestamp with time zone, CONSTRAINT "fk-match_reports-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [26/50] mission_completions
CREATE TABLE "mission_completions" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "mission_id" text NOT NULL, "xp_reward" integer, "completed_at" timestamp with time zone, CONSTRAINT "fk-mission_completions-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [27/50] narrative_sessions
CREATE TABLE "narrative_sessions" ( "id" text NOT NULL PRIMARY KEY, "user_id" text, "domain" text NOT NULL, "status" text, "duration_seconds" double precision, "started_at" timestamp with time zone, "ended_at" timestamp with time zone, CONSTRAINT "fk-narrative_sessions-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [28/50] navigator_memories
CREATE TABLE "navigator_memories" ( "id" bigserial NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "memory_level" text NOT NULL, "content" text NOT NULL, "context" text, "domain" text, "choice" text, "importance" double precision, "referenced_count" integer, "session_id" text, "created_at" timestamp with time zone, CONSTRAINT "fk-navigator_memories-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [29/50] orders
CREATE TABLE "orders" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "plan_id" text NOT NULL, "tier" text NOT NULL, "amount" double precision NOT NULL, "currency" text NOT NULL, "out_trade_no" text NOT NULL UNIQUE, "transaction_id" text, "status" text, "prepay_id" text, "qr_code_url" text, "metadata_json" text, "created_at" timestamp with time zone, "paid_at" timestamp with time zone, CONSTRAINT "fk-orders-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [30/50] quiz_sessions
CREATE TABLE "quiz_sessions" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "questions_json" text NOT NULL, "current_index" integer NOT NULL, "answers_json" text NOT NULL, "total_score" integer NOT NULL, "total_questions" integer NOT NULL, "correct_count" integer NOT NULL, "status" text NOT NULL, "result_type" text, "insights_json" text, "created_at" timestamp with time zone NOT NULL, "completed_at" timestamp with time zone, CONSTRAINT "fk-quiz_sessions-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [31/50] quota_records
CREATE TABLE "quota_records" ( "id" bigserial NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "resource" text NOT NULL, "date" text NOT NULL, "used" integer, "daily_limit" integer NOT NULL, CONSTRAINT "fk-quota_records-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [32/50] sales_inquiries
CREATE TABLE "sales_inquiries" ( "id" text NOT NULL PRIMARY KEY, "user_id" text, "company_name" text NOT NULL, "contact_name" text NOT NULL, "contact_email" text NOT NULL, "contact_phone" text, "scale" text, "message" text, "status" text, "created_at" timestamp with time zone, CONSTRAINT "fk-sales_inquiries-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [33/50] share_codes
CREATE TABLE "share_codes" ( "id" text NOT NULL PRIMARY KEY, "code" text NOT NULL UNIQUE, "owner_id" text NOT NULL, "scope" text NOT NULL, "max_access_count" integer NOT NULL, "access_count" integer NOT NULL, "expires_at" timestamp with time zone NOT NULL, "status" text NOT NULL, "created_at" timestamp with time zone, CONSTRAINT "fk-share_codes-owner_id" FOREIGN KEY ("owner_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [34/50] subscriptions
CREATE TABLE "subscriptions" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL UNIQUE, "tier" text NOT NULL, "plan_id" text NOT NULL, "status" text, "started_at" timestamp with time zone, "expires_at" timestamp with time zone NOT NULL, "auto_renew" integer, "cancel_at_period_end" integer, "created_at" timestamp with time zone, "updated_at" timestamp with time zone, CONSTRAINT "fk-subscriptions-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [35/50] timeline_events
CREATE TABLE "timeline_events" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "event_kind" text NOT NULL, "occurred_at" timestamp with time zone NOT NULL, "recorded_at" timestamp with time zone NOT NULL, "source_system" text NOT NULL, "source_ref" text, "title" text, "summary" text, "payload_json" text, "signal_quality" text NOT NULL, "confidence" double precision, "weight_role" text NOT NULL, "visibility" text NOT NULL, "consent_scope" text, "status" text NOT NULL, "superseded_by" text, "created_at" timestamp with time zone, "updated_at" timestamp with time zone, CONSTRAINT "fk-timeline_events-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [36/50] trust_attestations
CREATE TABLE "trust_attestations" ( "id" text NOT NULL PRIMARY KEY, "candidate_id" text NOT NULL, "claim_type" text NOT NULL, "claim_statement" text NOT NULL, "evidence_type" text NOT NULL, "evidence_refs" text, "verification_status" text, "confidence_score" double precision, "generated_by" text, "signature" text, "expires_at" timestamp with time zone, "created_at" timestamp with time zone, "updated_at" timestamp with time zone, CONSTRAINT "fk-trust_attestations-candidate_id" FOREIGN KEY ("candidate_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [37/50] trust_memories
CREATE TABLE "trust_memories" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "session_type" text NOT NULL, "session_id" text NOT NULL, "memory_content" text NOT NULL, "memory_level" integer, "created_at" timestamp with time zone, CONSTRAINT "fk-trust_memories-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [38/50] usage_logs
CREATE TABLE "usage_logs" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "endpoint" text NOT NULL, "tokens_used" integer, "created_at" timestamp with time zone, CONSTRAINT "fk-usage_logs-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [39/50] user_identities
CREATE TABLE "user_identities" ( "id" text NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "provider" text NOT NULL, "provider_uid" text NOT NULL, "provider_email" text, "provider_name" text, "metadata_json" text, "created_at" timestamp with time zone, CONSTRAINT "fk-user_identities-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [40/50] value_imprints
CREATE TABLE "value_imprints" ( "user_id" text NOT NULL PRIMARY KEY, "survival" integer NOT NULL, "freedom" integer NOT NULL, "belonging" integer NOT NULL, "updated_at" timestamp with time zone, CONSTRAINT "fk-value_imprints-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [41/50] verification_audit_log
CREATE TABLE "verification_audit_log" ( "id" text NOT NULL PRIMARY KEY, "share_code" text NOT NULL, "owner_id" text NOT NULL, "verifier_info" text, "attestation_ids" text NOT NULL, "result" text NOT NULL, "created_at" timestamp with time zone, CONSTRAINT "fk-verification_audit_log-owner_id" FOREIGN KEY ("owner_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [42/50] act1_sessions
CREATE TABLE "act1_sessions" ( "session_id" text NOT NULL PRIMARY KEY, "state_json" text NOT NULL, "updated_at" timestamp with time zone, CONSTRAINT "fk-act1_sessions-session_id" FOREIGN KEY ("session_id") REFERENCES "narrative_sessions" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [43/50] boost_consumptions
CREATE TABLE "boost_consumptions" ( "id" bigserial NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "boost_pack_id" integer NOT NULL, "resource" text NOT NULL, "credits_consumed" integer, "consumed_at" timestamp with time zone, CONSTRAINT "fk-boost_consumptions-boost_pack_id" FOREIGN KEY ("boost_pack_id") REFERENCES "boost_packs" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION, CONSTRAINT "fk-boost_consumptions-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [44/50] challenge_entries
CREATE TABLE "challenge_entries" ( "id" bigserial NOT NULL PRIMARY KEY, "round_id" integer NOT NULL, "user_id" text NOT NULL, "translation" text NOT NULL, "note" text NOT NULL, "license_accepted" integer NOT NULL, "vote_count" integer NOT NULL, "created_at" timestamp with time zone, "updated_at" timestamp with time zone, CONSTRAINT "fk-challenge_entries-round_id" FOREIGN KEY ("round_id") REFERENCES "challenge_rounds" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION, CONSTRAINT "fk-challenge_entries-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [45/50] domain_visits
CREATE TABLE "domain_visits" ( "id" bigserial NOT NULL PRIMARY KEY, "user_id" text NOT NULL, "session_id" text NOT NULL, "domain" text NOT NULL, "previous_domain" text, "interaction_level" text, "echo_triggered" integer NOT NULL, "entered_at" timestamp with time zone, CONSTRAINT "fk-domain_visits-session_id" FOREIGN KEY ("session_id") REFERENCES "narrative_sessions" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION, CONSTRAINT "fk-domain_visits-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [46/50] fleet_members
CREATE TABLE "fleet_members" ( "id" text NOT NULL PRIMARY KEY, "fleet_id" text NOT NULL, "user_id" text NOT NULL, "joined_at" timestamp with time zone, CONSTRAINT "fk-fleet_members-fleet_id" FOREIGN KEY ("fleet_id") REFERENCES "fleets" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION, CONSTRAINT "fk-fleet_members-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [47/50] match_report_items
CREATE TABLE "match_report_items" ( "id" text NOT NULL PRIMARY KEY, "report_id" text NOT NULL, "job_title" text NOT NULL, "company_name" text NOT NULL, "location" text, "salary_range" text, "jd_snapshot" text, "overall_score" double precision, "background_match" double precision, "skills_overlap" double precision, "experience_relevance" double precision, "seniority" double precision, "language_requirement" double precision, "company_score" double precision, "salary_match" double precision, "location_match" double precision, "culture_workload_match" double precision, "match_reason" text, "matched_skills" text, "missing_skills" text, "fit_bullets" text, "gap_analysis" text, "improvement_plan" text, "credit_snapshot" text, "rank_order" integer, "created_at" timestamp with time zone, CONSTRAINT "fk-match_report_items-report_id" FOREIGN KEY ("report_id") REFERENCES "match_reports" ("id") ON DELETE CASCADE ON UPDATE NO ACTION );

-- [48/50] narrative_events
CREATE TABLE "narrative_events" ( "id" bigserial NOT NULL PRIMARY KEY, "session_id" text NOT NULL, "event_type" text NOT NULL, "domain" text, "choice" text, "navigator_line" text, "metadata_json" text, "created_at" timestamp with time zone, CONSTRAINT "fk-narrative_events-session_id" FOREIGN KEY ("session_id") REFERENCES "narrative_sessions" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [49/50] narrative_feedback
CREATE TABLE "narrative_feedback" ( "id" bigserial NOT NULL PRIMARY KEY, "session_id" text NOT NULL UNIQUE, "resonated" integer NOT NULL, "navigator_quote" text, "would_replay" integer, "shared" integer NOT NULL, "share_channel" text, "open_feedback" text, "created_at" timestamp with time zone, CONSTRAINT "fk-narrative_feedback-session_id" FOREIGN KEY ("session_id") REFERENCES "narrative_sessions" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );

-- [50/50] report_sharing
CREATE TABLE "report_sharing" ( "id" text NOT NULL PRIMARY KEY, "report_id" text NOT NULL, "user_id" text NOT NULL, "shared_with_type" text NOT NULL, "shared_with_id" text, "shared_dimensions" text NOT NULL, "purpose" text, "status" text, "granted_at" timestamp with time zone, "revoked_at" timestamp with time zone, "expires_at" timestamp with time zone, "created_at" timestamp with time zone, "updated_at" timestamp with time zone, CONSTRAINT "fk-report_sharing-report_id" FOREIGN KEY ("report_id") REFERENCES "match_reports" ("id") ON DELETE CASCADE ON UPDATE NO ACTION, CONSTRAINT "fk-report_sharing-user_id" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON DELETE NO ACTION ON UPDATE NO ACTION );


-- ============================================================
-- 第二部分: Apple IAP 两表（schema_v0.1.sql 原文，v0.3 并入）
-- ============================================================
CREATE TABLE IF NOT EXISTS iap_subscriptions (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Apple 交易锚点（对 Google Play 即 order_id，对华为即 purchaseToken）
    original_transaction_id TEXT NOT NULL UNIQUE,
    user_id                 TEXT NOT NULL,
    -- 购买前由服务端生成并下发给客户端 StoreKit 的 appAccountToken，
    -- 用于"谁买的 = 谁享受"，防 A 账号购买 B 账号薅权益
    app_account_token       UUID,
    provider                TEXT NOT NULL DEFAULT 'apple_iap',   -- apple_iap | google_play | huawei_iap (预留)
    environment             TEXT NOT NULL CHECK (environment IN ('Sandbox', 'Production')),
    product_id              TEXT NOT NULL,          -- supporter_monthly | pro_monthly
    tier                    TEXT NOT NULL,          -- supporter | pro
    status                  TEXT NOT NULL CHECK (status IN ('active', 'grace', 'expired', 'revoked')),
    expires_at              TIMESTAMPTZ NOT NULL,
    grace_period_expires_at TIMESTAMPTZ,            -- 宽限期截止（DID_FAIL_TO_RENEW/GRACE_PERIOD 写入）
    auto_renew_status       BOOLEAN NOT NULL DEFAULT TRUE,
    in_app_ownership_type   TEXT NOT NULL DEFAULT 'PURCHASED'
                            CHECK (in_app_ownership_type IN ('PURCHASED', 'FAMILY_SHARED')),
    revocation_date         TIMESTAMPTZ,            -- REFUND / REVOKE 时间
    revocation_reason       TEXT,                   -- Apple revocationReason 原值留痕
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_iap_subs_user    ON iap_subscriptions(user_id);
CREATE INDEX IF NOT EXISTS idx_iap_subs_env     ON iap_subscriptions(environment, status);
CREATE INDEX IF NOT EXISTS idx_iap_subs_expires ON iap_subscriptions(expires_at);

ALTER TABLE iap_subscriptions ENABLE ROW LEVEL SECURITY;

-- ------------------------------------------------------------
-- apple_iap_transactions — 幂等核心（Apple 重试风暴的挡板）
--   Apple 对未收到 200 的通知会指数退避重试数天；同一 original_transaction_id
--   的续期也会产生多笔 transaction。写入一律 ON CONFLICT DO NOTHING，
--   以 transaction_id / notification_uuid 唯一约束做天然去重。
-- ------------------------------------------------------------
CREATE TABLE IF NOT EXISTS apple_iap_transactions (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    transaction_id          TEXT NOT NULL UNIQUE,   -- 每笔交易唯一（含每次续期）
    original_transaction_id TEXT NOT NULL,
    notification_uuid       TEXT UNIQUE,            -- 同一通知重试去重键（TEST 通知可为 NULL）
    notification_type       TEXT,                   -- DID_RENEW / REFUND / REVOKE / ...
    subtype                 TEXT,                   -- GRACE_PERIOD / AUTO_RENEW_DISABLED / ...
    environment             TEXT NOT NULL CHECK (environment IN ('Sandbox', 'Production')),
    raw_payload             JSONB NOT NULL,         -- 原始事件，出问题可回放重算
    processed_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_iap_tx_orig ON apple_iap_transactions(original_transaction_id);
CREATE INDEX IF NOT EXISTS idx_iap_tx_type ON apple_iap_transactions(notification_type, subtype);

ALTER TABLE apple_iap_transactions ENABLE ROW LEVEL SECURITY;
