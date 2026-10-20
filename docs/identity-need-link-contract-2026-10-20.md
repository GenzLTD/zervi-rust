# 身份键修补：409 need_link 契约 + supabase_uid 回填

日期：2026-10-20 ｜ 基线：`feat/apple-iap` = `2b107cd` ｜ 分支：`fix/identity-p0p1-20261020`

对应上架卡点：**P0-1 身份键断裂**（订阅记到孤儿行）、**P1-1 `set_user_tier` 静默降级**。

---

## 一、改了什么

| # | 卡点 | 改动 | 文件 |
|---|---|---|---|
| 1 | P0-1 | `resolve_principal_with(.., Provision::NoAutoCreate)`：uid 未命中时**不再新建 users 行**，返回 `409 need_link` | `shared/common/src/looma/auth.rs` |
| 2 | P0-1 | payment 的 `verify` / `restore` 统一走 `resolve_payer()`，即上面这个模式 | `payment-service/src/routes.rs` |
| 3 | P1-1 | `set_user_tier` 校验 `rows_affected == 1`；0 行 ⇒ `tracing::error!` + `DbErr::RecordNotUpdated`（→ 500） | `payment-service/src/routes.rs` |
| 4 | P0-1 | `supabase_uid` 回填脚本（dry-run / apply / verify 三段，含审计表） | `shared/backfill_supabase_uid_v0.1.sql` |
| 5 | — | 回填自包含验证（21 条断言，空库可跑） | `shared/verify_identity_link.sql` |

### 1.1 为什么是"NoAutoCreate"而不是"全禁"

`load_user` 有四条路径，真正制造"孤儿用户"的只有第 ④ 条：

| 路径 | 行为 | 新策略下的处理 |
|---|---|---|
| ① issuer=`looma` | 按 `users.id` 找 | 不变 |
| ② `users.supabase_uid == sub` | 稳定命中 | 放行 |
| ③ 未绑定但 email 命中历史行 | 回填 `supabase_uid`（**这是绑定，不是建档**） | 放行，并记 `identity: 按 email 绑定历史账号` |
| ④ 什么都没命中 | 旧代码 INSERT 新行（新 uuid / free / user） | **拒绝** → `409 need_link` |

③ 保留是有意的：这 10 个老账号靠它才能"第一次 Supabase 登录就落到老账号上"。
④ 拒绝也是必须的：Apple「隐藏我的邮箱」会让 ③ 失效，此时建档就等于把订阅写到孤儿行，
HTTP 200、日志干净、用户买完不解锁（3.1.1 拒审经典款）。

另外补了一条：生产库 `users.email` **没有 UNIQUE 约束**，若同一 email 命中多行，
不再"取第一行"，而是 `409 need_link` + `reason=ambiguous_email`，交人工裁决。

### 1.2 P1-1 的行为变化

```rust
if outcome.rows_affected != 1 {
    tracing::error!(user_id, tier, rows_affected, "…拒绝按成功返回");
    return Err(sea_orm::DbErr::RecordNotUpdated);
}
```

- `verify` / `restore`：沿用既有分支 → `500 db_error`，客户端可重试；
- webhook：`apply_transition` 返回 Err → 打 `webhook: apply failed … raw 已留痕待人工回放` 日志，
  仍回 Apple 200（Apple 要求必须 200，否则会疯狂重投）；
- 顺带把"能买到 500"的排查线索做实：日志里直接给 `user_id` + `rows_affected`。

---

## 二、客户端契约：`409 need_link`

`POST /v1/payment/apple/verify`、`/restore` 在"这个登录态没绑任何账号"时返回：

```http
HTTP/1.1 409 Conflict
Content-Type: application/json

{
  "error": "need_link",
  "message": "该登录态尚未绑定任何账号；请先显式绑定或新建账号，再重试",
  "code": "need_link",
  "reason": "unlinked_login",
  "supabase_uid": "8f3c…（JWT sub）",
  "email": "user@example.com"
}
```

`reason` 取值：

| reason | 含义 | 客户端该做什么 |
|---|---|---|
| `unlinked_login` | 该 Supabase 登录态没有任何对应账号 | 弹"绑定已有账号 / 新建账号"，完成后重试 verify |
| `ambiguous_email` | 该邮箱在 users 表对应多行 | 提示联系客服（不要自动选） |

处理要点：
1. **按 `error`/`reason` 分支，不要只看 HTTP 码**（409 也可能是其它冲突）；
2. 收到 `unlinked_login` **不要重试**，先走绑定；绑定成功后把同一次交易的 `transaction_id` 重放 verify（幂等）；
3. `reason=ambiguous_email` 属于数据问题，不要自己猜绑定对象。

> ⚠️ 绑定端点本身还没做（见"未做"第 1 条）。今天的改动先让"错的路"变成**明确报错**，
> 而不是静默写坏数据；客户端四屏可以按上面的契约先接起来。

