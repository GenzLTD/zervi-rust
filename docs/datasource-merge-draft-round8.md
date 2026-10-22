# Round 8 · 单源合并 SQL 草案（Win 拍板落地 + 本地 staging 实测）

> 日期：2026-10-22 11:0x GMT+8
> 拍板来源：Win 2026-10-22 回执（users / enterprises / 25 张相同表三条）
> 交付物：
> - `docs/datasource-merge-draft-round8.sql` —— 可执行草案（单事务、带闸门）
> - `scripts/merge/load_sg_snapshot_to_pg.py` —— 前置步骤 0：把 SG 快照导入目标库的 `sg` schema
> - 本文件 —— 拍板落地说明、实测校正、待确认项
>
> **本轮没有对腾讯云 1.14.202.161 的 genzltd 生产库执行任何写操作。**
> 草案只在本机临时库上跑过（见下）。
>
> **追加（2026-10-22 下午）**：Win 随后拍板「四项争议旧数据全清空、干干净净联调」，
> 语义从"合并"改为"清理"，另出 `datasource-clean-draft-round9.sql`（CLEAN 模式）。
> 本文件保留为**合并语义的存档**，两条路线都在 main 上。

---

## 0. 拍板落地：三件事在 SQL 里的对应动作

| # | Win 拍板 | 草案里的落地 | 落在哪一步 |
|---|---|---|---|
| 1 | 六个共同用户的 `password_hash` / `created_at` 取 PG | **不动这两列**（PG 值即结果）；只 `update updated_at = greatest(SG,PG)` | 步骤 1a |
| 1 | 四个 beta 合并后用境内口令，SG 口令作废 | 4 个 SG UUID 整行**不插入**，PG 行原样保留 | 步骤 1c |
| 1 | `updated_at` 取两侧较晚，不要一并写成 PG | `s.updated_at > u.updated_at` 才更新 | 步骤 1a + G3 复核 |
| 1 | `zervi@genz.ltd` 用 SG 的 UUID 和口令新插入，2 条 orders 随行插入 | 整行 insert（含 SG 的 `password_hash`），orders 紧跟其后 | 步骤 1b / 步骤 2 |
| 1 | beta_supporter（9d171561）没有外键，直接丢 | 实测确认 **0 行引用**，它只是"不插入"的自然结果 | 步骤 1c |
| 1 | 其余 140 行在插入时改写到对应 PG UUID | `coalesce(merge_map.pg_id, s.<外键列>)` 逐表改写 | 步骤 3 全部 |
| 1 | PG 里已有的外键不用改 | 草案不含任何"改写 PG 现有行 user_id"的语句 | 全文 |
| 2 | 剔除 SG 侧企业测试残留（Overseas Browser/E2E、DebugCo、DebugCo2） | `name not like 'Overseas Browser %' / 'Overseas E2E %' / not in ('DebugCo','DebugCo2')` | 步骤 5.2 |
| 2 | 闭环测试企业按 name 留一条 | PG 侧同名 7 条**删 6 条**（留 created_at 最早）+ SG 侧 6 条全部不插入 | 步骤 5.1 / 5.2 |
| 2 | 其余企业并集 | SG 侧 3 条（测试用户-专业版 / 测试用户-管理员 / 穆蓝）插入 | 步骤 5.2 |
| 3 | 25 张主键 md5 相同的表不插入、不覆盖 | 草案 24 张写入表里**没有一张**来自这 25 张 | 全文 |
| 3 | 22 张双空不做字段比对 | 只校验 columns / driver_profiles / poems | 闸门 G1 |
| 3 | 三张全行 md5 一致就跳过；poems 不一致就停 | G1 默认 `strict`：不一致即 RAISE EXCEPTION | 闸门 G1 |

---

## 1. 草案是怎么被验证的（本机临时库，非生产）

用 ServBay 自带的 PostgreSQL 17.9 在 `/tmp/pgmerge/pgdata` 起了一个**独立临时实例**
（socket `/tmp/pgmerge`，端口 55432，`listen_addresses=''`，不监听网络）：

| schema | 内容 | 来源 |
|---|---|---|
| `public` | genzltd 生产副本 | `snapshots/genzltd-20261022.sql.gz` 还原（去掉 PG17.10 的 `\restrict` 行） |
| `sg` | SG 生产快照 | `snapshots/looma-sg-20261022.db` 55 张表按 PG 列顺序 COPY 进去 |

