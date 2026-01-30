#!/bin/bash
# 下载 GGUF 格式的本地 LLM 模型

set -e

MODELS_DIR="./models"
mkdir -p "$MODELS_DIR"

echo "📥 正在下载 GGUF 模型..."

# 推荐模型：Qwen2.5-7B-Instruct-Q4_K_M (约 4.5GB)
# 中文能力强，适合信息提取任务

MODEL_NAME="Qwen2.5-7B-Instruct-Q4_K_M"
MODEL_FILE="${MODEL_NAME}.gguf"

if [ -f "$MODELS_DIR/$MODEL_FILE" ]; then
    FILE_SIZE=$(stat -f%z "$MODELS_DIR/$MODEL_FILE" 2>/dev/null || stat -c%s "$MODELS_DIR/$MODEL_FILE" 2>/dev/null || echo "0")
    if [ "$FILE_SIZE" -gt 1000000 ]; then
        echo "✅ 模型文件已存在: $MODELS_DIR/$MODEL_FILE (大小: $(du -h "$MODELS_DIR/$MODEL_FILE" | cut -f1))"
        exit 0
    else
        echo "⚠️ 检测到不完整的模型文件，将重新下载..."
        rm -f "$MODELS_DIR/$MODEL_FILE"
    fi
fi

echo "📥 正在下载: $MODEL_NAME"
echo "💡 提示：文件较大（约 4.5GB），下载可能需要一些时间..."
echo ""

# 尝试多种下载方式
echo "方法 1: 尝试使用 huggingface-cli..."
if command -v huggingface-cli &> /dev/null; then
    echo "使用 huggingface-cli 下载..."
    huggingface-cli download Qwen/Qwen2.5-7B-Instruct-GGUF "$MODEL_FILE" --local-dir "$MODELS_DIR" --local-dir-use-symlinks False && {
        echo "✅ 下载完成（使用 huggingface-cli）"
        ls -lh "$MODELS_DIR/$MODEL_FILE"
        exit 0
    }
fi

# 方法 2: 尝试直接下载链接（需要找到正确的 URL）
echo ""
echo "方法 2: 尝试直接下载..."
# 尝试多个可能的 URL
MODEL_URLS=(
    "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/${MODEL_FILE}"
    "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/blob/main/${MODEL_FILE}?download=true"
)

for MODEL_URL in "${MODEL_URLS[@]}"; do
    echo "尝试: $MODEL_URL"
    if curl -L -f -o "$MODELS_DIR/$MODEL_FILE.tmp" "$MODEL_URL" 2>/dev/null; then
        FILE_SIZE=$(stat -f%z "$MODELS_DIR/$MODEL_FILE.tmp" 2>/dev/null || stat -c%s "$MODELS_DIR/$MODEL_FILE.tmp" 2>/dev/null || echo "0")
        if [ "$FILE_SIZE" -gt 1000000 ]; then
            mv "$MODELS_DIR/$MODEL_FILE.tmp" "$MODELS_DIR/$MODEL_FILE"
            echo "✅ 下载完成（使用直接链接）"
            ls -lh "$MODELS_DIR/$MODEL_FILE"
            exit 0
        else
            rm -f "$MODELS_DIR/$MODEL_FILE.tmp"
        fi
    fi
done

# 如果都失败了，提供手动下载指南
echo ""
echo "❌ 自动下载失败"
echo ""
echo "💡 请手动下载模型："
echo ""
echo "方法 1: 使用 huggingface-cli（推荐）"
echo "   pip install huggingface_hub"
echo "   huggingface-cli download Qwen/Qwen2.5-7B-Instruct-GGUF $MODEL_FILE --local-dir $MODELS_DIR"
echo ""
echo "方法 2: 从 HuggingFace 网站下载"
echo "   1. 访问: https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF"
echo "   2. 找到 $MODEL_FILE 文件"
echo "   3. 点击下载"
echo "   4. 将文件放置到: $MODELS_DIR/"
echo ""
echo "方法 3: 使用其他模型（更小，适合测试）"
echo "   - Qwen2.5-1.5B-Instruct-Q4_K_M (约 1GB)"
echo "   - ChatGLM3-6B-Q4_K_M (约 3.8GB)"
echo ""
exit 1

echo "✅ 模型下载完成: $MODELS_DIR/$MODEL_FILE"
echo ""
echo "📊 文件信息："
ls -lh "$MODELS_DIR/$MODEL_FILE"

