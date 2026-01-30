#!/bin/bash
# 验证本地 LLM 模型下载状态

echo "🔍 本地 LLM 模型验证报告"
echo "================================"
echo ""

# 优先使用 Llama-2 模型（llama.cpp 原生支持）
if [ -f "./models/llama-2-7b-chat.Q4_K_M.gguf" ]; then
    MODEL_PATH="./models/llama-2-7b-chat.Q4_K_M.gguf"
    MODEL_NAME="Llama-2-7B-Chat (推荐 ✅)"
elif [ -f "./models/Phi-3-mini-4k-instruct-q4.gguf" ]; then
    MODEL_PATH="./models/Phi-3-mini-4k-instruct-q4.gguf"
    MODEL_NAME="Phi-3-mini (可能不支持 ⚠️)"
elif [ -f "./models/qwen2.5-1.5b-instruct-q4_k_m.gguf" ]; then
    MODEL_PATH="./models/qwen2.5-1.5b-instruct-q4_k_m.gguf"
    MODEL_NAME="Qwen2.5 (可能不支持 ⚠️)"
else
    MODEL_PATH="./models/llama-2-7b-chat.Q4_K_M.gguf"
    MODEL_NAME="Llama-2-7B-Chat (默认)"
fi

CONFIG_DEFAULT_PATH="./models/llama-2-7b-chat.Q4_K_M.gguf"

# 检查模型文件
echo "📦 模型文件检查："
echo ""

if [ -f "$MODEL_PATH" ]; then
    FILE_SIZE=$(stat -f%z "$MODEL_PATH" 2>/dev/null || stat -c%s "$MODEL_PATH" 2>/dev/null)
    FILE_SIZE_GB=$(echo "scale=2; $FILE_SIZE / 1024 / 1024 / 1024" | bc)
    FILE_SIZE_MB=$(echo "scale=0; $FILE_SIZE / 1024 / 1024" | bc)
    MODIFY_TIME=$(stat -f%Sm "$MODEL_PATH" 2>/dev/null || stat -c%y "$MODEL_PATH" 2>/dev/null)
    
    echo "✅ 模型文件存在: $MODEL_NAME"
    echo "   路径: $MODEL_PATH"
    echo "   大小: $FILE_SIZE 字节 ($FILE_SIZE_GB GB / $FILE_SIZE_MB MB)"
    echo "   修改时间: $MODIFY_TIME"
    
    # 检查文件完整性（应该大于 500MB）
    if [ "$FILE_SIZE" -gt 500000000 ]; then
        echo "   ✅ 文件完整（大小正常）"
    else
        echo "   ⚠️  文件可能不完整（小于 500MB）"
    fi
else
    echo "❌ 模型文件不存在: $MODEL_PATH"
    exit 1
fi

echo ""

# 检查配置
echo "⚙️  配置检查："
echo ""

# 检查环境变量
if [ -n "$LOCAL_LLM_MODEL_PATH" ]; then
    echo "✅ 环境变量 LOCAL_LLM_MODEL_PATH: $LOCAL_LLM_MODEL_PATH"
    if [ "$LOCAL_LLM_MODEL_PATH" = "$MODEL_PATH" ] || [ "$LOCAL_LLM_MODEL_PATH" = "$(realpath "$MODEL_PATH" 2>/dev/null || echo "$MODEL_PATH")" ]; then
        echo "   ✅ 路径匹配"
    else
        echo "   ⚠️  路径不匹配，请检查"
    fi
else
    echo "⚠️  环境变量 LOCAL_LLM_MODEL_PATH 未设置"
    echo "   💡 建议设置: export LOCAL_LLM_MODEL_PATH=\"$MODEL_PATH\""
fi

if [ -n "$USE_LOCAL_LLM" ]; then
    echo "✅ 环境变量 USE_LOCAL_LLM: $USE_LOCAL_LLM"
else
    echo "⚠️  环境变量 USE_LOCAL_LLM 未设置"
    echo "   💡 建议设置: export USE_LOCAL_LLM=true"
fi

echo ""

# 检查 .env 文件
if [ -f ".env" ]; then
    echo "📄 .env 文件检查："
    if grep -q "USE_LOCAL_LLM=true" .env 2>/dev/null; then
        echo "   ✅ USE_LOCAL_LLM=true 已配置"
    else
        echo "   ⚠️  USE_LOCAL_LLM 未设置为 true"
    fi
    
    if grep -q "LOCAL_LLM_MODEL_PATH" .env 2>/dev/null; then
        ENV_PATH=$(grep "LOCAL_LLM_MODEL_PATH" .env | cut -d'=' -f2 | tr -d '"' | tr -d "'")
        echo "   ✅ LOCAL_LLM_MODEL_PATH 已配置: $ENV_PATH"
    else
        echo "   ⚠️  LOCAL_LLM_MODEL_PATH 未配置"
    fi
    echo ""
fi

# 检查默认配置路径
if [ -f "$CONFIG_DEFAULT_PATH" ]; then
    echo "⚠️  注意：配置默认路径的模型也存在: $CONFIG_DEFAULT_PATH"
    echo "   当前使用的模型: $MODEL_PATH"
    echo ""
fi

# 总结
echo "📋 验证总结："
echo ""

if [ -f "$MODEL_PATH" ] && [ "$FILE_SIZE" -gt 500000000 ]; then
    echo "✅ 模型文件：已下载且完整"
    echo "✅ 文件大小：$FILE_SIZE_GB GB（正常）"
    
    if [ -n "$USE_LOCAL_LLM" ] && [ "$USE_LOCAL_LLM" = "true" ]; then
        echo "✅ 配置：已启用本地 LLM"
    else
        echo "⚠️  配置：需要设置 USE_LOCAL_LLM=true"
    fi
    
    if [ -n "$LOCAL_LLM_MODEL_PATH" ]; then
        echo "✅ 路径配置：已设置"
    else
        echo "⚠️  路径配置：建议设置 LOCAL_LLM_MODEL_PATH"
    fi
    
    echo ""
    echo "🎉 模型已就绪，可以开始测试！"
    echo ""
    echo "💡 快速启动："
    echo "   ./quick-test-local-llm.sh"
    echo ""
    echo "或手动启动："
    echo "   export USE_LOCAL_LLM=true"
    echo "   export LOCAL_LLM_MODEL_PATH=\"$MODEL_PATH\""
    echo "   cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster"
    echo "   cargo run -p upload-service"
else
    echo "❌ 模型文件不完整或不存在"
    exit 1
fi

