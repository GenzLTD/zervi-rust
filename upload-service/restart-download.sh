#!/bin/bash
# 重启模型下载（清理并重新开始）

set -e

MODEL_NAME="llama-2-7b-chat.Q4_K_M.gguf"
MODEL_REPO="TheBloke/Llama-2-7B-Chat-GGUF"

echo "🔄 重启 Llama-2-7B-Chat 模型下载"
echo "================================"
echo ""

# 停止旧进程
echo "1. 停止旧下载进程..."
pkill -f "huggingface-cli.*llama\|hf.*llama" 2>/dev/null || true
sleep 2

# 清理锁文件
echo "2. 清理锁文件..."
find models/.cache -name "*.lock" -delete 2>/dev/null || true
echo "   ✅ 锁文件已清理"

# 检查现有文件
INCOMPLETE_FILE=$(find models/.cache -name "*.incomplete" -type f 2>/dev/null | head -1)
if [ -n "$INCOMPLETE_FILE" ]; then
    SIZE=$(ls -l "$INCOMPLETE_FILE" 2>/dev/null | awk '{print $5}' || echo "0")
    SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
    echo ""
    echo "3. 发现未完成的下载文件:"
    echo "   文件: $(basename "$INCOMPLETE_FILE")"
    echo "   大小: $SIZE_GB GB"
    echo "   💡 将使用断点续传继续下载"
fi

# 重新启动下载
echo ""
echo "4. 启动下载（后台运行）..."
nohup huggingface-cli download "$MODEL_REPO" "$MODEL_NAME" \
    --local-dir ./models \
    --local-dir-use-symlinks False \
    > /tmp/llama2_download.log 2>&1 &

DOWNLOAD_PID=$!
echo "   ✅ 下载进程已启动，PID: $DOWNLOAD_PID"
echo ""

# 等待一下，检查是否正常启动
sleep 3
if ps -p "$DOWNLOAD_PID" > /dev/null 2>&1; then
    echo "✅ 下载进程运行正常"
    echo ""
    echo "📊 监控下载进度："
    echo "   ./monitor-download.sh"
    echo ""
    echo "📋 查看下载日志："
    echo "   tail -f /tmp/llama2_download.log"
else
    echo "❌ 下载进程启动失败，查看日志："
    tail -20 /tmp/llama2_download.log 2>/dev/null || echo "日志文件不存在"
    exit 1
fi

