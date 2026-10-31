#!/usr/bin/env bash
# local-dev-up：一键收尾 —— 停 stub/resume/gateway + 临时 pg
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$ROOT/.tmp-pg"
PID_FILE="$TMP/pids.txt"
DATA="$TMP/data"

detect_pg_bin() {
  for c in \
    "/c/ServBay/packages/postgresql/16/bin" \
    "/d/ServBay/packages/postgresql/16/bin" \
    "C:/ServBay/packages/postgresql/16/bin" \
    "D:/ServBay/packages/postgresql/16/bin"
  do
    if [[ -x "$c/pg_ctl" || -x "$c/pg_ctl.exe" ]]; then
      echo "$c"
      return 0
    fi
  done
  return 1
}

kill_pid() {
  local pid="$1"
  [[ -z "$pid" ]] && return 0
  if command -v taskkill >/dev/null 2>&1; then
    taskkill //F //PID "$pid" >/dev/null 2>&1 || true
  else
    kill "$pid" 2>/dev/null || true
  fi
}

kill_port() {
  local port="$1"
  powershell.exe -NoProfile -Command \
    "Get-NetTCPConnection -LocalPort $port -EA SilentlyContinue | ForEach-Object { taskkill /F /PID \$_.OwningProcess 2>\$null }" \
    >/dev/null 2>&1 || true
}

echo "== local-dev-down =="

if [[ -f "$PID_FILE" ]]; then
  # shellcheck disable=SC1090
  while IFS='=' read -r k v; do
    [[ -z "${v:-}" ]] && continue
    echo "kill $k pid=$v"
    kill_pid "$v"
  done < "$PID_FILE"
fi

# 兜底按端口清（防 PID 文件过期）
for p in 5200 3004 3003 3333; do
  kill_port "$p"
done

PG_BIN="$(detect_pg_bin || true)"
if [[ -n "${PG_BIN:-}" && -d "$DATA" ]]; then
  PG_CTL="$PG_BIN/pg_ctl"
  [[ -x "$PG_CTL.exe" ]] && PG_CTL="$PG_CTL.exe"
  if "$PG_CTL" -D "$DATA" status >/dev/null 2>&1; then
    echo "[pg] stop"
    "$PG_CTL" -D "$DATA" -m fast stop || true
  fi
fi

sleep 1
echo "done. 运行态目录保留：$TMP （含 consent_store / runtime.env / 数据目录）"
echo "小程序记得 USE_LOCAL_GATEWAY = false（若曾打开）。"
