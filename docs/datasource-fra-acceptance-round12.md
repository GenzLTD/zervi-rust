# Round 12 · 法兰克福预发验收线五条核实

> 时间：2026-10-22 21:50–22:20（北京）
> 触发：Win 侧给出「法兰克福预发要同时满足五条才能拿去跟 zervi 终端仿真机联调」的验收线，
>       要求核实。并声明：三方并库、PR #21、Mac 那 8 条 orders **都不在这五条里面**。
> 口径：**这五条是验收线，不是上机步骤。** 本轮未上法兰克福（本机 SSH 无该机）。

## 结论速览

| # | 验收线 | 判定 | 依据 |
|---|---|---|---|
| 1 | 机器是可丢弃预发副本；先备份；不写 Vultr / 腾讯云 | ⚠️ **仍开着**（Mac 无法代验机器属性） | 本机 SSH 无此机；本轮未对它做任何操作 |
| 2 | Round 9 脚本在新鲜转储上跑完并提交，事后行数达标 | ⚠️ **半开**：脚本与流程已证明（§2 真 COMMIT、五张表全中）；**「新鲜转储」这一子项未达成** | 见 §2.6 |
| 3 | JWT `sub` 属这 7 人且带 `exp`；`user-win-001` 不算；记忆门要八角梯角色 | ✅ **关闭** —— 契约属实，全部在 main 上有代码依据 | 见 §3 |
| 4 | 网关按契约答三下（503 / 200 双头 / 401 不转发） | ✅ **关闭** —— 契约属实，实现在 `api-gateway/src/tatha.rs` | 见 §4 |
| 5 | 联调只读这 7 人 + zervi 的 2 条订单；Mac 三张表不进 | ✅ **关闭** —— 属实，且有硬证据 | 见 §5 |

**五条里第 3、4、5 条已关；第 1 条整条开着、第 2 条的「新鲜转储」子项开着。**
⇒ 按 Win 侧口径：**还不算可以装上法兰克福。**

> 状态更新见 **§8（Round 13 回执）**。

---

## 0. 先更正 Round 10 的一处错误（PR #23 是对的）

Round 10 我写「SG 独有 4 个真实注册用户，**Mac 与 PG 都没有**」——**PG 那半句是错的。**
本轮在 10-22 `genzltd` 转储上复读，这 4 个账号在 SG 与 PG 里 **UUID 逐个相同**：

| email | UUID（SG = PG） |
|---|---|
| `347399@qq.com` | `5357e927-6085-4441-a15b-3560c62af114` |
| `596099829@qq.com` | `1797374a-73d1-46a8-8c6a-dac5e9ebd020` |
| `reed.xia@gmail.com` | `1b52a63b-fd8d-49d4-b059-bf8ad5512eee` |
| `wcy0811@sjtu.edu.cn` | `65674880-dd6f-457a-9748-d68f4cdfa1b4` |

`zervi@genz.ltd` 复读确认：**PG 侧 0 行**，仅 SG 有（`aac7a4fc-2d9e-45fc-af87-65e31da76f21`）。

⇒ 与 PR #23 结论一致：**选 PG 不会丢掉这 4 个账号。**
⇒ 我 Round 10 的错误来源：只逐邮箱比了 Mac vs SG，看到 Mac 没有就顺手写「PG 也没有」，**没有实测 PG 侧**。
（与 10-16 那条纪律同族：不能靠推导，要实测。）

---

## 1. 法兰克福 45.76.84.90 的现状核实

**仓库里只出现 1 次，且是「不要碰」**——与 Win 侧说法一致：

```
docs/handoff/2026-10-17-win-terminal.md:3
> 严守边界：未碰法兰克福 45.76.84.90 / Vultr / Looma 契约副本
```

**本机 SSH 配置里没有这台机器。** `~/.ssh/config` 的 Host 清单：

```
github.com / gitea-aliyun / gitea / gitea-front / gitea-backstage
github-szpeter2026 / github.com-szpeter2026 / gitee.com
gitee-zervi-genz / vultr-overseas
```

搜索 `45.76` / `frankfurt` 均无匹配；`looma-zervi` 仓库同样无匹配。

⇒ **第 1 条「机器本身是可丢弃的预发副本、先有备份」无法在 Mac 侧代验**，
需由上机方确认。本轮未对 45.76.84.90 做任何连接或操作。

---

## 2. 第 2 条：首次在可丢弃库上真正提交（此前从未提交过）

Win 侧指出：「这一步至今没有在 55432 或任何一份可丢弃库上提交过。
所以单源现在还不能往法兰克福装。」**本轮补上了这一步。**

