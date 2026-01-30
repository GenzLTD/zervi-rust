#!/bin/bash
# 下载 llama.cpp 支持的模型（替代 Qwen2）

set -e

MODELS_DIR="./models"
mkdir -p "$MODELS_DIR"

echo "🔧 下载 llama.cpp 支持的模型"
echo "================================"
echo ""

# 检查依赖
if ! command -v huggingface-cli &> /dev/null && ! command -v hf &> /dev/null; then
    echo "📥 安装 huggingface_hub..."
    pip3 install --user huggingface_hub || {
        echo "❌ 安装失败，请手动安装: pip3 install huggingface_hub"
        exit 1
    }
fi

DOWNLOAD_CMD="huggingface-cli"
if command -v hf &> /dev/null; then
    DOWNLOAD_CMD="hf"
fi

echo "请选择要下载的模型（llama.cpp 支持）："
echo ""
echo "1. Phi-3-mini-4k-instruct-Q4_K_M (约 2.3GB) - 推荐，小且高效"
echo "2. Mistral-7B-Instruct-v0.2-Q4_K_M (约 4.1GB) - 性能好"
echo "3. Llama-2-7B-Chat-Q4_K_M (约 4.1GB) - 经典模型"
echo "4. Llama-3.2-3B-Instruct-Q4_K_M (约 2.0GB) - 较新版本"
echo ""

read -p "请选择 (1-4) [默认: 1]: " choice
choice=${choice:-1}

case $choice in
    1)
        MODEL_REPO="TheBloke/Phi-3-mini-4k-instruct-GGUF"
        MODEL_FILE="Phi-3-mini-4k-instruct.Q4_K_M.gguf"
        MODEL_NAME="Phi-3-mini-4k-instruct-Q4_K_M"
        EXPECTED_SIZE="~2.3GB"
        ;;
    2)
        MODEL_REPO="TheBloke/Mistral-7B-Instruct-v0.2-GGUF"
        MODEL_FILE="mistral-7b-instruct-v0.2.Q4_K_M.gguf"
        MODEL_NAME="Mistral-7B-Instruct-v0.2-Q4_K_M"
        EXPECTED_SIZE="~4.1GB"
        ;;
    3)
        MODEL_REPO="TheBloke/Llama-2-7B-Chat-GGUF"
        MODEL_FILE="llama-2-7b-chat.Q4_K_M.gguf"
        MODEL_NAME="Llama-2-7B-Chat-Q4_K_M"
        EXPECTED_SIZE="~4.1GB"
        ;;
    4)
        MODEL_REPO="TheBloke/Llama-3.2-3B-Instruct-GGUF"
        MODEL_FILE="llama-3.2-3b-instruct.Q4_K_M.gguf"
        MODEL_NAME="Llama-3.2-3B-Instruct-Q4_K_M"
        EXPECTED_SIZE="~2.0GB"
        ;;
    *)
        echo "❌ 无效选择"
        exit 1
        ;;
esac

echo ""
echo "📥 准备下载模型："
echo "   名称: $MODEL_NAME"
echo "   仓库: $MODEL_REPO"
echo "   文件: $MODEL_FILE"
echo "   大小: $EXPECTED_SIZE"
echo "   保存到: $MODELS_DIR/"
echo ""

# 检查文件是否已存在
if [ -f "$MODELS_DIR/$MODEL_FILE" ]; then
    FILE_SIZE=$(stat -f%z "$MODELS_DIR/$MODEL_FILE" 2>/dev/null || stat -c%s "$MODELS_DIR/$MODEL_FILE" 2>/dev/null || echo "0")
    if [ "$FILE_SIZE" -gt 1000000000 ]; then
        echo "✅ 模型文件已存在: $MODELS_DIR/$MODEL_FILE"
        echo "   大小: $(du -h "$MODELS_DIR/$MODEL_FILE" | cut -f1)"
        echo ""
        read -p "是否重新下载? (y/N): " redo
        if [[ ! "$redo" =~ ^[Yy]$ ]]; then
            echo "使用现有模型文件"
            exit 0
        fi
        rm -f "$MODELS_DIR/$MODEL_FILE"
    fi
fi

read -p "确认下载? (y/N): " confirm
if [[ ! "$confirm" =~ ^[Yy]$ ]]; then
    echo "已取消"
    exit 0
fi

echo ""
echo "🚀 开始下载..."
echo "💡 提示：下载可能需要一些时间，请耐心等待"
echo ""

# 下载模型
$DOWNLOAD_CMD download "$MODEL_REPO" "$MODEL_FILE" \
    --local-dir "$MODELS_DIR" \
    --local-dir-use-symlinks False || {
    echo "❌ 下载失败"
    echo ""
    echo "💡 替代方案："
    echo "1. 检查网络连接"
    echo "2. 手动下载：访问 https://huggingface.co/$MODEL_REPO"
    echo "3. 将下载的 .gguf 文件放置到: $MODELS_DIR/"
    exit 1
}

# 验证文件
if [ -f "$MODELS_DIR/$MODEL_FILE" ]; then
    FILE_SIZE=$(du -h "$MODELS_DIR/$MODEL_FILE" | cut -f1)
    echo ""
    echo "✅ 模型下载完成！"
    echo "   路径: $MODELS_DIR/$MODEL_FILE"
    echo "   大小: $FILE_SIZE"
    echo ""
    
    # 创建配置文件
    cat > "$MODELS_DIR/.model_info" <<EOF
MODEL_NAME=$MODEL_NAME
MODEL_PATH=$MODELS_DIR/$MODEL_FILE
MODEL_REPO=$MODEL_REPO
DOWNLOAD_DATE=$(date +%Y-%m-%d)
EOF
    
    echo "📋 使用说明："
    echo ""
    echo "1. 设置环境变量："
    echo "   export USE_LOCAL_LLM=true"
    echo "   export LOCAL_LLM_MODEL_PATH=\"$MODELS_DIR/$MODEL_FILE\""
    echo ""
    echo "2. 或修改 .env 文件："
    echo "   USE_LOCAL_LLM=true"
    echo "   LOCAL_LLM_MODEL_PATH=$MODELS_DIR/$MODEL_FILE"
    echo ""
    echo "3. 启动服务："
    echo "   ./quick-test-local-llm.sh"
    echo ""
    echo "💡 提示：此模型与 llama.cpp 兼容，应该可以正常加载"
else
    echo "❌ 模型文件未找到，下载可能失败"
    exit 1
fi

