-- =============================================================================
-- genzltd 单源合并 SQL 草案 · Round 8
-- -----------------------------------------------------------------------------
-- 状态：DRAFT（草案）。
--   * 本文件在【本地 staging】上跑通过：public = genzltd 10-22 转储还原，
--     sg = SG SQLite 快照（looma-sg-20261022.db）导入的同名 schema。
--   * 严禁直接对腾讯云 1.14.202.161 的 genzltd 生产库执行。
--     生产执行前必须先完成「前置 0：把 SG 数据导入目标库的 sg schema」，
--     且必须先执行「前置 1：再打一份 pg_dump 备份」。
--   * 全程单事务；任何闸门 FAIL 即 rollback。外键为 DEFERRABLE，
--     末尾 set constraints all immediate 会把孤儿引用在 COMMIT 前暴露出来。
--
-- 拍板依据（Win 2026-10-22）：
--   1) users：password_hash / created_at 取 PG；updated_at 取两侧较晚；
--      zervi@genz.ltd 用 SG 的 UUID + SG 口令新插入，2 条 orders 随行插入；
--      被丢弃的 4 个 SG UUID 中 beta_supporter 无外键直接丢，其余 140 行
--      在插入时改写到对应 PG UUID；PG 已有外键不动。
--   2) enterprises：剔除 SG 侧 12 条 E2E/调试残留；「闭环测试企业」按 name
--      只留一条；其余并集。
--   3) 25 张主键 md5 相同的表：不插入、不覆盖。其中 22 张双空不做字段比对；
--      仅 columns / driver_profiles / poems 三张在执行前补全字段校验，
--      全行 md5 一致就跳过，poems 不一致就停。
-- =============================================================================

\set ON_ERROR_STOP on
\pset pager off
\timing off

-- poems 闸门严格度：strict（默认，不一致即停） / allow_created_at_skew（仅允许
-- 差异全部来自 created_at 导入时间戳）。生产首次执行必须保持 strict。
\if :{?poems_gate}
\else
  \set poems_gate strict
\endif
-- 把闸门严格度塞进 GUC，供下面的 DO 块读取（psql 变量在 dollar-quote 内不插值）
select set_config('merge.poems_gate', :'poems_gate', false) as poems_gate;

\echo '==================================================================='
\echo ' 闸门 G0 · 前置环境校验（sg schema 存在 + 关键表行数与验收值一致）'
\echo '==================================================================='

do $$
declare
  n int; bad text := '';
begin
  if not exists (select 1 from information_schema.schemata where schema_name = 'sg') then
    raise exception 'G0 FAIL: sg schema 不存在 —— 先把 SG 快照导入目标库的 sg schema（见文档前置 0）';
  end if;

  select count(*) into n from sg.users;          if n <> 11    then bad := bad || format(' sg.users=%s(期望11)', n); end if;
  select count(*) into n from sg.poems;          if n <> 58059 then bad := bad || format(' sg.poems=%s(期望58059)', n); end if;
  select count(*) into n from sg.enterprises;    if n <> 21    then bad := bad || format(' sg.enterprises=%s(期望21)', n); end if;
  select count(*) into n from sg.documents;      if n <> 39    then bad := bad || format(' sg.documents=%s(期望39)', n); end if;
  select count(*) into n from sg.orders;         if n <> 2     then bad := bad || format(' sg.orders=%s(期望2)', n); end if;
  select count(*) into n from public.users;      if n <> 10    then bad := bad || format(' public.users=%s(期望10)', n); end if;
  select count(*) into n from public.poems;      if n <> 58059 then bad := bad || format(' public.poems=%s(期望58059)', n); end if;
  select count(*) into n from public.enterprises;if n <> 13    then bad := bad || format(' public.enterprises=%s(期望13)', n); end if;

  if bad <> '' then
    raise exception 'G0 FAIL: 行数与 10-22 验收值不符 ->%', bad;
  end if;
  raise notice 'G0 OK: sg/public 行数与验收值一致';