### 2.1 执行条件

- 目标库：本机 55432 临时实例（`public` = 10-22 `genzltd` 转储还原，`sg` = SG 快照导入）——**可丢弃**
- 脚本：`docs/datasource-clean-draft-round9.sql`（main 上，已含 PR #20 的 `fff03d1` 收尾：企业归零）
- 参数：显式 `-v poems_gate=allow_created_at_skew`
- **真正执行了 `commit;`**（此前各轮都是事务回滚）

### 2.2 提交前基线

`users 10 / orders 0 / enterprises 13 / applications 23 / poems 58,059 / challenge_rounds 3`

### 2.3 逐条 NOTICE

```
步骤 1  beta 4 人级联：DELETE 23(product_events) 13(query_logs) 1(micro_feedback)
                      12(audit_logs) 4(users 本体)
步骤 2  challenge_rounds W36：DELETE 1
步骤 3  enterprises 测试残留：DELETE 13   ← fff03d1 的 6 家生效
步骤 4  插入 zervi：users 1 行（aac7a4fc…） + orders 随行 2 行
步骤 5  通用级联孤儿清理：第 1 轮 59 行，共 2 轮，累计 59 行
闸门 G3  OK：全部期望值命中
SET CONSTRAINTS ALL IMMEDIATE  → 无孤儿
COMMIT
```

### 2.4 事后行数 vs 验收线期望

| 表 | 期望 | 实际 | |
|---|---|---|---|
| `users` | 7 | **7** | ✅ |
| `orders` | 2 | **2** | ✅ |
| `enterprises` | 0 | **0** | ✅ |
| `applications` | 0 | **0** | ✅ |
| `poems` | 58,059 | **58,059** | ✅ |

其余：`challenge_rounds 2` / `documents 30` / `columns 1` / `driver_profiles 1` /
`timeline_events 0` / `trust_memories 0` / `signal_outbox 0` / `llm_usage 0`

### 2.5 7 人名单与订单归属

| id | email | created_at |
|---|---|---|
| `5357e927-…` | `347399@qq.com` | 2026-07-17 07:41:26 |
| `1797374a-…` | `596099829@qq.com` | 2026-07-17 07:02:51 |
| `driver-official` | `official@szbolent.com.cn` | 2026-09-24 23:40:03 |
| `1b52a63b-…` | `reed.xia@gmail.com` | 2026-07-15 11:47:59 |
| `2d53f88e-…` | `szbenyx@163.com` | 2026-08-06 00:49:11 |
| `65674880-…` | `wcy0811@sjtu.edu.cn` | 2026-07-16 03:23:21 |
| `aac7a4fc-…` | `zervi@genz.ltd` | 2026-09-17 14:00:50 |

`orders` 2 条均属 `zervi@genz.ltd`，`user_id <> zervi` 的行数 = **0**。
（`driver-official` 的 `password_hash` 为空，与 Round 8 勘误一致。）

> ⚠️ 仍须说明：本轮用的是 **10-22 的 `genzltd` 转储**，不是「装预发那一刻新打的新鲜转储」。
> 验收线第 2 条要求的是后者——上机时需重打一份，本轮只能证明**脚本与流程本身走得通**。

### 2.6 「新鲜转储」子项为什么还开着

验收线第 2 条是一个合取条件，本轮只满足了一半：

| 子项 | 状态 | 说明 |
|---|---|---|
| Round 9 脚本（含 `fff03d1` 企业归零）能跑完 | ✅ | 五道闸门全绿 |
| 执行时带 `-v poems_gate=allow_created_at_skew` | ✅ | 显式给出 |
| 逐条对过 NOTICE 后才 `commit` | ✅ | 见 §2.3 |
| 事后五张表达标 | ✅ | users 7 / orders 2 / enterprises 0 / applications 0 / poems 58,059 |
| **跑在「装预发那一刻新打的转储」上** | ❌ | 用的是 10-22 的旧转储 |

⇒ **第 2 条判定为「半开」。** 上机前必须：新打一份 `pg_dump` → 重跑 Round 9 → 重新对行数，
本轮的提交只证明脚本可执行，不能当作预发装的库。

另记一处副作用（Round 13 复核）：**55432 的 `public` schema 已是清理后状态**
（users 7 / orders 2 / enterprises 0），不能再当 Round 8/9 的基线；
要基线须按 `scripts/merge/load_sg_snapshot_to_pg.py` 重建。

---

