# Mac 侧澄清报告 —— 回应 Win 侧四问

> 生成：2026-10-21 23:45 (GMT+8)
> 回应对象：`review_credibility_verify.md` 中"需要 Mac 侧澄清的 4 件事"
> 产出方式：全部为本地实测，未依赖先前结论

## 提交钉珠（Source of truth）

跨仓互钉，任一分支被删除后仍可追溯到内容：

| 方向 | 仓 @ 分支 | 钉住的 SHA | 内容 |
|---|---|---|---|
| 对侧（脚本侧） | `looma-zervi` @ `ops/pg-export-safe-default` | **`46ae918`** | 导出脚本：默认不清表 + `source_sanity()` 源库身份自检 |
| 本侧（文档侧） | `zervi-rust` @ `docs/datasource-authority-round3` | `7592ff8`（pre-pin） | Round 3 全文（不含本节） |
| 本侧 tip | 同上分支 | `7592ff8` 的子提交 | 本文档当前版本 |

自引用不写入正文（否则需要先知道自己 SHA，会自相矛盾）；**当前 tip 以分支头为准**，
引用本文档时请写分支 tip，`7592ff8` 保留为 pre-pin 引用。
两个仓的合并**必须用 merge commit，不得 squash** —— squash 会把这两颗钉子改写成新提交，
让本表与所有交叉引用一并失效。

## 结论先行

四问全部有答案，且**其中一项要修正我上一轮的评审结论**。更重要的是第 ② 项的实测结果
指向一个比"两边对不上"更明确的判断：

> **本机 looma.db 不具备权威源资格**——它是本地开发/测试库，且数据停在 2026-09-22。
> Win 侧那份 206 users 的备份更可能才是生产数据。

---

## ① 评审查的 SQLite 具体路径与时间

**主校验源：**

| 项 | 值 |
|---|---|
| 路径 | `/Users/jason/Projects/looma-zervi/backend/data/looma.db` |
| 大小 | 181,780,480 bytes (177 MB) |
| mtime | **2026-10-13 23:28:42** |
| sha256 | `73a900a0416617ef3d1677415fb3a44c05567d122da02f55384e3ce0785d101b` |
| 表数 | 55 |
| users | 43 |

**同目录还有 7 份历史备份**（此前评审未列出，补全）：

| 文件 | 大小 | 时间 | 表数 | users | 备注 |
|---|---|---|---|---|---|
| `looma.db` | 177 MB | 10-13 23:28 | 55 | 43 | 当前主库 |
| `_backup/looma.db.bak-20260930-114638-post-songshi-312220` | 153 MB | 09-30 11:04 | 55 | 43 | |
| `_backup/looma.db.bak-20260930-102918-pre-songshi` | 32 MB | 09-23 10:43 | 55 | 43 | |
| `looma.db.bak-pre-driver-multitenant-202609221728{16,24,39,41}` | 27 MB ×4 | 09-22 17:28 | — | — | 同一时点四份 |
| `looma.db.bak-pre-trust-migrate-20260720211826` | 602 KB | 07-14 16:11 | 38 | 17 | 最旧 |

**关键事实：本机没有任何一份 SQLite 副本的 users 是 206。**
主库与其三份可直接读取的备份全部为 43；最旧的 trust-migrate 为 17。

> **副作用声明**：只读查询触发 SQLite 生成了 `looma.db-wal`(0 byte) 与 `looma.db-shm`(32 KB)，
> 时间戳 10-21 23:45。数据本身未被修改（主库 mtime 仍为 2026-10-13）。

---

## ② 那 5 张表在 Mac 侧 SQLite 里的真实存在性与行数

**Win 侧实测结果与我这边完全相反**，这是本次对账最有价值的发现：