end $$;

\echo '==================================================================='
\echo ' 闸门 G1 · 25 张 md5 相同表中 3 张非空表的全字段校验'
\echo '==================================================================='

do $$
declare
  t text; pgm text; sgm text; bad boolean := false;
begin
  foreach t in array array['columns','driver_profiles','poems'] loop
    execute format(
      'select md5(coalesce(string_agg(r,'','' order by r),'''')) from '
      '(select %s as r from public.%I) x',
      (select string_agg(format('coalesce(%I::text,'''')', c.column_name), ' || ''/'' || ' order by c.ordinal_position)
         from information_schema.columns c
        where c.table_schema='public' and c.table_name=t), t) into pgm;
    execute format(
      'select md5(coalesce(string_agg(r,'','' order by r),'''')) from '
      '(select %s as r from sg.%I) x',
      (select string_agg(format('coalesce(%I::text,'''')', c.column_name), ' || ''/'' || ' order by c.ordinal_position)
         from information_schema.columns c
        where c.table_schema='public' and c.table_name=t), t) into sgm;
    if pgm = sgm then
      raise notice 'G1 % : 全行 md5 一致 -> 跳过（不插入、不覆盖）', t;
    else
      bad := true;
      raise warning 'G1 % : 全行 md5 不一致 PG=% SG=%', t, pgm, sgm;
    end if;
  end loop;

  if bad then
    -- 判定差异是否 100% 来自 created_at（导入时间戳漂移），而不是业务字段
    if (select count(*) from public.poems p join sg.poems s using (id)
          where p.title is distinct from s.title or p.author is distinct from s.author
             or p.dynasty is distinct from s.dynasty or p.theme is distinct from s.theme
             or p.content is distinct from s.content or coalesce(p.tags,'') is distinct from coalesce(s.tags,'')
             or p.source is distinct from s.source
             or coalesce(p.publisher_id,'') is distinct from coalesce(s.publisher_id,'')
             or coalesce(p.column_id,'') is distinct from coalesce(s.column_id,'')) = 0
       and current_setting('merge.poems_gate') = 'allow_created_at_skew' then
      raise notice 'G1 放宽：三张表的差异被确认 100%% 来自 created_at（导入时刻），业务字段零差异；按 allow_created_at_skew 继续';
    else
      raise exception 'G1 STOP: 全行 md5 不一致（poems 按拍板必须停，禁止用任一侧覆盖）。如需豁免请显式 -v poems_gate=allow_created_at_skew，且必须先人工确认差异仅来自 created_at';
    end if;
  end if;
end $$;

begin;   -- ===================== 合并事务开始（可回滚） =====================

\echo '==================================================================='
\echo ' 步骤 0 · 建立 SG UUID -> PG UUID 重映射（canonical 身份）'
\echo '==================================================================='

create temp table merge_map as
select s.id as sg_id, u.id as pg_id, s.email
from sg.users s
join public.users u on u.email = s.email and u.id <> s.id;

do $$
declare n int;
begin
  select count(*) into n from merge_map;
  if n <> 4 then
    raise exception 'G2 FAIL: 重映射行数 = %（期望 4：beta_free / beta_supporter / beta_pro / beta_admin）', n;
  end if;
  raise notice 'G2 OK: 重映射 4 行 -> %', (select string_agg(email, ', ' order by email) from merge_map);
end $$;

-- 140 行外键回写计数（插入前基线，插入后复核）
create temp table merge_fk_baseline as
select 'consents' tbl, 'user_id' col from generate_series(1,1)
union all select 'fleet_members','user_id'
union all select 'fleets','captain_id'
union all select 'game_profiles','user_id'
union all select 'invite_codes','created_by'
union all select 'mission_completions','user_id'
union all select 'quota_records','user_id'
union all select 'timeline_events','user_id'
union all select 'trust_attestations','candidate_id'
union all select 'trust_memories','user_id'
union all select 'applications','seeker_user_id'
union all select 'enterprise_users','user_id'
union all select 'match_reports','user_id'
union all select 'share_codes','owner_id'
union all select 'verification_audit_log','owner_id';

