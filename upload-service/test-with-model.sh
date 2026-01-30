#!/bin/bash
# 使用指定的模型文件测试本地 LLM 功能

set -e

echo "🧪 本地 LLM 功能测试"
echo ""

# 如果用户提供了模型路径作为参数，使用它
if [ -n "$1" ]; then
    MODEL_PATH="$1"
elif [ -n "$LOCAL_LLM_MODEL_PATH" ]; then
    MODEL_PATH="$LOCAL_LLM_MODEL_PATH"
else
    # 默认路径
    MODEL_PATH="./models/Qwen2.5-7B-Instruct-Q4_K_M.gguf"
fi

# 检查模型文件
if [ ! -f "$MODEL_PATH" ]; then
    echo "❌ 模型文件不存在: $MODEL_PATH"
    echo ""
    echo "💡 使用方法："
    echo "   $0 /path/to/your/model.gguf"
    echo ""
    echo "或设置环境变量："
    echo "   export LOCAL_LLM_MODEL_PATH=/path/to/your/model.gguf"
    echo "   $0"
    echo ""
    echo "📂 正在搜索可能的模型文件..."
    find ~ -name "*.gguf" -type f -size +100M 2>/dev/null | head -5
    exit 1
fi

# 获取绝对路径
if [[ "$MODEL_PATH" != /* ]]; then
    MODEL_PATH="$(cd "$(dirname "$MODEL_PATH")" && pwd)/$(basename "$MODEL_PATH")"
fi

echo "✅ 找到模型文件: $MODEL_PATH"
FILE_SIZE=$(du -h "$MODEL_PATH" | cut -f1)
echo "   大小: $FILE_SIZE"
echo ""

# 设置环境变量
export USE_LOCAL_LLM=true
export LOCAL_LLM_MODEL_PATH="$MODEL_PATH"
export LOCAL_LLM_CONTEXT_SIZE="${LOCAL_LLM_CONTEXT_SIZE:-4096}"
export LOCAL_LLM_N_THREADS="${LOCAL_LLM_N_THREADS:-4}"
# 数据库配置（Docker 容器端口 3307）
export DATABASE_URL="${DATABASE_URL:-mysql://root:adirp_password@127.0.0.1:3307/adirp_upload}"

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