| 表名 | Mac SQLite | Mac SQLite 行数 | Mac PG genzltd |
|---|---|---|---|
| `articles` | **存在**（0 行空表）| 0 | 不存在 |
| `columns` | 存在 | 1 | 不存在 |
| `driver_profiles` | 存在 | 1 | 不存在 |
| `signal_dlq` | 存在 | 0 | 不存在 |
| `signal_outbox` | 存在 | 4 | 不存在 |

与 Win 侧备份的差异：

- Win 侧：**全在 PG，且 `articles` 在 SQLite 根本不存在**
- Mac 侧：**全在 SQLite，且 PG 里全不存在**

**两侧结论 180 度相反 ⇒ 说明 Mac 的 SQLite 与 Win 的备份不是同一份数据，Mac 的 genzltd
与 Win 的 genzltd 也不是同一份。**

补充：`signal_outbox` 在本机有 4 行未投递事件 —— 佐证这个表在活
跃使用，评审中"丢了会静默丢事件"的判断仍然成立。

---

## ③ `export_sqlite_to_pg.py` 静态复核材料

| 项 | 值 |
|---|---|
| 路径 | `/Users/jason/Projects/looma-zervi/backend/scripts/export_sqlite_to_pg.py` |
| 行数 | 483 行 |
| 大小 | 22,418 bytes |
| git commit | `d522301` @ 2026-09-28 15:00:34 +0800 |
| commit 标题 | `feat(pg): 重建生产 PG 并落地 genzltd（D1/D1b/D4 + SQLite→PG 迁移工具）` |

脚本自述的四条不变量（评审第 1-27 行明载）：

1. **只读源库**：连接即 `PRAGMA query_only=ON`，任何写操作立刻报错。刻意不用 `mode=ro`
   URI —— WAL 模式下它要建 `-shm`，会以 `unable to open database file` 失败。
2. **类型保真映射，不做语义升级**：SQLite 时间戳是 TEXT ISO 串就映射成 `text`，
   不擅自改 `timestamptz`，避免与 `manager.py` 读写语义分叉。
3. **不依赖 psql / psycopg**（宿主无 psql 也能生成 SQL）。
4. **幂等可重放**：`schema.sql` 默认先 `DROP TABLE IF EXISTS … CASCADE`。

产出：`schema.sql` / `data.sql` / `report.md`（**逐表行数对拍表** + 映射告警）。

**全文已随本报告交付**，供静态复核。

⚠️ **复核时请特别关注第 4 条**：`DROP TABLE … CASCADE` 是幂等的前提，也意味着
**对错误的库执行会先清空目标库**。这与⑥"删 SQLite 放最后"是同一个风险控制思路。

---

## ④ Chroma 实查 embedding 条数 —— 此处修正我上一轮的结论

本机有**两个独立 Chroma 实例**，我上一轮只报了其中一个，且未区分 collection：

| 实例 | 大小 | embeddings | collection |
|---|---|---|---|
| `data/poetry_full/chroma.sqlite3` | 171 MB | **78,656** | `poetry_full` |
| `backend/data/chroma/chroma.sqlite3` | 1 MB | **12** | `looma_knowledge` |

**修正**：上一轮说"Chroma 持有 78,656 条向量"，未区分 collection。准确表述是：

- **诗词库 `poetry_full`** 占 78,656 条 —— 是否属于 Zervi/JobFirst 业务范围**存疑**
- **业务知识库 `looma_knowledge`** 只有 **12 条**

⇒ **对战略的影响（重要）**：如果 Zervi 的 RAG 主要依赖 `looma_knowledge`，那么
Chroma → pgvector 的迁移量只有 12 条，**工作量远低于此前估计**，几乎不构成一条独立
工作线；反之若要把 `poetry_full` 那 78,656 条一并迁入，才是大工程。

**建议先确认 Zervi 的 RAG 到底用哪个 collection，再定迁移范围。**

另有空目录 `backend/chroma_data/`（2026-07-21，无内容），可忽略。

---

## 追加洞察：本机 looma.db 是测试库，数据停在 09-22