\echo '==================================================================='
\echo ' 步骤 1 · users'
\echo '==================================================================='

-- 基线：合并前 PG 侧 users 的口令与创建时间（供 G3 复核"取 PG"确实没被改写）
create temp table merge_users_before as
select id, email, password_hash, created_at from public.users;

-- 1a. 6 个共同 UUID：password_hash / created_at 保持 PG 侧（不动）；
--     updated_at 取两侧较晚值。
update public.users u
   set updated_at = s.updated_at
  from sg.users s
 where s.id = u.id
   and s.updated_at > u.updated_at;

-- 1a-可选（默认关闭）：把 4 个 beta 的 updated_at 也取两侧较晚值。
--   实测 SG = 2026-09-03 16:01:41 > PG = 2026-09-02 03:56:16。
--   Win 拍板字面只覆盖"六个共同用户"（共同 UUID 的那 6 人），故默认不执行；
--   若要一并更新，解开下面这段。
-- update public.users u
--    set updated_at = s.updated_at
--   from sg.users s
--   join merge_map m on m.pg_id = u.id
--  where s.id = m.sg_id
--    and s.updated_at > u.updated_at;

-- 1b. SG 独有 zervi@genz.ltd：用 SG 的 UUID + SG 的口令整行新插入。
--     email / supabase_uid / wechat_openid 三个唯一键均已在预检确认不冲突。
insert into public.users
  (id, email, password_hash, wechat_openid, name, tier, role,
   is_early_adopter, created_at, updated_at, email_hash, supabase_uid)
select s.id, s.email, s.password_hash, s.wechat_openid, s.name, s.tier, s.role,
       s.is_early_adopter, s.created_at, s.updated_at, s.email_hash, s.supabase_uid
  from sg.users s
 where s.email = 'zervi@genz.ltd'
   and not exists (select 1 from public.users u where u.email = s.email);

-- 1c. 4 个被丢弃的 SG UUID（beta_*）不插入。beta_supporter(9d171561) 实测 0 外键
--     引用，属"直接丢"；其余 140 行在下面各表插入时按 merge_map 改写。
--     PG 侧已有外键一行不动。

\echo '==================================================================='
\echo ' 步骤 2 · orders（随 zervi 插入，否则订单成孤儿）'
\echo '==================================================================='

insert into public.orders
  (id, user_id, plan_id, tier, amount, currency, out_trade_no, transaction_id,
   status, prepay_id, qr_code_url, metadata_json, created_at, paid_at)
select s.id, coalesce(m.pg_id, s.user_id), s.plan_id, s.tier, s.amount, s.currency,
       s.out_trade_no, s.transaction_id, s.status, s.prepay_id, s.qr_code_url,
       s.metadata_json, s.created_at, s.paid_at
  from sg.orders s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.orders o where o.id = s.id)
   and not exists (select 1 from public.orders o where o.out_trade_no = s.out_trade_no);

\echo '==================================================================='
\echo ' 步骤 3 · 14 张真·互斥表 + 6 张 SG 独有表（逐表规则）'
\echo '==================================================================='

-- 3.1 applications  9 -> 去重 1（SG j1/resume 23 与 PG 已存在，保留 PG 现有行）
insert into public.applications
  (id, seeker_user_id, resume_id, job_id, enterprise_id, status, metadata,
   created_at, updated_at, withdrawn_at)
select s.id, coalesce(m.pg_id, s.seeker_user_id), s.resume_id, s.job_id,
       s.enterprise_id, s.status, s.metadata, s.created_at, s.updated_at, s.withdrawn_at
  from sg.applications s
  left join merge_map m on m.sg_id = s.seeker_user_id
 where not exists (select 1 from public.applications a where a.id = s.id)
   and not exists (select 1 from public.applications a
                    where a.seeker_user_id = coalesce(m.pg_id, s.seeker_user_id)
                      and a.job_id = s.job_id and a.resume_id = s.resume_id);

