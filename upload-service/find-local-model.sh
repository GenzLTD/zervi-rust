#!/bin/bash
# 查找本地已有的 LLM 模型文件

echo "🔍 正在查找本地 LLM 模型文件..."
echo ""

# 搜索常见的模型文件格式和位置
SEARCH_PATHS=(
    "$HOME"
    "$HOME/Downloads"
    "$HOME/Documents"
    "$HOME/Desktop"
    "/Users/szjason72/xiajun"
    "/Users/szjason72/xiajun/adirp-ctyun/rust-cluster/upload-service/models"
    "$HOME/.cache/huggingface"
)

FOUND_MODELS=()

echo "📂 搜索路径："
for path in "${SEARCH_PATHS[@]}"; do
    if [ -d "$path" ]; then
        echo "  - $path"
        # 查找 GGUF 文件（大于 100MB）
        while IFS= read -r -d '' file; do
            FOUND_MODELS+=("$file")
        done < <(find "$path" -maxdepth 5 -name "*.gguf" -type f -size +100M -print0 2>/dev/null)
    fi
done

echo ""
if [ ${#FOUND_MODELS[@]} -eq 0 ]; then
    echo "❌ 未找到 GGUF 格式的模型文件（>100MB）"
    echo ""
    echo "💡 提示："
    echo "1. 如果您有模型文件，请告诉我路径"
    echo "2. 或者运行 ./download-model.sh 下载模型"
    echo "3. 支持的格式：.gguf（推荐）"
    exit 1
else
    echo "✅ 找到 ${#FOUND_MODELS[@]} 个模型文件："
    echo ""
    for i in "${!FOUND_MODELS[@]}"; do
        file="${FOUND_MODELS[$i]}"
        size=$(du -h "$file" | cut -f1)
        echo "  [$((i+1))] $file"
        echo "      大小: $size"
        echo ""
    done
    
    echo "💡 使用方法："
    echo "   export LOCAL_LLM_MODEL_PATH=\"${FOUND_MODELS[0]}\""
    echo "   或修改 .env 文件中的 LOCAL_LLM_MODEL_PATH"
fi

