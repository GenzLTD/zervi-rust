#!/bin/bash
# 下载较小的测试模型（适合快速测试）

set -e

MODELS_DIR="./models"
mkdir -p "$MODELS_DIR"

echo "📥 下载测试模型（较小，适合快速测试）..."
echo ""

# 使用更小的模型进行测试：Qwen2.5-1.5B-Instruct
MODEL_REPO="Qwen/Qwen2.5-1.5B-Instruct-GGUF"
MODEL_FILE="Qwen2.5-1.5B-Instruct-Q4_K_M.gguf"

if [ -f "$MODELS_DIR/$MODEL_FILE" ]; then
    FILE_SIZE=$(stat -f%z "$MODELS_DIR/$MODEL_FILE" 2>/dev/null || stat -c%s "$MODELS_DIR/$MODEL_FILE" 2>/dev/null || echo "0")
    if [ "$FILE_SIZE" -gt 1000000 ]; then
        echo "✅ 测试模型已存在: $MODELS_DIR/$MODEL_FILE (大小: $(du -h "$MODELS_DIR/$MODEL_FILE" | cut -f1))"
        exit 0
    fi
fi

echo "📥 正在下载测试模型: $MODEL_FILE (约 1GB)"
echo "💡 提示：这是较小的测试模型，下载速度更快"
echo ""

if command -v huggingface-cli &> /dev/null || command -v hf &> /dev/null; then
    DOWNLOAD_CMD="huggingface-cli"
    if command -v hf &> /dev/null; then
        DOWNLOAD_CMD="hf"
    fi
    
    echo "使用 $DOWNLOAD_CMD 下载..."
    $DOWNLOAD_CMD download "$MODEL_REPO" "$MODEL_FILE" --local-dir "$MODELS_DIR" --local-dir-use-symlinks False || {
        echo "❌ 下载失败，尝试其他方法..."
        exit 1
    }
    
    if [ -f "$MODELS_DIR/$MODEL_FILE" ]; then
        echo ""
        echo "✅ 测试模型下载完成: $MODELS_DIR/$MODEL_FILE"
        echo "📊 文件信息："
        ls -lh "$MODELS_DIR/$MODEL_FILE"
        echo ""
        echo "💡 提示：测试完成后，可以下载更大的模型以获得更好的效果"
        exit 0
    fi
else
    echo "❌ 未找到 huggingface-cli 或 hf 命令"
    echo "请安装: pip install huggingface_hub"
    exit 1
fi

