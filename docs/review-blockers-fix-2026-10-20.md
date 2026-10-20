# App Review 阻断项修复记录（2026-10-20）

> 上一轮评估出 3 条必拒 + 3 条高概率。本文只记录**已经动手做完并验证过**的部分，
> 以及**必须由其他系统/其他人完成**的部分，不含推测。

---

## 一、已完成并验证

### 1. IAP 两表 —— 解 3.1.1 必拒

**做法**：不新写 DDL，直接复用 `payment-service/schema_v0.1.sql`（它本来就是 `IF NOT EXISTS` 形式）。

**验证方式**：把 `genzltd_20261020.sql.gz` 还原成 `genzltd_sim`（PG 17.11，57 表 / users 10 行 / poems 58,059 行，owner=looma），在真实结构上执行。

| 检查项 | 结果 |
|---|---|
| 首次执行 | 零错误，两表 + 10 索引建成 |
| 二次执行 | 全部 `already exists, skipping`（幂等） |
| RLS | `relrowsecurity = t`，无 policy（对 anon/authenticated 全拒） |
| 与生产既有表冲突 | 无（生产库原本 0 张 `iap_*`） |

**生产执行**（唯一必须在提审前跑的 DDL）：

```bash
psql "$GENZLTD_DATABASE_URL" -v ON_ERROR_STOP=1 -f payment-service/schema_v0.1.sql
```

> ⚠️ 不要用 `shared/genzltd_schema_v0.3.sql` 代替：它全文 `CREATE TABLE IF NOT EXISTS`，
> 对已有 50 张表会静默跳过，看起来"同步成功"其实什么都没做。

### 2. 账号删除 —— 解 5.1.1(v) 必拒（库层）

**新增** `shared/account_delete_v0.1.sql`，三个函数：

| 函数 | 作用 |
|---|---|
| `account_purge_policy()` | 声明式清单，41 条（37 条有外键 + 4 条无外键的漏网表） |
| `account_purge_preview(uid)` | 只读预演，逐表报会影响多少行 |
| `account_purge(uid)` | 多轮收敛删除，最后删 `users` 行 |
| `account_purge_audit()` | 自检：列出"像用户数据但未被 policy 覆盖"的表 |

**为什么是多轮收敛而不是写死顺序**：表与表之间还有 115 条 FK（如 `match_consensus → candidates`），
写死顺序在表结构演进后必然失效。收敛式对任意拓扑成立，新增表只需补一行 policy。

**在真实数据上实测**（事务内执行后回滚，未改动任何真实行）：

| 账号 | 数据量 | 结果 |
|---|---|---|
| `beta_admin@looma.test` | 23 applications / 27 trust_memories / 19 product_events / 14 timeline_events | 第 1 轮全清，第 2 轮删 users，剩余 9 个用户 |
| `driver-official` | **58,059 首诗词** | poems 全部保留，`publisher_id` 置 NULL，users 行删除 |

**自检结果**：`account_purge_audit()` 返回 0 行（清单完整）。

**发现的漏网表**（原方案未覆盖，靠自检抓出来的）：
`llm_usage`、`micro_feedback`、`product_events`、`query_logs` —— 这四张有 `user_id` 但**没有外键**，
FK 挡不住、也不会报错，删号后会静默残留。已加入 policy，并让 `account_purge_audit()` 持续兜底。

### 3. role 词汇归一 —— 解 2.1 高概率

**改动**：`user-service/src/models/user.rs` 新增 `UserRole::from_any()`，`user_service.rs` 改用它。

| 输入 | 归一结果 |
|---|---|
| candidate / **user** / **driver** | Candidate |
| recruiter / **enterprise** | Recruiter |
| admin | Admin |
| observer / 空 / 其它 | `None` → 400（不兜底成 candidate） |

保持不变：`0c40b6f` 的既定语义"未知 role 不落 candidate"。

**顺带修掉一个既有缺陷**：`user-service/src/utils/jwt.rs` 的测试模块本来就编译不过
（`Uuid` 传给了要 `String` 的 `generate_token`，`assert_eq!` 也比错了类型）——
等于这个包的测试此前从未跑起来过。现已修复。

```
test result: ok. 6 passed; 0 failed
```

---

## 二、必须由其他系统完成（本仓库做不到）

### A. 删号的第二步：吊销认证（**不做等于没删**）

`account_purge` 只删业务库的 `users` 行。认证仍在 Supabase/Auth 侧，
旧 JWT 在过期前依然有效，而 `get_profile` 是"按 sub 建档"——**下次请求会把刚删的用户重新插回来**。

所以删号必须是两步，缺一不可：

```bash
# 1) 业务库
psql "$GENZLTD_DATABASE_URL" -c "BEGIN; SELECT * FROM account_purge('<sub>'); COMMIT;"

# 2) 认证侧
curl -X DELETE "$SUPABASE_URL/auth/v1/admin/users/<sub>" \
  -H "apikey: $SUPABASE_SECRET_KEY" \
  -H "Authorization: Bearer $SUPABASE_SECRET_KEY"
```

顺序建议：先在认证侧禁用（不是删除）→ 再删业务行 → 最后删认证用户。
反过来会出现"业务行已删、token 还能进、档案被重建"的窗口。

### B. 删号接口的落点不在本仓库

各服务实际连的库：

| 服务 | 连的库 |
|---|---|
| user-service | MySQL `adirp_user` |
| auth-service | MySQL `adirp_auth` |
| payment-service | PostgreSQL `genzltd`（唯一） |

`genzltd` 的账号服务不在这个 Rust 仓库里。建议契约：

```
DELETE /v1/account          # 鉴权后执行；返回 204
GET    /v1/account/preview  # 只读预演，前端二次确认用
```

服务端步骤：校验 token → `account_purge_preview` 供确认 → `BEGIN; account_purge(sub); COMMIT`
→ 删认证用户 → 吊销 token。

### C. Sign in with Apple —— 落点已就绪

生产库 `public.user_identities(id, user_id, provider, provider_uid, provider_email, provider_name, metadata_json, created_at)`
**已存在且为 0 行**。接 Apple 登录只需插 `provider='apple'`，无需改表结构。
前置仍是 App Store Connect 的 Service ID / .p8 / 回调地址。

---

## 三、明确不动

- 82 处 `text DEFAULT now()` 时间列 —— 审核不看 schema，改动风险大于收益
- 120 个索引 / v0.3 缺的 7 张表 —— 同上
- 角色词汇在**数据库层**未统一（只在服务层归一）—— 统一需要数据迁移，不在提审窗口内

---

## 四、执行顺序建议

1. 生产库跑 `payment-service/schema_v0.1.sql`（10 分钟，唯一硬阻塞）
2. 部署 role 归一（半天，挡审核员注册错位）
3. 在 genzltd 账号服务实现 `/v1/account`（1～2 天）
4. 接 Apple 登录（依赖 ASC 配置）
5. 订阅页合规文案

前两步做完，"提交即拒"的三条里就只剩 Apple 登录（依赖你的 ASC 配置）。
