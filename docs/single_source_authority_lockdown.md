# 单源迁移 · 权威源锁定清单（Authority Lockdown Checklist）

> 起草：嘟嘟（Win 侧）｜2026-10-22
> 修订：Win 本机｜2026-10-23（Round 14）。第 3 项已改：三方没有超集，禁止锁定单一文件，禁止 `--recreate`。
> 配合：Mac 侧 `export_sqlite_to_pg.py` 已 hardening（`46ae918` 已在 `looma-zervi` `main` `06bc5b5`）
> 目标：在跑任何 `export_sqlite_to_pg.py` 导出 / apply 之前，先确认能不能锁定权威源，
>       避免把测试 / 种子数据灌进生产 `genzltd`（腾讯云 1.14.202.161 · looma-pgvector）。
>
> 已据 Round 2/3 实测修正的两点战略前提：
> - 业务向量 `looma_knowledge`（12 条）= `seed_knowledge.py` 的可重放种子，**不是待迁移数据**，无需向量搬运；
>   `poetry_full`（78,656）是独立硬编码 collection，仅服务 `/v1/poetry/search`，与业务不共用实例。
> - `looma.db` 体量 99.8% 是派生数据（`poems` 312,220 行，`source='imported'`，来自 `~/Projects/poetry_src`）；
>   **对账必须排除 `poems` 等派生表**，否则行数被淹没，真实业务状态仅几百行。

---

## 第 0 项（总闸）：`--strict-source` 跑一遍，红了就停

对**每一个**候选 SQLite 先跑源健全性检查，**exit 3（红）即排除该候选，绝不进入导出/apply**：

```bash
# 用已 hardening 的脚本（默认不清表；--strict-source 下伪域名>50% 直接 exit 3）
python3 backend/scripts/export_sqlite_to_pg.py \
    --sqlite <候选路径> --out /tmp/recon/<候选名> --strict-source
# 退出码：0=绿（放行候选） / 3=红（[GUARD] 告警，排除；--force 可强覆）
```

候选清单（至少三方）：
1. 实时 Vultr 生产 SQLite（fresh dump，见第 1 项）
2. Mac 本地 `/Users/jason/Projects/looma-zervi/backend/data/looma.db`
3. Win 侧备份 `E:/cloud-backup/vultr-data/looma.db`（2026-09-10 副本，仅作对照）

> 口径已校正：Mac `looma.db` 伪域名仅 **27.9%（12/43）** 应放行；Mac `genzltd`（PG，9 条 `zervi7b@test.local` 污染）才是高伪域名那份。
> **伪域名密度不能单独作为非生产判据**——红/绿只决定「能否当候选」，最终权威源靠第 2 项三方对账，不靠占比猜。

---

## 第 1 项：对实时源做 fresh 快照，别用过期副本

`E:` 是 **2026-09-10 的副本**，实时 Vultr SQLite 可能更新。在实时生产主机上先快照再导出，避免 WAL / 锁问题，且**不触碰活文件**：

```sql
-- 在生产主机 SQLite 上（只读连接内执行）
VACUUM INTO '/tmp/snapshot_YYYYMMDD.db';   -- 生成一致快照，不写原库
```
随后所有对账 / 导出都针对该快照文件。SG 活动库的 10-22 快照已经取过（`6b08bdae…79b509` 与 fresh `735e08db…ac0986` 判定同源）。本机有 Vultr ops 钥；原「Win 侧无凭据」作废。腾讯云 `genzltd` 的新鲜 `pg_dump` 这台机器仍做不了。

---

## 第 2 项：三方同口径对账（rubric）

对三个候选，仅对**业务表**（排除 `poems` 及 `source='imported'` / `publisher_id='driver-official'` 的派生表）计算：

| 层级 | 判据 | 说明 |
|---|---|---|
| 1 | `sha256` 相同 | 文件级同源；`VACUUM` 改哈希但同源可用层级 2-4 兜底 |
| 2 | **主键集合**相同 | 表内容身份一致，比行数更稳 |
| 3 | 行数相同 **+** `created_at` 跨度相同 | 业务表行数 + 时间窗双比对 |
| 4 | 表结构相同 | `CREATE TABLE` 定义一致 |

