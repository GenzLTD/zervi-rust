#!/usr/bin/env bash
# local-dev-up：一键起本机联调栈
#   pg :55433 → resume-service :3003 → api-gateway :3333 → legacy stub v2 :5200
#
# 用法（Git Bash / WSL）：
#   bash scripts/local-dev-up/up.sh
# 可选：
#   LOOMA_ENV=/path/to/looma/backend/.env bash scripts/local-dev-up/up.sh
#   SKIP_BUILD=1 bash scripts/local-dev-up/up.sh
set -euo pipefail
export NO_PROXY="${NO_PROXY:-127.0.0.1,localhost,::1}"
export no_proxy="$NO_PROXY"
unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy || true

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TMP="$ROOT/.tmp-pg"
DATA="$TMP/data"
LOG="$TMP"
PID_FILE="$TMP/pids.txt"
RUNTIME_ENV="$TMP/runtime.env"
STORE="$TMP/consent_store.json"

# --- 路径探测（ServBay Windows）---
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
  echo "找不到 PostgreSQL 16 bin（ServBay）。请设置 PG_BIN。" >&2
  exit 1
}

PG_BIN="${PG_BIN:-$(detect_pg_bin)}"
PSQL="$PG_BIN/psql"
PG_CTL="$PG_BIN/pg_ctl"
INITDB="$PG_BIN/initdb"
[[ -x "$PSQL.exe" ]] && PSQL="$PSQL.exe"
[[ -x "$PG_CTL.exe" ]] && PG_CTL="$PG_CTL.exe"
[[ -x "$INITDB.exe" ]] && INITDB="$INITDB.exe"

mkdir -p "$TMP"

port_listening() {
  local port="$1"
  if command -v curl >/dev/null 2>&1; then
    curl -sS -m 1 --noproxy '*' "http://127.0.0.1:$port/health" >/dev/null 2>&1 && return 0
  fi
  # fallback: PowerShell
  powershell.exe -NoProfile -Command \
    "if (Get-NetTCPConnection -LocalPort $port -State Listen -EA SilentlyContinue) { exit 0 } else { exit 1 }" \
    >/dev/null 2>&1
}

kill_port() {
  local port="$1"
  powershell.exe -NoProfile -Command \
    "Get-NetTCPConnection -LocalPort $port -EA SilentlyContinue | ForEach-Object { taskkill /F /PID \$_.OwningProcess 2>\$null }" \
    >/dev/null 2>&1 || true
}

echo "== local-dev-up =="
echo "ROOT=$ROOT"
echo "PG_BIN=$PG_BIN"

# --- 1) Postgres ---
if [[ ! -f "$DATA/PG_VERSION" ]]; then
  echo "[pg] initdb $DATA"
  "$INITDB" -D "$DATA" -U postgres --auth=trust --encoding=UTF8 --locale=C >/dev/null
  # port + listen
  if grep -q "^port" "$DATA/postgresql.conf" 2>/dev/null; then
    sed -i 's/^port = .*/port = 55433/' "$DATA/postgresql.conf" || true
  else
    echo "port = 55433" >> "$DATA/postgresql.conf"
  fi
  if grep -q "^listen_addresses" "$DATA/postgresql.conf" 2>/dev/null; then
    sed -i "s/^listen_addresses = .*/listen_addresses = '127.0.0.1'/" "$DATA/postgresql.conf" || true
  else
    echo "listen_addresses = '127.0.0.1'" >> "$DATA/postgresql.conf"
  fi
fi

if ! "$PG_CTL" -D "$DATA" status >/dev/null 2>&1; then
  echo "[pg] start :55433"
  "$PG_CTL" -D "$DATA" -l "$LOG/pg.log" start
  sleep 1
else
  echo "[pg] already running"
fi

echo "[pg] ensure database genzltd_sim + schema"
"$PSQL" -h 127.0.0.1 -p 55433 -U postgres -d postgres -v ON_ERROR_STOP=1 \
  -c "SELECT 1 FROM pg_database WHERE datname='genzltd_sim'" | grep -q 1 \
  || "$PSQL" -h 127.0.0.1 -p 55433 -U postgres -d postgres -c "CREATE DATABASE genzltd_sim;"