-- 3.2 audit_logs  225 全并集（append-only，(actor,action,resource_id) 交集 0）
insert into public.audit_logs
  (id, actor, action, resource_type, resource_id, consent_id, metadata, ip, user_agent, created_at)
select s.id, s.actor, s.action, s.resource_type, s.resource_id, s.consent_id,
       s.metadata, s.ip, s.user_agent, s.created_at
  from sg.audit_logs s
 where not exists (select 1 from public.audit_logs a where a.id = s.id);

-- 3.3 consents  27 -> 跳过与 PG (user,scope) 重叠的 9 行，净 32
insert into public.consents
  (id, user_id, scope, purpose, status, ip, user_agent, granted_at, revoked_at,
   created_at, updated_at)
select s.id, coalesce(m.pg_id, s.user_id), s.scope, s.purpose, s.status, s.ip,
       s.user_agent, s.granted_at, s.revoked_at, s.created_at, s.updated_at
  from sg.consents s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.consents c where c.id = s.id)
   and not exists (select 1 from public.consents c
                    where c.user_id = coalesce(m.pg_id, s.user_id) and c.scope = s.scope);

-- 3.4 enterprise_users  2 全并集（uq(enterprise_id,user_id) 预检 0 冲突）
insert into public.enterprise_users (id, enterprise_id, user_id, role, created_at)
select s.id, s.enterprise_id, coalesce(m.pg_id, s.user_id), s.role, s.created_at
  from sg.enterprise_users s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.enterprise_users e where e.id = s.id)
   and not exists (select 1 from public.enterprise_users e
                    where e.enterprise_id = s.enterprise_id
                      and e.user_id = coalesce(m.pg_id, s.user_id));

-- 3.5 fleet_members  4 全并集（uq(fleet_id,user_id) 预检 0 冲突）
insert into public.fleet_members (id, fleet_id, user_id, joined_at)
select s.id, s.fleet_id, coalesce(m.pg_id, s.user_id), s.joined_at
  from sg.fleet_members s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.fleet_members f where f.id = s.id)
   and not exists (select 1 from public.fleet_members f
                    where f.fleet_id = s.fleet_id and f.user_id = coalesce(m.pg_id, s.user_id));

-- 3.6 fleets  4 全并集（uq(invite_code) 预检 0 冲突；captain_id 重映射）
insert into public.fleets (id, name, captain_id, description, created_at, invite_code)
select s.id, s.name, coalesce(m.pg_id, s.captain_id), s.description, s.created_at, s.invite_code
  from sg.fleets s
  left join merge_map m on m.sg_id = s.captain_id
 where not exists (select 1 from public.fleets f where f.id = s.id);

-- 3.7 game_profiles  按 user_id 取 updated_at 最新：
--     3eeff409 SG(07-23) > PG(07-07) -> 用 SG 覆盖；fcf2c1fc PG(08-17) > SG(07-25) -> 不动
update public.game_profiles g
   set personality_type = s.personality_type, personality_detail = s.personality_detail,
       xp = s.xp, level = s.level, updated_at = s.updated_at, identity = s.identity
  from sg.game_profiles s
  join merge_map m on m.sg_id = s.user_id
 where g.user_id = m.pg_id
   and s.updated_at > g.updated_at;

insert into public.game_profiles
  (id, user_id, personality_type, personality_detail, xp, level, updated_at, identity)
select s.id, coalesce(m.pg_id, s.user_id), s.personality_type, s.personality_detail,
       s.xp, s.level, s.updated_at, s.identity
  from sg.game_profiles s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.game_profiles g
                    where g.user_id = coalesce(m.pg_id, s.user_id));

-- 3.8 invite_codes  3 全并集（uq(code) 预检 0 冲突；created_by/used_by 重映射）
insert into public.invite_codes
  (id, code, created_by, used_by, tier_grant, expires_at, used_at, created_at)
