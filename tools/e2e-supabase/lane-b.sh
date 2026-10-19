#!/usr/bin/env bash
# B 阶段（真 Supabase ES256 端到端，13 项）
# 用法：
#   export SUPABASE_URL="https://<ref>.supabase.co"
#   二选一取 token：
#     A) export SUPABASE_ANON_KEY=... TEST_EMAIL=... TEST_PASSWORD=...
#     B) export SUPABASE_ACCESS_TOKEN="eyJ..."
#   bash tools/e2e-supabase/lane-b.sh
# 前置：已构建 target/debug/api-gateway（cargo build -p api-gateway）
# 对拍锚点：末尾 verdict_sha256 双方必须完全相同。
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

require_cmd git curl || exit 2
ensure_mingw_toolchain || exit 2

OUT="$HERE/out"
mkdir -p "$OUT"
LOG="$OUT/lane-b-$(date +%Y%m%d-%H%M%S).log"
PASS=0
FAIL=0
GW_PID=""
ECHO_PID=""
GW_PORT="${GW_PORT:-3333}"
TATHA_PORT="${TATHA_PORT:-5210}"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) EXE=".exe" ;;
  *) EXE="" ;;
esac
BIN="$REPO/target/debug/api-gateway$EXE"

say() { echo "$*" | tee -a "$LOG"; }
ok() { PASS=$((PASS + 1)); say "PASS  $1"; }
bad() { FAIL=$((FAIL + 1)); say "FAIL  $1 :: $2"; }
cleanup() {
  [ -n "$GW_PID" ] && kill "$GW_PID" 2>/dev/null
  [ -n "$ECHO_PID" ] && kill "$ECHO_PID" 2>/dev/null
  return 0
}
trap cleanup EXIT

: "${SUPABASE_URL:?需要 SUPABASE_URL}"
SUPABASE_URL="${SUPABASE_URL%/}"
JWKS_URL="$SUPABASE_URL/auth/v1/.well-known/jwks.json"

say "=== 环境 ==="
say "base_commit $BASE_COMMIT"
say "head_commit $(git -C "$REPO" rev-parse HEAD)"
say "code_tree   $(git -C "$REPO" rev-parse "${BASE_COMMIT}^{tree}")"
say "binary      $(ls -l --time-style=long-iso "$BIN" 2>/dev/null | awk '{print $6, $7}')"
say "jwks        $JWKS_URL"

assert_commit || exit 2

if [ ! -x "$BIN" ] && [ ! -f "$BIN" ]; then
  say "FATAL: 找不到 $BIN，请先 cargo build -p api-gateway（建议 --release 时用 RELEASE=1 另行指定）"
  exit 2
fi

# ---------- 1) JWKS 预检 ----------
JWKS="$(curl -sS -m 15 "$JWKS_URL" 2>>"$LOG")"
if [ -z "$JWKS" ]; then
  bad "jwks_reachable" "空响应"
else
  ALGS="$(printf '%s' "$JWKS" | "$PY" -c 'import sys,json; d=json.load(sys.stdin); print(",".join(sorted({k.get("alg","?") for k in d.get("keys",[])})))' 2>>"$LOG")"
  KIDS="$(printf '%s' "$JWKS" | "$PY" -c 'import sys,json; d=json.load(sys.stdin); print(",".join(k.get("kid","?") for k in d.get("keys",[])))' 2>>"$LOG")"
  say "alg set [$ALGS]  kid [$KIDS]"
  case "$ALGS" in *ES256*) ok "jwks_has_es256" ;; *) bad "jwks_has_es256" "alg=$ALGS" ;; esac
fi

# ---------- 2) 取 token ----------
if [ -n "${SUPABASE_ACCESS_TOKEN:-}" ]; then
  TOKEN="$SUPABASE_ACCESS_TOKEN"
  say "token  来自 SUPABASE_ACCESS_TOKEN"