还原后复核：`poems 58,059` / `users 10` / `enterprises 13` / `documents 30` —— 与 10-22 验收值一致。
SG 侧导入 55 表行数与 Round 7 的 55 行表逐行一致（`users 11`、`enterprises 21`、`documents 39`、`orders 2`…）。

**跑通结果**：`strict` 模式在 G1 按预期 STOP；`allow_created_at_skew` 模式下全流程跑通，
G3 校验通过（users=11 且 email 唯一、4 个丢弃 UUID 零残留、poems 未动、外键无孤儿）。
事务未提交（`-- commit;` 保持注释），staging 库可随时重建。

---

## 2. 闸门 G1：三张表的全字段校验结果（拍板第 3 条）

口径：行内所有列 `coalesce(::text,'')` 以 `/` 连接 → 按文本序排序 → `,` 连接 → `md5`。
**三张全部不一致**：

| 表 | 行 | PG 全行 md5 | SG 全行 md5 | 差异来源（逐列实测） |
|---|---:|---|---|---|
| `columns` | 1 | `a2c47fd299345f9e313a1109d22cc2c5` | `091840747d61a1694c66b7af3381342e` | 仅 `created_at`（PG `2026-09-24 23:40:03` / SG `2026-09-25 02:30:40`）；其余 8 列完全相同 |
| `driver_profiles` | 1 | `9eca235b84b9e21904406c38e29904ca` | `2d18b33d9bbd220472f70acb1b58a625` | `created_at` + `updated_at` 同上；另 PG 独有 `status='active'`（SG 表根本没有 status 列 → 取空串） |
| `poems` | 58,059 | `59caac1a0d1b8df058b83d1fe9d4a401` | `4f668e020b6571b82a27b64a907293ab` | `created_at` 58,059 行**全部不同**；其余 10 列（title/author/dynasty/theme/content/tags/source/publisher_id/column_id）**逐列零差异** |

`poems.created_at` 的分布说明这是导入时刻，不是数据差异：

| 侧 | created_at 取值 | 行数 |
|---|---|---:|
| PG | `2026-07-06 07:50:28`–`07:50:31`（4 个值） | 58,059 |
| SG | `2026-07-15 14:46:25`–`14:46:33`（9 个值） | 58,059 |

两侧 id 集合完全相同（仅 PG 0 / 仅 SG 0），业务字段全同 ⇒ **poems 是同一批诗词的两次独立导入**。

**按拍板"poems 不一致就停"**：草案 `strict` 模式下在 G1 直接 STOP（已实测），
不会走到任何写入。要放行必须显式 `-v poems_gate=allow_created_at_skew`，
且 G1 内部会再验一次" poems 业务字段零差异"才放行。

⇒ **这一条要 Win 亲自拍**：放行（承认 created_at 是导入噪声，poems 不动）
还是按字面 STOP（先做一次 created_at 对齐再合并）。

---

## 3. 与 Round 7 文档口径不一致的 6 处（本地实测校正）

| # | 文档口径 | 实测 | 影响 |
|---|---|---|---|
| 1 | `challenge_rounds`：W29–31 / W36–39 无重叠，并集 9 | **2026-W36 两侧各有一条**：PG id=1《輞川集二十首 斤竹嶺》、SG id=5《華嚴寺望樊川》，均 closed、同 starts_at | 撞 `uq_challenge_rounds_week_key`。草案默认保留 PG、跳过 SG ⇒ **净 8（文档 9）** |
| 2 | `enterprises`：测试残留 13 条；name 交集 1 条；净 20 | 测试残留 **12 条**（Overseas E2E ×7、Overseas Browser ×3、DebugCo ×1、DebugCo2 ×1）；「闭环测试企业」SG **6 条** + PG **7 条** = 13 条同名 | 按"留一条"需 DELETE PG 侧 6 行 ⇒ **净 10（文档 20）**。已确认 `enterprise_users` 引用的是"测试用户-*"不是闭环、`applications.enterprise_id` 全空 ⇒ 删除不产生孤儿 |
| 3 | `timeline_events`：按 (user,title) 去重 7 条 → 净 80 | 7 组里「对话交互」一组是 SG 12 + PG 5 = 17 行，**occurred_at 各不相同** | 按 title 去重会丢 16 条真实对话。草案默认改 **(user,title,occurred_at)** ⇒ **净 87** |
| 4 | `trust_memories`：memory_content 交集 1（星云艺术家 JSON），净 ≤56 | 按 (canon, session_type, memory_content) 重叠 **0 组** | 全并集 ⇒ **净 57** |
| 5 | `applications`：交集 1，两边同为 withdrawn | 交集确为 1（fcf2c1fc / j1 / resume 23），但 **PG 那条是 `submitted`**（08-08 20:35:25），SG 是 withdrawn（21:35:42） | 草案默认保留 PG 现有行、跳过 SG ⇒ 净 31（与文档一致，但"同为 withdrawn"的表述不成立） |
| 6 | `users`：共同 UUID 的 6 人中 password_hash 4 个不同 | 共同 **UUID** 的 6 人 `password_hash` **完全一致**（pw_diff 全 false）；`created_at` 只有 driver-official 不同，且它**两侧 password_hash 均为空**（Win 本机复核同口径） | 拍板 1 对这 6 人的实际效果是：只有 driver-official 的 created_at/updated_at 有差异（口令两侧皆空，无安全含义），其余 5 人 no-op。"4 个 bcrypt 不同"说的是 email 相同但 UUID 不同的 beta 4 人 |