---

## 三、supabase_uid 回填：怎么跑

现状实测（生产副本 `genzltd_b`）：`users 10 / 已绑 supabase_uid 0 / 有 email 10`。

```bash
psql "$GENZLTD_DATABASE_URL" -v ON_ERROR_STOP=1 -f shared/backfill_supabase_uid_v0.1.sql

-- ① 体检（只读）：逐行给出 ready / unmatched / ambiguous_* / conflict_*
SELECT * FROM public.identity_backfill_preview();

-- ② 试运行（同一套决策逻辑，不写库）
SELECT * FROM public.identity_backfill(false);

-- ③ 确认后写入（只写 supabase_uid IS NULL 的行）
SELECT * FROM public.identity_backfill(true);

-- ④ 复核
SELECT * FROM public.identity_backfill_verify();
```

**在 Supabase SQL Editor 里跑**时能读到 `auth.users`，会自动按小写 email 匹配 uid；
**在自建 PG 上跑**读不到 `auth.users`，全部落到 `unmatched`，需要在手工映射表里填：

```sql
INSERT INTO public.supabase_uid_backfill_map (email, supabase_uid, note) VALUES
  ('reed.xia@gmail.com', '<auth.users.id>', '手工确认'),
  …
ON CONFLICT (email) DO NOTHING;
```

安全边界（脚本层面已硬编码）：

- 只写 `supabase_uid IS NULL`，**永不覆盖**已有绑定（`already_bound_other` 只报告）；
- 候选 uid 被别的行占用 ⇒ `conflict_uid_taken`，不抢；
- 同 email 在 `auth.users` 或本地 `users` 命多行 ⇒ `ambiguous_*`，不猜；
- 手工 uid 在 `auth.users` 查不到 ⇒ `uid_not_in_auth`（除非显式 `p_allow_unverified_uid=true`）；
- 每次写入落 `identity_link_log`（`user_id` / `email` / `supabase_uid` / `action` / `source` / `actor` / `applied_at`）；
- 默认 `p_apply=false`；
- `updated_at` 按列的实际类型拼表达式 —— 生产库是 `text DEFAULT now()`（v0.3 是 `timestamptz`），
  直接 `SET updated_at = now()` 会报类型错。

---

## 四、验收（Mac 侧已跑，Win 侧可复现）

### 4.1 代码

```bash
cargo check -p common -p payment-service --all-targets   # 通过（2 条未使用导入警告为本分支原有）
cargo test  -p common --all-targets                      # 6 passed
cargo test  -p payment-service                           # 0 failed
```

### 4.2 回填脚本（空库，21 条断言）

```bash
createdb zervi_verify_identity
psql -d zervi_verify_identity -v ON_ERROR_STOP=1 -f shared/verify_identity_link.sql
# → total_checks 21 / passed 21 / failed 0 / ALL CHECKS PASSED
```

覆盖分支：`ready`、`already_bound`、`already_bound_other`、`ambiguous_email_auth`、
`ambiguous_email_local`、`unmatched`、`conflict_uid_taken`、`uid_not_in_auth`、`no_email`、
dry-run 不写库、写入后 `updated_at`（text 列）不坏、被拦行不动、重入 0 写入、日志不翻倍。

### 4.3 生产形状副本演练

`genzltd_20261020` dump 还原库（10 users / 0 绑定 / `updated_at` 为 text）：

| 步骤 | 结果 |
|---|---|
| 回填前 | `0/10` 已绑 |
| `identity_backfill(false)` | `ready 10` |
| `identity_backfill(true)` | `linked 10`，`applied=true` |
| 回填后 | `10/10` 已绑 |
| `identity_backfill_verify()` | unbound PASS / duplicate PASS / no_email PASS |
| 再跑一次 apply | `0` 行写入（幂等） |

---

## 五、没做的（按优先级）

1. **绑定/建档端点**（`POST /v1/identity/link`、`/create`）：需要"证明你拥有这个旧账号"
   （旧密码 / 邮件验证码 / 一次性 link token），不能凭 JWT 里的 email 直接认领，否则等于账号接管。
   今天先不动，是因为做半个不安全的绑定端点比不做更糟。
2. **`iap_subscriptions.user_id` 加 FK**（P1-2）：需要先清孤儿数据，且家庭共享要另设
   `(original_transaction_id)` 唯一性策略，属于 schema 迁移，不在本轮。
3. **verify 的事务化**：现在 `upsert_subscription` 成功、`set_user_tier` 失败时订阅行已落库（tier 未生效）。
   重试可自愈，但严格做法是包一个事务。
4. **`get_profile` 按 sub 重建档案**（P2-2 账号删除）：删号后仍会被重建，与 P0-1 同源，需一起改。
