# Round 10 · 远端同步 + Vultr fresh dump 实测 + 三方候选首次对拍

> 时间：2026-10-22 20:41–21:05（北京）
> 触发：Win 侧论断「纸面锁定已完成且可追溯，但『真源是谁』还没用真实 fresh dump 跑出来，
>       所以离真正迁移生产还差最硬的一道坎 —— 而这道坎在 Win 上做不了，得在 Vultr 上跑。」
> 结论（先说）：**三句话里两句与实测不符，一句需要限定。列出如下。**

- 第 1 句「纸面锁定已完成且可追溯」→ **不成立（差一个 merge）**
- 第 2 句「真源是谁还没用 fresh dump 跑出来」→ **fresh dump 已跑，但结论与预设相反**
- 第 3 句「这道坎在 Win 上做不了，得在 Vultr 上跑」→ **成立，但 Mac 上 10 分钟就跑完了，不是硬坎**

---

## 1. 两仓远端同步结果（先 fetch 再判，这是上次的教训）

| 仓库 | `origin/main` | 本地对齐后 | 远端分支数 | 备注 |
|---|---|---|---|---|
| `GenzLTD/zervi-rust` | `beb7b9b` | `beb7b9b` | 25 | 自上次同步前进了 2 个提交 |
| `GenzLTD/looma-zervi` | `06bc5b5` | `06bc5b5` | 38 | 无变化；`v1.4.0` tag 冲突，`--force` 解决 |

`beb7b9b` 是 **PR #20 的真两亲 merge commit**（parents `7fe272f` + `fff03d1`，
`merge_commit_sha=beb7b9b` ≠ `head.sha=fff03d1`，merged_by `szpeter2026`）——
这是本轮系列里第一个真正的 merge commit，不是 fast-forward。

`fff03d1`（Win 侧 `szjason72`，10-22 19:50）是**收尾拍板**，回答了 Round 9 末尾我留的两个问题：

1. **poems 闸门保持 strict**：执行必须显式 `-v poems_gate=allow_created_at_skew`，不改默认值。
2. **PG 剩下那 6 家也删**：`测试用户-免费版 ×4`、`测试用户-支持者 ×1`、`演示科技 ×1`；
   `enterprises` 期望从 6 改为 0，净 `-259`（58,392 → 58,133）。

> ⚠️ 该改动在提交信息里自述「尚未在 55432 重跑全流程」，截至本轮**仍未经 staging 回归**。

---

## 2. 第 1 句核验：纸面锁定**没有**完成——清单停在 OPEN 的 PR #21

`docs/single_source_authority_lockdown.md`（112 行，`ee27f93`，Win 侧 `szpeter2026` 起草 10-22 20:29）
存在于远端分支 `docs/authority-lockdown-checklist` 上，但：

```
git merge-base --is-ancestor ee27f93 origin/main  →  NOT in main

gh pr list → 21  OPEN  docs/authority-lockdown-checklist  (未合入)
```

即：**这份清单（含第 0–6 项）目前不在 main 上**，不在规范引用链里，可追溯性有缺口。
分散在 main 上的 Round 4–9 文档仍在，但「锁定清单」本身没有落地。

这修正了上一轮 Win 侧「两个仓库都没有 `single_source_authority_lockdown.md`，没有可回写的目标」
的表述——文件在 20:29 被创建了，只是还停在 OPEN 的 PR 里。

---

## 3. 第 3 句核验：这道坎在 Mac 上 10 分钟就跑完了

清单第 1 项写「我（Win 侧）无 Vultr 凭据，这一步需你或运维执行」。实测：

```
~/.ssh/config:  Host vultr-overseas → 139.180.184.25  root  ~/.ssh/vultr_overseas
ssh vultr-overseas  →  OK / genz-ltd / up 4:48
```

整条链路（定位容器 → `VACUUM INTO` → 传回 → 对拍）在本次会话内完成，**无需运维介入**。
活动库位于容器 `looma-backend` 内：

```
DATABASE_PATH = /app/data/looma.db
挂载：/var/lib/docker/volumes/looma-zervi_backend-data/_data -> /app/data (rw)
活动库：33,714,176 B，mtime 2026-10-22 12:19:44 UTC
容器内时区 = UTC（不是北京时间）
```

同一目录下还有两份**从未被纳入候选清单**的历史备份：