被丢弃 UUID 的引用分布复核：**20 / 0 / 39 / 81 = 140**，与文档逐表逐 UUID 完全一致 ✅
（consents 17 · timeline_events 41 · quota_records 19 · trust_memories 18 · match_reports 10 · applications 9 ·
trust_attestations 6 · fleets.captain 3 · fleet_members 3 · game_profiles 3 · mission_completions 3 ·
enterprise_users 2 · invite_codes.created_by 2 · share_codes.owner 2 · verification_audit_log.owner 2）。

**唯一索引冲突预检（重编主键后仍会撞的业务键）**：

| 键 | 冲突数 | 结论 |
|---|---:|---|
| `uq_documents_file_path` | 0 | documents 39 行可全插 ✅ |
| `uq_challenge_rounds_week_key` | **1** | 见校正 1 |
| `uq_invite_codes_code` / `uq_share_codes_code` / `idx_fleets_invite_code` / `uq_orders_out_trade_no` | 0 | ✅ |
| `uq_users_email` | 10（= 6 共同 + 4 beta） | 只有 zervi 是净新增 ✅ |
| `uq_users_supabase_uid` / `uq_users_wechat_openid` | 0（两侧全 NULL） | zervi 插入安全 ✅ |
| `uq_game_profiles_user_id` | 2 | 按 updated_at 取最新，非 union ✅ |
| `uq_mission_completions_*` | 1 | 跳过 SG ✅ |
| `uq_trust_attestations_claim` | 3 | 跳过 SG（PG 侧 0.93 > SG 0.45）✅ |
| `uq_quota_records_*` / `uq_enterprise_users_*` / `uq_fleet_members_*` | 0 | ✅ |

---

## 4. 合并后行数清单（staging 实测，事务未提交）

| 表 | SG | PG | 合并后 | 与文档 3.2 |
|---|---:|---:|---:|---|
| `applications` | 9 | 23 | **31** | 一致 ✅ |
| `audit_logs` | 225 | 37 | **262** | 一致 ✅ |
| `challenge_rounds` | 6 | 3 | **8** | 文档 9（校正 1） |
| `consents` | 27 | 14 | **32** | 一致 ✅ |
| `documents` | 39 | 30 | **69** | 一致 ✅ |
| `enterprise_users` | 2 | 5 | **7** | 一致 ✅ |
| `enterprises` | 21 | 13 | **10** | 文档 20（校正 2） |
| `fleet_members` | 4 | 2 | **6** | 一致 ✅ |
| `fleets` | 4 | 2 | **6** | 一致 ✅ |
| `game_profiles` | 7 | 3 | **8** | 一致 ✅ |
| `invite_codes` | 3 | 0 | **3** | ✅ |
| `match_report_items` | 11 | 0 | **11** | ✅ |
| `match_reports` | 11 | 0 | **11** | ✅ |
| `mission_completions` | 8 | 5 | **12** | 一致 ✅ |
| `orders` | 2 | 0 | **2** | ✅ |
| `product_events` | 25 | 28 | **53** | 一致 ✅ |
| `query_logs` | 69 | 13 | **82** | 一致 ✅ |
| `quota_records` | 29 | 4 | **33** | 一致 ✅ |
| `share_codes` | 4 | 0 | **4** | ✅ |
| `timeline_events` | 68 | 19 | **87** | 文档 80（校正 3） |
| `trust_attestations` | 8 | 3 | **8** | 一致 ✅ |
| `trust_memories` | 29 | 28 | **57** | 文档 ≤56（校正 4） |
| `users` | 11 | 10 | **11** | 一致 ✅ |
| `verification_audit_log` | 6 | 0 | **6** | ✅ |
| `poems` | 58,059 | 58,059 | **58,059（未动）** | ✅ |

