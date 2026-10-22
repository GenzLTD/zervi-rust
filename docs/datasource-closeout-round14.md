# Round 14 · 本机收口：改正锁定清单，两道门仍开

> 时间：2026-10-23 06:22（北京）
> 基线：`main` = `380388f`（PR #21 已合入未修订的清单）
> 执行机器：Win。检出 `feat/zervi-rust-gap-close` `c6a2ee3` 未移动；改动在 `docs/datasource-closeout-round14`。

## 结论

单源迁移不能收口。本机做完了自己能做的那一步：把 `docs/single_source_authority_lockdown.md` 第 3 项改成「没有超集就停」，并写明禁止 `--recreate`。第 6 项写明必留的是表，Mac 上的现有行不导入。

下面两道门本机关不上，预发装库因此也不执行：

| 门 | 本机结果 |
|---|---|
| 法兰克福 `45.76.84.90` 可丢弃且已备份 | 未关。`~/.ssh/config` 没有这台机器。用 Vultr ops 钥以 `root` 连接，`Permission denied (publickey)`。没有改用其他钥匙，也没有登录成功。 |
| 装机当时的新鲜 `pg_dump` 上重跑 Round 9 | 未关。腾讯云 `1.14.202.161` 这台机器没有可用的 SSH 钥。本机的 `genzltd_parity_1020` 是 10-20 转储，不能代替新转储。 |

未对腾讯云 `genzltd`、Vultr 活动库、法兰克福写入。

## 清单改了什么

`380388f` 合入的第 3 项仍写着可以锁定单一权威源，并对确认过的 `genzltd` 使用 `--recreate`。这和 Round 13 已经写进 `main` 的结论相反。本轮改成：

- 三方没有超集时，不锁定单一文件。
- 不对生产库执行 `--recreate` 或 `DROP TABLE … CASCADE`。
- 预发只允许装 Round 9 清理结果，而且要等上面两道门都关上，执行时带 `-v poems_gate=allow_created_at_skew`。
- `signal_outbox` / `signal_dlq` / `llm_usage` 留表。Mac 的 8 条 orders、9 条 llm_usage、4 条 signal_outbox 不导入。

原「Win 侧无 Vultr 凭据」作废：本机有 ops 钥，SG 10-22 快照已在 Round 10 判定同源。`46ae918` 已在 `looma-zervi` `main` `06bc5b5`。
