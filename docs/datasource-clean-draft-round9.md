# Round 9 · 单源「干净联调」清理 SQL 草案

> 日期：2026-10-22 下午 GMT+8
> 拍板来源：Win 2026-10-22 回执（四项争议旧数据全清空 + 追加两项问答）
> 交付物：
> - `docs/datasource-clean-draft-round9.sql` —— 可执行清理草案（单事务、五道闸门、commit 注释掉）
> - 本文件 —— 拍板落地、staging 实测清单、三个新增技术发现、执行前置
>
> **本轮没有对腾讯云 1.14.202.161 的 genzltd 生产库执行任何写操作。**
> 草案只跑在本机 `/tmp/pgmerge` 的临时 PG 实例（55432，socket only，不监听网络）上，事务始终未提交。

---

## 0. 语义变更：从「合并」变成「清理」

Round 8 是合并语义（SG 行经 `merge_map` 改写后并入 PG）。本轮 Win 拍板：

> 为实现按计划联调，beta 4 人 updated_at / W36 留哪条 / timeline 去重口径 / 闭环企业留哪个 UUID
> 这些旧数据全清空，干干净净联调。

四项争议**都不再"选边保留"，而是整组清除**：

| 争议项 | Round 8 的做法 | Round 9 CLEAN |
|---|---|---|
| beta 4 人的 `updated_at` | 是否采纳 SG 较晚值 | **账号本身删除**，连同全部数据 |
| `challenge_rounds` 2026-W36 | PG 侧保留、SG 侧跳过（净 8） | **两侧都删**（净 2） |
| `timeline_events` 去重口径 | 87 / 80 / 68 三选一 | **争议组整组删除**，不再选口径 |
| `enterprises` 闭环测试企业 | 留一条（净 10） | **13 条全删**；收尾再删 PG 其余 6 家测试/演示企业，**净 0** |

追加两问（Win 2026-10-22 现场拍板）：

| 问题 | 拍板 |
|---|---|
| beta 4 人清多深？ | **A 深清**：删账号本身 + 全部级联，120 行（不是只删争议行的 B 平清 20 行） |
| 6 个共同 UUID 用户的 SG 侧业务数据？ | **也清零，只留账号**：一行不并入 |

⇒ 合并语义收缩为：**清旧 + 只留账号 + 入 zervi（含其 2 条 orders）**。
Round 8 的 `datasource-merge-draft-round8.sql` 保留在 main 上不动，作为另一种口径的存档。

---

## 1. 怎么验证的

同一台临时实例：`public` = `snapshots/genzltd-20261022.sql.gz` 还原，`sg` = `snapshots/looma-sg-20261022.db`
经 `scripts/merge/load_sg_snapshot_to_pg.py` 导入的 55 张同名表。

- `strict` 模式：**G1 直接 STOP**（已实测），走不到任何写入。
- `-v poems_gate=allow_created_at_skew` 模式：全流程跑通，**五道闸门全绿**，事务未提交。
- 末尾 `SET CONSTRAINTS ALL IMMEDIATE` 不再报错 ⇒ 无外键孤儿。

### 五道闸门

| 闸门 | 校验内容 |
|---|---|
| G0 | `sg`/`public` 行数与 10-22 验收值一致（sg.users 11 / public.users 10 / enterprises 13 / poems 58,059…） |
| G1 | columns / driver_profiles / poems 全字段 md5；不一致则 STOP，除非显式放行且业务字段零差异 |
| G2 | beta=4 / 共同 UUID=6 / zervi=1；**并拦截三个危险项**（见第 2 节） |
| G2b | 清理前基线孤儿必须为 0（保证后续"孤儿清理"只清本事务造成的孤儿） |
| G3 | 25 项期望行数逐条比对 + beta UUID 零残留 + zervi 口令按 SG 落地 + email 唯一 |

---

## 2. 三个新增技术发现（靠手工枚举会漏）

### 2.1 第二层外键：手写级联清单不可靠

按 `users` 的 38 条外键逐表删除后，`SET CONSTRAINTS ALL IMMEDIATE` 仍然抓到孤儿：

```
ERROR: update or delete on table "narrative_sessions" violates foreign key
constraint "fk_act1_sessions_session_id" on table "act1_sessions"
```

`act1_sessions` 引用的是 `narrative_sessions`，不是 `users` —— 属于第二层。
⇒ 草案改为 **步骤 5「通用级联孤儿清理」**：按 `information_schema` 里 57 条外键迭代 `delete … not exists`
到不动点（staging 实测第 1 轮清 59 行，第 2 轮 0 行），不再依赖手写清单的深度。

### 2.2 有用户指向列却**没有外键约束**的表

这类表孤儿扫描覆盖不到，必须显式删。实测：

| 表 | 无约束的用户列 | 属于 beta 的行数 | 表总数 |
|---|---|---:|---:|
| `product_events` | `user_id` | 23 | 28 |
| `query_logs` | `user_id` | 13 | 13 |
| `micro_feedback` | `user_id` | 1 | 2 |
| `audit_logs` | `actor` | 12 | 37 |

（`iap_subscriptions.user_id`、`llm_usage.user_id` 同样无约束，但两侧都是 0 行。）
其中 `micro_feedback.user_id` 还能取 `guest-28ad1ce4-814` 这类**不在 users 表里的伪 ID** ——
所以它不可能有 FK，也不会被任何补丁扫到。