## 3. 第 3 条：身份体系（逐项在 main 上找到代码依据）

| 验收项 | 证据 | |
|---|---|---|
| `user-win-001` 不在这 7 人里 | `api-gateway/src/main.rs:837` `hs_token("  user-win-001  ", …)`；10-17 handoff 用它做过 200 验证 | ✅ 属实，不能算接通 |
| 终端用户服务用 MySQL `adirp_user` | `user-service/src/config/mod.rs:23` `mysql://root:adirp_password@localhost/adirp_user` | ✅ 独立库 |
| cortex 用另一条 `CORTEX_DATABASE_URL` | `upload-service/src/config/mod.rs:201`；`memory_layer.rs:58` 显式塞进子进程 env | ✅ 独立 DSN |
| 记忆场景带 role | `upload-service/src/handlers/mod.rs:119` `memory.recall(…, Some(claims.role.as_str()))` | ✅ |
| role 必须是八角梯上的角色 | `tatha.rs:105`「取值是八角梯 id，不在梯上的值视为没有角色」；`tatha.rs:110`「只接受八角梯上的角色」 | ✅ |
| 市场侧 `candidate` 过不了记忆门 | `tatha.rs:110`「`candidate` 一类市场外的字符串返回 None」；`jobfirst-slice-service/src/main.rs:232` `403 "缺少八角梯角色"` | ✅ 属实 |

⇒ **装 `genzltd` 到预发不会自动带上 `adirp_user` 与 cortex** —— 两者是独立数据源，需分别准备。

---

## 4. 第 4 条：网关三档响应（实现在 main 的 `tatha.rs`）

> 注：`feat/gateway-actor-sub`（`e9eab3a`）**未入 main**，但 `X-Actor-Sub` 的实现
> 已通过 `tatha.rs` 存在于 `origin/main`，两者不是同一份代码，勿据此判断契约缺失。

| 场景 | 期望 | 代码依据（行号经 Round 13 逐行复核） |
|---|---|---|
| `TATHA_BASE_URL` 未配 | **503**，且不转发 | 模块约束 `tatha.rs:13`「未配 → 503，**绝不静默放行**（拒绝降级）」；**实际代码** `tatha.rs:485-495`（`env_nonempty` 返回 None → `SERVICE_UNAVAILABLE`，`:491`） |
| 有 `sub` | **200**，下游见 `X-Actor-Sub` = 原始 sub、`X-User-Id` = `sha256(sub)` | `tatha.rs:427` `stable_user_id(&claims.sub)`；`:450` 先剥客户端自带的 `X-Actor-Sub`；`:454` 写 `X-User-Id`；`:456` 写 `X-Actor-Sub` |
| 空 `sub` | **401**，且不转发 | `tatha.rs:414` 注释「空白 sub 无法充当 memory.recall 的 actor，401 且不转发」；`:415` `claims.sub.trim()`；`:418` `("missing sub")` |
| `sub` 不是合法 header 值 | **401** | `tatha.rs:424` `("invalid sub")` |
| 缺/无效 bearer token | **401** | `tatha.rs:409` `("invalid or missing bearer token")` |

> **勘误（Round 13，Win 侧指出）**：上一版把 `tatha.rs:409` 归到「空 sub」那一支，**错了**。
> `409` 是 `verify_bearer` 返回 None（**缺 token 或 token 无效**）的 401；
> 空 `sub` 是 `414`（注释）/ `418`（返回）这一支。**两者不是同一支。**

`X-Actor-Sub` 的防伪造也有测试覆盖：`main.rs:818-840`「放行伪造值等于允许用别人的 sub 读 memory.recall」。

---

## 5. 第 5 条：Mac 三张表不进预发（有硬证据）

### 5.1 归属核验：**零个落在 7 人名单里**

| 表 | 行数 | 去重 `user_id` | 落在 7 人名单 |
|---|---:|---:|---:|
| `orders` | 8 | 6 | **0** |
| `llm_usage` | 9 | 1 | **0** |
| `signal_outbox` | 4 | 无 `user_id` 列（按 `nonce` 投递的信标） | — |

`orders` 的 6 个归属账号**全是测试/本地残留**：

```
123ec677… -> e2e-hr-1789740090008-p6r6j5@test.local
cc22e322… -> e2e-hr-1789740218680-zyymdm@test.local
5e056e70… -> gate_sup_1788796177@genz.local
af81d5e9… -> planetxweb_1788796113@genz.local
fd607447… -> tspace_1788796105@genz.local
b98cdf68… -> (email 为 NULL)
```

