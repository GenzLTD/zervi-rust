#!/bin/bash
# 快速测试本地 LLM 功能

set -e

echo "🧪 本地 LLM 快速测试"
echo "================================"
echo ""

# 获取脚本所在目录的绝对路径
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# 优先使用 Llama-2 模型（llama.cpp 原生支持）
if [ -f "$SCRIPT_DIR/models/llama-2-7b-chat.Q4_K_M.gguf" ]; then
    MODEL_PATH="$SCRIPT_DIR/models/llama-2-7b-chat.Q4_K_M.gguf"
elif [ -f "$SCRIPT_DIR/models/Phi-3-mini-4k-instruct-q4.gguf" ]; then
    MODEL_PATH="$SCRIPT_DIR/models/Phi-3-mini-4k-instruct-q4.gguf"
    echo "⚠️  使用 Phi-3 模型（可能不被支持），建议使用 Llama-2 模型"
elif [ -f "$SCRIPT_DIR/models/qwen2.5-1.5b-instruct-q4_k_m.gguf" ]; then
    MODEL_PATH="$SCRIPT_DIR/models/qwen2.5-1.5b-instruct-q4_k_m.gguf"
    echo "⚠️  使用 Qwen2 模型（可能不被支持），建议使用 Llama-2 模型"
else
    MODEL_PATH="$SCRIPT_DIR/models/llama-2-7b-chat.Q4_K_M.gguf"
fi

# 检查模型文件
if [ ! -f "$MODEL_PATH" ]; then
    echo "❌ 模型文件不存在: $MODEL_PATH"
    echo ""
    echo "💡 请先下载模型："
    echo "   cd $SCRIPT_DIR"
    echo "   ./setup-local-model.sh"
    exit 1
fi

echo "✅ 模型文件存在: $MODEL_PATH"
FILE_SIZE=$(du -h "$MODEL_PATH" | cut -f1)
echo "   大小: $FILE_SIZE"
echo ""

# 设置环境变量（使用绝对路径）
export USE_LOCAL_LLM=true
export LOCAL_LLM_MODEL_PATH="$MODEL_PATH"
export LOCAL_LLM_CONTEXT_SIZE=4096
export LOCAL_LLM_N_THREADS=4
# 数据库配置（Docker 容器端口 3307）
export DATABASE_URL="mysql://root:adirp_password@127.0.0.1:3307/adirp_upload"

echo "📋 配置信息："
echo "   - 使用本地 LLM: true"
echo "   - 模型路径: $LOCAL_LLM_MODEL_PATH"
echo "   - 上下文大小: $LOCAL_LLM_CONTEXT_SIZE"
echo "   - 线程数: $LOCAL_LLM_N_THREADS"
echo ""

# 检查依赖服务
echo "🔍 检查依赖服务..."
echo ""

# 检查数据库（Docker 容器端口 3307）
if command -v mysql &> /dev/null; then
    if mysql -h127.0.0.1 -P3307 -uroot -padirp_password -e "USE adirp_upload;" 2>/dev/null; then
        echo "✅ 数据库连接正常（Docker 容器端口 3307）"
    else
        echo "⚠️  数据库连接失败，尝试创建数据库..."
        mysql -h127.0.0.1 -P3307 -uroot -padirp_password -e "CREATE DATABASE IF NOT EXISTS adirp_upload;" 2>/dev/null && \
            echo "✅ 数据库已创建" || \
            echo "⚠️  请确保 Docker 容器 watchtower-mysql 正在运行"
    fi
else
    echo "⚠️  未找到 mysql 命令，跳过数据库检查"
fi

# 检查 Consul
if curl -s http://localhost:8500/v1/status/leader > /dev/null 2>&1; then
    echo "✅ Consul 服务运行中"
else
    echo "⚠️  Consul 服务未运行，请启动: consul agent -dev"
fi

echo ""
echo "🚀 准备启动服务..."
echo "💡 提示："
echo "   - 首次启动需要加载模型，可能需要 10-30 秒"
echo "   - 模型加载完成后，服务将监听 http://localhost:3007"
echo "   - 按 Ctrl+C 停止服务"
echo ""

read -p "按 Enter 继续启动服务，或 Ctrl+C 取消..." 

# 检查并停止占用端口的旧进程
echo ""
echo "🔍 检查端口占用..."
PORT=3007
if lsof -ti:$PORT > /dev/null 2>&1; then
    echo "⚠️  检测到端口 $PORT 被占用，正在停止旧进程..."
    lsof -ti:$PORT | xargs kill -9 2>/dev/null || true
    sleep 1
    if lsof -ti:$PORT > /dev/null 2>&1; then
        echo "❌ 无法停止占用端口的进程，请手动处理"
        echo "   运行: lsof -ti:$PORT | xargs kill -9"
        exit 1
    else
        echo "✅ 旧进程已停止"
    fi
else
    echo "✅ 端口 $PORT 可用"
fi

# 切换到 rust-cluster 目录（服务的工作目录）
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster

# 确保环境变量中的路径是绝对路径
export LOCAL_LLM_MODEL_PATH="$MODEL_PATH"

echo ""
echo "🚀 启动服务（工作目录: $(pwd)）"
echo "   模型路径: $LOCAL_LLM_MODEL_PATH"
echo ""

cargo run -p upload-service

