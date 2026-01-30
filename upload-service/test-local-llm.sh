#!/bin/bash
# 快速测试本地 LLM 功能

set -e

echo "🧪 本地 LLM 功能测试"
echo ""

# 检查模型文件
MODEL_PATH="${LOCAL_LLM_MODEL_PATH:-./models/Qwen2.5-7B-Instruct-Q4_K_M.gguf}"

if [ ! -f "$MODEL_PATH" ]; then
    echo "❌ 模型文件不存在: $MODEL_PATH"
    echo ""
    echo "💡 请先下载模型："
    echo "   ./download-model.sh"
    echo ""
    echo "或设置环境变量："
    echo "   export LOCAL_LLM_MODEL_PATH=/path/to/your/model.gguf"
    exit 1
fi

echo "✅ 模型文件存在: $MODEL_PATH"
echo ""

# 设置环境变量
export USE_LOCAL_LLM=true
export LOCAL_LLM_MODEL_PATH="$MODEL_PATH"
export LOCAL_LLM_CONTEXT_SIZE="${LOCAL_LLM_CONTEXT_SIZE:-4096}"
export LOCAL_LLM_N_THREADS="${LOCAL_LLM_N_THREADS:-4}"

echo "📋 配置信息："
echo "   - 使用本地 LLM: true"
echo "   - 模型路径: $LOCAL_LLM_MODEL_PATH"
echo "   - 上下文大小: $LOCAL_LLM_CONTEXT_SIZE"
echo "   - 线程数: $LOCAL_LLM_N_THREADS"
echo ""

echo "🚀 启动服务..."
echo "💡 提示：首次启动需要加载模型，可能需要 10-30 秒"
echo ""

cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster
cargo run -p upload-service