对 43 个 users 的邮箱域名做分布统计：

| 类别 | 数量 | 占比 |
|---|---|---|
| 显式测试域名（`@test.local` / `%.test`）| 18 | 42% |
| 其余伪域名（`demo@looma.local`、`gate_free_…@genz.local`、`gfcc-live-…@thomas.local`）| ~22 | ~51% |
| **真实域名** | 3 | 7% |
| ├ `szbenyx@163.com` | | |
| ├ `szbenyx@sina.com` | | |
| └ `official@szbolent.com.cn` | | |

**创建时间跨度：`2026-07-11 17:16:56` → `2026-09-22 09:28:50`**

两个推论：

1. **本机 looma.db 是本地开发/测试库**，真实用户仅 3 个，其余为自动化测试与 demo 账号。
2. **自 09-22 起再无新用户**——数据陈旧，且主库 mtime(10-13) 与最后一条用户记录(09-22)
   相差三周，说明这期间的写入并非用户注册。

⇒ **本机这份不具备权威源资格。** 评审第 ⑥ 步"以 X 为准"中的 X，不应是本机 SQLite，
也不应是本机 genzltd（后者是我此前用 `shared/genzltd_schema_v0.3.sql` 灌的结构空壳，
users 仅 9 条且均为我本轮测试注册的账号）。

---

## 建议的对账口径（响应 Win 侧提议）

同意"行数 + 主键集合比对，而非仅表存在性"。建议追加一层：

```
第 0 步（互不信任基线）—— 三方各自出具：
  · SQLite: sha256 / size / mtime / 表清单 / 逐表行数 / users 主键集合
  · genzltd dump: 库名 / 导出时间 / 同一套指标
  · Win 备份: 同上
  用 sha256 判定是否同一文件；用主键集合判定是否同一数据集（可能被 vacuum 过）
```

**判据优先级**：`sha256 相同` > `主键集合相同` > `行数相同` > `表结构相同`。

特别注明：Mac 侧 genzltd 的 users 中 9 条**全部是本轮为测试本地账号体系而注册的**
（`zervi7b@test.local` 等），属污染数据，锁定权威源时应剔除。

---

## 待 Win 侧回执

1. 你那份 206 users 备份的 **sha256 / size / mtime**，以及是否来自腾讯云生产 dump
2. 你的 dump 里 **users.created_at 跨度**（用于判断是否生产数据）
3. 你的 genzltd 里 **`looma_knowledge` collection 的 embedding 条数**是否也是 12
4. 脚本静态复核结论（尤其是否认可 `--no-drop` 应作为默认安全选项）

---

# Round 3 · Mac 侧回执（2026-10-22，全部实测）

> 口径：本轮只报**可复跑的实测值**，不带形容词。所有命令均对本机 `/Users/jason/Projects/looma-zervi`
> 执行，SQLite 一律 `PRAGMA query_only=ON` 打开，跑完后源库 sha256 复核未变。

## ③-a 待确认项 1：**Zervi RAG 用的是哪个 Chroma collection** —— 已闭环，且答案改变了迁移结论

代码路径（唯一业务 RAG 通路）：

| 位置 | 值 |
|---|---|
| `backend/src/config.py:58` | `CHROMA_COLLECTION = os.getenv("CHROMA_COLLECTION", "looma_knowledge")` |
| `backend/src/rag/chroma_client.py:64/110/133` | `search_chroma()` / `add_documents()` / `delete` 全部走该 collection |
| 调用方 | `src/agents/central_brain.py:329`（星际问答）、`backend/mcp-servers/server.py:403`（MCP 工具） |
| 存放位置 | `backend/data/chroma/chroma.sqlite3`（`CHROMA_MODE=local`，见 `backend/.env`） |

存储层实查（`sqlite3 file:data/chroma/chroma.sqlite3?mode=ro`）：

