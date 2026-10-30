# local-dev-up (Windows PowerShell)
# pg :55433 → resume :3003 → gateway :3333 → legacy stub v2 :5200
param(
  [switch]$SkipBuild,
  [switch]$RefreshEnv,
  [string]$LoomaEnv = ""
)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Tmp = Join-Path $Root ".tmp-pg"
$Data = Join-Path $Tmp "data"
$PidFile = Join-Path $Tmp "pids.txt"
$RuntimeEnv = Join-Path $Tmp "runtime.env"
$Store = Join-Path $Tmp "consent_store.json"
$PgBin = if ($env:PG_BIN) { $env:PG_BIN } else { "C:\ServBay\packages\postgresql\16\bin" }
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null

function Kill-Port([int]$Port) {
  Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue |
    ForEach-Object { taskkill /F /PID $_.OwningProcess 2>$null | Out-Null }
}

function Load-DotEnv([string]$Path) {
  $map = @{}
  if (-not (Test-Path $Path)) { return $map }
  Get-Content $Path | ForEach-Object {
    if ($_ -match '^\s*#' -or $_ -notmatch '=') { return }
    $k, $v = $_.Split('=', 2)
    $map[$k.Trim()] = $v.Trim().Trim('"').Trim("'")
  }
  $map
}

function Write-RuntimeEnv {
  $candidates = @(
    $LoomaEnv,
    "c:\ServBay\www\looma-zervi\backend\.env",
    (Join-Path $Root "..\looma-zervi\backend\.env")
  ) | Where-Object { $_ -and (Test-Path $_) }
  $src = $candidates | Select-Object -First 1
  $jwt = "local-dev-jwt-secret-change-me"
  $supabase = ""; $deepseek = ""; $model = "deepseek-chat"; $base = "https://api.deepseek.com/v1"
  if ($src) {
    Write-Host "[env] seeding from $src"
    $m = Load-DotEnv $src
    if ($m["JWT_SECRET"]) { $jwt = $m["JWT_SECRET"] }
    if ($m["SUPABASE_JWT_SECRET"]) { $supabase = $m["SUPABASE_JWT_SECRET"] }
    if ($m["DEEPSEEK_API_KEY"]) { $deepseek = $m["DEEPSEEK_API_KEY"] }
    if ($m["DEEPSEEK_MODEL"]) { $model = $m["DEEPSEEK_MODEL"] }
    if ($m["DEEPSEEK_BASE_URL"]) { $base = $m["DEEPSEEK_BASE_URL"] }
  } else {
    Write-Host "[env] WARN: no looma .env; JWT placeholder — wx.login may 401 on resume"
  }
  @"
DATABASE_URL=postgres://postgres@127.0.0.1:55433/genzltd_sim
JWT_SECRET=$jwt
GATEWAY_JWT_SECRET=$jwt
SUPABASE_JWT_SECRET=$supabase
ZERVI_DEV_AUTH=1
ZERVI_DEV_BYPASS_CONSENT=1
RESUME_PORT=3003
RESUME_SWEEP_INTERVAL_SECS=900
DEPLOY_REGION=CN
DEEPSEEK_API_KEY=$deepseek
DEEPSEEK_MODEL=$model
DEEPSEEK_BASE_URL=$base
NO_PROXY=127.0.0.1,localhost,::1
"@ | Set-Content -Path $RuntimeEnv -Encoding ascii
  Write-Host "[env] wrote $RuntimeEnv"
}

function Apply-RuntimeEnv {
  Get-Content $RuntimeEnv | ForEach-Object {
    if ($_ -match '^\s*#' -or $_ -notmatch '=') { return }
    $k, $v = $_.Split('=', 2)
    Set-Item -Path "Env:$($k.Trim())" -Value $v.Trim()
  }
  $env:NO_PROXY = "127.0.0.1,localhost,::1"
  $env:HTTP_PROXY = ""; $env:HTTPS_PROXY = ""; $env:http_proxy = ""; $env:https_proxy = ""
}

Write-Host "== local-dev-up (ps1) =="
Write-Host "ROOT=$Root"

# --- pg ---
if (-not (Test-Path (Join-Path $Data "PG_VERSION"))) {
  Write-Host "[pg] initdb"
  & "$PgBin\initdb.exe" -D $Data -U postgres --auth=trust --encoding=UTF8 --locale=C | Out-Null
  $conf = Join-Path $Data "postgresql.conf"
  $text = Get-Content $conf -Raw
  if ($text -match '(?m)^#?port\s*=') { $text = $text -replace '(?m)^#?port\s*=.*', 'port = 55433' }
  else { $text += "`nport = 55433`n" }
  if ($text -match '(?m)^#?listen_addresses\s*=') { $text = $text -replace '(?m)^#?listen_addresses\s*=.*', "listen_addresses = '127.0.0.1'" }
  else { $text += "`nlisten_addresses = '127.0.0.1'`n" }
  Set-Content $conf $text -Encoding ascii
}
$pgStatus = & "$PgBin\pg_ctl.exe" -D $Data status 2>&1
if ($LASTEXITCODE -ne 0) {
  Write-Host "[pg] start :55433"
  & "$PgBin\pg_ctl.exe" -D $Data -l (Join-Path $Tmp "pg.log") start | Out-Null
  Start-Sleep -Seconds 1
} else { Write-Host "[pg] already running" }

