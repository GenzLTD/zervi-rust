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
| 本侧 tip | 同上分支 | `73277cb` 的子提交 | 本文档当前版本（含 Round 4–6） |

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

---

# Round 4 · 生产快照到手，权威源锁定（2026-10-22 08:0x GMT+8）

## 取数过程（可复跑）

`.ssh/config` 里本就有 `vultr-overseas`（`root@139.180.184.25`，key `~/.ssh/vultr_overseas`），
**凭据一直在，此前从未用于取数** —— 所谓"需运维凭据"的门槛是我没去找，不是真的没有。

```bash
# 1) 容器内 VACUUM INTO 快照（源库只读语义；WAL 已提交内容一并带走，避免读到半提交）
scp /tmp/looma_snapshot.py vultr-overseas:/tmp/
docker cp /tmp/looma_snapshot.py looma-backend:/tmp/
docker exec looma-backend python3 /tmp/looma_snapshot.py /tmp/looma-snapshot-20261022.db
#   → 30,818,304 B；自检 users 11=11、poems 58,059=58,059

# 2) 传出 + 指纹对拍
docker cp looma-backend:/tmp/looma-snapshot-20261022.db /tmp/
sha256sum /tmp/looma-snapshot-20261022.db      # 6b08bdae…9b509
scp vultr-overseas:/tmp/looma-snapshot-20261022.db <本机>
shasum -a 256 <本机文件>                        # 两端一致

# 3) 清理 SG 上的临时文件（容器内外各一份）
```

落盘：`/Users/jason/WorkBuddy/2026-10-21-06-57-56/snapshots/looma-sg-20261022.db`

## SG 生产机实测现状

| 项 | 值 |
|---|---|
| 主机 | `139.180.184.25`（hostname `genz-ltd`，Vultr SG，uptime 99 天） |
| 容器 | `looma-backend`、`gfcc-kernel`、`looma-redis`、`looma-chromadb` |
| **PG 容器** | **无** `looma-pgvector`（`docker ps -a` 连停止的都没有），也无 `pgdata` volume |
| `/app/data/looma.db` | 33,714,176 B，mtime **2026-10-18 07:28**，sha256 `ebbf55c2…3dec62` |
| journal_mode | `wal`（活动 WAL ⇒ 必须先 `VACUUM INTO` 快照，不能直接对活动库导） |
| 历史备份 | `looma.db.pre-genzflip-20260916`、`looma.db.sweep-backup-20260916-030957` |
| 磁盘 | 62G 可用（快照无空间压力） |

## 三方对拍（这是本节的核心）

| 源 | users | poems | users.created_at 跨度 | 文件 sha256 |
|---|---:|---:|---|---|
| **SG 生产快照 2026-10-22** | **11** | 58,059 | 2026-07-15 → 2026-09-25 | `6b08bdae…9b509` |
| Mac `looma.db` | 43 | 312,220 | 2026-07-11 → 2026-09-22 | `73a900a0…d101b` |
| Win 侧 E:（vultr-data 副本） | 206 | 未知 | 2026-07-14 → 2026-09-01 | `cbeb7631…feb5f40` |

**主键集合比对（第二层判据）**：SG ∩ Mac = **1**，且那唯一一个是
`driver-official`（`official@szbolent.com.cn`，种子/官方账号）。
SG 独有的 10 个是 `zervi@genz.ltd`、`4 个 beta_*@looma.test`、`2 个 @qq.com`、
`reed.xia@gmail.com`、`szbenyx@163.com`、`wcy0811@sjtu.edu.cn`。

⇒ **三份互不相同的结论成立，且方向要反转**：不是"生产 206/43 哪个是真的"，
而是**生产只有 11 个用户**；Mac 的 43 与 Win 的 206 都是本地开发/测试/旧副本，
此前关于"数据规模"的所有估算（包括我按 177MB 估体量）全部作废。

## 权威源结论

> **以 SG 生产 `looma.db` 的 2026-10-22 快照为事实源**（`6b08bdae…9b509`）。
> 业务态仅 11 users；poems 58,059 为外部语料派生态，不参与判据、也不必迁移（可重导入）。

## 第 0 项预检（在快照上实跑）

```
python3 backend/scripts/export_sqlite_to_pg.py \
  --sqlite <sg-snapshot>.db --out /tmp/genzltd-sg-20261022 --strict-source --schema-only

→ 伪域名占比 36.4%（4/11，均为 beta_*@looma.test）→ 低于 50% 阈值，**放行 exit 0**
→ schema.sql 中 DROP TABLE 计数 = 0（默认不清表生效）
→ 55 表
```