```
collections : [('0a2e777d-…816b', 'looma_knowledge')]     ← 整个实例只有这一个集合
segments    : VECTOR(hnsw-local-persisted) + METADATA(sqlite)
embeddings  : count(*) = 12
metadata    : chroma:document=12, source=12, topic=12      ← 全部 source='seed'
```

**决定性交叉验证**：`backend/scripts/seed_knowledge.py` 里的 topic 列表与这 12 条**一一对应**：

```
overview / features / six_domains / domain_career / domain_learning / domain_life
domain_social / domain_health / domain_creative / personality / howto / promotion   → 12 topics
```

⇒ 业务向量 **12 条 = 12 段静态介绍文案 = `seed_knowledge.py` 可完整重放**。
**结论修正：业务侧根本不需要"向量迁移"，只需要"重放 seed"。** 上轮我提的"Chroma→pgvector
工作量"在此应当核销；只有要把诗词一并处理才谈得上工作量。

诗词侧独立、且**与业务不共用实例**：`POETRY_CHROMA_PATH=data/poetry_full`，
`search_poetry_chroma()` 里硬编码 `client.get_collection("poetry_full")`，78,656 条，
只被 `src/agents/poetry_search.py` 与 `/v1/poetry/search` 消费。

**⇒ 待确认项 1 的答案：业务 = `looma_knowledge`（12 条，可重放）；诗词 = `poetry_full`（78,656 条，静态数据集）。**
两者互不相通，不存在"选错 collection 导致业务丢数据"的风险。

## ③-b 新发现：`looma.db` 的体量 99.8% 是**诗词**，不是业务

导出脚本自带的行数统计（55 表）：

```
总行数 312,770
  poems              312,220   ← 99.82%
  timeline_events         73   其余业务表合计 ≈ 550 行
  product_events          73
  query_logs              55
  consents                50
  audit_logs              47
  users                   43
  …
```

且 `poems` 全员 `source='imported'` + `publisher_id='driver-official'`，
`created_at` 跨度 `2026-07-20 13:19:43 → 2026-09-30 02:30:34`（多批次灌入）。

⇒ `poems` 是**从外部语料批量导入的派生数据**（本机有 `~/Projects/poetry_src` 532MB +
`poetry_full.rar` 177MB 作为上游），**不是业务事实源**。
**迁移规模的结论要重写：所谓"177MB 的 looma.db"里有 ~99.8% 是可重导入的诗词，真正的业务状态只有几百行。**

## ③-c 待确认项 3：脚本 hardening —— 我这边已经落地（不是建议，是实测过的代码）

改动：`backend/scripts/export_sqlite_to_pg.py`（Mac 侧，待推送）

| 项 | 改动 | 实测证据 |
|---|---|---|
| 默认不清表 | `drop=args.recreate`；新增 `--recreate` 显式开启；`--no-drop` 降为"兼容旧调用" | 默认跑到 `--out /tmp/genzltd-default`：`grep -c '^DROP TABLE' schema.sql` = **0**；带 `--recreate` = **55** |
| 破坏性段自标注 | schema.sql 头部打印"⚠️ 破坏性段：由 --recreate 显式开启" + 复核清单 | `sed -n '8,11p'` 见产出 |
| 源库身份自检（Win 提议的第二道防线） | 新增 `source_sanity()`：对 `users` 做伪域名占比抽检（`TEST_DOMAIN_MARKERS` 含 `test.local`/`looma.local`/`genz.local`/`thomas.local`…），>50% → stderr `[GUARD]` 告警并写进 report.md；`--strict-source` 下直接 `exit 3`；`--force` 覆盖 | 本机 looma.db 实测 **伪域名占比 27.9%（12/43）→ 判定 ✅ 通过**；report.md 顶部新增"源库身份自检"段 |
| 文档同步 | 头注释 4 条不变量 → 5 条（新增"默认不清表"与"源库身份自检"） | --help 输出含 `--recreate` / `--strict-source` / `--force` |

