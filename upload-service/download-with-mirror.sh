#!/bin/bash
# 使用镜像源下载模型（解决网络问题）

set -e

MODEL_NAME="llama-2-7b-chat.Q4_K_M.gguf"
MODEL_REPO="TheBloke/Llama-2-7B-Chat-GGUF"

echo "🌐 使用镜像源下载 Llama-2-7B-Chat 模型"
echo "================================"
echo ""

# 设置 HuggingFace 镜像（如果需要）
export HF_ENDPOINT="${HF_ENDPOINT:-https://huggingface.co}"

echo "📥 下载配置："
echo "   模型: $MODEL_NAME"
echo "   仓库: $MODEL_REPO"
echo "   镜像: $HF_ENDPOINT"
echo ""

# 检查现有文件
INCOMPLETE_FILE=$(find models/.cache -name "*.incomplete" -type f 2>/dev/null | head -1)
if [ -n "$INCOMPLETE_FILE" ]; then
    SIZE=$(ls -l "$INCOMPLETE_FILE" 2>/dev/null | awk '{print $5}' || echo "0")
    SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
    PERCENTAGE=$(echo "scale=1; $SIZE * 100 / 4100000000" | bc)
    
    echo "📊 发现未完成的下载："
    echo "   当前大小: $SIZE_GB GB"
    echo "   完成度: ${PERCENTAGE}%"
    echo "   💡 将使用断点续传"
    echo ""
fi

# 停止旧进程
pkill -f "huggingface-cli.*llama\|hf.*llama" 2>/dev/null || true
sleep 1

# 清理锁文件
find models/.cache -name "*.lock" -delete 2>/dev/null || true

echo "🚀 开始下载..."
echo ""

# 使用 huggingface-cli 下载（支持断点续传）
huggingface-cli download "$MODEL_REPO" "$MODEL_NAME" \
    --local-dir ./models \
    --local-dir-use-symlinks False \
    --resume-download || {
    echo ""
    echo "❌ 下载失败"
    echo ""
    echo "💡 替代方案："
    echo "1. 检查网络连接"
    echo "2. 使用镜像源: export HF_ENDPOINT=https://hf-mirror.com"
    echo "3. 手动下载: 访问 https://huggingface.co/$MODEL_REPO"
    exit 1
}

# 验证下载
FINAL_FILE="./models/${MODEL_NAME}"
if [ -f "$FINAL_FILE" ]; then
    SIZE=$(ls -l "$FINAL_FILE" 2>/dev/null | awk '{print $5}' || echo "0")
    SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
    echo ""
    echo "✅ 下载完成！"
    echo "   文件: $FINAL_FILE"
    echo "   大小: $SIZE_GB GB"
else
    echo "❌ 文件未找到，下载可能失败"
    exit 1
fi