else
  : "${SUPABASE_ANON_KEY:?需要 SUPABASE_ANON_KEY 或 SUPABASE_ACCESS_TOKEN}"
  : "${TEST_EMAIL:?需要 TEST_EMAIL 或 SUPABASE_ACCESS_TOKEN}"
  : "${TEST_PASSWORD:?需要 TEST_PASSWORD 或 SUPABASE_ACCESS_TOKEN}"
  RESP="$(curl -sS -m 20 -X POST "$SUPABASE_URL/auth/v1/token?grant_type=password" \
    -H "apikey: $SUPABASE_ANON_KEY" -H "Content-Type: application/json" \
    -d "{\"email\":\"$TEST_EMAIL\",\"password\":\"$TEST_PASSWORD\"}" 2>>"$LOG")"
  TOKEN="$(printf '%s' "$RESP" | "$PY" -c 'import sys,json; d=json.load(sys.stdin); print(d.get("access_token",""))' 2>>"$LOG")"
  if [ -z "$TOKEN" ]; then
    bad "signin" "$(printf '%s' "$RESP" | head -c 200)"
  else
    ok "signin"
  fi
fi
[ -z "$TOKEN" ] && { say "无 token，终止"; exit 1; }

# ---------- 3) token 形状 ----------
HDR="$(printf '%s' "$TOKEN" | "$PY" -c 'import sys,base64,json; t=sys.stdin.read().strip().split(".")[0]; t+="="*(-len(t)%4); print(json.dumps(json.loads(base64.urlsafe_b64decode(t))))' 2>>"$LOG")"
CLM="$(printf '%s' "$TOKEN" | "$PY" -c 'import sys,base64,json; p=sys.stdin.read().strip().split("."); t=p[1]; t+="="*(-len(t)%4); d=json.loads(base64.urlsafe_b64decode(t)); print(json.dumps({k:d.get(k) for k in ("sub","aud","role","exp","email","phone")}))' 2>>"$LOG")"
say "header  $HDR"
say "claims  $CLM"
ALG="$(printf '%s' "$HDR" | "$PY" -c 'import sys,json; print(json.load(sys.stdin).get("alg",""))')"
[ "$ALG" = "ES256" ] && ok "token_alg_es256" || bad "token_alg_es256" "alg=$ALG"
SUB="$(printf '%s' "$CLM" | "$PY" -c 'import sys,json; print(json.load(sys.stdin).get("sub") or "")')"
EXPECT_UID="$(printf '%s' "$SUB" | "$PY" -c 'import sys,hashlib; print(hashlib.sha256(sys.stdin.read().strip().encode()).hexdigest())')"
say "sub=$SUB"
say "sha256(sub)=$EXPECT_UID"

# ---------- 4) 起 echo 上游 + 网关 ----------
"$PY" "$HERE/echo_server.py" >>"$LOG" 2>&1 &
ECHO_PID=$!
GATEWAY_JWKS_URL="$JWKS_URL" GATEWAY_JWT_SECRET="gateway-secret-not-used-for-es256" \
  TATHA_BASE_URL="http://127.0.0.1:$TATHA_PORT" RUST_LOG=warn \
  "$BIN" >>"$LOG" 2>&1 &
GW_PID=$!

for _ in $(seq 1 40); do
  curl -s -m 2 -o /dev/null "http://127.0.0.1:$GW_PORT/health" && break
  sleep 0.5
done
curl -s -m 2 -o /dev/null "http://127.0.0.1:$GW_PORT/health" || {
  bad "gateway_up" ":$GW_PORT 未监听"
  say "--- log ---"
  tail -20 "$LOG"
  exit 1
}
ok "gateway_up"

# ---------- 5) /v1/auth/verify ----------
CODE="$(curl -s -m 10 -o "$OUT/verify.json" -w '%{http_code}' "http://127.0.0.1:$GW_PORT/v1/auth/verify" -H "Authorization: Bearer $TOKEN")"
say "verify  HTTP=$CODE body=$(head -c 300 "$OUT/verify.json")"
[ "$CODE" = "200" ] && ok "verify_200" || bad "verify_200" "HTTP=$CODE"
GOT_UID="$(cat "$OUT/verify.json" | "$PY" -c 'import sys,json; print(json.load(sys.stdin).get("user_id",""))' 2>/dev/null)"
[ "$GOT_UID" = "$EXPECT_UID" ] && ok "verify_user_id_is_sha256_sub" || bad "verify_user_id_is_sha256_sub" "got=$GOT_UID want=$EXPECT_UID"

