-- =============================================================================
-- genzltd 单源「干净联调」清理 SQL 草案 · Round 9（CLEAN 模式）
-- -----------------------------------------------------------------------------
-- 状态：DRAFT（草案）。严禁直接对腾讯云 1.14.202.161 的 genzltd 生产库执行。
--
-- 与 Round 8 的关系：Round 8 是「合并」语义（SG 行经 merge_map 并入 PG）。
-- 本文件是「清理」语义 —— Win 2026-10-22 拍板：
--   为实现按计划联调，四项争议旧数据一律**不选边保留、全部清空**：
--     ① beta 4 人：删账号本身 + 全部级联（不再纠结 updated_at 取哪侧）
--     ② challenge_rounds 2026-W36：PG《輞川集二十首 斤竹嶺》与 SG《華嚴寺望樊川》**两侧都删**
--     ③ timeline_events 去重口径：争议组整组删除（不再纠结 87 / 80 / 68 三种口径）
--     ④ enterprises「闭环测试企业」：13 条**全删**（PG 7 删、SG 6 不并），不再"留一条"
--   并追加拍板：6 个共同 UUID 用户在 SG 侧的业务数据**也清零，只留账号**。
--   ⇒ 合并语义收缩为：清旧 + 只留账号 + 入 zervi（含其 2 条 orders）。
--   poems / columns / driver_profiles 一行不写（沿用拍板第 3 条）。
--
-- 本机 staging 实测（public = 10-22 转储还原，sg = SG 快照导入）：
--   删除 120 行；users 10 → 7（6 共同 + zervi）；11 张业务表归零；
--   enterprises 13 → 6；challenge_rounds 3 → 2；poems 58,059 未动。
--
-- 全程单事务，末尾 commit 注释掉（执行者确认后自行提交）。
-- 外键为 DEFERRABLE，末尾 set constraints all immediate 会在 COMMIT 前暴露孤儿。
-- =============================================================================

\set ON_ERROR_STOP on
\pset pager off
\timing off

-- poems 闸门严格度：strict（默认，不一致即停）/ allow_created_at_skew。
-- 本模式下 poems 一行不写，但闸门仍保留 —— 它挡的是"源数据与我以为的不一样"。
\if :{?poems_gate}
\else
  \set poems_gate strict
\endif
select set_config('merge.poems_gate', :'poems_gate', false) as poems_gate;

-- =============================================================================
\echo '==================================================================='
\echo ' 闸门 G0 · 前置环境校验'
\echo '==================================================================='
-- =============================================================================

do $$
declare
  n int; bad text := '';
begin
  if not exists (select 1 from information_schema.schemata where schema_name = 'sg') then
    raise exception 'G0 FAIL: sg schema 不存在 —— 先执行前置 0（load_sg_snapshot_to_pg.py）';
  end if;

  select count(*) into n from sg.users;           if n <> 11    then bad := bad || format(' sg.users=%s(期望11)', n); end if;
  select count(*) into n from sg.poems;           if n <> 58059 then bad := bad || format(' sg.poems=%s(期望58059)', n); end if;
  select count(*) into n from sg.enterprises;     if n <> 21    then bad := bad || format(' sg.enterprises=%s(期望21)', n); end if;
  select count(*) into n from public.users;       if n <> 10    then bad := bad || format(' public.users=%s(期望10)', n); end if;
  select count(*) into n from public.poems;       if n <> 58059 then bad := bad || format(' public.poems=%s(期望58059)', n); end if;
  select count(*) into n from public.enterprises; if n <> 13    then bad := bad || format(' public.enterprises=%s(期望13)', n); end if;
  select count(*) into n from public.documents;   if n <> 30    then bad := bad || format(' public.documents=%s(期望30)', n); end if;

  if bad <> '' then
    raise exception 'G0 FAIL: 行数与 10-22 验收值不符 ->%', bad;
  end if;
  raise notice 'G0 OK: sg/public 行数与 10-22 验收值一致';
end $$;

-- =============================================================================
\echo '==================================================================='
\echo ' 闸门 G1 · poems / columns / driver_profiles 全字段校验（只校验不写入）'
\echo '==================================================================='
-- =============================================================================