> **必须排除 `poems`**：它占 99.8% 行数，会淹没判据。聚焦真实业务状态：`users` / `consents` / `timeline_events` / `orders` / `query_logs` / `candidates` / `llm_usage` / `signal_dlq` / `signal_outbox` 等。

---

## 第 3 项：没有超集就停

Round 10 到 Round 13 已实测：Mac 本地、SG、腾讯云 PG 三方互补，没有一方是另外两方的超集。下面三条因此不能同时成立，不得选出一个 SQLite 文件当权威源：

- (a) 第 0 项绿；
- (b) 业务表 `created_at` 跨度最新；
- (c) 业务表主键集合与行数不落后于其他候选。

在没有超集时禁止：

- 把 Mac `looma.db`、SG 活动库或任何一份旧转储整库灌进 `genzltd`；
- 对腾讯云 `1.14.202.161` 的 `genzltd`、Vultr 活动库执行 `--recreate` 或 `DROP TABLE … CASCADE`。

预发副本只允许装 Round 9 清理结果（`docs/datasource-clean-draft-round9.sql`，含企业归零）。装之前还要同时满足：机器已确认可丢弃且已备份；来源是装机当时新打的 `pg_dump`；执行时带 `-v poems_gate=allow_created_at_skew`；NOTICE 对过才 `commit`；事后 `users` 7、`orders` 2、`enterprises` 0、`applications` 0、`poems` 58059。这两条在 Round 14 仍开着，所以现在也不装预发。导出脚本默认保持不清表。

---

## 第 4 项：业务向量 = 可重放种子，不是迁移数据

- `looma_knowledge`（12 条）= `scripts/seed_knowledge.py` 的 12 个 topic（overview/features/six_domains/…/promotion）一一对应，**重放 seed 即可，无向量搬运**。
- `poetry_full`（78,656）= 硬编码 `get_collection("poetry_full")` 独立实例，仅 `/v1/poetry/search` 用。
  **是否迁 poetry 是独立、可选的范围决策**，与业务单源无关；默认不纳入迁移，需单独立项。
- ⇒ 单源迁移**不含任何向量工作线**（上轮"Chroma→pgvector 大工程"已作废）。

---

## 第 5 项：apply 后对账 + 源保护复核

```bash
# 1) 目标库校验（脚本生成的 verify.sql）
docker exec -i looma-pgvector psql -U looma -d genzltd -f - < verify.sql
# 2) 业务表 PK 集合 + 行数 + created_at 跨度 与权威源再对一遍（仍排除 poems）
# 3) 源库 sha256 前后一致（参考 46ae918 实测：导出前后均为 73a900a0…d101b，未动源文件）
```

---

## 第 6 项：通过后才进入评审落地的迁移顺序

① 只读导出 + 行数对拍（已含本清单前 5 项）→ ② 决策那 5 张表（articles/columns/driver_profiles/signal_dlq/signal_outbox，
 其中 signal_dlq/signal_outbox/llm_usage **必留的是表**。Mac 上现有的行不导入：8 条 orders、9 条 llm_usage、4 条 signal_outbox 都不属于预发保留的 7 个用户）→ ③ **仅在 poetry 搜索被立项时**才做 pgvector 方案 →
④ looma `manager.py` 切 PG 或退役 → ⑤ 网关砍 fallback → ⑥ **最后**才删 SQLite。

> 不可逆的一步（删 SQLite / 砍 fallback）永远放最后；此前任何步骤都可回滚。

---

## 待办 / 责任分工

- **预发通道已通，演练未做**：法兰克福 `45.76.84.90` 用 `id_ed25519_vultr_staging` 登录。2026-10-23 已对活动库做 `VACUUM INTO` 备份（sha256 `3b162fb0…19c06`），活动库本身未改。当前库只有 `driver-official` 一名用户，还不是清理后的那份数据。新加坡活动库和腾讯云生产在演练成功前不写入。
- **已落地**：`46ae918` 在 `looma-zervi` `main`。SG 10-22 快照已判定同源。Mac 三张表的行不进预发。
- **禁止**：在第 3 项的两条门关上之前执行 `--recreate`，或把本清单旧版「锁定单一文件」当作操作许可。
