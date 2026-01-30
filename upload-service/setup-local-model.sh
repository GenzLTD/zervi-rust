#!/bin/bash
# 本地构建/下载 LLM 模型用于测试
# 支持多种模型选择和下载方式

set -e

MODELS_DIR="./models"
mkdir -p "$MODELS_DIR"

echo "🔧 本地 LLM 模型设置工具"
echo "================================"
echo ""

# 检查依赖
echo "📦 检查依赖..."
if ! command -v python3 &> /dev/null; then
    echo "❌ 需要 Python 3"
    exit 1
fi

# 检查 huggingface-cli
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

echo "✅ 依赖检查完成"
echo ""

# 模型选择菜单
echo "请选择要下载的模型："
echo ""
echo "1. Qwen2.5-1.5B-Instruct-Q4_K_M (约 1GB) - 推荐用于快速测试"
echo "2. Qwen2.5-7B-Instruct-Q4_K_M (约 4.5GB) - 平衡性能和大小"
echo "3. ChatGLM3-6B-Q4_K_M (约 3.8GB) - 中文优化"
echo "4. Qwen2.5-3B-Instruct-Q4_K_M (约 2GB) - 中等大小"
echo "5. 自定义模型（手动输入）"
echo ""

read -p "请选择 (1-5) [默认: 1]: " choice
choice=${choice:-1}

case $choice in
    1)
        MODEL_REPO="Qwen/Qwen2.5-1.5B-Instruct-GGUF"
        MODEL_FILE="qwen2.5-1.5b-instruct-q4_k_m.gguf"
        MODEL_NAME="Qwen2.5-1.5B-Instruct-Q4_K_M"
        EXPECTED_SIZE="~1GB"
        ;;
    2)
        MODEL_REPO="Qwen/Qwen2.5-7B-Instruct-GGUF"
        MODEL_FILE="qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf"
        MODEL_NAME="Qwen2.5-7B-Instruct-Q4_K_M (Part 1)"
        EXPECTED_SIZE="~2.3GB"
        echo "⚠️  注意：7B 模型分为两部分，需要下载两个文件"
        ;;
    3)
        MODEL_REPO="THUDM/chatglm3-6b-gguf"
        MODEL_FILE="chatglm3-6b-q4_k_m.gguf"
        MODEL_NAME="ChatGLM3-6B-Q4_K_M"
        EXPECTED_SIZE="~3.8GB"
        ;;
    4)
        MODEL_REPO="Qwen/Qwen2.5-3B-Instruct-GGUF"
        MODEL_FILE="qwen2.5-3b-instruct-q4_k_m.gguf"
        MODEL_NAME="Qwen2.5-3B-Instruct-Q4_K_M"
        EXPECTED_SIZE="~2GB"
        ;;
    5)
        read -p "请输入模型仓库 (例如: Qwen/Qwen2.5-7B-Instruct-GGUF): " MODEL_REPO
        read -p "请输入模型文件名 (例如: qwen2.5-7b-instruct-q4_k_m.gguf): " MODEL_FILE
        MODEL_NAME="自定义模型"
        EXPECTED_SIZE="未知"
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
if [ "$choice" == "2" ]; then
    # 7B 模型需要下载两部分
    echo "下载第 1 部分..."
    $DOWNLOAD_CMD download "$MODEL_REPO" "qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf" \
        --local-dir "$MODELS_DIR" --local-dir-use-symlinks False || {
        echo "❌ 下载失败"
        exit 1
    }
    
    echo "下载第 2 部分..."
    $DOWNLOAD_CMD download "$MODEL_REPO" "qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf" \
        --local-dir "$MODELS_DIR" --local-dir-use-symlinks False || {
        echo "❌ 下载失败"
        exit 1
    }
    
    # 合并文件（如果需要）
    echo "合并模型文件..."
    cat "$MODELS_DIR/qwen2.5-7b-instruct-q4_k_m-00001-of-00002.gguf" \
        "$MODELS_DIR/qwen2.5-7b-instruct-q4_k_m-00002-of-00002.gguf" > \
        "$MODELS_DIR/qwen2.5-7b-instruct-q4_k_m.gguf" || {
        echo "⚠️  合并失败，将使用分片文件"
    }
    
    MODEL_FILE="qwen2.5-7b-instruct-q4_k_m.gguf"
else
    $DOWNLOAD_CMD download "$MODEL_REPO" "$MODEL_FILE" \
        --local-dir "$MODELS_DIR" --local-dir-use-symlinks False || {
        echo "❌ 下载失败"
        exit 1
    }
fi

# 验证文件
FULL_PATH="$MODELS_DIR/$MODEL_FILE"
if [ -f "$FULL_PATH" ]; then
    FILE_SIZE=$(du -h "$FULL_PATH" | cut -f1)
    echo ""
    echo "✅ 模型下载完成！"
    echo "   路径: $FULL_PATH"
    echo "   大小: $FILE_SIZE"
    echo ""
    
    # 创建配置文件
    cat > "$MODELS_DIR/.model_info" <<EOF
MODEL_NAME=$MODEL_NAME
MODEL_PATH=$FULL_PATH
MODEL_REPO=$MODEL_REPO
DOWNLOAD_DATE=$(date +%Y-%m-%d)
EOF
    
    echo "📋 使用说明："
    echo ""
    echo "1. 设置环境变量："
    echo "   export USE_LOCAL_LLM=true"
    echo "   export LOCAL_LLM_MODEL_PATH=\"$FULL_PATH\""
    echo ""
    echo "2. 或修改 .env 文件："
    echo "   USE_LOCAL_LLM=true"
    echo "   LOCAL_LLM_MODEL_PATH=$FULL_PATH"
    echo ""
    echo "3. 启动服务："
    echo "   ./test-with-model.sh \"$FULL_PATH\""
    echo ""
else
    echo "❌ 模型文件未找到，下载可能失败"
    exit 1
fi

