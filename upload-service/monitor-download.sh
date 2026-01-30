#!/bin/bash
# 监控模型下载进度（实时更新版本）

MODEL_NAME="llama-2-7b-chat.Q4_K_M.gguf"
EXPECTED_SIZE=4100000000  # 约 4.1GB

echo "📊 监控 Llama-2-7B-Chat 模型下载进度"
echo "================================"
echo ""

# 首先查找 .incomplete 文件（实际下载中的文件）
INCOMPLETE_FILE=$(find models/.cache -name "*.incomplete" -type f 2>/dev/null | head -1)

if [ -n "$INCOMPLETE_FILE" ] && [ -f "$INCOMPLETE_FILE" ]; then
    # 读取文件大小（强制刷新，使用 ls 获取实时大小）
    SIZE=$(ls -l "$INCOMPLETE_FILE" 2>/dev/null | awk '{print $5}' || stat -f%z "$INCOMPLETE_FILE" 2>/dev/null || stat -c%s "$INCOMPLETE_FILE" 2>/dev/null || echo "0")
    
    if [ "$SIZE" -gt 0 ]; then
        SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
        PERCENTAGE=$(echo "scale=1; $SIZE * 100 / $EXPECTED_SIZE" | bc)
        REMAINING_BYTES=$((EXPECTED_SIZE - SIZE))
        REMAINING_MB=$(echo "scale=0; $REMAINING_BYTES / 1024 / 1024" | bc)
        
        echo "📥 下载中: $(basename "$INCOMPLETE_FILE")"
        echo "   当前大小: $SIZE_GB GB / 4.1 GB"
        echo "   完成度: ${PERCENTAGE}%"
        echo "   剩余: 约 $REMAINING_MB MB"
        echo "   检查时间: $(date '+%H:%M:%S')"
        echo "   文件路径: $INCOMPLETE_FILE"
        echo ""
        
        # 检查下载进程状态（更宽松的匹配）
        DOWNLOAD_PID=$(pgrep -f "huggingface.*llama\|hf.*llama\|python.*download.*llama" | head -1)
        if [ -z "$DOWNLOAD_PID" ]; then
            # 备用：直接搜索包含 llama 的进程
            DOWNLOAD_PID=$(ps aux | grep -i "llama" | grep -v grep | grep -i "download\|huggingface\|hf" | awk '{print $2}' | head -1)
        fi
        
        if [ -n "$DOWNLOAD_PID" ]; then
            # 检查进程状态
            PROC_STATE=$(ps -p "$DOWNLOAD_PID" -o state= 2>/dev/null | tr -d ' ')
            PROC_CPU=$(ps -p "$DOWNLOAD_PID" -o %cpu= 2>/dev/null | tr -d ' ')
            PROC_ETIME=$(ps -p "$DOWNLOAD_PID" -o etime= 2>/dev/null | tr -d ' ')
            
            if [ -n "$PROC_STATE" ]; then
                echo "   ✅ 下载进程运行中 (PID: $DOWNLOAD_PID)"
                echo "   进程状态: $PROC_STATE"
                echo "   CPU 使用: ${PROC_CPU}%"
                echo "   运行时间: $PROC_ETIME"
                
                # 如果进程状态是 S (sleeping) 且 CPU 为 0，可能卡住了
                if [ "$PROC_STATE" = "S" ] && [ "$(echo "$PROC_CPU" | cut -d. -f1)" = "0" ]; then
                    echo "   ⚠️  进程可能处于等待状态（网络或 I/O）"
                fi
            else
                echo "   ⚠️  下载进程未运行（可能已完成或中断）"
            fi
        else
            echo "   ⚠️  下载进程未运行（可能已完成或中断）"
        fi
    else
        echo "⏳ 下载文件存在但大小为 0，可能刚开始下载"
    fi
elif [ -n "$INCOMPLETE_FILES" ]; then
    # 备用：查找其他可能的下载文件
    for file in $INCOMPLETE_FILES; do
        if [ -f "$file" ] && [ ! -f "$file.lock" ]; then
            SIZE=$(stat -f%z "$file" 2>/dev/null || stat -c%s "$file" 2>/dev/null || echo "0")
            if [ "$SIZE" -gt 0 ]; then
                SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
                PERCENTAGE=$(echo "scale=1; $SIZE * 100 / $EXPECTED_SIZE" | bc)
                
                echo "📥 下载中: $(basename "$file")"
                echo "   当前大小: $SIZE_GB GB"
                echo "   完成度: ${PERCENTAGE}%"
                echo "   路径: $file"
                echo ""
            fi
        fi
    done
else
    echo "⏳ 未找到下载中的文件"
    echo "   检查下载进程..."
    
    if pgrep -f "huggingface-cli.*llama\|hf.*llama" > /dev/null; then
        echo "   ✅ 下载进程正在运行"
        ps aux | grep -E "huggingface-cli.*llama|hf.*llama" | grep -v grep | head -1
        echo ""
        echo "   💡 提示：下载文件可能在缓存目录中"
        echo "   检查: find models/.cache -name '*.incomplete' -exec du -h {} \\;"
    else
        echo "   ⚠️  下载进程未运行"
    fi
fi

# 检查最终文件
FINAL_FILE="./models/${MODEL_NAME}"
if [ -f "$FINAL_FILE" ]; then
    SIZE=$(stat -f%z "$FINAL_FILE" 2>/dev/null || stat -c%s "$FINAL_FILE" 2>/dev/null || echo "0")
    SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
    echo "✅ 最终文件已存在: $FINAL_FILE"
    echo "   大小: $SIZE_GB GB"
    
    if [ "$SIZE" -gt 3500000000 ]; then
        echo "   ✅ 下载完成！"
    else
        echo "   ⚠️  文件可能不完整"
    fi
fi

