# Round 14 · 本机收口：改正锁定清单，两道门仍开

> 时间：2026-10-23 06:22（北京）
> 基线：`main` = `380388f`（PR #21 已合入未修订的清单）
> 执行机器：Win。检出 `feat/zervi-rust-gap-close` `c6a2ee3` 未移动；改动在 `docs/datasource-closeout-round14`。

## 结论

单源迁移不能收口。本机做完了自己能做的那一步：把 `docs/single_source_authority_lockdown.md` 第 3 项改成「没有超集就停」，并写明禁止 `--recreate`。第 6 项写明必留的是表，Mac 上的现有行不导入。

新加坡活动库和腾讯云 `genzltd` 保持不写。这两处要等法兰克福预发演练成功之后才可信。

| 门 | 本机结果 |
|---|---|
| 法兰克福 `45.76.84.90` 通道 | 已通。钥匙是 `id_ed25519_vultr_staging`，用户 `root`。`~/.ssh/config` 里原先没有 Host 段，新加坡那把 ops 钥会被拒绝。本机已补上 Host `vultr-staging`。 |
| 预发库备份 | 已做。活动库 `/app/data/looma.db` 仍是 942,080 字节，mtime 未变。`VACUUM INTO` 的副本在 `/opt/vultr-genzltd-stack/backups/looma-staging-20261023.db`，sha256 `3b162fb0b675358558c29622a89b690d8ee10acdbdd26b4a589ef04453119c06`。此前该目录只有 10-13 的三份 compose 文件副本。 |
| 预发演练 | 未开始。当前 `looma.db` 只有 1 个用户 `official@szbolent.com.cn`（`driver-official`），`orders` / `enterprises` / `applications` / `poems` 都是 0。这不是清理后的 7 人、58,059 首诗那份库。`gfcc` 库 7,575 kB。nginx 在公网 80/443，`server_name` 是 `_`。 |
| 生产新鲜转储 | 不在本轮做。腾讯云和新加坡生产都不写入，也不把它们当成演练已成功的证据。 |

未对腾讯云 `genzltd` 和新加坡活动库写入。法兰克福只新增了上述备份文件。

## 清单改了什么

`380388f` 合入的第 3 项仍写着可以锁定单一权威源，并对确认过的 `genzltd` 使用 `--recreate`。这和 Round 13 已经写进 `main` 的结论相反。本轮改成：

- 三方没有超集时，不锁定单一文件。
- 不对生产库执行 `--recreate` 或 `DROP TABLE … CASCADE`。
- 预发只允许装 Round 9 清理结果，而且要等上面两道门都关上，执行时带 `-v poems_gate=allow_created_at_skew`。
- `signal_outbox` / `signal_dlq` / `llm_usage` 留表。Mac 的 8 条 orders、9 条 llm_usage、4 条 signal_outbox 不导入。

原「Win 侧无 Vultr 凭据」作废：本机有 ops 钥，SG 10-22 快照已在 Round 10 判定同源。`46ae918` 已在 `looma-zervi` `main` `06bc5b5`。