select s.id, s.code,
       coalesce((select m.pg_id from merge_map m where m.sg_id = s.created_by), s.created_by),
       coalesce((select m.pg_id from merge_map m where m.sg_id = s.used_by), s.used_by),
       s.tier_grant, s.expires_at, s.used_at, s.created_at
  from sg.invite_codes s
 where not exists (select 1 from public.invite_codes i where i.id = s.id)
   and not exists (select 1 from public.invite_codes i where i.code = s.code);

-- 3.9 match_reports  11 全并集（PG 侧 0 行；report_id 无冲突）
insert into public.match_reports
  (id, user_id, resume_id, resume_snapshot, title, status, summary, metadata,
   created_at, updated_at)
select s.id, coalesce(m.pg_id, s.user_id), s.resume_id, s.resume_snapshot, s.title,
       s.status, s.summary, s.metadata, s.created_at, s.updated_at
  from sg.match_reports s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.match_reports r where r.id = s.id);

-- 3.10 match_report_items  11 全并集（report_id 引用上面的 SG match_reports）
insert into public.match_report_items
  (id, report_id, job_title, company_name, location, salary_range, jd_snapshot,
   overall_score, background_match, skills_overlap, experience_relevance, seniority,
   language_requirement, company_score, salary_match, location_match,
   culture_workload_match, match_reason, matched_skills, missing_skills, fit_bullets,
   gap_analysis, improvement_plan, credit_snapshot, rank_order, created_at)
select s.id, s.report_id, s.job_title, s.company_name, s.location, s.salary_range,
       s.jd_snapshot, s.overall_score, s.background_match, s.skills_overlap,
       s.experience_relevance, s.seniority, s.language_requirement, s.company_score,
       s.salary_match, s.location_match, s.culture_workload_match, s.match_reason,
       s.matched_skills, s.missing_skills, s.fit_bullets, s.gap_analysis,
       s.improvement_plan, s.credit_snapshot, s.rank_order, s.created_at
  from sg.match_report_items s
 where not exists (select 1 from public.match_report_items i where i.id = s.id)
   and exists (select 1 from public.match_reports r where r.id = s.report_id);

-- 3.11 mission_completions  8 -> 跳过与 PG (user,mission) 重叠的 1 行，净 12
insert into public.mission_completions (id, user_id, mission_id, xp_reward, completed_at)
select s.id, coalesce(m.pg_id, s.user_id), s.mission_id, s.xp_reward, s.completed_at
  from sg.mission_completions s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.mission_completions c
                    where c.user_id = coalesce(m.pg_id, s.user_id)
                      and c.mission_id = s.mission_id);

-- 3.12 quota_records  29 全并集（uq(user,resource,date) 预检 0 冲突；id 用 identity）
insert into public.quota_records (user_id, resource, date, used, daily_limit)
select coalesce(m.pg_id, s.user_id), s.resource, s.date, s.used, s.daily_limit
  from sg.quota_records s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.quota_records q
                    where q.user_id = coalesce(m.pg_id, s.user_id)
                      and q.resource = s.resource and q.date = s.date);

-- 3.13 share_codes  4 全并集（uq(code) 预检 0 冲突；owner_id 重映射）
insert into public.share_codes
  (id, code, owner_id, scope, max_access_count, access_count, expires_at, status, created_at)
select s.id, s.code, coalesce(m.pg_id, s.owner_id), s.scope, s.max_access_count,
       s.access_count, s.expires_at, s.status, s.created_at
  from sg.share_codes s
  left join merge_map m on m.sg_id = s.owner_id
 where not exists (select 1 from public.share_codes c where c.id = s.id)
   and not exists (select 1 from public.share_codes c where c.code = s.code);

