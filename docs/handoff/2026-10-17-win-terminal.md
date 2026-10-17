# Win 侧执行汇总报告（交 Mac 侧核对）

> 日期：2026-10-17 ｜ Win 侧（Dell / 深圳家网）｜ 严守边界：未碰法兰克福 `45.76.84.90` / Vultr / Looma 契约副本；未改消费方 pin；`feat/zervi-rust-gap-close` 工作区未动。

## 0. 基线事实（已核对，双方一致）
- 契约在 `feat/gfcc-1.3.0` 的 `3dcb084`；消费方 pin 仍停 `435c80b`。
- `zervi-rust` main = `244b07f`；终端分支 `127e4fc`；cortex-mcp 用 `9293785`（不用 `b57de32` 旧二进制）。
- main 与终端分支 `POST /v1/ask` 注入 `X-User-Id = sha256(sub)`；1.3.0 里两边只靠原始 `actor_sub` 关联，这一跳未打过。

## 1. Task 1 — 联调（Looma SaaS ↔ 终端，Win 本机）
> 工作区固定 zervi-rust main `244b07f`；本机起 Tatha，`QUANTUM_AUTH_URL` 指 `127.0.0.1` 网关 `GET /v1/auth/verify`；用 `GATEWAY_JWT_SECRET` 签 HS256，claims 仅 `{sub, role, name, phone}`。

| 用例 | 结果 | 说明 |
|---|---|---|
| 有 sub（带 `exp`）打 `POST /v1/ask` | **200** 且**转发**到下游 | 下游 `x-user-id = ab5e523131be858dd50fa55e8520952795978adbef1597c3c3ad0d654b2f220d` = `sha256("user-win-001")` |
| 无 sub 打 `POST /v1/ask` | **401** 且**不转发** | 缺 sub → 解码失败 → 不转发 |
| `GET /v1/auth/verify` | `{"tier":"free","user_id":"ab5e5231…"}` | 返回 `sha256(sub)` |

- ⚠️ **关键坑**：网关 `jsonwebtoken` 默认 `validate_exp=true`，仅 4 个 claim 无 `exp` 会被 401；补 `exp`/`iat` 后通过。
- ⚠️ **未对齐项（待下一跳处理，本轮不改 pin）**：下游身份是 `sha256(sub)`，**`actor_sub` 没有原样到达**。1.3.0 只靠原始 `actor_sub` 关联，这一跳尚未打通。下一步才改头；本轮仅记录。
- Mac 侧只需核对：终端 auth-service 签发的 JWT 是否恰为 `{sub, role, name, phone}` 四字段，且能被 Win 网关 `GATEWAY_JWT_SECRET` 验过；终端侧不改 sub 成哈希。

## 2. Task 2 — 终端自洽（Win 本机，`127e4fc` 工作区 + `9293785` cortex-mcp）

### 2.1 两道记忆门（退出码，Win/Mac 须一致）
| 门 | 命令要点 | Win 退出码 | 握手耗时 | 结论 |
|---|---|---|---|---|
| 非法 kind | `--kind bogus --text …` | **1** | 1924 ms（总 3342 ms） | ✅ 服务端 is_error「kind 必须是 (working,episodic,semantic,procedural) 之一」 |
| 空库冷启动 recall | `--verify-only --verify-query …` | **3** | 1903 ms（总 11720 ms） | ✅ `hits=0` ⇒ exit 3（原 14–24s 已修复） |

### 2.2 upload-service `/health` 路由改造（`127e4fc` 工作区）
- 把受保护路由抽成独立子路由 + `.route_layer(auth_middleware)`，`/health` 留在外层（与 user-service 同构）。
- 验证：**无 token `GET /health` = 200**；**坏 token / 无 token `POST /upload` = 401**。

### 2.3 user-service `get_profile` 按 sub 建档（upsert）
- 逻辑：以 `id = sub` 命中则直接返回（不增行）。
- **插入前两道前置校验（任一不满足都【不插入】、两个库行数都不变）**：
  1. `role` 必须是 `candidate` / `recruiter` / `admin`，否则返回 **400**（未知 role 含 `observer` 不再落成 `candidate`）。
  2. `phone` 不得已被其它 sub 占用，否则返回 **409**（不再折成 500 的 DB 唯一键错误）。
- 通过校验才插入默认行（`status=active` / `password_hash` 空串＝未设置），**绝不触碰 `adirp_auth`**。

| 断言 | 结果 | 证据 |
|---|---|---|
| ① 首次 200 且 `id == sub` | ✅ PASS | `status=200 id=user-win-profile-001` |
| ② 二次 200 且 `users` 行数不增 | ✅ PASS | 行数 `1 → 1` |
| ③ 伪造/坏 token 仍 401 | ✅ PASS | 坏 token=401，无 token=401 |
| ④ 不往 `adirp_auth` 写新行 | ✅ PASS | `adirp_auth` 库不存在；user-service 不连接 auth-service，天然不写 |
| ④' 建档行 `password_hash` 为空（未设置） | ✅ PASS | `password_hash=''` |
| ⑤ 未知 role（observer）返回 **400**、不插入 | ✅ PASS | `status=400`，行数仍为 1、`OBS_SUB` 不存在 |
| ⑥ 另一 sub 配已存在手机号返回 **409**、不插入 | ✅ PASS | `status=409`，行数仍为 1、`COL_SUB` 不存在、`adirp_auth` 仍 0 |

### 2.4 环境补充（仓库无 SQL，已自建）
- 按 `models/user.rs` 反推，在本地 MySQL 8.4（`localhost:3306`，凭据走环境变量注入，**不入库**）新建：
  - `adirp_user` 库 + `users` 表（`id VARCHAR(36) PK`、`phone VARCHAR(255) UNIQUE`、`email/avatar NULL`、`password_hash`、`name`、`role/status VARCHAR(20)`、`created_at/updated_at DATETIME`）。
  - `adirp_upload` 库（upload-service 启动需连，仅建库即可）。
- 服务以 `DATABASE_URL`/`JWT_SECRET` 环境变量覆盖默认（`user-service` 用 `config` crate `Environment` 双下划线分隔、大小写不敏感；`upload-service` 直接 `std::env::var`）。

## 3. 未做 / 待下截
- **25 例入库**：明确留待下一截，本轮未做（不在仓库）。
- **Mac 侧本机三件事**（请 Mac 自行）：非法 kind exit 1、空库冷启动 exit 3、嵌入握手耗时（原 14–24s）；不装 LLVM / llama_cpp_rs，简历解析不写 `capability_trace`。

## 4. 结论
- Task 1 联调全部符合通过标准；唯一未对齐是 `actor_sub` 哈希跳跃（已记录，下一跳改头，本轮不改 pin）。
- Task 2 记忆门退出码（1 / 3）、upload `/health`、get_profile 四条断言**全部 PASS**，Win/Mac 退出码预期一致。
- 边界严守：法兰克福 / Vultr / Looma 契约副本零触碰；pin 未改；gap-close 工作区未动。

---
附带运行中的本机服务（供复核）：`user-service` :3002、`upload-service` :3007（终端分支最新提交构建产物）。