do $$
declare
  t text; pgm text; sgm text; bad boolean := false; skew boolean := true;
begin
  foreach t in array array['columns','driver_profiles','poems'] loop
    execute format(
      'select md5(coalesce(string_agg(r,'','' order by r),'''')) from (select %s as r from public.%I) x',
      (select string_agg(format('coalesce(%I::text,'''')', c.column_name), ' || ''/'' || ' order by c.ordinal_position)
         from information_schema.columns c where c.table_schema='public' and c.table_name=t), t) into pgm;
    execute format(
      'select md5(coalesce(string_agg(r,'','' order by r),'''')) from (select %s as r from sg.%I) x',
      (select string_agg(format('coalesce(%I::text,'''')', c.column_name), ' || ''/'' || ' order by c.ordinal_position)
         from information_schema.columns c where c.table_schema='public' and c.table_name=t), t) into sgm;
    if pgm = sgm then
      raise notice 'G1 % : 一致 -> 跳过', t;
    else
      bad := true;
      raise warning 'G1 % : 不一致 PG=% SG=%', t, pgm, sgm;
    end if;
  end loop;

  if bad then
    -- 只有当差异"除 created_at 外零差异"时才允许放行
    select (count(*) = 0) into skew
      from (
        select p.id from public.poems p join sg.poems s using (id)
         where coalesce(p.title,'')      is distinct from coalesce(s.title,'')
            or coalesce(p.author,'')     is distinct from coalesce(s.author,'')
            or coalesce(p.dynasty,'')    is distinct from coalesce(s.dynasty,'')
            or coalesce(p.theme,'')      is distinct from coalesce(s.theme,'')
            or coalesce(p.content,'')    is distinct from coalesce(s.content,'')
            or coalesce(p.tags,'')       is distinct from coalesce(s.tags,'')
            or coalesce(p.source,'')     is distinct from coalesce(s.source,'')
            or coalesce(p.publisher_id,'') is distinct from coalesce(s.publisher_id,'')
            or coalesce(p.column_id,'')  is distinct from coalesce(s.column_id,'')
      ) d;

    if skew and current_setting('merge.poems_gate') = 'allow_created_at_skew' then
      raise notice 'G1 OK(skew): 三张表业务字段零差异，仅 created_at 不同 -> 放行（poems 一行不写）';
    else
      raise exception 'G1 STOP: 全字段 md5 不一致，且未显式放行。本模式不写 poems/columns/driver_profiles；如需放行请 -v poems_gate=allow_created_at_skew';
    end if;
  else
    raise notice 'G1 OK: 三张表全字段一致';
  end if;
end $$;

BEGIN;
SET CONSTRAINTS ALL DEFERRED;

-- =============================================================================
\echo '==================================================================='
\echo ' 步骤 0 · 建立三组 id 清单：beta 待删 / 共同保留 / zervi 待插'
\echo '==================================================================='
-- =============================================================================

create temp table beta(id text primary key);
insert into beta select id from public.users where email like 'beta_%@looma.test';

create temp table keep(id text primary key);
insert into keep select u.id from public.users u join sg.users s using (id);

create temp table zervi(id text primary key);
insert into zervi select id from sg.users where email = 'zervi@genz.ltd';

\echo '--- beta 待删账号 ---'
select u.id, u.email, u.created_at, u.updated_at
from public.users u join beta b on b.id = u.id order by u.email;

-- =============================================================================
\echo '==================================================================='
\echo ' 闸门 G2 · 清单校验 + 危险项拦截'
\echo '==================================================================='
-- =============================================================================

do $$
declare
  n int; bad text := '';
