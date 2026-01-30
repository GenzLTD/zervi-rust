#!/bin/bash
# 实时监控模型下载进度（带时间戳和变化检测）

MODEL_NAME="llama-2-7b-chat.Q4_K_M.gguf"
EXPECTED_SIZE=4100000000  # 约 4.1GB

echo "📊 实时监控 Llama-2-7B-Chat 模型下载进度"
echo "================================"
echo "按 Ctrl+C 停止监控"
echo ""

PREV_SIZE=0
NO_CHANGE_COUNT=0

while true; do
    # 查找 .incomplete 文件
    INCOMPLETE_FILE=$(find models/.cache -name "*.incomplete" -type f 2>/dev/null | head -1)
    
    if [ -n "$INCOMPLETE_FILE" ] && [ -f "$INCOMPLETE_FILE" ]; then
        SIZE=$(stat -f%z "$INCOMPLETE_FILE" 2>/dev/null || stat -c%s "$INCOMPLETE_FILE" 2>/dev/null || echo "0")
        SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
        PERCENTAGE=$(echo "scale=1; $SIZE * 100 / $EXPECTED_SIZE" | bc)
        REMAINING_BYTES=$((EXPECTED_SIZE - SIZE))
        REMAINING_MB=$(echo "scale=0; $REMAINING_BYTES / 1024 / 1024" | bc)
        
        # 检测变化
        if [ "$SIZE" -eq "$PREV_SIZE" ]; then
            NO_CHANGE_COUNT=$((NO_CHANGE_COUNT + 1))
        else
            NO_CHANGE_COUNT=0
            DIFF=$((SIZE - PREV_SIZE))
            if [ "$PREV_SIZE" -gt 0 ]; then
                DIFF_MB=$(echo "scale=2; $DIFF / 1024 / 1024" | bc)
                echo "📈 增长: +$DIFF_MB MB"
            fi
        fi
        
        # 清屏并显示当前状态
        clear
        echo "📊 实时监控 Llama-2-7B-Chat 模型下载进度"
        echo "================================"
        echo ""
        echo "📥 下载中: $(basename "$INCOMPLETE_FILE")"
        echo "   当前大小: $SIZE_GB GB / 4.1 GB"
        echo "   完成度: ${PERCENTAGE}%"
        echo "   剩余: 约 $REMAINING_MB MB"
        echo "   检查时间: $(date '+%Y-%m-%d %H:%M:%S')"
        echo ""
        
        if [ "$NO_CHANGE_COUNT" -gt 0 ]; then
            echo "   ⚠️  文件大小已 ${NO_CHANGE_COUNT} 次未变化"
            if [ "$NO_CHANGE_COUNT" -gt 3 ]; then
                echo "   ❌ 下载可能已卡住，建议检查网络或重启下载"
            fi
        fi
        
        # 检查进程
        if pgrep -f "huggingface-cli.*llama\|hf.*llama" > /dev/null; then
            echo "   ✅ 下载进程运行中"
        else
            echo "   ⚠️  下载进程未运行"
        fi
        
        PREV_SIZE=$SIZE
        
        # 检查是否完成
        if [ "$SIZE" -ge 4000000000 ]; then
            echo ""
            echo "🎉 下载接近完成！"
            break
        fi
    else
        # 检查最终文件
        FINAL_FILE="./models/${MODEL_NAME}"
        if [ -f "$FINAL_FILE" ]; then
            SIZE=$(stat -f%z "$FINAL_FILE" 2>/dev/null || stat -c%s "$FINAL_FILE" 2>/dev/null || echo "0")
            SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
            clear
            echo "✅ 下载完成！"
            echo "   文件: $FINAL_FILE"
            echo "   大小: $SIZE_GB GB"
            break
        else
            clear
            echo "⏳ 等待下载开始..."
            echo "   时间: $(date '+%Y-%m-%d %H:%M:%S')"
        fi
    fi
    
    sleep 2
done