| 文件 | 大小 | mtime |
|---|---|---|
| `looma.db.pre-genzflip-20260916` | 27,897,856 B | 09-16 02:30 |
| `looma.db.sweep-backup-20260916-030957` | 27,910,144 B | 09-16 03:09 |

---

## 4. 第 2 句核验：fresh dump 已跑，结论与预设相反

### 4.1 取快照（只读，不触碰活动库）

```
VACUUM INTO '/tmp/looma-sg-fresh-20261022-utc.db'
  BEFORE  size=33714176  mtime=2026-10-22 12:19:44 UTC
  AFTER   size=33714176  mtime=2026-10-22 12:19:44 UTC   ← 活动库未被改动
  SNAPSHOT size=30818304
```

传回本机后复算 `sha256` = `735e08dbc03b091682925e9cd31db7c9a217c1281c53542402fe140b56ac0986`
（与远端 `sha256sum` 一致，传输完整）。

### 4.2 与 10-22 快照对拍：字节数完全相同，sha256 不同 → 定位到**唯一 1 个字节**

| 项 | 10-22 快照 | fresh 快照 |
|---|---|---|
| 大小 | 30,818,304 B | 30,818,304 B |
| sha256 | `6b08bdae…79b509` | `735e08db…ac0986` |
| **差异字节数** | — | **1**（偏移 43：`0x06` → `0x08`） |
| schema_cookie | 518 | **520** |

⇒ 活动库在 10-22 快照之后执行过 **2 次 DDL**（cookie 每次 DDL +1），
但 `sqlite_master` 217 = 217、索引 159 = 159、表定义逐字一致 ⇒ **这 2 次 DDL 净零**。

### 4.3 按清单第 2 项 rubric 逐层判定

| 层级 | 判据 | 结果 |
|---|---|---|
| 1 | `sha256` 相同 | ❌ 不同（那 1 字节） |
| 2 | **主键集合**相同 | ✅ 55 张非空表主键 md5 **全部相同**（含 `poems` 58,059） |
| 3 | 行数相同 + `created_at` 跨度相同 | ✅ 全部相同 |
| 4 | 表结构相同 | ✅ `sqlite_master` 与全部 `CREATE TABLE` 逐字一致 |

清单第 2 项原文：「`VACUUM` 改哈希但同源可用层级 2-4 兜底」⇒ **判定同源**。

### 4.4 结论

**SG 活动库自 10-22 快照以来，数据层面零变化**（13 小时窗口：
10-22 07:54 北京取快照 → 10-22 20:57 北京取 fresh）。

所以它推翻的是两件事：

- 「fresh dump 会带来新信息」→ 不会。10-22 快照是有效数据源。
- 「活动库自 10-18 后没有新的数据」→ **需要限定**：数据净变化确实为零，
  但活动库 mtime 是 12:19 UTC 且 schema_cookie 前进了 2，说明**库被打开写过，只是净效果为零**。
  「无新数据」成立，「无写入活动」不成立。

---

## 5. 真正的发现：三方候选首次对拍 —— 三方互补，没有一方是真源

清单第 0 项列了 3 个候选，但此前**只跑过 Vultr 那一方**。本轮首次把第二方（Mac 本地）拉进来对拍。

### 5.1 三方业务表行数（排除 `poems` 之外一并列出，`poems` 单独标出）

| 表 | Mac 本地 `looma.db` | SG（=10-22=fresh） | 腾讯云 PG `genzltd` |
|---|---:|---:|---:|
| `users` | **43** | 11 | 10 |
| `orders` | **8** | 2 | **0** |
| `llm_usage` | **9** | **0** | **0** |
| `signal_outbox` | **4** | **0** | **0** |
| `candidates` | **3** | **0** | 0 |
| `applications` | **0** | 9 | **23** |
| `documents` | 3 | **39** | 30 |
| `consents` | **50** | 27 | 14 |
| `timeline_events` | **73** | 68 | 19 |
| `query_logs` | 55 | **69** | 13 |
| `enterprises` | 10 | **21** | 13 |
| `trust_memories` | 31 | 29 | 28 |
| `poems`（派生表） | 312,220 | 58,059 | 58,059 |

**没有任何一方是另外两方的超集。**

### 5.2 `users` 专项：Mac 与 SG 的共同 UUID 只有 **1 个**

```
UUID   仅 Mac 有: 42   仅 SG 有: 10   共同: 1
email  仅 Mac 有: 22   仅 SG 有:  4   共同: 7
```

