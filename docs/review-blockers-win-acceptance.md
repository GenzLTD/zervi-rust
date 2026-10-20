# 评审阻断项修复 验收单（Win 侧）

## 分支拓扑（先看这个，不要按天字面核对 SHA）

同一份改动存在于两个分支上，**代码内容逐字节相同**，差别只在历史基座：

| 分支 | 基座 | 用途 |
|---|---|---|
| `fix/review-blockers` | `feat/apple-iap`（`2b107cd`） | 首轮验收用，Win 侧已按此验过 A 组 |
| `fix/role-account-on-main` | `main`（`f00e737`） | **可合并分支**，只含 3 个提交，不带 gap-close / Apple IAP |

对应提交：

| 内容 | apple-iap 版 | main 版 |
|---|---|---|
| role 归一 + 账号删除 | `a522915` | `386e412` |
| 预检脚本 + role 证据 | `d16ff22` | `6100307` |
| 本验收单 | `876e4da` | `b5750ac` |

**所以验收时不要盯 SHA**——`git rev-parse HEAD` 在两个分支上必然不同，这不是缺陷。
用下面的"结果断言"代替：改动文件数 = 7、`cargo test` = 6 passed。

## 安全边界（先看这条）

| 操作 | 生产库可以跑吗 |
|---|---|
| `cargo test` | 不碰库，随便跑 |
| `preflight_review_blockers.sql` | **可以**，纯 SELECT |
| `account_purge_preview()` | **可以**，纯 count |
| `account_purge()` | **绝对不行**，除非包在 `BEGIN … ROLLBACK` 里，或对测试库跑 |

`account_purge` 是不可逆的硬删除。下面 C 组命令全部带 `BEGIN/ROLLBACK`，**不要去掉 ROLLBACK**。

---

## A. 代码正确性（1 分钟，无副作用）

```powershell
git fetch origin
git checkout fix/role-account-on-main     # 可合并版；要复现首轮验收则用 fix/review-blockers
git log --oneline -3
git diff --stat origin/main..HEAD          # 期望：7 files changed
cargo test -p user-service                 # 期望：6 passed
```

**结果断言**（三条全中即通过，不必对 SHA）：

1. `git log --oneline -3` 显示 3 个提交，基座是 `f00e737`
2. `git diff --stat origin/main..HEAD` = **7 files changed**，且**不含** `payment-service/` 或 `genzltd_schema_v0.3`
3. `cargo test -p user-service` = `6 passed; 0 failed`

**期望**：`test result: ok. 6 passed; 0 failed`

四个 `role_tests` 是新增的，`utils::jwt::tests` 是**修好的**——注意：这个测试模块在 `feat/apple-iap` 上本来就编译不过（`Uuid` 传给要 `String` 的 `generate_token`），
所以在 `apple-iap` 上跑同样的命令会直接报 `E0308` / `E0277`。这本身就是一条对照证据：

```powershell
git checkout feat/apple-iap; cargo test -p user-service   # 期望：编译失败
git checkout fix/review-blockers                          # 期望：6 passed
```

---

## B0. 没有生产库连接时怎么验（Win 侧当前就是这种情形）

不需要 `GENZLTD_DATABASE_URL`、也不需要 dump，任意空库即可：

```bash
createdb zervi_verify
psql -d zervi_verify -v ON_ERROR_STOP=1 -f shared/account_delete_v0.1.sql
psql -d zervi_verify -v ON_ERROR_STOP=1 -f shared/verify_account_delete.sql
dropdb zervi_verify
```

**期望末行**：`NOTICE: ===== ALL CHECKS PASSED =====`

脚本会按 policy 自动建 41 张最小表、给每张表挂 1 行（poems 挂 3 行），
再跑完整 purge 并断言四件事：users 行已删 / D 档 0 残留 /
poems 3 行完好且 `publisher_id` 置空 / iap 留痕保留。

macOS 侧实测输出：42 条删除记录（41 张表 + users），断言全过。

