#!/bin/bash
# 修复模型文件（重新下载或从缓存恢复）

set -e

MODEL_NAME="llama-2-7b-chat.Q4_K_M.gguf"
MODEL_REPO="TheBloke/Llama-2-7B-Chat-GGUF"
TARGET_FILE="./models/${MODEL_NAME}"

echo "🔧 修复 Llama-2 模型文件"
echo "================================"
echo ""

# 检查当前文件
if [ -f "$TARGET_FILE" ]; then
    echo "📋 检查当前文件..."
    SIZE=$(stat -f%z "$TARGET_FILE" 2>/dev/null || stat -c%s "$TARGET_FILE" 2>/dev/null || echo "0")
    SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
    
    # 检查文件magic number
    MAGIC=$(head -c 4 "$TARGET_FILE" 2>/dev/null | hexdump -C | head -1 | awk '{print $2$3$4$5}' | tr -d ' ')
    EXPECTED_MAGIC="47475546"  # "GGUF" in hex
    
    echo "   大小: $SIZE_GB GB"
    echo "   Magic: $MAGIC (期望: $EXPECTED_MAGIC)"
    
    if [ "$MAGIC" = "$EXPECTED_MAGIC" ]; then
        echo "   ✅ 文件格式正确"
        exit 0
    else
        echo "   ❌ 文件格式错误，需要修复"
        echo ""
        read -p "是否删除并重新下载？(y/n) " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            rm -f "$TARGET_FILE"
            echo "   ✅ 已删除损坏的文件"
        else
            echo "   ⚠️  保留文件，退出"
            exit 1
        fi
    fi
fi

# 检查缓存目录
echo ""
echo "🔍 检查缓存目录..."
CACHE_FILE=$(find models/.cache -name "*llama-2-7b-chat*" -type f ! -name "*.lock" ! -name "*.metadata" 2>/dev/null | head -1)

if [ -n "$CACHE_FILE" ] && [ -f "$CACHE_FILE" ]; then
    CACHE_SIZE=$(stat -f%z "$CACHE_FILE" 2>/dev/null || stat -c%s "$CACHE_FILE" 2>/dev/null || echo "0")
    CACHE_SIZE_GB=$(echo "scale=2; $CACHE_SIZE / 1024 / 1024 / 1024" | bc)
    
    echo "   ✅ 找到缓存文件: $CACHE_FILE"
    echo "   大小: $CACHE_SIZE_GB GB"
    
    # 检查缓存文件magic
    CACHE_MAGIC=$(head -c 4 "$CACHE_FILE" 2>/dev/null | hexdump -C | head -1 | awk '{print $2$3$4$5}' | tr -d ' ')
    echo "   Magic: $CACHE_MAGIC"
    
    if [ "$CACHE_MAGIC" = "47475546" ]; then
        echo "   ✅ 缓存文件格式正确，复制到目标位置..."
        mkdir -p models
        cp "$CACHE_FILE" "$TARGET_FILE"
        echo "   ✅ 文件已复制"
        exit 0
    else
        echo "   ⚠️  缓存文件格式也不正确"
    fi
fi

# 清理锁文件（可能是僵尸锁）
echo ""
echo "🧹 清理锁文件..."
LOCK_FILE="models/.cache/huggingface/download/llama-2-7b-chat.Q4_K_M.gguf.lock"
if [ -f "$LOCK_FILE" ]; then
    # 检查是否有进程在使用锁文件
    if ! lsof "$LOCK_FILE" > /dev/null 2>&1; then
        echo "   ⚠️  发现僵尸锁文件，正在清理..."
        rm -f "$LOCK_FILE"
        echo "   ✅ 锁文件已清理"
    else
        echo "   ⚠️  锁文件正在被使用，等待进程完成..."
        # 等待最多30秒
        for i in {1..30}; do
            if ! lsof "$LOCK_FILE" > /dev/null 2>&1; then
                rm -f "$LOCK_FILE"
                echo "   ✅ 锁文件已清理"
                break
            fi
            sleep 1
        done
    fi
fi

# 重新下载
echo ""
echo "📥 重新下载模型文件..."
echo "   使用镜像源: https://hf-mirror.com"
echo ""

export HF_ENDPOINT=https://hf-mirror.com

huggingface-cli download "$MODEL_REPO" "$MODEL_NAME" \
    --local-dir ./models \
    --local-dir-use-symlinks False || {
    echo ""
    echo "❌ 下载失败"
    exit 1
}

# 验证下载的文件
if [ -f "$TARGET_FILE" ]; then
    FINAL_SIZE=$(stat -f%z "$TARGET_FILE" 2>/dev/null || stat -c%s "$TARGET_FILE" 2>/dev/null || echo "0")
    FINAL_SIZE_GB=$(echo "scale=2; $FINAL_SIZE / 1024 / 1024 / 1024" | bc)
    FINAL_MAGIC=$(head -c 4 "$TARGET_FILE" 2>/dev/null | hexdump -C | head -1 | awk '{print $2$3$4$5}' | tr -d ' ')
    
    echo ""
    echo "✅ 下载完成"
    echo "   文件: $TARGET_FILE"
    echo "   大小: $FINAL_SIZE_GB GB"
    echo "   Magic: $FINAL_MAGIC"
    
    if [ "$FINAL_MAGIC" = "47475546" ]; then
        echo "   ✅ 文件格式正确"
    else
        echo "   ⚠️  文件格式可能有问题"
    fi
else
    echo "❌ 文件未找到"
    exit 1
fi