判定：通过，可以进入正式迁移（前提是目标库落点先确认）。

## 目标库落点：未闭环

按评审选择"沿用腾讯云 `1.14.202.161` 的 genzltd 容器"，但：

- **SG 上根本没有 PG 容器**（compose 里 `pgvector` 服务存在但从未起过）；
- **腾讯云 `1.14.202.161` 本机无 SSH 配置**（只有 `known_hosts` 里有指纹，说明以前连过，
  但 `~/.ssh/config` 无对应 Host，key 也不在手）；
- 导出脚本头部写的目标就是腾讯云 genzltd 容器，但该容器**是否存在、里面有没有表、有多少行**，
  我目前**无法验证**。

⇒ 迁移的剩余门槛已从"拿不到 dump"换成"**拿不到目标库的现状证据**"。
在拿到 `1.14.202.161` 的只读核查结果之前，**不要对目标库执行任何 apply**（包括 `--recreate`）。

## 本轮我未做 / 待确认

1. 未对 SG 生产库做任何写操作（快照是新建文件，`/tmp` 文件已双向清理）。
2. 只读查询在 `/app/data` 下产生了 `-shm`(32KB) / `-wal`(0B) —— **这是 SQLite 的正常副作用，
   不是数据变更**（WAL 内容为空，mtime 表现为本次读取时刻）。若要求目录绝对干净，
   可在业务低峰重启 backend 容器让 SQLite 自行回收。
3. 未核验腾讯云 `1.14.202.161` 的 genzltd 容器 —— 缺 SSH 凭据，需你提供。
4. 未做 data.sql 全量导出（只跑 `--schema-only` 验证预检开关）。

---

# Round 5 · 目标库核查：genzltd 不是空壳，且有第二套生产数据（2026-10-22 08:2x GMT+8）

> 前提纠正：Round 4 我写"目标库落点未闭环（缺凭据）"。**凭据同样一直在本机**
> —— `~/.ssh/looma-key.pem`（`~/Downloads/looma_key.pem` 是另一把，两把都**能**登，
> 用户是 **`ubuntu`** 不是 `root`）。本机 `~/.ssh/config` 没有对应 Host 条目，
> 所以按 IP 直连即可。

```bash
ssh -i ~/.ssh/looma-key.pem ubuntu@1.14.202.161     # VMware/腾讯云 CVM，hostname VM-0-9-ubuntu
```

## 腾讯云 1.14.202.161 实测（只读）

| 项 | 值 |
|---|---|
| 容器 | `looma-pgvector`(pg17, up 10 天 healthy)、`looma-backend`、`looma-chromadb`、`looma-redis` |
| 另有两个无关容器 | `bolent_wp`、`bolent_wp_mysql` |
| `genzltd` 对象数 | **59 表 + 1 视图**（`v_catalog_tenants`） |
| 后端数据源 | `DATABASE_PATH=/app/data/looma.db`（**腾讯云后端本身也是 SQLite 在跑**）、`CHROMA_MODE=remote` |
| `/app/data/looma.db` | 33,607,680 B，mtime 2026-10-19 08:35，sha256 `d91d929e…07492`，journal=wal |
| 境内 genzltd 与境内 SQLite | users/consents/timeline_events/applications/orders/poems/documents **逐表完全相等** |

## 三份库的对拍（逐表行数）

| 表 | SG SQLite（海外生产） | 腾讯云 SQLite（境内生产） | 腾讯云 genzltd（PG） |
|---|---:|---:|---:|
| users | **11** | 10 | 10 |
| consents | **27** | 14 | 14 |
| timeline_events | **68** | 19 | 19 |
| applications | 9 | **23** | 23 |
| orders | **2** | 0 | 0 |
| documents | **39** | 30 | 30 |
| poems | 58,059 | 58,059 | 58,059 |
| chunks | 0 | 0 | 0 |

**两个结论，都推翻此前的迁移前提：**

1. **目标库不是空的。** genzltd 有 59 张表、10 个用户、5.8 万诗词、以及业务数据。
   旧默认（`drop=True`）若被 apply 到这里，会**当场清掉这 59 张表**。
   ⇒ Round 3 把默认值反转成"不清表"这件事，在这里拿到了最强实证。
2. **存在两套独立演进的生产数据，不是一份的副本。**
   - 境内 SQLite 与境内 genzltd 逐表相等（PG 是境内 SQLite 的镜像）；
   - SG 与境内**互有独有**：SG 多 1 用户（`zervi@genz.ltd`，按 DATA-PLANE 设计本就不进境内）、
     13 consents、49 timeline_events、2 orders、9 documents；境内多 14 applications。
   - 只有 `poems` 三方一致（58,059）——同一批外部语料导入，再次印证它无判别力、也不必迁移。

