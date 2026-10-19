#!/usr/bin/env bash
# 公共库：仓库定位 / Python 探测 / 环境指纹 / commit 门禁
# 被 lane-a.sh、lane-b.sh、verify.sh 引用，不单独执行。
set -u

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---------- 1) 仓库根 ----------
# 优先 $REPO，其次由脚本自身位置向上找（支持 git worktree）
resolve_repo() {
  if [ -n "${REPO:-}" ]; then
    (cd "$REPO" 2>/dev/null && pwd) && return
    echo ""
    return
  fi
  git -C "$E2E_DIR" rev-parse --show-toplevel 2>/dev/null
}
REPO="$(resolve_repo)"
if [ -z "$REPO" ] || [ ! -d "$REPO/.git" -a ! -f "$REPO/.git" ]; then
  echo "FATAL: 定位不到 git 仓库根。请设置 REPO=<zervi-rust 路径>" >&2
  exit 2
fi

# ---------- 2) Python ----------
resolve_py() {
  if [ -n "${PY:-}" ]; then
    echo "$PY"
    return
  fi
  for c in python3 python py; do
    if command -v "$c" >/dev/null 2>&1 && "$c" -c 'import sys; sys.exit(0 if sys.version_info>=(3,8) else 1)' >/dev/null 2>&1; then
      echo "$c"
      return
    fi
  done
  echo ""
}
PY="$(resolve_py)"
if [ -z "$PY" ]; then
  echo "FATAL: 未找到 Python >= 3.8（可用 PY=<path> 指定）" >&2
  exit 2
fi

# ---------- 3) 被测基线 ----------
BASE_COMMIT="$(tr -d '[:space:]' < "$E2E_DIR/EXPECTED_COMMIT" 2>/dev/null)"
if [ -z "$BASE_COMMIT" ]; then
  echo "FATAL: 缺少 EXPECTED_COMMIT" >&2
  exit 2
fi

# 只允许"新增/修改本目录"这一种漂移，其余一律拒绝执行（fail-closed）
assert_commit() {
  local head drift
  head="$(git -C "$REPO" rev-parse HEAD 2>/dev/null)"
  if ! git -C "$REPO" cat-file -e "${BASE_COMMIT}^{commit}" 2>/dev/null; then
    echo "FATAL: 本地缺少基线提交 $BASE_COMMIT，请先 git fetch --all" >&2
    return 1
  fi
  if ! git -C "$REPO" merge-base --is-ancestor "$BASE_COMMIT" "$head" 2>/dev/null; then
    echo "FATAL: HEAD($head) 不包含基线 $BASE_COMMIT" >&2
    return 1
  fi
  drift="$(git -C "$REPO" diff --name-only "$BASE_COMMIT".."$head" -- . \
            | grep -v '^tools/e2e-supabase/' || true)"
  if [ -n "$drift" ]; then
    echo "FATAL: 被测代码与基线不一致，以下文件发生漂移：" >&2
    while IFS= read -r line; do
      [ -n "$line" ] && echo "  $line" >&2
    done <<< "$drift"
    echo "若确认要在漂移历史上运行，设置 ALLOW_COMMIT_DRIFT=1 重跑。" >&2
    [ "${ALLOW_COMMIT_DRIFT:-0}" = "1" ] || return 1
  fi
  return 0
}

# ---------- 4) 环境指纹 ----------
# 对方回贴这一整段即可比对。os/arch/bash 允许不同，其余应一致。
print_env() {
  local head dirty
  head="$(git -C "$REPO" rev-parse HEAD)"
  dirty="$(git -C "$REPO" status --porcelain | wc -l | tr -d ' ')"
  echo "=== 环境指纹 ==="
  echo "os          $(uname -s -r 2>/dev/null || echo unknown)"
  echo "arch        $(uname -m 2>/dev/null || echo unknown)"
  echo "bash        ${BASH_VERSION:-unknown}"
  echo "git         $(git --version 2>/dev/null | awk '{print $3}')"
  echo "rustc       $(rustc -V 2>/dev/null || echo MISSING)"
  echo "cargo       $(cargo -V 2>/dev/null || echo MISSING)"
  echo "python      $("$PY" -c 'import platform,sys; print(sys.version.split()[0], platform.machine())' 2>/dev/null || echo MISSING)"
  echo "base_commit $BASE_COMMIT"
  echo "head_commit $head"
  echo "code_tree   $(git -C "$REPO" rev-parse "${BASE_COMMIT}^{tree}")"
  echo "worktree_dirty $dirty"
}

require_cmd() {
  local missing=0
  for c in "$@"; do
    command -v "$c" >/dev/null 2>&1 || { echo "FATAL: 缺少命令 $c" >&2; missing=1; }
  done
  return $missing
}