begin
  select count(*) into n from beta;  if n <> 4 then bad := bad || format(' beta=%s(期望4)', n); end if;
  select count(*) into n from keep;  if n <> 6 then bad := bad || format(' 共同UUID=%s(期望6)', n); end if;
  select count(*) into n from zervi; if n <> 1 then bad := bad || format(' zervi=%s(期望1)', n); end if;

  -- 危险项： poems.publisher_id 不得指向待删账号（否则删用户会动到诗词）
  select count(*) into n from public.poems p join beta b on b.id = p.publisher_id;
  if n <> 0 then bad := bad || format(' poems 引用 beta 账号 %s 行（禁止继续）', n); end if;

  -- 危险项：columns / driver_profiles 的 driver 不得是 beta（CASCADE 会带走）
  select count(*) into n from public.columns c join beta b on b.id = c.driver_id;
  if n <> 0 then bad := bad || format(' columns 引用 beta %s 行', n); end if;
  select count(*) into n from public.driver_profiles d join beta b on b.id = d.id;
  if n <> 0 then bad := bad || format(' driver_profiles 属于 beta %s 行', n); end if;

  -- zervi 不能已在 PG 侧
  select count(*) into n from public.users where email = 'zervi@genz.ltd';
  if n <> 0 then bad := bad || format(' zervi 已存在于 PG(%s 行)', n); end if;

  if bad <> '' then raise exception 'G2 FAIL:%', bad; end if;
  raise notice 'G2 OK: beta 4 / 共同 6 / zervi 1；poems·columns·driver_profiles 无级联风险';
end $$;

\echo '==================================================================='
\echo ' 闸门 G2b · 基线孤儿必须为 0（保证后续"孤儿清理"只清我们造成的孤儿）'
\echo '==================================================================='

do $$
declare fk record; n int; total int := 0;
begin
  for fk in
    select tc.table_name src, kcu.column_name col, ccu.table_name ref, ccu.column_name refcol
      from information_schema.table_constraints tc
      join information_schema.key_column_usage kcu
        on tc.constraint_name = kcu.constraint_name and tc.table_schema = kcu.table_schema
      join information_schema.constraint_column_usage ccu
        on ccu.constraint_name = tc.constraint_name and ccu.table_schema = tc.table_schema
     where tc.constraint_type = 'FOREIGN KEY' and tc.table_schema = 'public'
  loop
    execute format(
      'select count(*) from public.%I s where s.%I is not null '
      'and not exists (select 1 from public.%I r where r.%I = s.%I)',
      fk.src, fk.col, fk.ref, fk.refcol, fk.col) into n;
    if n > 0 then
      raise notice '既有孤儿 %.% -> %.% : % 行', fk.src, fk.col, fk.ref, fk.refcol, n;
      total := total + n;
    end if;
  end loop;

  if total <> 0 then
    raise exception 'G2b FAIL: 基线已存在 % 行孤儿 —— 先查清它们的来源，不要在本事务里清理', total;
  end if;
  raise notice 'G2b OK: 57 条外键，基线孤儿 0';
end $$;

-- =============================================================================
\echo '==================================================================='
\echo ' 步骤 1 · beta 4 人级联删除（38 条外键逐表清，行数逐个 NOTICE）'
\echo '==================================================================='
-- =============================================================================

delete from public.match_report_items where report_id in (select id from public.match_reports where user_id in (select id from beta));
delete from public.applications        where seeker_user_id in (select id from beta);
delete from public.match_reports       where user_id in (select id from beta);
delete from public.match_consensus     where initiator_id in (select id from beta) or candidate_id in (select id from beta);
delete from public.trust_attestations  where candidate_id in (select id from beta);
delete from public.trust_memories      where user_id in (select id from beta);
delete from public.navigator_memories  where user_id in (select id from beta);
delete from public.quota_records       where user_id in (select id from beta);
delete from public.usage_logs          where user_id in (select id from beta);
delete from public.timeline_events     where user_id in (select id from beta);
delete from public.consents            where user_id in (select id from beta);
delete from public.game_profiles       where user_id in (select id from beta);
delete from public.mission_completions where user_id in (select id from beta);
delete from public.fleet_members       where user_id in (select id from beta);
delete from public.fleets              where captain_id in (select id from beta);
delete from public.invite_codes        where created_by in (select id from beta) or used_by in (select id from beta);
delete from public.share_codes         where owner_id in (select id from beta);
delete from public.verification_audit_log where owner_id in (select id from beta);
delete from public.report_sharing      where user_id in (select id from beta);
delete from public.subscriptions       where user_id in (select id from beta);
delete from public.boost_consumptions  where user_id in (select id from beta);
delete from public.boost_packs         where user_id in (select id from beta);
delete from public.candidates          where user_id in (select id from beta);
delete from public.challenge_entries   where user_id in (select id from beta);
delete from public.job_posts           where user_id in (select id from beta);
delete from public.huawei_push_tokens  where user_id in (select id from beta);
delete from public.domain_visits       where user_id in (select id from beta);
delete from public.emergent_strategy_log where user_id in (select id from beta);
delete from public.narrative_sessions  where user_id in (select id from beta);
delete from public.orders              where user_id in (select id from beta);
delete from public.quiz_sessions       where user_id in (select id from beta);
delete from public.sales_inquiries     where user_id in (select id from beta);
delete from public.enterprise_users    where user_id in (select id from beta);

