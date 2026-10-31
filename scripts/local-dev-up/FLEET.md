# 多机联调（ServBay / Dell / Surface / Mac）

默认拓扑：**一机跑栈，多机跑开发者工具**。不要四机各起一套 PG——账号、consent 落盘、简历库会对不齐。

```
Surface / Mac / Dell 开发者工具
        │  http://<栈主机局域网IP>:3333
        ▼
   栈主机（推荐 ServBay）
   api-gateway :3333 (0.0.0.0) ──► resume :3003 / stub :5200 / pg :55433
                                   （后三者只听本机回环即可）
```

| 角色 | 推荐机 | 做什么 |
| --- | --- | --- |
| **栈主机** | ServBay（已有 PG 16） | `up` / `smoke` / `down`；防火墙放行 **TCP 3333** |
| **客户端** | Surface、Mac、Dell | clone 两仓；`USE_LOCAL_GATEWAY=true` + `LOCAL_GATEWAY_ORIGIN` 指栈主机 LAN IP |

Dell / Surface 若自备 PostgreSQL 16，也可临时当栈主机；**Mac 本刀不承栈**（`up.sh` 仍绑 ServBay/`powershell` 杀端口），Mac 只做客户端。

## 栈主机一次准备

1. clone（**不要** `--single-branch`；fetch refspec 保持 `+refs/heads/*:refs/remotes/origin/*`）：

   ```bash
   git clone git@github.com:GenzLTD/zervi-rust.git
   git clone git@github.com:GenzLTD/planetx-miniprogram.git
   ```

2. 起栈并记下打印的局域网入口（或自查 WLAN IPv4）：

   ```powershell
   # Windows
   powershell -File scripts/local-dev-up/up.ps1
   powershell -File scripts/local-dev-up/open-firewall-3333.ps1   # 管理员；仅首次
   ```

3. 本机冒烟：`smoke_check.ps1` → `FAIL=0`。

4. 把 **WLAN/以太网局域网 IP**（不是 `26.x`/`172.x` 的 WSL/虚拟网卡）发给客户端同学。

网关进程本身已听 `0.0.0.0:3333`；若客户端超时，优先查防火墙，而不是改 bind。

## 客户端（Surface / Mac / Dell）

1. 同两仓 clone；`git fetch --all` 拉齐 PR 分支（单分支 clone 会看不到 `docs/local-dev-pointer` 等）。
2. 临时改 `planetx-miniprogram/packages/miniprogram/utils/config.ts`（**勿提交**）：

   ```ts
   const USE_LOCAL_GATEWAY = true
   const LOCAL_GATEWAY_ORIGIN = 'http://192.168.x.x:3333'  // 栈主机 LAN IP
   ```

3. **登录两条路**（stub 跑在**栈主机**，客户端不要起 :5200）：
   - **local_auth（Mac/华为已通）**：planetx `api.ts` 在 `USE_LOCAL_GATEWAY` 下改打 register/login（`mp-devtools@local.test` / `Passw0rd!`）。
   - **原生 wechat（Dell 零注入）**：`legacy_stub_v2` 的 `POST /v1/auth/wechat` 用 **`JWT_SECRET` + `iss=looma`** 签发真 HS256（可过 resume）。勿用未验签的 `dev-signature` 假 JWT——能登录但 career 会 401。  
   自验：`curl.exe -s -X POST http://192.168.3.81:3333/v1/auth/wechat -H "Content-Type: application/json" -d "{\"code\":\"dev\"}"` 应含 `access_token`。

4. 微信开发者工具：**不校验合法域名** → 清缓存编译 → 求职 Tab。  
   `game/profile`、`auth/profile` 回 stub 预期；`/v1/jobs/list` 应 200（需栈已起 job-service + 可选 seed-demo）；鸿蒙真机不要走 LAN HTTP。
5. 远端抽检（可选，在客户端或任意同网段机）：

   ```powershell
   $env:BASE = 'http://192.168.x.x:3333'
   powershell -File scripts/local-dev-up/smoke_check.ps1
   ```

   ```bash
   BASE=http://192.168.x.x:3333 bash scripts/local-dev-up/smoke_check.sh
   ```

6. 收工：`USE_LOCAL_GATEWAY` 改回 `false`（勿提交）；栈主机执行 `down`。

### Mac 注意

- 只需 Node/微信开发者工具 + 两仓源码；**不要**在 Mac 上跑当前 `up.sh` 当栈。
- 若 DevTools 在虚拟机/隔离网络，确认能 `curl http://<栈IP>:3333/health`。

### Surface 注意

- 与 Dell 相同：Windows 客户端改 `LOCAL_GATEWAY_ORIGIN` 即可。
- 公司/家庭 Wi‑Fi 访客隔离会导致互访失败——Surface 与栈主机须同一可互访网段。

## 核实纪律（多 clone）

声明 stash / 分支 / 工作区是否干净时，必须写明 **哪台机、哪份绝对路径**。  
Dell 侧单分支 fetch、缺 remote-tracking，是 clone 配置差，不是远端或 PR 坏了。

## 已知空白（与单机相同）

| 缺口 | 现象 |
| --- | --- |
| `/v1/jobs/match` | LLM 即席生成，不查职位库（list 才是真库） |
| 真机预览 | 仍受微信合法域名与 HTTP 限制；本路径优先 **开发者工具** |
| Mac 作栈主机 | 另刀（Homebrew PG + 去 PowerShell 依赖） |