-- 3.14 timeline_events  68 并集
--     去重口径（草案默认，保守）：(user_id, title, occurred_at) —— 只删同刻同名的真重复。
--     ⚠ 若改用 Round 7 文档的「仅按 (user,title)」口径，会把 beta_admin 的
--       「对话交互」SG 12 + PG 5 = 17 行压成 1 行（丢 16 条真实对话记录）。
--       两种口径的结果行数见文档；默认取保守口径，净 87。
insert into public.timeline_events
  (id, user_id, event_kind, occurred_at, recorded_at, source_system, source_ref,
   title, summary, payload_json, signal_quality, confidence, weight_role, visibility,
   consent_scope, status, superseded_by, created_at, updated_at)
select s.id, coalesce(m.pg_id, s.user_id), s.event_kind, s.occurred_at, s.recorded_at,
       s.source_system, s.source_ref, s.title, s.summary, s.payload_json, s.signal_quality,
       s.confidence, s.weight_role, s.visibility, s.consent_scope, s.status,
       s.superseded_by, s.created_at, s.updated_at
  from sg.timeline_events s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.timeline_events e
                    where e.user_id = coalesce(m.pg_id, s.user_id)
                      and e.title = s.title and e.occurred_at = s.occurred_at);

-- 3.15 trust_attestations  8 -> 跳过与 PG (candidate,claim_type) 重叠的 3 行
--       （2 组置信度相同保留 PG 现有；identity 组 PG 0.93 > SG 0.45 取 PG）
insert into public.trust_attestations
  (id, candidate_id, claim_type, claim_statement, evidence_type, evidence_refs,
   verification_status, confidence_score, generated_by, created_at, updated_at,
   signature, expires_at)
select s.id, coalesce(m.pg_id, s.candidate_id), s.claim_type, s.claim_statement,
       s.evidence_type, s.evidence_refs, s.verification_status, s.confidence_score,
       s.generated_by, s.created_at, s.updated_at, s.signature, s.expires_at
  from sg.trust_attestations s
  left join merge_map m on m.sg_id = s.candidate_id
 where not exists (select 1 from public.trust_attestations a
                    where a.candidate_id = coalesce(m.pg_id, s.candidate_id)
                      and a.claim_type = s.claim_type);

-- 3.16 trust_memories  29 全并集（实测 (user,session_type,content) 重叠 0）
insert into public.trust_memories
  (id, user_id, session_type, session_id, memory_content, memory_level, created_at)
select s.id, coalesce(m.pg_id, s.user_id), s.session_type, s.session_id,
       s.memory_content, s.memory_level, s.created_at
  from sg.trust_memories s
  left join merge_map m on m.sg_id = s.user_id
 where not exists (select 1 from public.trust_memories t where t.id = s.id);

-- 3.17 verification_audit_log  6 全并集（owner_id 重映射）
insert into public.verification_audit_log
  (id, share_code, owner_id, verifier_info, attestation_ids, result, created_at)
select s.id, s.share_code, coalesce(m.pg_id, s.owner_id), s.verifier_info,
       s.attestation_ids, s.result, s.created_at
  from sg.verification_audit_log s
  left join merge_map m on m.sg_id = s.owner_id
 where not exists (select 1 from public.verification_audit_log v where v.id = s.id);

\echo '==================================================================='
\echo ' 步骤 4 · 3 张自增主键「伪交集」表：重编主键后并集（id 交给 identity）'
\echo '==================================================================='

-- 4.1 documents  39 行全插（uq(file_path) 预检 0 冲突；新 id 由 documents_id_seq 生成）
insert into public.documents
  (title, file_path, doc_type, file_size, metadata, status, chunk_count, created_at, processed_at)
select s.title, s.file_path, s.doc_type, s.file_size, s.metadata, s.status,
       s.chunk_count, s.created_at, s.processed_at
  from sg.documents s
 where not exists (select 1 from public.documents d where d.file_path = s.file_path);

-- 4.2 product_events  25 行全插（user_id 重映射；新 id 由序列生成）
insert into public.product_events
  (event_name, user_id, session_id, platform, share_code, source, success,
   properties_json, created_at)