# ---------- 6) /v1/ask + 身份注入（含伪造头剥离） ----------
curl -s -m 10 -o "$OUT/ask.json" -w '%{http_code}' -X POST "http://127.0.0.1:$GW_PORT/v1/ask" \
  -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" \
  -H "X-Actor-Sub: attacker" -H "X-User-Id: evil" -H "X-User-Email-Hash: evil" \
  -d '{"intent":"knowledge","text":"e2e"}' > "$OUT/ask.code"
say "ask     HTTP=$(cat "$OUT/ask.code") body=$(head -c 300 "$OUT/ask.json")"
[ "$(cat "$OUT/ask.code")" = "200" ] && ok "ask_200" || bad "ask_200" "HTTP=$(cat "$OUT/ask.code")"
E_SUB="$(cat "$OUT/ask.json" | "$PY" -c 'import sys,json; d=json.load(sys.stdin); print(d.get("headers",{}).get("x-actor-sub",""))' 2>/dev/null)"
E_UID="$(cat "$OUT/ask.json" | "$PY" -c 'import sys,json; d=json.load(sys.stdin); print(d.get("headers",{}).get("x-user-id",""))' 2>/dev/null)"
E_EH="$(cat "$OUT/ask.json" | "$PY" -c 'import sys,json; d=json.load(sys.stdin); print(d.get("headers",{}).get("x-user-email-hash",""))' 2>/dev/null)"
[ "$E_SUB" = "$SUB" ] && ok "ask_injects_actor_sub" || bad "ask_injects_actor_sub" "got=$E_SUB"
[ "$E_UID" = "$EXPECT_UID" ] && ok "ask_injects_user_id" || bad "ask_injects_user_id" "got=$E_UID"
[ "$E_SUB" != "attacker" ] && [ "$E_UID" != "evil" ] && [ "$E_EH" != "evil" ] \
  && ok "ask_strips_forged_headers" || bad "ask_strips_forged_headers" "sub=$E_SUB uid=$E_UID eh=$E_EH"

# ---------- 7) 负例 ----------
C1="$(curl -s -m 8 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$GW_PORT/v1/auth/verify")"
[ "$C1" = "401" ] && ok "neg_no_token_401" || bad "neg_no_token_401" "HTTP=$C1"
C2="$(curl -s -m 8 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$GW_PORT/v1/auth/verify" -H "Authorization: Bearer abc.def.ghi")"
[ "$C2" = "401" ] && ok "neg_garbage_token_401" || bad "neg_garbage_token_401" "HTTP=$C2"
FORGED="$("$PY" - "$SUB" <<'PYEOF'
import sys, base64, hmac, hashlib, json, time
def b64(b): return base64.urlsafe_b64encode(b).rstrip(b"=")
sub = sys.argv[1] or "forged"
h = b64(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
p = b64(json.dumps({"sub": sub, "aud": "authenticated", "role": "admin", "exp": int(time.time()) + 3600}, separators=(",", ":")).encode())
s = b64(hmac.new(b"attacker-secret", h + b"." + p, hashlib.sha256).digest())
print((h + b"." + p + b"." + s).decode())
PYEOF
)"
C3="$(curl -s -m 8 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$GW_PORT/v1/auth/verify" -H "Authorization: Bearer $FORGED")"
[ "$C3" = "401" ] && ok "neg_hs256_forged_401" || bad "neg_hs256_forged_401" "HTTP=$C3"

# ---------- 8) 结果指纹 ----------
grep -E '^(PASS|FAIL)  ' "$LOG" | awk '{print $2, $1}' | LC_ALL=C sort > "$OUT/lane-b-verdicts.txt"
VCOUNT="$(wc -l < "$OUT/lane-b-verdicts.txt" | tr -d ' ')"
VSHA="$(sha256sum "$OUT/lane-b-verdicts.txt" | cut -d' ' -f1)"

say "=== 汇总 PASS=$PASS FAIL=$FAIL ==="
say "cases       $VCOUNT"
say "verdict_sha256 $VSHA"
say "日志 $LOG"
[ "$FAIL" -eq 0 ] || exit 1
