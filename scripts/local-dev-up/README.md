# local-dev-up —— 本机联调一键栈

把「开发者工具一次点通」变成可复用路径：临时 PG + resume-service + api-gateway + **consent 感知的 legacy stub v2**（顶替真 Flask）。

```
小程序 USE_LOCAL_GATEWAY=true
        │
        ▼
 api-gateway :3333
   ├─ /v1/resume/*     → resume-service :3003
   ├─ /v1/auth/register|login|me → local_auth（ZERVI_DEV_AUTH=1）
   └─ 其余 fallback    → legacy stub v2 :5200
                            └─ consent status/grant/revoke 落盘
 pg :55433 / genzltd_sim
```

## 一键起 / 收尾

### Windows PowerShell（本机默认）

在 `zervi-rust` 仓根：

```powershell
powershell -File scripts/local-dev-up/up.ps1
powershell -File scripts/local-dev-up/smoke_check.ps1
# …开发者工具联调…
powershell -File scripts/local-dev-up/down.ps1
```

参数：`-SkipBuild`、`-RefreshEnv`、`-LoomaEnv <path>`。

### Git Bash

```bash
bash scripts/local-dev-up/up.sh
bash scripts/local-dev-up/smoke_check.sh
bash scripts/local-dev-up/down.sh
```

| 变量 | 含义 |
| --- | --- |
| `LOOMA_ENV` | 从哪份 looma `.env` 抽 `JWT_SECRET` / `SUPABASE_JWT_SECRET` / `DEEPSEEK_*` |
| `SKIP_BUILD=1` | 不重编，直接用现有 `target/debug/*.exe` |
| `REFRESH_ENV=1` | 强制重写 `.tmp-pg/runtime.env` |
| `PG_BIN` | 覆盖 ServBay PostgreSQL 16 bin 路径 |

运行态一律进 **`.tmp-pg/`**（已 gitignore）：数据目录、日志、PID、consent 落盘、runtime.env。

## 小程序开关

`planetx-miniprogram/packages/miniprogram/utils/config.ts`：

```ts
const USE_LOCAL_GATEWAY = true  // 本机临时
```

- 指向 `http://127.0.0.1:3333`
- **提交前必须改回 `false`**（`config.test.ts` + CI 会红）
- 开发者工具：详情 → 本地设置 → **不校验合法域名** → 清缓存并编译

详见同级清单下一节；planetx 侧速查：`packages/miniprogram/LOCAL_DEV.md`。

## 开发者工具一次点通清单

1. `bash scripts/local-dev-up/up.sh` 等到打印「栈已起」
2. `bash scripts/local-dev-up/smoke_check.sh` → `FAIL=0`
3. 小程序 `USE_LOCAL_GATEWAY = true`，不校验合法域名，重新编译
4. 启动 → splash → **求职 Tab**
5. 贴简历 → 解析  
   - 配了 `DEEPSEEK_API_KEY`：应「已入库 #…」+「我的简历」  
   - 未配 key：parse 可能失败；可先只验证模板/屏蔽/已有列表进 detail
6. 点简历条目 → detail 编辑 / 谁可看
7. consent：v2 stub 下 **授权一次后跨重启仍直接放行**（落盘 `.tmp-pg/consent_store.json`）
8. 收尾：`bash scripts/local-dev-up/down.sh`，开关改回 `false`

### 已知空白（本刀不收）

| 缺口 | 现象 | 后续 |
| --- | --- | --- |
| job-service `:3004` | `/v1/jobs/list` / `/v1/jobs/match` → **502** | 另刀入编排 |
| 真 Flask 栈 | 本路径默认 stub | 「本机可起真 Flask」挂账 |
| `/v1/game/match` | 游戏域，**不是**求职岗位匹配 | 勿当 career 验收 |

## stub v2 行为

- `GET /v1/compliance/consent/status` → `{ status: { jobseeker_core: bool, … } }`
- `POST …/grant` / `…/revoke` → 按 JWT `sub`（不验签，仅开发）落盘
- 其它路径回声 `{ via: "legacy-stub", method, path, auth }`，供网关透传探针

## 鉴权注意

| 登录方式 | resume 需要的密钥 |
| --- | --- |
| 网关 `local_auth`（冒烟默认） | `JWT_SECRET`（iss=`looma`） |
| 小程序 `wx.login` → 若仍走兼容 wechat 签发 | 另需 `SUPABASE_JWT_SECRET` 与签发方一致 |

`up.sh` 会尽量从 `looma-zervi/backend/.env` 自动灌入。
