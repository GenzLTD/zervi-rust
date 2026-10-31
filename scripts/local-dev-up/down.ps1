# local-dev-down (Windows PowerShell)
$ErrorActionPreference = "Continue"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Tmp = Join-Path $Root ".tmp-pg"
$PidFile = Join-Path $Tmp "pids.txt"
$Data = Join-Path $Tmp "data"
$PgBin = if ($env:PG_BIN) { $env:PG_BIN } else { "C:\ServBay\packages\postgresql\16\bin" }

Write-Host "== local-dev-down =="
if (Test-Path $PidFile) {
  Get-Content $PidFile | ForEach-Object {
    if ($_ -match '^(?<k>\w+)=(?<p>\d+)$') {
      Write-Host "kill $($Matches.k) pid=$($Matches.p)"
      taskkill /F /PID $Matches.p 2>$null | Out-Null
    }
  }
}
foreach ($port in 5200, 3004, 3003, 3333) {
  Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue |
    ForEach-Object { taskkill /F /PID $_.OwningProcess 2>$null | Out-Null }
}
if (Test-Path (Join-Path $Data "PG_VERSION")) {
  $st = & "$PgBin\pg_ctl.exe" -D $Data status 2>&1
  if ($LASTEXITCODE -eq 0) {
    Write-Host "[pg] stop"
    & "$PgBin\pg_ctl.exe" -D $Data -m fast stop | Out-Null
  }
}
Start-Sleep -Seconds 1
$left = Get-NetTCPConnection -LocalPort 55433, 3003, 3004, 3333, 5200 -ErrorAction SilentlyContinue
if ($left) { $left | Format-Table LocalPort, State, OwningProcess -AutoSize }
else { Write-Host "CLEAN: 五端口无监听" }
Write-Host "小程序若开过开关：USE_LOCAL_GATEWAY = false"