SG 独有、Mac 与 PG 都没有的 4 个**真实注册用户**：

```
347399@qq.com   596099829@qq.com   reed.xia@gmail.com   wcy0811@sjtu.edu.cn
```

Mac 独有的 22 个 email 大多是测试残留（`e2e-hr-*@test.local`、`poetry_ask*@local.test`、
`timeline_demo_*@looma.test`、`tspace_*@genz.local`、`gate_*@genz.local`…），
与清单第 0 项「Mac `looma.db` 伪域名 27.9%（12/43）」互相印证。

⇒ 三方**不是副本/新旧关系，而是三个独立演化的库**。
「锁定单一权威源」这个动作在当前状态下必然丢数据：

- 选 SG → 丢 Mac 的 8 `orders` / 9 `llm_usage` / 4 `signal_outbox` / 3 `candidates`
- 选 PG → 丢 SG 的 4 个真实注册用户 + Mac 的上述全部
- 选 Mac → 丢 SG 的 4 个真实用户 + PG 的 23 `applications`

---

## 6. 三句话的最终判定

| # | 论断 | 判定 | 依据 |
|---|---|---|---|
| 1 | 纸面锁定已完成且可追溯 | ❌ **未完成** | 清单 112 行在 `ee27f93`，PR **#21 仍 OPEN**，不在 main 上 |
| 2 | 真源是谁还没用 fresh dump 跑出来 | ⚠️ **限定成立** | fresh dump **已跑**且证明 SG 13 小时零数据变化；<br>但「真源是谁」确实没解决——**卡点不在 fresh dump，在三方互补** |
| 3 | 这道坎在 Win 做不了，得在 Vultr 跑 | ✅ 成立，但**不是硬坎** | Mac 上 `ssh vultr-overseas` 直连，10 分钟内跑完，无需运维 |

**重排后的真正卡点（按阻塞程度）：**

1. **三方互补如何收单源**——目前无一方是超集，任何"锁定一个"都会丢独占数据。
   这是业务归属决策，不是技术门槛。
2. **PR #21 未入 main**——纸面锁定的最后一步。
3. **Mac 本地那份 43 用户的处置**——清单第 0 项倾向排除（伪域名 27.9%），
   但它持有 SG/PG 都没有的 `llm_usage`/`signal_outbox`/`orders`，
   而清单第 6 项又说「signal_dlq/signal_outbox 建议必留」——两者直接冲突。
4. Round 9 草案的 `fff03d1` 收尾改动未在 staging 回归。

---

## 7. 本轮的方法论收获

**「sha256 不同」不等于「数据不同」。** 两个 30,818,304 字节的文件可以只差 1 个字节
（schema_cookie），而 55 张表的行数、主键集合、全行内容、表结构全部一致。
判定同源必须下沉到 rubric 的层级 2/3/4，不能停在层级 1。
——这条与清单第 2 项原文一致，本轮是第一次把它跑成实测。

**容器时区是 UTC。** `ls -la` 看到的 `Oct 22 12:19` 是 UTC，不是北京时间 12:19，
而是北京时间 20:19。跨机对时间戳前先 `date && date -u`。

---

## 8. 可复现脚本（本轮入库）

| 文件 | 用途 |
|---|---|
| `scripts/merge/take_sg_fresh_snapshot.sh` | 在 SG 活动库上 `VACUUM INTO` + 传回本机 + 两端校验指纹（只读） |
| `scripts/merge/compare_snapshots.py` | 两个 SQLite 快照全表对拍：行数 / 主键 md5 / 全行 md5 / 表结构 |
| `scripts/merge/compare_candidates.py` | 候选三方业务表对账（清单第 2 项），排除 `poems` 等派生表 |

---

## 9. 本轮对生产环境的操作边界

**全程只读。** 在 Vultr 上只做了 5 类操作：

1. `docker ps` / `docker inspect`（只读元数据）
2. `docker exec … ls` / `date` / `env`（只读）
3. `VACUUM INTO` 到容器 `/tmp`（生成新文件，**不写原库**，实测活动库 size/mtime 前后未变）
4. `docker exec … cat` 流式取回
5. `rm -f` 删除自己创建的临时快照（已清理，`/tmp` 已空）

**未触碰**：活动库本身、腾讯云 1.14.202.161 的 `genzltd`、`looma-backend` 容器状态。
SG 活动库最终状态：33,714,176 B / mtime 12:19 UTC，与操作前一致。