select s.event_name, coalesce(m.pg_id, s.user_id), s.session_id, s.platform,
       s.share_code, s.source, s.success, s.properties_json, s.created_at
  from sg.product_events s
  left join merge_map m on m.sg_id = s.user_id;

-- 4.3 query_logs  69 行全插（新 id 由序列生成）
insert into public.query_logs
  (query_text, provider, response_time_ms, chunk_count, rating, intent_label,
   user_id, created_at)
select s.query_text, s.provider, s.response_time_ms, s.chunk_count, s.rating,
       s.intent_label, coalesce(m.pg_id, s.user_id), s.created_at
  from sg.query_logs s
  left join merge_map m on m.sg_id = s.user_id;

-- 4.4 challenge_rounds  6 行：按 week_key 并集
--     ⚠ 实测 2026-W36 两侧各有一条（PG《輞川集二十首 斤竹嶺》 / SG《華嚴寺望樊川》），
--       uq_challenge_rounds_week_key 会撞。草案默认保留 PG 现有行、跳过 SG 该行
--       （净 8 而非 9）。如需改成保留 SG，把下面 where 里的 not exists 条件反选。
insert into public.challenge_rounds
  (week_key, poem_id, title, status, starts_at, ends_at, created_at)
select s.week_key, s.poem_id, s.title, s.status, s.starts_at, s.ends_at, s.created_at
  from sg.challenge_rounds s
 where not exists (select 1 from public.challenge_rounds c where c.week_key = s.week_key);

\echo '==================================================================='
\echo ' 步骤 5 · enterprises：剔除测试残留 + 闭环测试企业按 name 留一条'
\echo '==================================================================='

-- 5.1 删除 PG 侧多余的「闭环测试企业」，只留 created_at 最早的一条
--     （预检确认：enterprise_users / applications.enterprise_id 均未引用这些 id）
delete from public.enterprises e
 where e.name = '闭环测试企业'
   and e.created_at > (select min(created_at) from public.enterprises where name = '闭环测试企业');

-- 5.2 SG 侧并集：剔除 12 条 E2E/调试残留 + 6 条同名「闭环测试企业」，其余 3 条插入
insert into public.enterprises (id, name, domain, plan, created_at)
select s.id, s.name, s.domain, s.plan, s.created_at
  from sg.enterprises s
 where s.name not like 'Overseas Browser %'
   and s.name not like 'Overseas E2E %'
   and s.name not in ('DebugCo', 'DebugCo2')
   and s.name <> '闭环测试企业'
   and not exists (select 1 from public.enterprises e where e.id = s.id);

\echo '==================================================================='
\echo ' 闸门 G3 · 合并后校验（行数 / 孤儿 / 唯一键）'
\echo '==================================================================='

set constraints all immediate;   -- 提前暴露外键孤儿，失败即回滚