### 2.3 三个危险项必须在删除前拦死（G2）

| 风险 | 为什么危险 | 拦截方式 |
|---|---|---|
| `poems.publisher_id → users.id` | 若有 beta 行指向它，删用户会连带**动到 58,059 行诗词** | 实测 0 行；非 0 则 FAIL |
| `columns.driver_id → users.id ON DELETE CASCADE` | driver 是 beta 的话会带走唯一的 1 行 `columns` | 实测 0 行；非 0 则 FAIL |
| `driver_profiles.id → users.id ON DELETE CASCADE` | 同上，会带走唯一的 1 行 `driver_profiles` | 实测 0 行；非 0 则 FAIL |

`tenants.driver_id` 是 `ON DELETE SET NULL`，不需要处理。

---

## 3. 清理前后全表清单

表内除 `enterprises` 的「后」已按收尾拍板从实测 6 改为期望 0 之外，其余格子仍是上一轮 staging 的实测值。

上一轮 staging（尚未删那 6 家）：**58,392 → 58,139，净 -253 行**（删除 255 行，插入 2 行）。
收尾把 6 家测试/演示企业也删掉之后，期望再减 6 行：**58,392 → 58,133，净 -259**。这一步只在本机 10-20 副本上用事务回滚核对过删除本身，没有重跑 55432 全流程。

| 表 | 前 | 后 | 差 |
|---|---:|---:|---:|
| `narrative_events` | 43 | 0 | -43 |
| `trust_memories` | 28 | 0 | -28 |
| `applications` | 23 | 0 | -23 |
| `product_events` | 28 | 5 | -23 |
| `timeline_events` | 19 | 0 | -19 |
| `act1_sessions` | 14 | 0 | -14 |
| `consents` | 14 | 0 | -14 |
| `domain_visits` | 14 | 0 | -14 |
| `narrative_sessions` | 14 | 0 | -14 |
| `query_logs` | 13 | 0 | -13 |
| `audit_logs` | 37 | 25 | -12 |
| `enterprises` | 13 | 0 | -13 |
| `enterprise_users` | 5 | 0 | -5 |
| `mission_completions` | 5 | 0 | -5 |
| `quota_records` | 4 | 0 | -4 |
| `game_profiles` | 3 | 0 | -3 |
| `trust_attestations` | 3 | 0 | -3 |
| `users` | 10 | 7 | -3 |
| `fleets` | 2 | 0 | -2 |
| `fleet_members` | 2 | 0 | -2 |
| `value_imprints` | 2 | 0 | -2 |
| `orders` | 0 | 2 | **+2** |
| `challenge_rounds` | 3 | 2 | -1 |
| `micro_feedback` | 2 | 1 | -1 |

**一行未动**：`poems` 58,059、`documents` 30、`columns` 1、`driver_profiles` 1，
以及 Round 7 认定的 25 张主键 md5 相同表中的其余表。
PG 上闭环之外的 6 家名称是 4×「测试用户-免费版」、1×「测试用户-支持者」、1×「演示科技」。Win 收尾拍板：这 6 家也删，企业侧归零。SG 侧 12 条 E2E/调试残留与 6 条闭环仍不并入。

**users 7 = 6 个共同 UUID + zervi@genz.ltd**（`aac7a4fc-2d9e-45fc-af87-65e31da76f21`，
带 SG 的 `password_hash` 整行插入，其 2 条 orders 随行）。

一个值得注意的实测结论：**国内 PG 侧的业务数据 100% 属于那 4 个 beta 账号**
——timeline 19/19、trust_memories 28/28、applications 23/23 全部指向它们。所以"深清"的实质是
国内侧的旧业务数据整体离场。

---

## 4. 执行前置（未做，与 Round 8 相同）

0. **导入 SG 数据到目标库的 `sg` schema**：G0 会在缺 `sg` 时直接 FAIL。
   `scripts/merge/load_sg_snapshot_to_pg.py --sqlite <快照> --psql "<psql 前缀>"`（只写 sg schema）。
1. **再打一份 pg_dump**（执行前的新鲜备份）—— 这次是**删除**操作，备份比 Round 8 更重要。
2. **停写**：长事务会持锁。
3. 跑 `psql -v poems_gate=allow_created_at_skew -f datasource-clean-draft-round9.sql`，
   逐条核对 NOTICE 行数与第 3 节清单，确认后再手工 `commit;`（脚本末尾的 `commit;` 默认注释掉）。

## 5. 本轮未做 / 待确认

1. **未对生产库执行任何写入**。
2. **poems 闸门保持 strict，执行时必须显式放行。** Win 2026-10-22 收尾确认：不加 `-v poems_gate=allow_created_at_skew` 就在 G1 STOP。本机已核对 58,059 行差异只在 `created_at`，业务列无差异。CLEAN 模式不写 `poems`，开关只确认源数据仍是这份，不改脚本默认值。
3. **zervi 在 SG 侧的 `product_events` 3 行不随行**（Round 8 拍板只带 2 条 orders）。
   要带的话解开脚本里那段注释即可。
4. **6 家测试/演示企业已纳入步骤 3 的删除名单**，G3 期望 `enterprises = 0`。全流程尚未在 55432 上重跑。
5. **`micro_feedback` 剩下的 1 行属于 guest 伪 ID**（`guest-28ad1ce4-814`，不在 users 表），未删。
