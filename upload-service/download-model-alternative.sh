#!/bin/bash
# 使用 huggingface-cli 下载模型（推荐方法）

set -e

MODELS_DIR="./models"
mkdir -p "$MODELS_DIR"

echo "📥 使用 huggingface-cli 下载模型..."
echo ""

# 检查是否安装了 huggingface-cli
if ! command -v huggingface-cli &> /dev/null; then
    echo "❌ huggingface-cli 未安装"
    echo ""
    echo "💡 安装方法："
    echo "   pip install huggingface_hub"
    echo ""
    echo "或使用 pip3："
    echo "   pip3 install huggingface_hub"
    exit 1
fi

MODEL_NAME="Qwen2.5-7B-Instruct-Q4_K_M"
MODEL_FILE="${MODEL_NAME}.gguf"

echo "📥 正在下载: $MODEL_NAME"
echo "💡 提示：文件较大（约 4.5GB），下载可能需要一些时间..."
echo ""

# 使用 huggingface-cli 下载
huggingface-cli download \
    Qwen/Qwen2.5-7B-Instruct-GGUF \
    "$MODEL_FILE" \
    --local-dir "$MODELS_DIR" \
    --local-dir-use-symlinks False

if [ -f "$MODELS_DIR/$MODEL_FILE" ]; then
    echo ""
    echo "✅ 模型下载完成: $MODELS_DIR/$MODEL_FILE"
    echo "📊 文件信息："
    ls -lh "$MODELS_DIR/$MODEL_FILE"
else
    echo "❌ 下载失败，请检查网络连接或模型名称"
    exit 1
fi