"$PSQL" -h 127.0.0.1 -p 55433 -U postgres -d genzltd_sim -v ON_ERROR_STOP=1 \
  -c "SELECT 1 FROM information_schema.tables WHERE table_name='documents'" | grep -q 1 \
  || "$PSQL" -h 127.0.0.1 -p 55433 -U postgres -d genzltd_sim -v ON_ERROR_STOP=1 \
       -f "$ROOT/shared/genzltd_schema_v0.3.sql"
"$PSQL" -h 127.0.0.1 -p 55433 -U postgres -d genzltd_sim -v ON_ERROR_STOP=1 \
  -c "SELECT 1 FROM information_schema.tables WHERE table_name='resume_blacklist'" | grep -q 1 \
  || "$PSQL" -h 127.0.0.1 -p 55433 -U postgres -d genzltd_sim -v ON_ERROR_STOP=1 \
       -f "$ROOT/shared/resume_blacklist_v0.1.sql"

# --- 2) runtime.env ---
write_runtime_env() {
  local looma_env="${LOOMA_ENV:-}"
  if [[ -z "$looma_env" ]]; then
    for c in \
      "/c/ServBay/www/looma-zervi/backend/.env" \
      "/d/ServBay/www/looma-zervi/backend/.env" \
      "$ROOT/../looma-zervi/backend/.env"
    do
      [[ -f "$c" ]] && looma_env="$c" && break
    done
  fi

  local jwt="" supabase="" deepseek="" deepseek_model="deepseek-chat" deepseek_base="https://api.deepseek.com/v1"
  if [[ -n "$looma_env" && -f "$looma_env" ]]; then
    echo "[env] seeding from $looma_env"
    jwt=$(grep -E '^JWT_SECRET=' "$looma_env" | head -1 | cut -d= -f2- | tr -d '\r' | sed 's/^["'\'']//;s/["'\'']$//')
    supabase=$(grep -E '^SUPABASE_JWT_SECRET=' "$looma_env" | head -1 | cut -d= -f2- | tr -d '\r' | sed 's/^["'\'']//;s/["'\'']$//')
    deepseek=$(grep -E '^DEEPSEEK_API_KEY=' "$looma_env" | head -1 | cut -d= -f2- | tr -d '\r' | sed 's/^["'\'']//;s/["'\'']$//')
    deepseek_model=$(grep -E '^DEEPSEEK_MODEL=' "$looma_env" | head -1 | cut -d= -f2- | tr -d '\r' | sed 's/^["'\'']//;s/["'\'']$//' || true)
    deepseek_base=$(grep -E '^DEEPSEEK_BASE_URL=' "$looma_env" | head -1 | cut -d= -f2- | tr -d '\r' | sed 's/^["'\'']//;s/["'\'']$//' || true)
  fi
  if [[ -z "$jwt" ]]; then
    jwt="local-dev-jwt-secret-change-me"
    echo "[env] WARN: JWT_SECRET 回落占位；local_auth 可用，wx.login 需配真密钥" >&2
  fi

  cat > "$RUNTIME_ENV" <<EOF
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
DEEPSEEK_MODEL=${deepseek_model:-deepseek-chat}
DEEPSEEK_BASE_URL=${deepseek_base:-https://api.deepseek.com/v1}
NO_PROXY=127.0.0.1,localhost,::1
EOF
  echo "[env] wrote $RUNTIME_ENV (secrets not printed)"
}

if [[ ! -f "$RUNTIME_ENV" || "${REFRESH_ENV:-0}" == "1" ]]; then
  write_runtime_env
else
  echo "[env] reuse $RUNTIME_ENV (REFRESH_ENV=1 to rebuild)"
fi

set -a
# shellcheck disable=SC1090
source "$RUNTIME_ENV"
set +a

# --- 3) build ---
need_build() {
  [[ "${SKIP_BUILD:-0}" == "1" ]] && return 1
  local rs="$ROOT/target/debug/resume-service.exe"
  local gw="$ROOT/target/debug/api-gateway.exe"
  [[ ! -f "$rs" || ! -f "$gw" ]] && return 0
  # 源码比二进制新则重建
  find "$ROOT/resume-service/src" "$ROOT/api-gateway/src" "$ROOT/shared/common/src" \
    -type f -name '*.rs' -newer "$rs" 2>/dev/null | grep -q . && return 0
  return 1
}

if need_build; then
  echo "[build] cargo build -p resume-service -p api-gateway"
  (
    cd "$ROOT"
    # ServBay rustc/cargo PATH
    export PATH="/c/ServBay/packages/rust/current/cargo/bin:/c/ServBay/packages/rust/current/rustc/bin:$PATH"
    cargo build -p resume-service -p api-gateway
  )
else
  echo "[build] skip (binaries fresh; SKIP_BUILD=1 or newer)"
fi

# --- 4) legacy stub v2 ---
kill_port 5200
echo "[stub] legacy_stub_v2.py :5200"
python "$SCRIPT_DIR/legacy_stub_v2.py" --port 5200 --store "$STORE" \
  >"$LOG/stub.log" 2>&1 &
STUB_PID=$!

# --- 5) resume + gateway ---
kill_port 3003
kill_port 3333
echo "[resume] :3003"
(
  set -a; source "$RUNTIME_ENV"; set +a
  export NO_PROXY=127.0.0.1,localhost,::1
  unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy || true
  cd "$ROOT"
  exec ./target/debug/resume-service.exe
) >"$LOG/resume.log" 2>&1 &
RESUME_PID=$!

echo "[gateway] :3333"
(
  set -a; source "$RUNTIME_ENV"; set +a
  export NO_PROXY=127.0.0.1,localhost,::1
  unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy || true
  cd "$ROOT"
  exec ./target/debug/api-gateway.exe
) >"$LOG/gateway.log" 2>&1 &
GW_PID=$!

cat > "$PID_FILE" <<EOF
stub=$STUB_PID
resume=$RESUME_PID
gateway=$GW_PID
EOF

# --- wait ---
for i in $(seq 1 20); do
  if curl -sS -m 1 --noproxy '*' "http://127.0.0.1:3333/health" >/dev/null 2>&1 \
     && curl -sS -m 1 --noproxy '*' "http://127.0.0.1:5200/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

echo ""
echo "栈已起："
echo "  pg            :55433  (genzltd_sim)"
echo "  resume-service:$RESUME_PORT  pid=$RESUME_PID"
echo "  api-gateway   :3333  pid=$GW_PID  (0.0.0.0)"
echo "  legacy stub v2:5200  pid=$STUB_PID  store=$STORE"
echo ""
echo "局域网入口（Surface/Mac/Dell 客户端改 LOCAL_GATEWAY_ORIGIN）："
if command -v ipconfig.exe >/dev/null 2>&1; then
  ipconfig.exe | tr -d '\r' | awk '
    /^[A-Za-z].*:$/ { iface=$0; next }
    /IPv4/ {
      for (i=1;i<=NF;i++) if ($i ~ /^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$/) {
        ip=$i
        if (ip !~ /^127\./ && ip !~ /^169\.254\./)
          print "  http://" ip ":3333  (" iface ")"
      }
    }'
elif command -v ip >/dev/null 2>&1; then
  ip -4 -o addr show scope global | awk '{print "  http://" $4 ":3333  (" $2 ")"}' | sed 's|/.*:3333|:3333|'
fi
echo "防火墙（Windows 栈主机首次）：管理员运行 open-firewall-3333.ps1"
echo "多机说明：scripts/local-dev-up/FLEET.md"
echo ""
echo "小程序：packages/miniprogram/utils/config.ts → USE_LOCAL_GATEWAY = true"
echo "       LOCAL_GATEWAY_ORIGIN = http://<栈主机LAN>:3333（跨机必改）"
echo "       （提交前改回 false；config.test.ts 会拦）"
echo "抽检：  bash scripts/local-dev-up/smoke_check.sh"
echo "       BASE=http://<LAN>:3333 bash scripts/local-dev-up/smoke_check.sh  # 远端"
echo "收尾：  bash scripts/local-dev-up/down.sh"
echo "清单：  scripts/local-dev-up/README.md"
