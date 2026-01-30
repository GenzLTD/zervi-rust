#!/bin/bash
# 检查 Llama-2-7B-Chat 模型下载进度

MODEL_FILE="./models/llama-2-7b-chat.Q4_K_M.gguf"
EXPECTED_SIZE=4100000000  # 约 4.1GB

echo "🔍 Llama-2-7B-Chat 模型下载状态检查"
echo "================================"
echo ""

if [ -f "$MODEL_FILE" ]; then
    FILE_SIZE=$(stat -f%z "$MODEL_FILE" 2>/dev/null || stat -c%s "$MODEL_FILE" 2>/dev/null || echo "0")
    FILE_SIZE_GB=$(echo "scale=2; $FILE_SIZE / 1024 / 1024 / 1024" | bc)
    PERCENTAGE=$(echo "scale=1; $FILE_SIZE * 100 / $EXPECTED_SIZE" | bc)
    
    echo "✅ 模型文件存在: $MODEL_FILE"
    echo "   当前大小: $FILE_SIZE_GB GB"
    echo "   预期大小: ~4.1 GB"
    echo "   完成度: ${PERCENTAGE}%"
    
    if [ "$FILE_SIZE" -gt 3500000000 ]; then
        echo "   ✅ 文件大小正常，下载可能已完成"
    elif [ "$FILE_SIZE" -gt 1000000000 ]; then
        echo "   ⏳ 文件正在下载中..."
    else
        echo "   ⚠️  文件较小，可能刚开始下载"
    fi
else
    echo "⏳ 模型文件尚未创建"
    
    # 检查下载缓存
    CACHE_FILES=$(find models/.cache -name "*llama-2-7b-chat*" -type f 2>/dev/null | head -1)
    if [ -n "$CACHE_FILES" ]; then
        CACHE_SIZE=$(du -h "$CACHE_FILES" 2>/dev/null | cut -f1)
        echo "   📥 发现下载缓存: $CACHE_FILES"
        echo "   缓存大小: $CACHE_SIZE"
    fi
fi

echo ""

# 检查下载进程
if pgrep -f "huggingface-cli.*llama\|hf.*llama" > /dev/null; then
    echo "📥 下载进程正在运行"
    ps aux | grep -E "huggingface-cli.*llama|hf.*llama" | grep -v grep | head -1
else
    echo "✅ 下载进程已完成或未运行"
fi

echo ""

# 检查所有模型文件
echo "📦 当前所有模型文件："
ls -lh models/*.gguf 2>/dev/null | awk '{print "   " $9 " (" $5 ")"}'

