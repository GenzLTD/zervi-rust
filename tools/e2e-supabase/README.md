# Supabase 端到端对拍：验收包

本目录是**可验收、可复现**的测试资产，用来验证 `feat/zervi-rust-gap-close` 这条历史上
「Supabase ES256 token → 网关验签 → 身份注入 → 伪造头剥离」整条链路。

分两阶段：

| 阶段 | 脚本 | 网络 | 凭据 | 用例 |
|---|---|---|---|---|
| A 离线 P0 | `lane-a.sh` | 不需要 | 不需要 | 21 |
| B 真 Supabase | `lane-b.sh` | 需访问 `*.supabase.co` | 需要 | 13 |

**先跑 A，A 绿了再谈 B。** A 阶段完全不碰你的 Supabase，也不产生任何外部流量。

---

## 一、为什么用 Git 传，而不是发压缩包

验收的对象有三层，各层的"一致"要靠不同手段保证：

| 层 | 风险 | 保障手段 |
|---|---|---|
| 脚本字节 | 微信/邮件传 zip 会改换行、加 BOM、丢执行位，双方看到的不是同一份文件 | **git 对象哈希**（内容寻址），文件级再叠一层 `SHA256SUMS` |
| 被测代码 | 在错误的 commit 上跑，结论无效 | `EXPECTED_COMMIT` + 运行时门禁，**漂移即拒绝执行** |
| 运行结果 | 用例集合不同却都显示"通过" | 归一化后取 `names_sha256` / `verdict_sha256`，**逐字比对** |

压缩包只能保证"看起来一样"，git 能保证"字节一样"，且事后任何一方改了哪个字节都可举证。
所以本包走独立分支推送，不覆盖、不影响 `feat/zervi-rust-gap-close` 与 `main`。

---

## 二、拿包

```bash
git fetch origin
git worktree add ../zervi-e2e test/e2e-supabase-acceptance
cd ../zervi-e2e
```

（不用 worktree 的话，`git checkout test/e2e-supabase-acceptance` 等价，但会切走当前工作区。）

该分支 = 基线 `c6a2ee3` + **仅新增 `tools/e2e-supabase/`**，不含任何被测代码改动。
`verify.sh` 会强制校验这一点。

---

## 三、三步验收

### 第 1 步：验包（约 1 秒，无需编译）

```bash
bash tools/e2e-supabase/verify.sh
```

预期末尾 `==== VERIFY PASS ====`。它做四件事：

1. `SHA256SUMS` 逐文件校验字节
2. 检查 CRLF / UTF-8 BOM（Windows 上 `core.autocrlf=true` 最容易踩的坑）
3. 基线门禁：HEAD 必须包含 `c6a2ee3`，且除本目录外无代码漂移
4. 打印环境指纹 + 必需命令可用性

### 第 2 步：A 阶段（离线 21 项）

```bash
bash tools/e2e-supabase/lane-a.sh
```

首次要编译，慢是正常的。预期：

```
summary   test result: ok. 21 passed; 0 failed; ...
cases     21
names_sha256 <一串 64 位十六进制>
PASS=21 FAIL=0 cargo_exit=0
```

`names_sha256` 是**用例名集合**的哈希，与机器无关，只要源码一致就必须完全相同。
这是本包最强的对拍信号：它证明双方跑的是同一批断言，而不是碰巧都"绿"。

### 第 3 步：回贴

把这两段原样贴回即可：

```
==== VERIFY ====
os / arch / bash / git / rustc / cargo / python
base_commit / head_commit / code_tree
==== LANE-A ====
summary / cases / names_sha256 / PASS / FAIL
```

`os`、`arch`、`bash`、`git` 允许不同；`code_tree`、`names_sha256`、`cases` 必须相同。
若有差异，先对齐再谈 B 阶段。

---

## 四、B 阶段的凭据约定

**凭据不进仓库、不进 issue、不进群聊。** 由发起方点对点传递，且优先用短时效方案：

