#!/bin/bash
# 检查模型下载进度

MODEL_FILE="./models/Phi-3-mini-4k-instruct-q4.gguf"

if [ -f "$MODEL_FILE" ]; then
    SIZE=$(du -h "$MODEL_FILE" | cut -f1)
    echo "✅ 模型文件存在: $MODEL_FILE"
    echo "   大小: $SIZE"
    
    # 检查是否完整（应该大于 1GB）
    FILE_SIZE_BYTES=$(stat -f%z "$MODEL_FILE" 2>/dev/null || stat -c%s "$MODEL_FILE" 2>/dev/null || echo "0")
    if [ "$FILE_SIZE_BYTES" -gt 1000000000 ]; then
        echo "✅ 文件大小正常，下载可能已完成"
    else
        echo "⚠️  文件较小，可能还在下载中"
    fi
else
    echo "⏳ 模型文件尚未下载完成"
    echo "   检查下载日志: tail -f /tmp/model_download.log"
fi

# 检查下载进程
if pgrep -f "huggingface-cli.*Phi-3" > /dev/null; then
    echo "📥 下载进程正在运行"
else
    echo "✅ 下载进程已完成或未运行"
fi