\echo '--- 无外键约束但有用户指向列的表：孤儿扫描覆盖不到，必须显式清 ---'
\echo '    （product_events / query_logs / micro_feedback / audit_logs.actor）'
delete from public.product_events where user_id in (select id from beta);
delete from public.query_logs     where user_id in (select id from beta);
delete from public.micro_feedback where user_id in (select id from beta);
delete from public.audit_logs     where actor   in (select id from beta);

\echo '--- 删除账号本体（articles/columns/driver_profiles 走 ON DELETE CASCADE，G2 已确认安全）---'
delete from public.users where id in (select id from beta);

-- =============================================================================
\echo '==================================================================='
\echo ' 步骤 2 · challenge_rounds 2026-W36 两侧都删'
\echo '==================================================================='
-- =============================================================================

-- PG 侧删（SG 侧本来就不插入）。实测 challenge_entries 两侧均 0 行 ⇒ 无级联风险。
delete from public.challenge_entries
 where round_id in (select id from public.challenge_rounds where week_key = '2026-W36');
delete from public.challenge_rounds where week_key = '2026-W36';

-- =============================================================================
\echo '==================================================================='
\echo ' 步骤 3 · enterprises「闭环测试企业」13 条全删（PG 7 删 + SG 6 不并）'
\echo '==================================================================='
-- =============================================================================

delete from public.enterprise_users where enterprise_id in (select id from public.enterprises where name = '闭环测试企业');
delete from public.candidates       where enterprise_id in (select id from public.enterprises where name = '闭环测试企业');
delete from public.enterprises      where name = '闭环测试企业';

-- =============================================================================
\echo '==================================================================='
\echo ' 步骤 4 · 插入 zervi@genz.ltd（SG 的 UUID + SG 的口令）+ 其 2 条 orders'
\echo '==================================================================='
\echo ' 注：zervi 在 SG 侧另有 product_events 3 行，按 CLEAN 口径不随行（如需请解开下面注释）'
-- =============================================================================

do $$
declare
  cols text; z text; n int;
begin
  select id into z from zervi limit 1;

  select string_agg(format('%I', column_name), ',' order by ordinal_position) into cols
    from information_schema.columns
   where table_schema = 'public' and table_name = 'users';

  execute format('insert into public.users (%s) select %s from sg.users where id = $1', cols, cols) using z;
  get diagnostics n = row_count;
  raise notice 'users: 插入 zervi %（% 行）', z, n;

  select string_agg(format('%I', column_name), ',' order by ordinal_position) into cols
    from information_schema.columns
   where table_schema = 'public' and table_name = 'orders';

  execute format('insert into public.orders (%s) select %s from sg.orders where user_id = $1', cols, cols) using z;
  get diagnostics n = row_count;
  raise notice 'orders: 随行插入 % 行', n;
end $$;

-- 如需让 zervi 的 product_events 随行，解开下面这段：
-- insert into public.product_events (<列>) select <列> from sg.product_events where user_id in (select id from zervi);

