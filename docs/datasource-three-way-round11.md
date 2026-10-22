# Round 11 · 三方互补的收法，以及请 Mac 只读复核的四件事

> 时间：2026-10-22 21:40（北京）
> 基线：`main` = `d780691`（PR #22）。本轮不写任何生产库。
> 回答 Round 10 留下的两句，并修正其中一句损失判断。

## 1. 已核对的事实

Round 10 写：SG 独占 4 个真实注册用户，Mac 和腾讯云都没有；因此选 PG 会丢掉这 4 个人。

`main` 上更早的拍板表已经写了相反的结论。`docs/datasource-authority-mac-response.md` 第 3.1 节：

| email | UUID | 两侧 |
|---|---|---|
| `reed.xia@gmail.com` | `1b52a63b-fd8d-49d4-b059-bf8ad5512eee` | SG=PG |
| `wcy0811@sjtu.edu.cn` | `65674880-dd6f-457a-9748-d68f4cdfa1b4` | SG=PG |
| `596099829@qq.com` | `1797374a-73d1-46a8-8c6a-dac5e9ebd020` | SG=PG |
| `347399@qq.com` | `5357e927-6085-4441-a15b-3560c62af114` | SG=PG |
| `zervi@genz.ltd` | `aac7a4fc-2d9e-45fc-af87-65e31da76f21` | 仅 SG |

Win 本机又对了一遍，结果与这张表一致：

- SG 侧用的是 10-22 快照 `looma-snapshot-20261022.db`（`6b08bdae…79b509`）。上述 5 个邮箱都在，UUID 与表中相同。`users` 11，`orders` 2，`llm_usage` 0，`signal_outbox` 0，`applications` 9。
- PG 侧用的是本机库 `genzltd_parity_1020`，由 `genzltd_20261020.sql.gz`（sha256 `fe45ba12…7641`，7,804,827 字节）还原。这不是 10-22 生产转储（那份是 `8c50e17d…0aba`）。4 个真实邮箱的 UUID 与 SG 逐字相同。`zervi@genz.ltd` 不在该库。`users` 10，`orders` 0，`applications` 23。

因此：这 4 个账号 Mac 没有，腾讯云 PG 有，而且和 SG 是同一个 UUID。选 PG 不会丢掉他们。SG 相对 PG 仍要单独插入的是 `zervi@genz.ltd` 及其 2 条 orders，Round 9 草案里已经有这一步。

三方互补仍然成立，损失范围收窄为 Mac 独占、且尚未逐行定性的数据：8 条 `orders`、9 条 `llm_usage`、4 条 `signal_outbox`、3 条 `candidates`，以及 Mac 独有的测试邮箱。Mac 的 `poems` 312,220 行继续排除，不覆盖 SG/PG 的 58,059 行。

## 2. 收法

不锁定单一文件，不把 Mac 本地库升为权威源，也不对 `genzltd` 执行 `--recreate`。

1. 身份沿用第 3.1 节：两边都有的保留 PG UUID；仅 SG 有的 `zervi` 插入 PG。Mac 用户表不导入。
2. PG 上的 beta 业务和 6 家测试/演示企业仍按 `fff03d1` 的草案删除。`applications` 不以行数多的一方为准。
3. `signal_outbox` / `signal_dlq` / `llm_usage` 在目标库保留表。清单第 6 项的「必留」指这三张表还在，不指 Mac 上现有的行必须导入。这 13 行默认不导入。
4. Mac 的 8 条 `orders` 先出白名单（见第 4 节）。白名单回来之前，不把 Mac 库并进 `genzltd`。

清单第 0 项与第 6 项因此不再冲突：第 0 项排除的是整份 Mac 文件；第 6 项留下的是表。

## 3. 对 Round 10 两句的答复

**PR #21 先不合。** `docs/single_source_authority_lockdown.md` 的第 3 项仍允许在「绿 + 最新 + 不落后」时锁定单一权威源。Round 10 已证明没有一方是超集，这三条不能同时满足。合进去会和 `d780691` 对着干。请 Mac 改完第 3 项（没有超集就停，禁止 `--recreate`）和第 6 项（必留的是表，不是 Mac 那 4 行）后再合，或关 #21、另开修正。

**`fff03d1` 的 55432 回归不是这道卡点的前置。** 该提交已随 PR #20 进入 `main`（合并提交 `beb7b9b`）。它自述尚未在 55432 重跑，这句话仍然属实。那次回归只检查两方清理草案里 `enterprises` 能否到 0，不回答三方归属。回归可以补，通过之后仍然不能对腾讯云执行。

## 4. 请 Mac 只读复核

下面四项都是只读。不要 `--recreate`，不要把 Round 9 SQL 应用到 `1.14.202.161`。

1. 在 10-22 的 `genzltd` 转储或生产只读连接上执行，核对是否仍是第 3.1 节的 UUID。若有任一邮箱缺失或 UUID 不同，停下来贴结果，本轮收法作废。

```sql
SELECT email, id
  FROM users
 WHERE email IN (
   'reed.xia@gmail.com',
   'wcy0811@sjtu.edu.cn',
   '596099829@qq.com',
   '347399@qq.com',
   'zervi@genz.ltd'
 )
 ORDER BY email;
```

2. 从 Mac 本地 `looma.db`（sha256 `73a900a0…d101b`）导出 8 条 orders，列：`id`、用户 email、金额、状态、`created_at`。不要导入。
3. 同样列出 4 条 `signal_outbox` 和 9 条 `llm_usage`：是否仍有未投递。默认结论维持「留表、不导行」，除非某一行能指到第 1 项里的真实用户且仍未投递。
4. 回写时说明 PR #21 是改完再合，还是关闭后另开。不要直接合并现在的 `ee27f93`。