> 这验证的是**函数逻辑**（表名拼写、收敛、N 档语义、audit 完整性），
> 不是"生产库是否真的长这样"。后者只能在有连接时用下面的 B 组。

---

## B. 预检脚本（有生产连接时；只读，可在生产库跑）

```powershell
psql $env:GENZLTD_DATABASE_URL -f shared/preflight_review_blockers.sql
```

七段输出，重点看这几项：

| 段 | 期望 | 含义 |
|---|---|---|
| 1 | 0 行 | IAP 两表不存在 → 需要跑 schema_v0.1.sql |
| 2 | bare_fk = 37 / cascade = 3 / set_null = 1 / total = 41 | 删号难度基线 |
| 3 | 5 行（iap_subscriptions 打头） | 有 user_id 无外键的表 |
| 5 | user 8 / admin 1 / driver 1 | **role 冲突的现存证据** |

第 5 段如果 `candidate`/`recruiter` 计数为 0，就印证了 role 归一不是预防性修复。

---

## C. 删号能力（有生产连接时；事务内，必须回滚）

### C1 装载函数（幂等，可重复执行）

```powershell
psql $env:GENZLTD_DATABASE_URL -v ON_ERROR_STOP=1 -f shared/account_delete_v0.1.sql
psql $env:GENZLTD_DATABASE_URL -c "SELECT * FROM account_purge_audit();"
```

**期望**：装载零错误；`account_purge_audit()` 返回 **0 行**（清单完整）。

> 若这里报出某张表 → 说明生产库比 dump 多了新表，把它补进 `account_purge_policy()` 再验。
> 三种情况会被正常放过：`ON DELETE CASCADE / SET NULL` 的表、`iap_*`（脱敏档）。

### C2 只读预演（挑一个真实用户）

```powershell
psql $env:GENZLTD_DATABASE_URL -c "SELECT * FROM account_purge_preview('driver-official');"
```

**期望**：`poems | N | 58059`（如果 `driver-official` 仍是 58059 首诗词的作者）。

### C3 事务内真删 + 回滚（**这一步是核心验收**）

```powershell
@'
BEGIN;
SELECT count(*) AS poems_before FROM public.poems;
SELECT tbl, action, rows_affected FROM public.account_purge('driver-official');
SELECT count(*) AS poems_after FROM public.poems;
SELECT count(*) AS driver_left FROM public.users WHERE id='driver-official';
ROLLBACK;
'@ | psql $env:GENZLTD_DATABASE_URL
```

**期望**：

| 项 | 值 |
|---|---|
| `poems_before` | 58059 |
| 返回行 | `poems | N | 58059` → `users | DEL_USER | 1` |
| `poems_after` | **58059**（内容一行不少） |
| `driver_left` | 0 |

最后必须有 `ROLLBACK` 字样。**没有看到 ROLLBACK 说明事务没回滚，立刻核对 `poems` 行数。**

macOS 侧的实测记录（供对照）：`beta_admin@looma.test` 涉及 23 applications / 27 trust_memories / 19 product_events，
第 1 轮全部删除、第 2 轮删 users 行；`driver-official` 的 58,059 首诗词全部保留、仅 `publisher_id` 置空。

---

## D. 可选：role 归一的真实链路（需起服务）

需要 MySQL `adirp_user` + user-service 起得来。用一条 `role` 为 `user` 的 JWT 打 `GET /profile`：

- `feat/apple-iap`：400 `不支持的 role: user`
- `fix/review-blockers`：200，且库里新建行的 role 落成 `candidate`

若起服务成本高，跳过这组——A 组的单测已覆盖同一逻辑。

---

## E. 不通过时请回报这几项

1. 失败的组号（A/B/C/D）与**完整**报错原文
2. `git rev-parse HEAD` 的实际值
3. 若 C1 的 audit 非空：报出多出来的表名和列名
4. 若 C3 的 `poems_after ≠ poems_before`：立刻停手并贴出 `SELECT count(*) FROM public.poems;`