& "$PgBin\psql.exe" -h 127.0.0.1 -p 55433 -U postgres -d postgres -c "SELECT 1 FROM pg_database WHERE datname='genzltd_sim'" 2>$null | Out-Null
$exists = & "$PgBin\psql.exe" -h 127.0.0.1 -p 55433 -U postgres -d postgres -tAc "SELECT 1 FROM pg_database WHERE datname='genzltd_sim'"
if ($exists -ne "1") {
  & "$PgBin\psql.exe" -h 127.0.0.1 -p 55433 -U postgres -d postgres -c "CREATE DATABASE genzltd_sim;" | Out-Null
}
$hasDocs = & "$PgBin\psql.exe" -h 127.0.0.1 -p 55433 -U postgres -d genzltd_sim -tAc "SELECT 1 FROM information_schema.tables WHERE table_name='documents'"
if ($hasDocs -ne "1") {
  & "$PgBin\psql.exe" -h 127.0.0.1 -p 55433 -U postgres -d genzltd_sim -v ON_ERROR_STOP=1 -f (Join-Path $Root "shared\genzltd_schema_v0.3.sql") | Out-Null
}
$hasBl = & "$PgBin\psql.exe" -h 127.0.0.1 -p 55433 -U postgres -d genzltd_sim -tAc "SELECT 1 FROM information_schema.tables WHERE table_name='resume_blacklist'"
if ($hasBl -ne "1") {
  & "$PgBin\psql.exe" -h 127.0.0.1 -p 55433 -U postgres -d genzltd_sim -v ON_ERROR_STOP=1 -f (Join-Path $Root "shared\resume_blacklist_v0.1.sql") | Out-Null
}
Write-Host "[pg] genzltd_sim ready"

if ($RefreshEnv -or -not (Test-Path $RuntimeEnv)) { Write-RuntimeEnv } else { Write-Host "[env] reuse $RuntimeEnv" }
Apply-RuntimeEnv

# --- build ---
$rsBin = Join-Path $Root "target\debug\resume-service.exe"
$gwBin = Join-Path $Root "target\debug\api-gateway.exe"
if (-not $SkipBuild -and (-not (Test-Path $rsBin) -or -not (Test-Path $gwBin))) {
  Write-Host "[build] cargo build"
  $cargoBin = "C:\ServBay\packages\rust\current\cargo\bin"
  $rustcBin = "C:\ServBay\packages\rust\current\rustc\bin"
  $env:PATH = "$cargoBin;$rustcBin;$env:PATH"
  Push-Location $Root
  cargo build -p resume-service -p api-gateway
  Pop-Location
} else {
  Write-Host "[build] skip"
}

# --- processes ---
Kill-Port 5200; Kill-Port 3003; Kill-Port 3333
$pyCmd = Get-Command python -ErrorAction SilentlyContinue
$py = if ($pyCmd) { $pyCmd.Source } else { "python" }
Write-Host "[stub] legacy_stub_v2 :5200"
$stub = Start-Process -FilePath $py -ArgumentList @(
  (Join-Path $PSScriptRoot "legacy_stub_v2.py"), "--port", "5200", "--store", $Store
) -WorkingDirectory $Root -RedirectStandardOutput (Join-Path $Tmp "stub.log") -RedirectStandardError (Join-Path $Tmp "stub.err.log") -PassThru -WindowStyle Hidden

Apply-RuntimeEnv
Write-Host "[resume] :3003"
$resume = Start-Process -FilePath $rsBin -WorkingDirectory $Root `
  -RedirectStandardOutput (Join-Path $Tmp "resume.log") -RedirectStandardError (Join-Path $Tmp "resume.err.log") `
  -PassThru -WindowStyle Hidden
Write-Host "[gateway] :3333"
$gw = Start-Process -FilePath $gwBin -WorkingDirectory $Root `
  -RedirectStandardOutput (Join-Path $Tmp "gateway.log") -RedirectStandardError (Join-Path $Tmp "gateway.err.log") `
  -PassThru -WindowStyle Hidden

@"
stub=$($stub.Id)
resume=$($resume.Id)
gateway=$($gw.Id)
"@ | Set-Content $PidFile -Encoding ascii

for ($i = 0; $i -lt 20; $i++) {
  try {
    $r1 = Invoke-WebRequest -Uri "http://127.0.0.1:3333/health" -UseBasicParsing -TimeoutSec 1
    $r2 = Invoke-WebRequest -Uri "http://127.0.0.1:5200/health" -UseBasicParsing -TimeoutSec 1
    if ($r1.StatusCode -eq 200 -and $r2.StatusCode -eq 200) { break }
  } catch { Start-Sleep -Milliseconds 500 }
}

Write-Host ""
Write-Host "栈已起："
Write-Host "  pg             :55433"
Write-Host "  resume-service :3003  pid=$($resume.Id)"
Write-Host "  api-gateway    :3333  pid=$($gw.Id)"
Write-Host "  legacy stub v2 :5200  pid=$($stub.Id)"
Write-Host "抽检:  powershell -File scripts/local-dev-up/smoke_check.ps1"
Write-Host "收尾:  powershell -File scripts/local-dev-up/down.ps1"