-- =============================================================================
\echo '==================================================================='
\echo ' 步骤 5 · 通用级联孤儿清理（按 57 条外键迭代到不动点）'
\echo '==================================================================='
\echo ' 手工枚举级联深度不可靠：staging 实测 narrative_sessions 的下游 act1_sessions'
\echo ' 就是第二层外键，靠手写清单漏掉了。改为：清 + 迭代扫全表外键。'
-- =============================================================================

do $$
declare
  fk record; cnt int; n_del int; round int := 0; grand int := 0;
begin
  loop
    round := round + 1; cnt := 0;
    for fk in
      select tc.table_name src, kcu.column_name col, ccu.table_name ref, ccu.column_name refcol
        from information_schema.table_constraints tc
        join information_schema.key_column_usage kcu
          on tc.constraint_name = kcu.constraint_name and tc.table_schema = kcu.table_schema
        join information_schema.constraint_column_usage ccu
          on ccu.constraint_name = tc.constraint_name and ccu.table_schema = tc.table_schema
       where tc.constraint_type = 'FOREIGN KEY' and tc.table_schema = 'public'
    loop
      execute format(
        'delete from public.%I s where s.%I is not null '
        'and not exists (select 1 from public.%I r where r.%I = s.%I)',
        fk.src, fk.col, fk.ref, fk.refcol, fk.col);
      get diagnostics n_del = row_count;
      cnt := cnt + n_del;
    end loop;
    exit when cnt = 0 or round >= 10;
    raise notice '第 % 轮级联清理：% 行', round, cnt;
    grand := grand + cnt;
  end loop;
  raise notice '级联清理完成：共 % 轮，累计 % 行', round, grand;
end $$;

-- =============================================================================
\echo '==================================================================='
\echo ' 闸门 G3 · 事后校验'
\echo '==================================================================='
-- =============================================================================

do $$
declare
  n int; bad text := '';
  expect constant jsonb := '{
    "users":7,"applications":0,"timeline_events":0,"trust_memories":0,"trust_attestations":0,
    "consents":0,"quota_records":0,"game_profiles":0,"mission_completions":0,
    "fleets":0,"fleet_members":0,"enterprise_users":0,"candidates":0,"match_reports":0,
    "narrative_sessions":0,"narrative_events":0,"act1_sessions":0,
    "domain_visits":0,"value_imprints":0,"micro_feedback":0,
    "enterprises":6,"challenge_rounds":2,"documents":30,"poems":58059,
    "columns":1,"driver_profiles":1,"orders":2,
    "product_events":5,"query_logs":0,"micro_feedback":1,"audit_logs":25
  }';
  k text; v int;
begin
  for k, v in select * from jsonb_each_text(expect) loop
    execute format('select count(*) from public.%I', k) into n;
    if n <> v then bad := bad || format(' %s=%s(期望%s)', k, n, v); end if;
  end loop;

  -- beta 4 个 UUID 必须零残留
  select count(*) into n from public.users where id in (select id from beta);
  if n <> 0 then bad := bad || format(' beta 账号残留 %s', n); end if;

  -- zervi 必须带 SG 的口令落地
  select count(*) into n
    from public.users p join sg.users s using (id)
   where s.email = 'zervi@genz.ltd' and p.password_hash is not distinct from s.password_hash;
  if n <> 1 then bad := bad || format(' zervi 口令未按 SG 落地(%s)', n); end if;

  -- email 唯一性
  select count(*) into n from (select email from public.users group by email having count(*) > 1) x;
  if n <> 0 then bad := bad || format(' email 重复 %s 组', n); end if;

  if bad <> '' then raise exception 'G3 FAIL:%', bad; end if;
  raise notice 'G3 OK: 全部期望值命中（users 7 = 6 共同 + zervi；11 张业务表归零）';
end $$;

-- 立刻校验全部外键，把任何孤儿引用挡在 COMMIT 之前
SET CONSTRAINTS ALL IMMEDIATE;

\echo '==================================================================='
\echo ' 事务未提交。确认无误后手工执行： COMMIT;'
\echo ' 如需回滚： ROLLBACK;'
\echo '==================================================================='

-- COMMIT;

-- ROLLBACK;