do $$
declare n int; bad text := '';
begin
  -- users 必须是 11 人且 email 唯一
  select count(*) into n from public.users;
  if n <> 11 then bad := bad || format(' users=%s(期望11)', n); end if;
  select count(*) into n from (select email from public.users group by email having count(*) > 1) x;
  if n <> 0 then bad := bad || format(' users.email 重复组=%s', n); end if;

  -- 被丢弃的 4 个 SG UUID 不得出现在任何外键列
  select count(*) into n from public.consents where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' consents 残留=%s', n); end if;
  select count(*) into n from public.timeline_events where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' timeline 残留=%s', n); end if;
  select count(*) into n from public.trust_memories where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' trust_memories 残留=%s', n); end if;
  select count(*) into n from public.quota_records where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' quota 残留=%s', n); end if;
  select count(*) into n from public.applications where seeker_user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' applications 残留=%s', n); end if;
  select count(*) into n from public.game_profiles where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' game_profiles 残留=%s', n); end if;
  select count(*) into n from public.fleets where captain_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' fleets 残留=%s', n); end if;
  select count(*) into n from public.fleet_members where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' fleet_members 残留=%s', n); end if;
  select count(*) into n from public.enterprise_users where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' enterprise_users 残留=%s', n); end if;
  select count(*) into n from public.match_reports where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' match_reports 残留=%s', n); end if;
  select count(*) into n from public.share_codes where owner_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' share_codes 残留=%s', n); end if;
  select count(*) into n from public.verification_audit_log where owner_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' verification_audit 残留=%s', n); end if;
  select count(*) into n from public.invite_codes where created_by in (select sg_id from merge_map); if n>0 then bad := bad || format(' invite_codes 残留=%s', n); end if;
  select count(*) into n from public.mission_completions where user_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' mission_completions 残留=%s', n); end if;
  select count(*) into n from public.trust_attestations where candidate_id in (select sg_id from merge_map); if n>0 then bad := bad || format(' trust_attestations 残留=%s', n); end if;

  -- 拍板 1 复核：10 个已有用户的 password_hash / created_at 必须与合并前完全一致（取 PG）
  select count(*) into n
    from public.users u join merge_users_before b on b.id = u.id
   where u.password_hash is distinct from b.password_hash
      or u.created_at is distinct from b.created_at;
  if n <> 0 then bad := bad || format(' 已有用户口令/创建时间被改写=%s 行', n); end if;

  -- 拍板 1 复核：zervi@genz.ltd 必须带着 SG 侧口令进来
  select count(*) into n
    from public.users u join sg.users s on s.email = u.email
   where u.email = 'zervi@genz.ltd' and u.password_hash is distinct from s.password_hash;
  if n <> 0 then bad := bad || ' zervi 口令与 SG 侧不一致'; end if;

  -- poems 必须原封不动
  select count(*) into n from public.poems;
  if n <> 58059 then bad := bad || format(' poems=%s(期望58059)', n); end if;

  if bad <> '' then
    raise exception 'G3 FAIL ->%', bad;
  end if;
  raise notice 'G3 OK: users=11 且 email 唯一；4 个被丢弃 UUID 已无引用；poems 未动';
end $$;

\echo '==================================================================='
\echo ' 合并后行数清单（人工过目；确认无误再 COMMIT）'
\echo '==================================================================='

select 'users' t, count(*) n from public.users
union all select 'orders', count(*) from public.orders
union all select 'applications', count(*) from public.applications
union all select 'audit_logs', count(*) from public.audit_logs
union all select 'consents', count(*) from public.consents
union all select 'enterprise_users', count(*) from public.enterprise_users
union all select 'enterprises', count(*) from public.enterprises
union all select 'fleet_members', count(*) from public.fleet_members
union all select 'fleets', count(*) from public.fleets
union all select 'game_profiles', count(*) from public.game_profiles
union all select 'invite_codes', count(*) from public.invite_codes
union all select 'match_report_items', count(*) from public.match_report_items
union all select 'match_reports', count(*) from public.match_reports
union all select 'mission_completions', count(*) from public.mission_completions
union all select 'quota_records', count(*) from public.quota_records
union all select 'share_codes', count(*) from public.share_codes
union all select 'timeline_events', count(*) from public.timeline_events
union all select 'trust_attestations', count(*) from public.trust_attestations
union all select 'trust_memories', count(*) from public.trust_memories
union all select 'verification_audit_log', count(*) from public.verification_audit_log
union all select 'documents', count(*) from public.documents
union all select 'product_events', count(*) from public.product_events
union all select 'query_logs', count(*) from public.query_logs
union all select 'challenge_rounds', count(*) from public.challenge_rounds
union all select 'poems(不动)', count(*) from public.poems
order by 1;

-- -----------------------------------------------------------------------------
-- 到这里为止一切只读 + 事务内写入。确认无误后手工执行：
--   commit;
-- 有任何疑问执行：
--   rollback;
-- 生产执行默认保持下面这行注释掉（不自动提交）。
-- -----------------------------------------------------------------------------
-- commit;