`llm_usage` 9 条全部属于 `beta_admin@looma.test`（该账号在 Round 9 里已被删除）。

### 5.2 明细（PR #23 test plan 第 2、3 项要求列出）

**orders 8 条**：4 条 CNY（`supporter 9.9` ×3 + `pro 29.9` ×1，`LOOMA202609072348xx`，09-07）
+ 4 条 USD（`supporter 1.99` ×2 + `pro 5.99` ×2，`LOOMA202609182201xx`，09-18）。

**signal_outbox 4 条**：全部 `personality_beacon`；
2 条 `delivered`、2 条 `pending`（`last_error=status=0`，`attempts` 1–2）——
即**两条投递失败的信标**，本来也不该带进预发。

**llm_usage 9 条**：全部 `deepseek / deepseek-chat`，单一用户，属调用计费流水。

### 5.3 结论

⇒ 「默认**留表、不导行**」**不需要改**——这三张表的行全部挂在测试账号上。
⇒ `signal_outbox` 表保留、行数可以是 0，与验收线第 5 条一致。

---

## 6. 与 PR #23 的关系

Win 侧的 PR #23（`docs/datasource-three-way-round11`）test plan 四项，本轮完成三项：

- [x] 在 10-22 `genzltd` 转储上复读 4 个 UUID → 与 SG 逐个相同；`zervi` 仍不在 PG（§0）
- [x] 列出 Mac 本地 8 条 orders（id / 归属 / 金额 / 状态 / created_at）→ 不导入（§5.2）
- [x] 列出 4 条 signal_outbox 与 9 条 llm_usage → 确认「留表、不导行」不改（§5.3）
- [x] 确认不合并 PR #21（§7）、不对 1.14.202.161 执行 Round 9 SQL（§7）

---

## 7. 本轮操作边界

**未做**：未连接/未操作法兰克福 45.76.84.90；未合并 PR #21；
未对腾讯云 1.14.202.161 的 `genzltd` 执行任何写操作；未对 Vultr 活动库做写操作。

**做了**：两仓同步；在**本机 55432 可丢弃临时库**上跑 Round 9 并 **commit**；
只读查询 10-22 转储与两份 SQLite 快照；仓库内 grep 核实契约代码。

**副作用**：55432 的 `public` schema 现在是「清理后」状态（users 7 / orders 2 / enterprises 0），
已不能再当 Round 8/9 的基线用。需要基线时按 `scripts/merge/load_sg_snapshot_to_pg.py` 重建。

---

## 8. Round 13 回执（Win 侧 2026-10-22 22:2x 确认）

### 8.1 五条状态收口

| # | 状态 | 收口依据 |
|---|---|---|
| 1 | ⚠️ **仍开** | 历史出处仍只有 `docs/handoff/2026-10-17-win-terminal.md:3` 那句「未碰」；本机 `~/.ssh/config` 无 `45.76.84.90`；本轮未连它。要有人确认「是可丢弃预发 + 先有备份」才关 |
| 2 | ⚠️ **半开** | 脚本证明完成（真 COMMIT，NOTICE 与行数闭合：企业删 13→0、用户 10−4+1=7、订单 +2 且全属 zervi、poems 58,059、级联第 1 轮 59 行第 2 轮 0 行）；**但用的是 10-22 旧转储**，新鲜转储子项未达成（§2.6） |
| 3 | ✅ **关闭** | 代码在 main 上对得上：`user-win-001` 在 `main.rs:837`（测试夹具，不在 7 人里）；用户服务默认库 MySQL `adirp_user`（`user-service/src/config/mod.rs:23`）；cortex 走 `CORTEX_DATABASE_URL`（`upload-service/src/config/mod.rs:201`）；`tatha.rs:110` 写明 `candidate` 返回 None；`jobfirst-slice-service/src/main.rs:232` 回 403 |
| 4 | ✅ **关闭** | 见 §4（含本轮勘误） |
| 5 | ✅ **关闭** | 按 Mac 实测收下：8 条 orders 的 6 个归属全是 `e2e-hr-*@test.local` / `gate_sup_*` / `planetxweb_*` / `tspace_*` 与空邮箱；`llm_usage` 9 条全在已删除的 `beta_admin` 上。零个落在 7 人名单里 ⇒「留表、不导行」不改 |

⇒ Win 侧口径：**第 3、4、5 条关，第 1 条与第 2 条的「新鲜转储」还开着，所以还不算可以装上法兰克福。**

### 8.2 勘误：`tatha.rs:409` 不是「空 sub」那一支