另有 6 张 PG 独有表（`act1_sessions` / `domain_visits` / `narrative_events` /
`narrative_sessions` / `micro_feedback` / `value_imprints`）SG 为空 ⇒ 不动；
22 张双空表不动；`columns` / `driver_profiles` 按拍板不插入不覆盖。

---

## 5. 执行前还必须 Win 拍板的 5 个问题

1. **poems 闸门放行与否**（第 2 节）。放行的实质是承认"created_at 是两次导入的时刻差"，
   poems 依然一行不改；不放行则整个合并卡在这里。
2. **`updated_at` 取较晚的范围**：拍板字面是"六个共同用户"。实测 4 个 beta 的
   SG `updated_at` = `2026-09-03 16:01:41`，PG = `2026-09-02 03:56:16`（SG 更晚）。
   要不要把这 4 人也更新成 SG 的值？草案默认**不动**（严格按字面），
   需要的话解开步骤 1a 后面那段注释掉的 SQL 即可。
3. **challenge_rounds 2026-W36 留哪条**：草案默认留 PG《輞川集二十首 斤竹嶺》。
4. **timeline_events 去重口径**：87（默认，(user,title,occurred_at)）/ 80（文档，仅 title）
   / 68（严格按 title 跳过 SG 全部重叠行）。默认口径一行不删。
5. **enterprises 闭环测试企业留哪一条 UUID**：草案留 PG 侧 created_at 最早的
   `66db8f77-6e2d-4fd9-bb15-349853a4846c`（2026-07-06 07:49:24），删其余 6 条 PG 行。
   这是草案里**唯一会删除生产库现有行**的动作，需单独确认。

---

## 6. 生产执行的前置步骤（未做）

0. **导入 SG 数据到目标库的 `sg` schema**：草案所有源数据都从 `sg.*` 读，
   生产库上没有 `sg` schema 时 G0 会直接 FAIL 并拒绝执行（已实测）。
   脚本：`scripts/merge/load_sg_snapshot_to_pg.py --sqlite <快照> --psql "<psql 前缀>"`
   （`create table sg.X (like public.X excluding all)` + 去掉 NOT NULL + COPY；
   列顺序按 PG 侧，SG 缺的列留空。只写 sg schema，public 一行不动）。
   本机就是用它导入的：`--psql "/Applications/ServBay/bin/psql -h /tmp/pgmerge -p 55432 -U looma -d genzltd"`。
1. **再打一份 pg_dump**（执行前的新鲜备份），不要复用 10-22 那份。
2. **停写**：合并期间应用侧不能有并发写入（草案是长事务，会持锁）。
3. 跑 `psql -f datasource-merge-draft-round8.sql`（保持 `strict`），
   逐条看 NOTICE / WARNING，确认行数清单后再手工 `commit;`。
   脚本末尾的 `commit;` 默认是注释掉的 —— 不自动提交。

---

## 7. 本轮未做

1. **未对生产库执行任何写入**：本轮所有 SQL 只跑在本机 55432 的临时实例上。
2. 未把 `sg` schema 导入生产库（那是执行步骤，不是草案步骤）。
3. `consents` 5 个 scope 在 PG 侧各有 2 行（job_match / jobseeker_core / report_generate /
   resume_parse / resume_upload），草案按"跳过 SG、保留 PG 全部"处理（净 32 与文档一致）；
   没有做"逐行按 granted_at/revoked_at 取最新"的行级取舍 —— 那会改写 PG 现有行，
   超出拍板范围，列为待确认。
4. `driver_profiles` 的 PG 独有列 `status='active'` 按拍板 3 保持不动（SG 侧无此列）。
5. 未跑 25 张相同表里 22 张双空表的字段比对（拍板说不用做）。

---

## 8. Round 8 复核（Win 回执 · 2026-10-22）

以下全部是对**远端 tip** 的实测，补入本文档以免口头结论丢失。

### 8.1 跨仓互钉