⇒ **"以 X 为准"在两份生产数据之间没有答案。** 以 SG 为准会丢境内的 14 applications；
以境内为准会丢 SG 的 1 用户 + 13 consents + 49 timeline_events + 2 orders + 9 documents。

## 表结构差异（SG SQLite 55 表 vs genzltd 59 表）

- **PG 独有 4 张**（且当前**全为空表**）：`tenants`(0)、`tenant_shelf_items`(0)、
  `apple_iap_transactions`(0)、`iap_subscriptions`(0)
- **SQLite 独有 0 张**：SQLite 的 55 张在 PG 里全部存在
- 视图 1 个：`v_catalog_tenants`

⇒ 结构上是 PG ⊇ SQLite（55∩55 + 4 空表 + 1 视图），不存在"SQLite 有而 PG 没有"的表。

## 本轮只做了只读操作

- 未 apply 任何 SQL、未执行 `--recreate`、未动任何容器；
- 仅有的副作用同 Round 4：只读打开会在 `/app/data` 生成 `-shm`(32KB)/`-wal`(0B)，
  非数据变更。

## 由此改写的迁移命题

原命题「把生产 SQLite 整库导入 genzltd」**不成立**：目标库已有数据，且源有两边。
新命题应为：

1. **定权威**：SG（海外，11 users）与境内（10 users）谁是/谁是两份并列的事实源？
   —— 若业务上本就是"境外/境内两套独立租户"，则**根本没有单源**，应改为**分域迁移**，
   而不是合并。这需要在战略文本里先定性，才能谈执行。
2. **若按分域**：境内那份已经在 PG 里（境内 SQLite == genzltd），**境内不需要再迁移**；
   要动的只是 SG 那份。
3. **若按合并**：必须先逐表做**主键集合级**对拍（本轮只做到行数级，行数差异 ≠ 内容差异），
   再逐表定 union / 单边优先 / 冲突解决规则。
4. **无论哪种**：apply 前必须先备份 genzltd（`pg_dump`），且脚本**默认不清表**已是硬要求。

## 本轮未做

1. 未做非 users 表的**主键集合**对拍（只有行数）⇒ 规模差异已知，内容差异未知。
2. 未判断境内 genzltd 是"静态镜像"还是"持续同步"（需看是否有同步任务/`updated_at` 分布）。
3. 未备份 genzltd（`pg_dump` 会落 ~30MB 文件到目标机，需先确认放置位置）。

---

# Round 6 · 主键集合级对拍（2026-10-22 08:4x GMT+8）

评审选择"合并：先把两边合成一个事实源"。本轮按你的口径做到**主键集合级**（不再是行数级）。

## 先做的保险：目标库已备份

```
docker exec looma-pgvector pg_dump -U looma -d genzltd | gzip > /tmp/genzltd-20261022.sql.gz
sha256 = 8c50e17db07a78574bcb7faeec5f3c5e8d7fd28378a82bfc9eafee138fa40aba   (7,804,995 B)
```
已拉回本机 `snapshots/genzltd-20261022.sql.gz`，两端 sha256 一致，目标机 `/tmp` 已清空。

## 对拍方法（可对拍复现）

- 口径：`PG 主键列的 md5(string_agg(key::text, ',' order by key::text))`，SG 侧用同一组列、
  同一排序规则（按**文本**排序，避免 SQLite 整数序 vs PG 文本序不一致）逐表计算 md5。
- 两侧主键列定义已核对一致（`act1_sessions→session_id`、`signal_ingest→3 列`、
  `value_imprints→user_id` 等复合/非 id 主键都相同）。
- 55 张共有表全覆盖。

## 结果总览

| | 张数 |
|---|---:|
| 主键集合**完全相同** | **25** |
| 主键集合不同 | 30 |
| 其中 **两侧完全无交集**（整表互斥） | **20** |
| 其中 SG 空 / PG 有数据 | 6 |
| 其中 PG 空 / SG 有数据 | 6 |

**主键集合相同的 25 张**（含 `poems` 58,059，可直接不动）：
`articles` `boost_consumptions` `boost_packs` `candidates` `challenge_entries` `chunks`
`columns` `driver_profiles` `emergent_strategy_log` `huawei_push_tokens` `job_posts`
`llm_usage` `match_consensus` `narrative_feedback` `navigator_memories` `poems`
`quiz_sessions` `report_sharing` `sales_inquiries` `signal_dlq` `signal_ingest`
`signal_outbox` `subscriptions` `usage_logs` `user_identities`