- 方案 1（推荐）：一枚现成 ES256 `access_token`，用完即弃
- 方案 2：`SUPABASE_URL` + `SUPABASE_ANON_KEY` + 一次性测试用户 `TEST_EMAIL`/`TEST_PASSWORD`

拿到后本地这样跑（变量只在当前 shell 生效，不落盘）：

```bash
export SUPABASE_URL="https://<ref>.supabase.co"
export SUPABASE_ACCESS_TOKEN="eyJ..."      # 或方案 2 的四个变量
bash tools/e2e-supabase/lane-b.sh
```

预期输出 `cases 13`、`verdict_sha256 <hash>`、`FAIL=0`。

`lane-b.sh` 会自己拉起 `echo_server.py`（假上游，模拟 Tatha）和
`target/debug/api-gateway`，跑完清理进程。它**不会**连你的数据库，只走
`/auth/v1/.well-known/jwks.json`、`/auth/v1/token` 和网关的
`/v1/auth/verify`、`/v1/ask`。

如果不想碰真 Supabase，可以先用 `mock_supabase.py` 自建 JWKS + 密码换 token：

```bash
python mock_supabase.py &                  # 打印它监听的 URL
SUPABASE_URL=http://127.0.0.1:54321 bash tools/e2e-supabase/lane-b.sh
```

mock 只能证"逻辑通"，不能证"真 Supabase 的 JWKS 形状/签名算法可用"，两者结论不可互相替代。

---

## 五、这个包没覆盖什么（避免过度解读）

- 记忆端口 `min_role` 闸门：未接线，B 阶段不测
- `/v1/ask` 的本地 brain：未起，B 阶段走 echo 上游，只验身份注入链路
- `capability_trace` 写入：未接线
- GFCC 1.3.0 契约的 `forbidden_in_default_tools` / `weight=auxiliary`：不在本包范围

A/B 全绿的含义是：**验签、身份映射、伪造头剥离、四条负例**这条链路成立。
不等于整个 gap-close 分支可发布。

---

## 六、失败排查

| 现象 | 原因 | 处理 |
|---|---|---|
| `FATAL: 定位不到 git 仓库根` | 脚本被单独拷出仓库 | 必须在仓库内运行，或设 `REPO=<path>` |
| `FATAL: HEAD 不包含基线` | 拉错分支 / 本地缺对象 | `git fetch --all`，确认在 `test/e2e-supabase-acceptance` |
| `FATAL: 被测代码与基线不一致` | 你改过代码 | 别改，或 `ALLOW_COMMIT_DRIFT=1`（会导致结果不可比） |
| `sha256sum -c` 失败 | 文件被编辑 / 换行被改 | 重新 `git checkout -- tools/e2e-supabase` |
| `CRLF 换行` | Windows 检出配置 | 本目录 `.gitattributes` 已锁 `eol=lf`，`git config core.autocrlf false` 后重检 |
| `缺少命令 cargo` | 未装 Rust | 装 1.80+ |
| `dlltool.exe: program not found` | Windows 用的是 `*-pc-windows-gnu` 工具链，缺 MinGW binutils | 脚本会自动探测常见路径；仍失败则 `EXTRA_PATH=<mingw bin>`，或装 MinGW-w64，或 `rustup default stable-x86_64-pc-windows-msvc` |
| `:3333 未监听` | 端口被占 / 二进制未构建 | 先 `cargo build -p api-gateway`；或 `GW_PORT=13333` 双方同步覆盖 |
| `no such file api-gateway.exe` | 未编译 | `cargo build -p api-gateway` |

---

## 七、目录内容

```
verify.sh          验收入口（先跑这个）
lane-a.sh          A 阶段 21 项离线
lane-b.sh          B 阶段 13 项端到端
lib.sh             公共库：仓库定位 / Python 探测 / 环境指纹 / commit 门禁
EXPECTED_COMMIT    被测基线，门禁依据
ENV.lock           环境声明（不含密钥）
SHA256SUMS         脚本字节清单
echo_server.py     B 阶段假上游
mock_supabase.py   可选：本地 JWKS + token 签发
out/               运行产物（已 gitignore）
```