| 项 | 实测结果 |
|---|---|
| zervi-rust `origin/main` | `742dd05` |
| `datasource-authority-mac-response.md` 第 13 行 | `looma-zervi` @ `ops/pg-export-safe-default` = **`46ae918`** ✅ |
| 同文件第 14、18 行 | **`7592ff8`**（pre-pin）✅ |
| 同文件是否含 `a886a6ce` | **不含**（`grep` 无命中）。`a886a6ce` 是 `docs/datasource-authority-round3` 的分支尖端，从未写入正文 |
| zervi-rust 祖先关系 | `a886a6ce` / `7592ff8` / `71a2c97` / `73277cb` / `ef63b9a` / `5cc7c5b` 全部 `is-ancestor origin/main` ✅ |
| looma-zervi `origin/main` | **`06bc5b5`**，parents = **`c8e3455` `46ae918`** —— PR #28 的**真两亲 merge commit** |
| `46ae918` 对 looma main | IS ancestor ✅ |
| `a886a6ce` / `7592ff8` 在 looma-zervi | **对象不存在** ⇒ 反向钉只存在于 PR #28 的描述里（含"禁止 squash"） |
| `742dd05` 改动面 | 仅 3 个新文件（md / sql / py），**未动 `datasource-authority-mac-response.md`** ⇒ 前向钉未被触及 |

### 8.2 上一轮误判的根因：本地 `main` 过期

本机 `refs/heads/main` 停在 `4ec064b`（未 fetch），而 `origin/main` 已到 `742dd05`。
对着过期的本地 ref 跑 `git merge-base --is-ancestor a886a6ce main` 得到 **NOT ancestor** 的假阴性，
据此一度反向质疑了本来为真的互钉。已 fast-forward 两个仓的本地 `main`
（本机主线无独有提交，fast-forward 安全）：

- zervi-rust `main` → `742dd05`
- looma-zervi `main` → `06bc5b5`

**纪律：判祖先/判远端状态一律先 `git fetch`，对 `origin/<branch>` 判，不要相信本地主线的陈旧 ref。**

### 8.3 PR #17 的真实形态（勘误）

`742dd05` **只有一个父提交 `1a46a6d`**，不是 merge commit：

```
number=17  state=closed  merged=true
merge_commit_sha == head.sha == 742dd05e3dd250903a95eab8280954c464455ccc
merged_by = szpeter2026
```

分支从 `origin/main`（=`1a46a6d`）切出、只加一个提交后再推回 main ⇒ GitHub 记为
**fast-forward 合并**，`merge_commit_sha` 指向 head 本身，**没有生成 merge commit**。
禁令（禁止 squash / SHA 不得变）仍然守住；但上一轮汇报里写成"merge commit `742dd05`"不准确，以此为准。

另外 `a8d541b..742dd05` 共 **9 个提交**（Round 4/5/6/7 各提交 + 各 merge commit + Round 8），
不只有 Round 8 一个。

### 8.4 Win 本机独立复核（未复跑 staging、未登录腾讯云）

| 核对项 | Win 本机 | 与本文档第 3 节 |
|---|---|---|
| SG 测试残留企业 | 12 条（Overseas E2E 7 / Overseas Browser 3 / DebugCo 1 / DebugCo2 1） | 一致 ✅（不再是先前拍板的 13） |
| 闭环测试企业 | SG 6 + PG 7 = 13 条同名 ⇒ 净 10 | 一致 ✅ |
| `poems` | 两侧均 58,059 行、主键一一对应；`created_at` 全不同；业务列差异 0 | 与 G1 一致 ✅ |
| 6 个共同 UUID 的 `password_hash` | 一致；driver-official 两侧**均为空口令**，仅 `created_at` 不同 | 一致 ✅ |

⇒ 四点独立复核全部吻合。但 staging 上的行清单、G3 校验、草案执行结果仍**只有 Mac 侧证据**，
服务器侧无法再证一次（本轮未登录腾讯云）。

### 8.5 生产前置状态更新

- **SG 快照不缺**：本机 `snapshots/looma-sg-20261022.db`，sha256 `6b08bdae…9b509`（30,818,304 B），
  与 10-22 从生产取的快照是同一份；活动库自 10-18 起无新数据 ⇒ §6 前置 0 的**数据源已就位且有效**。
- 仍未做的三项：把 `sg` schema 导入生产库、执行前的**新鲜 `pg_dump`**（PG 侧备份，与 SG 快照是两回事）、
  停写。
- 两个仓都**不存在** `single_source_authority_lockdown.md` ⇒ 没有回写目标；
  本轮勘误只落在本文档内，不另抽清单。