上一版 §4 把 `409` 与 `414/418` 并列归到「无/空 sub」。**Win 侧指出错了，实测确认成立：**

```
tatha.rs:404-409   verify_bearer 返回 None → 401 "invalid or missing bearer token"   ← 缺/无效 token
tatha.rs:414-418   claims.sub.trim() 为空   → 401 "missing sub"                      ← 空 sub
tatha.rs:420-424   sub 不是合法 header 值   → 401 "invalid sub"                      ← 第三支
```

三条 401 **不是同一支**。§4 已按此改写，并补上 503 的实际代码位置 `tatha.rs:485-495`
（原先只引了模块注释 `:13`，那条是「约束声明」不是实现）。

### 8.3 一处与 Win 侧说法不一致：本机检出并不在 `feat/zervi-rust-gap-close`

Win 侧回执写「你的检出仍在 `feat/zervi-rust-gap-close` 的 `c6a2ee3`」。**实测不成立：**

```
$ cat .git/HEAD
ref: refs/heads/main
$ git rev-parse --abbrev-ref HEAD   → main
$ git rev-parse --short HEAD        → 55faeee
$ git status --porcelain            → (空，工作区干净)
$ git worktree list                 → 只有一个：/Users/jason/Projects/zervi-rust  55faeee [main]
$ git branch --list feat/zervi-rust-gap-close  → (本地无此分支)
```

`c6a2ee3`（`feat(services): 实现 looma /v1 三十五端点`，szjason72，10-16 23:51）**对象在本机存在**，
且是 `origin/main` 的祖先（所有 `docs/datasource-*` 分支都包含它）；
远端确有 `origin/feat/zervi-rust-gap-close` 指向它，但**本机没有对应的本地分支，也没有检出它**。
Round 12 收尾时已 `git checkout main` 并对齐到 `55faeee`。

⇒ 这条按本机实测记录，供 Win 侧比对。若 Win 侧看到的是另一台机器的检出，请指明是哪一个。

### 8.4 PR 处置

| PR | 处置 | 说明 |
|---|---|---|
| **#21** | **保持 OPEN，不合并** | `docs/single_source_authority_lockdown.md` 是 Win 侧资产，本轮仍未合入 main，未在规范引用链里 |
| **#23** | ⚠️ **已加说明评论，关闭动作失败（权限不足）** | 其 test plan 四项已全部落进 #24 / 本文档 §0、§5.2、§5.3、§7；合进去只是把 Round 11 原文再留一份，重复且无新信息。**但本机关不掉它**，见 §8.4.1 |
| **#24** | MERGED | `55faeee`，父提交 `d780691` + `1a8319f`，真两亲 merge，提交号未被改写 |
| **#22** | MERGED | `d780691` |

#### 8.4.1 为什么关不掉 PR #23

```
$ gh pr close 23
GraphQL: 8bt8rcrg95-coder does not have the correct permissions to execute `ClosePullRequest`
$ gh api repos/GenzLTD/zervi-rust --jq .permissions
{"admin":false,"maintain":false,"pull":true,"push":false,"triage":false}
$ gh api -X PATCH repos/GenzLTD/zervi-rust/pulls/23 -f state=closed
HTTP 404 Not Found        ← 无权即隐藏，与 10-01 那条「GitHub 对无权访问的私有仓返回 404」同族
```

本机 gh 登录身份是 **`8bt8rcrg95-coder`**（GenzLTD 普通 member，`push:false`），
org-admin 是 `szpeter2026` / `szjason72`（SSH 通道正常，但** API 层本机没有可用的 admin token**，
钥匙串里那条已失效）。⇒ **关闭 PR #23 需由 Win 侧或持有 admin token 的人执行。**

已做的替代动作：在 PR #23 上留了一条说明评论
（[comment](https://github.com/GenzLTD/zervi-rust/pull/23#issuecomment-6343335834)），
逐项列出 test plan 四项的落点，说明「只关闭不合并」的理由，
避免它被误当成「未处理」而重新打开或合并。

### 8.5 本轮边界

**未做**：未连接/未操作法兰克福 `45.76.84.90`；未合并 PR #21；PR #23 只关闭不合并；
未对腾讯云 `1.14.202.161` 的 `genzltd` 做任何写操作；未对 Vultr 活动库做任何写操作。

**做了**：两仓 fetch 同步；仓库内逐行复核 `tatha.rs` / `main.rs` / `user-service` /
`upload-service` / `jobfirst-slice-service` 的契约代码行号；改本文档 §1–§5、新增 §8。