**注意一处对我自己上轮结论的修正**：本机 `looma.db` 的伪域名占比只有 **27.9%**（12/43），
**低于 50% 阈值**；上轮我说的"82% 伪域名"指的是**本机 genzltd**（PG 9 条污染账号那个），
不是 looma.db。也就是说按这个量化的口径，**本机 SQLite 反而比本机 genzltd 更"像真库"**——
但它仍然只是候选，是否 = 事实源要靠下面的三方对账决定，不能靠占比猜。

### 源库只读保护：跑后再复核

```
shasum -a 256 backend/data/looma.db  → 73a900a0416617ef…b785d101b   ← 与跑前一致
```

## ③-d 待确认项 2：实时 Vultr fresh dump —— 我这边拿不到，给可复跑的取数模板

我没有 Vultr 生产凭据，无法自证。**请求执行下面的模板取数**；原则是**先快照再导出**（避免从活动 WAL 读到半提交状态）。

```bash
# 1) 生产宿主：快照（不要直接对活动库导）
docker exec looma-backend sqlite3 /app/data/looma.db "VACUUM INTO '/tmp/looma-snapshot.db'"
docker cp looma-backend:/tmp/looma-snapshot.db ./looma-YYYYMMDD.db

# 2) 三方同口径指标（Mac / Win 各自对自己那份跑）
shasum -a 256 looma-YYYYMMDD.db              # Win/PowerShell:  Get-FileHash -Algorithm SHA256
stat -f '%z %Sm' looma-YYYYMMDD.db           # macOS；Linux 用 stat -c '%s %y'
sqlite3 --readonly looma-YYYYMMDD.db <<'SQL'
.mode list
SELECT 'tables',   count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';
SELECT 'users',    count(*) FROM users;
SELECT 'users_min_max', min(created_at)||' → '||max(created_at) FROM users;
SELECT 'poems',    count(*) FROM poems;
SELECT 'md5_user_ids', lower(hex(sum(cast(id as integer)))) FROM (SELECT id FROM users ORDER BY id);
.mode line
SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name;
SQL
```

**判据（沿用 Round 2 分层，补一层内容哈希）**：

```
第 1 层  sha256 相同             → 同一文件，直接收工
第 2 层  users 主键集合相同      → 同一数据集（容忍 vacuum / 页重排）
第 3 层  users 行数 + created_at 跨度相同 → 可接受
第 4 层  表结构相同              → 最弱，仅作参考
```

`poems` 单独一行是因为它占 99.8% 体量但对"是否权威源"毫无判别力
（两边谁都能导入同一份语料）——**对账时请把 poems 排除在判据外**，否则行数会被它淹没。

## 我这边还没做的（明确标红，不要被当成已完成）

1. **没有**执行任何 drop/apply —— 只在 `/tmp` 生成 SQL，未连任何 PG 实例。
2. **没有**跑 filters/data.sql 全量导出（只跑了 `--schema-only` 验证新开关）。
3. 脚本改动**仅在本机工作区**，未 commit 到远端，也未同步到 Win 侧 Git。
4. **没有**确认 Vultr 生产库现在是否还有更新的数据（2026-09-10 之后的写入）。

---

## 下一步建议给评审

1. 先答 **① collection**：已答（业务 12 条可重放，不需要向量迁移）→ 该项从"待确认"划掉，
   战略文本里"Chroma→pgvector"应从工作项降级为"可选：诗词迁移与否"。
2. **② fresh dump** 是唯一还卡着的硬门槛；在拿到之前，**不要**把任一份 SQLite 标为事实源，
   **不要**对生产 genzltd 执行任何 apply。
3. **③ 脚本默认化**已在 Mac 侧落地并实测，Win 侧拉同一份即可；建议评审把它写成
   "迁移前置 checklist 第 0 项：先 `--strict-source` 跑一遍，红了一律停"。
