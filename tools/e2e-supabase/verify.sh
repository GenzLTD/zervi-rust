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
# 直接遍历 SHA256SUMS 的清单，保证"被校验的文件"与"被检查编码的文件"是同一集合，
# 避免清单里有、这里漏检（ENV.lock 就曾被漏掉）。
CHECK_LIST="$(awk 'NF>=2 {print $NF}' "$HERE/SHA256SUMS" 2>/dev/null)"
# SHA256SUMS 不包含自身，单独补进来
case " $CHECK_LIST " in
  *" SHA256SUMS "*) ;;
  *) CHECK_LIST="$CHECK_LIST SHA256SUMS" ;;
esac
if [ -z "$CHECK_LIST" ]; then
  echo "FAIL: 无法从 SHA256SUMS 读取文件列表"
  CR_BAD=1
fi
for name in $CHECK_LIST; do
  f="$HERE/$name"
  [ -f "$f" ] || { echo "FAIL: 清单中的文件不存在 -> $name"; CR_BAD=1; continue; }
  if grep -q $'\r' "$f" 2>/dev/null; then
    echo "FAIL: CRLF 换行 -> $name"
    CR_BAD=1
  fi
  # BOM 用解释器读前 3 字节判定：od 在精简版 Git for Windows 里可能不存在，
  # 且缺失时必须报错而不是静默放过（fail-closed）。
  if ! BOM="$("$PY" -c 'import sys; print("BOM" if open(sys.argv[1], "rb").read(3) == b"\xef\xbb\xbf" else "OK")' "$f" 2>/dev/null)"; then
    echo "FAIL: BOM 探测无法执行 -> $name"
    CR_BAD=1
  elif [ "$BOM" = "BOM" ]; then
    echo "FAIL: UTF-8 BOM -> $name"
    CR_BAD=1
  fi
done
echo "已检查 $(( $(printf '%s\n' $CHECK_LIST | wc -l | tr -d ' ') )) 个文件"
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
# lane-a / lane-b 内部实际用到的外部命令。tee、seq 已从脚本中移除
# （say 改内建 printf 双写；等待循环改 while 计数），不再列为依赖。
for c in grep sed sort awk; do
  if command -v "$c" >/dev/null 2>&1; then
    echo "OK   $c"
  else
    echo "MISS $c（lane-a/lane-b 运行需要；包本身无误，是环境缺这个命令）"
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