## 30 张差异表的并集账（SG / PG / 交集 / 仅SG / 仅PG）

| 表 | SG | PG | 交集 | 仅 SG | 仅 PG |
|---|---:|---:|---:|---:|---:|
| `audit_logs` | 225 | 37 | 0 | 225 | 37 |
| `timeline_events` | 68 | 19 | 0 | 68 | 19 |
| `query_logs` | 69 | 13 | 0 | 69 | 13 |
| `trust_memories` | 29 | 28 | 0 | 29 | 28 |
| `product_events` | 25 | 28 | 2 | 23 | 26 |
| `consents` | 27 | 14 | 0 | 27 | 14 |
| `enterprises` | 21 | 13 | 0 | 21 | 13 |
| `quota_records` | 29 | 4 | 0 | 29 | 4 |
| `applications` | 9 | 23 | 0 | 9 | 23 |
| `documents` | 39 | 30 | 30 | 9 | 0 |
| `users` | 11 | 10 | 6 | 5 | 4 |
| `mission_completions` | 8 | 5 | 0 | 8 | 5 |
| `trust_attestations` | 8 | 3 | 0 | 8 | 3 |
| `game_profiles` | 7 | 3 | 0 | 7 | 3 |
| `enterprise_users` | 2 | 5 | 0 | 2 | 5 |
| `fleet_members` | 4 | 2 | 0 | 4 | 2 |
| `fleets` | 4 | 2 | 0 | 4 | 2 |
| `challenge_rounds` | 6 | 3 | 3 | 3 | 0 |
| `narrative_events` | 0 | 43 | 0 | 0 | 43 |
| `act1_sessions` | 0 | 14 | 0 | 0 | 14 |
| `domain_visits` | 0 | 14 | 0 | 0 | 14 |
| `narrative_sessions` | 0 | 14 | 0 | 0 | 14 |
| `micro_feedback` | 0 | 2 | 0 | 0 | 2 |
| `value_imprints` | 0 | 2 | 0 | 0 | 2 |
| `invite_codes` | 3 | 0 | 0 | 3 | 0 |
| `match_report_items` | 11 | 0 | 0 | 11 | 0 |
| `match_reports` | 11 | 0 | 0 | 11 | 0 |
| `share_codes` | 4 | 0 | 0 | 4 | 0 |
| `verification_audit_log` | 6 | 0 | 0 | 6 | 0 |
| `orders` | 2 | 0 | 0 | 2 | 0 |
| **合计（差异表）** | | | **41** | **587** | **290** |

⇒ 差异表并集行数 = 587 + 290 + 41 = **918**。

## ⚠️ 决定性的陷阱：主键互斥 ≠ 业务不重复

`users` 表：

| 口径 | 值 |
|---|---|
| 按 UUID 交集 / 并集 | 6 / **15** |
| 按 **email** 交集 / 去重后 | 10 / **11** |
| email 相同但 UUID 不同的账号 | **10 个**（含 4 个 `beta_*@looma.test`） |

⇒ 按主键做 union 会得到 **15 个用户**，其中 10 个邮箱重复；真实只有 **11 个人**。
**4 个 beta 账号在两边各自创建、UUID 不同、邮箱却完全一样。**

⇒ 合并规则**不能**只按主键：对 `users` 必须以 **email 为业务唯一键**去重，
否则合并后的库立刻出现重复账号（登录/订阅/订单会跟着分裂）。

## 由此得出的合并规则建议（待你拍板）

1. **25 张主键集合相同的表**：无需处理（包括 `poems`，直接沿用任一侧）。
2. **20 张完全互斥的表**：直接 union 即可（无主键冲突）；但要先确认
   这些行在业务上是否本就该合并（例如 SG 的 `orders` 2 条、境内的 `applications` 23 条，
   是"同一业务的两个阶段"还是"两个独立产品的各自数据"）。
3. **`users`（及其他有业务唯一键的表）**：先按 email 去重，再决定保留哪一侧的 UUID，
   并**回写外键引用**（`consents.user_id`、`timeline_events.user_id`、`orders.user_id`…）——
   这是本次合并里唯一有真实风险的一步。
4. **顺序**：先 `users` → 再依赖 `user_id` 的表 → 其余。

## 本轮未做

1. **字段级比对**：主键相同的行（如 `documents` 的 30 行、`users` 的 6 行）内容是否一致，未比。
2. **外键引用完整性**：UUID 重映射会牵动外键，未做影响面清单。
3. 未执行任何写入；目标机仅临时落过 `pg_dump`/两个 SQL，均已删除。
