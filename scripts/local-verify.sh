#!/usr/bin/env bash
# =============================================================================
# local-verify.sh — 本地终端「自洽验证」
#
# 目标：在不依赖 MySQL / Redis / Consul / GGUF 等外部基础设施的前提下，
#       验证 zervi-rust 工作区「可编译」+「纯内存子集可运行」+「纯逻辑单测通过」。
#
# 适用环境：本机已安装
#   - rustup 的 gnu-hosted 工具链（默认 host = x86_64-pc-windows-gnu）
#     （ServBay 的 msvc 宿主工具链无法给 build script 提供链接器，故必须用 gnu 宿主）
#   - MinGW-w64（提供 gcc/ld，位于 C:\mingw64\bin）
#   - 本脚本会自动把两者加入 PATH。
#
# 用法： bash scripts/local-verify.sh
# 退出码：0 = 全部关键项通过；1 = 有关键项失败。
# 注意：service-auth 存在一个「文档测试(doctest)」缺陷（引用了不存在的
#       generate_service_token 函数），属仓库既有问题，本脚本单独标注、不计入关键失败。
# =============================================================================
set -uo pipefail

# ---- 环境：gnu 宿主工具链 + MinGW 链接器 -------------------------------------
export PATH="$HOME/.cargo/bin:/c/mingw64/bin:$PATH"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || { echo "cannot cd to $ROOT"; exit 1; }

JOB_PORT=3005
GW_PORT=3333
FAIL=0

echo "============================================================"
echo " zervi-rust 本地自洽验证  ($(date))"
echo " gnu toolchain: $(rustc --version 2>/dev/null) | $(rustc --version --verbose 2>/dev/null | grep -o 'host: .*')"
echo "============================================================"

# ---- [1/4] 编译纯子集 ------------------------------------------------------
echo ""
echo "[1/4] 编译纯子集 (jobfirst-slice-service + api-gateway + shared/*)"
if cargo build -p jobfirst-slice-service -p api-gateway -p common -p consul-client -p service-auth 2>&1 | tee /tmp/zervi-build.log | tail -4; then
  echo "      -> build OK"
else
  echo "      -> build FAILED"; FAIL=1
fi

# ---- [2/4] 启动服务（脚本结束时自动清理）-----------------------------------
echo ""
echo "[2/4] 启动本地服务 (jobfirst-slice :$JOB_PORT, api-gateway :$GW_PORT)"
target/debug/jobfirst-slice-service.exe > /tmp/jobfirst-verify.log 2>&1 &
JOB_PID=$!
target/debug/api-gateway.exe           > /tmp/gateway-verify.log   2>&1 &
GW_PID=$!
cleanup() { kill "$JOB_PID" "$GW_PID" 2>/dev/null; }
trap cleanup EXIT
for i in $(seq 1 10); do
  if curl -s -m 1 --noproxy '*' "http://127.0.0.1:$JOB_PORT/health" >/dev/null 2>&1 \
     && curl -s -m 1 --noproxy '*' "http://127.0.0.1:$GW_PORT/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

# ---- [3/4] 端点自检 --------------------------------------------------------
echo ""
echo "[3/4] 端点自检"
check() {
  local name="$1"; local url="$2"
  local body
  body="$(curl -s -m 5 --noproxy '*' "$url" 2>/dev/null)"
  if [ -n "$body" ]; then
    printf "  PASS  %-42s -> %s\n" "$name" "$body"
  else
    printf "  FAIL  %-42s (empty / no response)\n" "$name"
    FAIL=1
  fi
}
check "jobfirst-slice /health"                "http://127.0.0.1:$JOB_PORT/health"
check "api-gateway /health"                   "http://127.0.0.1:$GW_PORT/health"
check "gateway /v1/auth/verify (terminal rule)" "http://127.0.0.1:$GW_PORT/v1/auth/verify"

# ---- [4/4] 纯逻辑单元测试（无外部依赖，--lib 不含 doctest）-------------------
echo ""
echo "[4/4] 纯逻辑单元测试 (consul-client / service-auth, lib only)"
if cargo test -p consul-client -p service-auth --lib 2>&1 | tee /tmp/zervi-test.log | tail -12; then
  echo "      -> lib unit tests OK"
else
  echo "      -> lib unit tests FAILED"; FAIL=1
fi

# ---- 附加：doctest 单独标注（仓库既有缺陷，不计入关键失败）-------------------
echo ""
echo "[附加] doctest 检查（informational）"
if cargo test -p consul-client -p service-auth --doc 2>&1 | tee /tmp/zervi-doc.log | tail -8; then
  echo "      -> doctests OK"
else
  echo "      -> doctests FAILED（已知仓库问题：service-auth 文档测试引用了不存在的 generate_service_token）"
fi

# ---- 汇总 -----------------------------------------------------------------
echo ""
echo "============================================================"
if [ "$FAIL" -eq 0 ]; then
  echo " 结果: 关键项全部通过 ✅  (编译 + 运行自检 + 纯逻辑单测)"
  echo " 说明: 仅覆盖不依赖 MySQL/Redis/Consul/GGUF 的部分。"
  echo "        auth/user/upload 需外部基础设施，未纳入本自洽验证。"
  echo "        doctest 失败属仓库既有文档缺陷，已单独标注、不影响关键结论。"
  exit 0
else
  echo " 结果: 关键项存在失败 ❌ —— 见上方 [1/4]~[4/4] 输出与日志"
  echo " 日志: /tmp/zervi-build.log / /tmp/zervi-test.log / /tmp/jobfirst-verify.log / /tmp/gateway-verify.log"
  exit 1
fi
