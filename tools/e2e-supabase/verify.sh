#!/usr/bin/env bash
# 验收入口：先跑这个，全绿再跑 lane-a / lane-b。
#   bash tools/e2e-supabase/verify.sh
# 作用：字节清单校验 + 换行检查 + 基线 commit 门禁 + 环境指纹输出。
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

FAILED=0
echo "############ 1/4 字节清单 ############"
if [ ! -f "$HERE/SHA256SUMS" ]; then
  echo "FAIL: 缺少 SHA256SUMS"
  FAILED=1
else
  if ( cd "$HERE" && sha256sum -c SHA256SUMS --quiet ); then
    echo "OK: 所有文件字节与清单一致"
  else
    echo "FAIL: 有文件被改动（不要手工编辑脚本，改动请走新 commit）"
    FAILED=1
  fi
fi
echo ""

echo "############ 2/4 换行与编码 ############"
CR_BAD=0
for f in "$HERE"/*.sh "$HERE"/*.py "$HERE"/EXPECTED_COMMIT "$HERE"/SHA256SUMS; do
  [ -f "$f" ] || continue
  if grep -q $'\r' "$f" 2>/dev/null; then
    echo "FAIL: CRLF 换行 -> $(basename "$f")"
    CR_BAD=1
  fi
  if [ "$(head -c3 "$f" | od -An -tx1 | tr -d ' \n')" = "efbbbf" ]; then
    echo "FAIL: UTF-8 BOM -> $(basename "$f")"
    CR_BAD=1
  fi
done
[ "$CR_BAD" -eq 0 ] && echo "OK: 全部 LF、无 BOM" || FAILED=1
echo ""

echo "############ 3/4 基线门禁 ############"
if assert_commit; then
  echo "OK: HEAD 包含基线 $BASE_COMMIT，且被测代码无漂移"
else
  FAILED=1
fi
echo ""

echo "############ 4/4 环境指纹 ############"
print_env
echo ""

echo "############ 命令可用性 ############"
for c in git cargo rustc curl sha256sum; do
  if command -v "$c" >/dev/null 2>&1; then
    echo "OK   $c"
  else
    echo "MISS $c"
    FAILED=1
  fi
done
if command -v rustc >/dev/null 2>&1; then
  if ensure_mingw_toolchain; then
    echo "OK   windows-gnu 工具链（或不需要）"
  else
    echo "WARN 缺少 dlltool：lane-a 编译阶段会失败，按上面提示补齐后再跑"
  fi
fi
echo ""

if [ "$FAILED" -eq 0 ]; then
  echo "==== VERIFY PASS ===="
  exit 0
else
  echo "==== VERIFY FAIL ===="
  exit 1
fi
