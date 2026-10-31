#!/usr/bin/env bash
# 本机联调抽检（经 api-gateway :3333）。不替代完整 dual_origin 冒烟。
set -euo pipefail
export NO_PROXY="${NO_PROXY:-127.0.0.1,localhost,::1}"
unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy || true

BASE="${BASE:-http://127.0.0.1:3333}"
PASS=0
FAIL=0

ok() { echo "  ok  $*"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL $*"; FAIL=$((FAIL + 1)); }

json_field() {
  python - "$1" "$2" <<'PY'
import json,sys
doc=json.loads(sys.argv[1] or "null")
if doc is None:
  print(""); raise SystemExit(0)
path=sys.argv[2].split(".")
cur=doc
for p in path:
  if isinstance(cur, dict) and p in cur: cur=cur[p]
  else: print(""); raise SystemExit(0)
if isinstance(cur, bool):
  print("true" if cur else "false")
elif isinstance(cur, (dict, list)):
  print(json.dumps(cur))
else:
  print("" if cur is None else cur)
PY
}

req() {
  local method="$1" path="$2" token="${3:-}" body="${4:-}"
  local args=(-sS -m 30 --noproxy '*' -X "$method" "$BASE$path" -H "Accept: application/json")
  [[ -n "$token" ]] && args+=(-H "Authorization: Bearer $token")
  if [[ -n "$body" ]]; then
    args+=(-H "Content-Type: application/json" -d "$body")
  fi
  local tmp code
  tmp="$(mktemp)"
  code="$(curl "${args[@]}" -o "$tmp" -w "%{http_code}" || true)"
  BODY="$(cat "$tmp")"
  rm -f "$tmp"
  CODE="$code"
}

echo "== smoke_check @ $BASE =="

req GET /health
[[ "$CODE" == "200" ]] && ok "gateway /health $CODE" || bad "gateway /health $CODE"

# local_auth（不依赖 Flask wechat）
EMAIL="dev-$(date +%s)@local.test"
PASSWD="Passw0rd!"
req POST /v1/auth/register "" "{\"email\":\"$EMAIL\",\"password\":\"$PASSWD\",\"name\":\"dev\"}"
if [[ "$CODE" == "201" || "$CODE" == "200" || "$CODE" == "409" ]]; then
  ok "register $CODE"
else
  bad "register $CODE $BODY"
fi
req POST /v1/auth/login "" "{\"email\":\"$EMAIL\",\"password\":\"$PASSWD\"}"
TOKEN="$(json_field "$BODY" "token")"
[[ -z "$TOKEN" || "$TOKEN" == "None" ]] && TOKEN="$(json_field "$BODY" "access_token")"
if [[ "$CODE" == "200" && -n "$TOKEN" && "$TOKEN" != "None" ]]; then
  ok "login got token"
else
  bad "login $CODE $BODY"
  echo "ABORT"
  echo "PASS=$PASS FAIL=$FAIL"
  exit 1
fi

req POST /v1/compliance/consent/grant "$TOKEN" "{\"scope\":\"jobseeker_core\"}"
[[ "$CODE" == "200" ]] && ok "consent grant $CODE" || bad "consent grant $CODE $BODY"

req GET /v1/compliance/consent/status "$TOKEN"
ST="$(json_field "$BODY" "status.jobseeker_core")"
if [[ "$CODE" == "200" && "$ST" == "true" ]]; then
  ok "consent status jobseeker_core=true"
else
  bad "consent status $CODE st=$ST body=$BODY"
fi

req GET /v1/resume/templates "$TOKEN"
[[ "$CODE" == "200" ]] && ok "templates $CODE" || bad "templates $CODE"

req POST /v1/resume/ingest "$TOKEN" \
  "{\"markdown\":\"# local-dev-up\\nsmoke\",\"filename\":\"smoke.md\"}"
RID="$(json_field "$BODY" "resume_id")"
[[ "$CODE" == "200" && -n "$RID" && "$RID" != "None" ]] && ok "ingest resume_id=$RID" || bad "ingest $CODE $BODY"

req GET /v1/resume/list "$TOKEN"
[[ "$CODE" == "200" ]] && ok "list $CODE" || bad "list $CODE"

if [[ -n "$RID" && "$RID" != "None" ]]; then
  req GET "/v1/resume/$RID" "$TOKEN"
  [[ "$CODE" == "200" ]] && ok "detail $CODE" || bad "detail $CODE"
  req GET "/v1/resume/$RID/permission" "$TOKEN"
  [[ "$CODE" == "200" ]] && ok "permission $CODE" || bad "permission $CODE"
fi

req GET /v1/jobs/list "$TOKEN"
if [[ "$CODE" == "200" ]]; then
  ok "jobs/list $CODE"
else
  bad "jobs/list 期望 200 得 $CODE $BODY"
fi

echo ""
echo "PASS=$PASS FAIL=$FAIL"
[[ "$FAIL" -eq 0 ]]
