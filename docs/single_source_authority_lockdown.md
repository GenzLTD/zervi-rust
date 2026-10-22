# 单源迁移 · 权威源锁定清单（Authority Lockdown Checklist）

> 起草：嘟嘟（Win 侧）｜2026-10-22
> 配合：Mac 侧 `export_sqlite_to_pg.py` 已 hardening（commit `46ae918`，分支 `ops/pg-export-safe-default`）
> 目标：在跑任何 `export_sqlite_to_pg.py` 导出 / apply 之前，先锁定「哪个 SQLite 是权威源」，
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
随后所有对账 / 导出都针对该快照文件。**我（Win 侧）无 Vultr 凭据，这一步需你或运维执行。**

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

## 第 3 项：判定权威源

权威源 = 同时满足：
- (a) 第 0 项**绿**；
- (b) 业务表 `created_at` 跨度**最新**（数据最新）；
- (c) 业务表 PK 集合 + 行数对得上 / 不落后于其他候选。

判定后：
- 仅对**确认目标** `genzltd` 用 `--recreate`（生成 `DROP TABLE … CASCADE`）；
- 其余情况一律用**默认（不清表）** `CREATE TABLE IF NOT EXISTS`，绝不把默认 DROP 误 apply 到未确认库。

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
 其中 signal_dlq/signal_outbox 为可靠性组件，建议必留）→ ③ **仅在 poetry 搜索被立项时**才做 pgvector 方案 →
④ looma `manager.py` 切 PG 或退役 → ⑤ 网关砍 fallback → ⑥ **最后**才删 SQLite。

> 不可逆的一步（删 SQLite / 砍 fallback）永远放最后；此前任何步骤都可回滚。

---

## 待办 / 责任分工
- **Win 侧（嘟嘟）**：本清单起草；Vultr fresh dump 需你或运维跑（无凭据）。
- **Mac 侧（评审）**：认领重写战略文本「规模与工作量」段落（poems 排除 + collection 定性后的真实规模）；
  推动 `46ae918` 合入；`--no-drop` 默认化已落地。
- **共同**：两份本地 commit（`looma-zervi` `46ae918`、`zervi-rust` `7592ff8`）推送前先对齐，避免与 Win 侧未推送的
  `zervi-rust` 改动（main `1a3850a` 提交 `scripts/local-verify.sh`、本地独有 `adr-0012/gateway-dualtrack`）同名文件打架。
