#!/usr/bin/env bash
# A 阶段（离线，21 项 P0）
#   - 不联网、不需要 .env、不需要 Supabase 凭据
#   - 在基线 commit 上跑 cargo test -p api-gateway
# 用法：
#   bash tools/e2e-supabase/lane-a.sh
# 对拍锚点：末尾 names_sha256 双方必须完全相同。
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

require_cmd git cargo rustc || exit 2

OUT="$HERE/out"
mkdir -p "$OUT"
RAW="$OUT/lane-a-raw.log"
NAMES="$OUT/lane-a-names.txt"

print_env
echo ""
assert_commit || exit 2

[ -n "${EXTRA_PATH:-}" ] && export PATH="$EXTRA_PATH:$PATH"

echo "=== 运行 cargo test -p api-gateway（首次需编译，耗时较长） ==="
( cd "$REPO" && cargo test -p api-gateway ) > "$RAW" 2>&1
RC=$?
tail -30 "$RAW"
echo ""

echo "=== 结果 ==="
SUMMARY="$(grep -E '^test result:' "$RAW" | tail -1)"
if [ -z "$SUMMARY" ]; then
  echo "FAIL: 没有拿到 test result 行，编译或链接未通过。"
  echo "--- 日志尾部 ---"
  tail -40 "$RAW"
  exit 1
fi
echo "summary   $SUMMARY"

grep -E '^test .* \.\.\. ok$' "$RAW" | sed 's/^test //; s/ \.\.\. ok$//' | LC_ALL=C sort > "$NAMES"
COUNT="$(wc -l < "$NAMES" | tr -d ' ')"
SHA="$(sha256sum "$NAMES" | cut -d' ' -f1)"

echo "cases     $COUNT"
echo "names_sha256 $SHA"
echo "names_file   $NAMES"

PASSED="$(printf '%s' "$SUMMARY" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+')"
FAILED="$(printf '%s' "$SUMMARY" | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+')"
echo "PASS=$PASSED FAIL=$FAILED cargo_exit=$RC"
echo ""
echo ">>> 请把上面 [环境指纹] 与 names_sha256 一起回贴，两侧一致即视为 A 阶段对拍通过。"

[ "$RC" -eq 0 ] || exit 1
