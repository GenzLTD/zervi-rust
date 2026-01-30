#!/bin/bash
# 继续下载模型（使用断点续传）

set -e

MODEL_NAME="llama-2-7b-chat.Q4_K_M.gguf"
MODEL_REPO="TheBloke/Llama-2-7B-Chat-GGUF"

echo "📥 继续下载 Llama-2 模型"
echo "================================"
echo ""

# 检查未完成的下载
INCOMPLETE=$(find models/.cache -name "*.incomplete" -type f 2>/dev/null | head -1)
if [ -n "$INCOMPLETE" ]; then
    SIZE=$(stat -f%z "$INCOMPLETE" 2>/dev/null || stat -c%s "$INCOMPLETE" 2>/dev/null || echo "0")
    SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
    PERCENTAGE=$(echo "scale=1; $SIZE * 100 / 4100000000" | bc)
    
    echo "📊 发现未完成的下载:"
    echo "   文件: $(basename "$INCOMPLETE")"
    echo "   大小: $SIZE_GB GB"
    echo "   进度: ${PERCENTAGE}%"
    echo ""
    echo "💡 将使用断点续传继续下载"
    echo ""
fi

# 确保没有其他下载进程
echo "🧹 清理旧进程和锁文件..."
pkill -f "huggingface-cli.*llama" 2>/dev/null || true
sleep 2
rm -f models/.cache/huggingface/download/*.lock 2>/dev/null || true
echo "✅ 清理完成"
echo ""

# 使用镜像源下载
export HF_ENDPOINT=https://hf-mirror.com

echo "🚀 开始下载（后台运行）..."
echo "   使用镜像: $HF_ENDPOINT"
echo ""

nohup huggingface-cli download "$MODEL_REPO" "$MODEL_NAME" \
    --local-dir ./models \
    --local-dir-use-symlinks False \
    > /tmp/llama2_download.log 2>&1 &

DOWNLOAD_PID=$!
echo "✅ 下载进程已启动，PID: $DOWNLOAD_PID"
echo ""
echo "📋 监控下载进度："
echo "   ./monitor-download.sh"
echo ""
echo "📋 查看下载日志："
echo "   tail -f /tmp/llama2_download.log"
echo ""

