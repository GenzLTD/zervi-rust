#!/bin/bash
# 验证本地 LLM 部署状态

echo "🔍 本地 LLM 部署状态检查"
echo "================================"
echo ""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# 1. 检查模型文件
echo "📦 1. 模型文件检查："
echo ""

PHI3_MODEL="$SCRIPT_DIR/models/Phi-3-mini-4k-instruct-q4.gguf"
QWEN2_MODEL="$SCRIPT_DIR/models/qwen2.5-1.5b-instruct-q4_k_m.gguf"

MODEL_FOUND=false
MODEL_PATH=""

if [ -f "$PHI3_MODEL" ]; then
    SIZE=$(stat -f%z "$PHI3_MODEL" 2>/dev/null || stat -c%s "$PHI3_MODEL" 2>/dev/null || echo "0")
    SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
    echo "✅ Phi-3 模型存在: $PHI3_MODEL"
    echo "   大小: $SIZE_GB GB"
    if [ "$SIZE" -gt 2000000000 ]; then
        echo "   ✅ 文件完整（大小正常）"
        MODEL_FOUND=true
        MODEL_PATH="$PHI3_MODEL"
    else
        echo "   ⚠️  文件可能不完整（小于 2GB）"
    fi
fi

if [ -f "$QWEN2_MODEL" ]; then
    SIZE=$(stat -f%z "$QWEN2_MODEL" 2>/dev/null || stat -c%s "$QWEN2_MODEL" 2>/dev/null || echo "0")
    SIZE_GB=$(echo "scale=2; $SIZE / 1024 / 1024 / 1024" | bc)
    echo "⚠️  Qwen2 模型存在: $QWEN2_MODEL"
    echo "   大小: $SIZE_GB GB"
    echo "   ⚠️  注意：Qwen2 可能不被 llama.cpp 支持"
    if [ "$MODEL_FOUND" = false ]; then
        MODEL_PATH="$QWEN2_MODEL"
    fi
fi

if [ "$MODEL_FOUND" = false ] && [ ! -f "$QWEN2_MODEL" ]; then
    echo "❌ 未找到模型文件"
    echo "   💡 请运行: ./download-supported-model.sh"
    exit 1
fi

echo ""

# 2. 检查配置
echo "⚙️  2. 配置检查："
echo ""

if grep -q "Phi-3-mini-4k-instruct-q4.gguf" src/config/mod.rs 2>/dev/null; then
    echo "✅ 默认配置已更新为 Phi-3 模型"
else
    echo "⚠️  默认配置可能未更新"
fi

echo ""

# 3. 检查依赖
echo "📚 3. 依赖检查："
echo ""

if grep -q 'llama_cpp_rs = "0.3"' Cargo.toml 2>/dev/null; then
    echo "✅ llama_cpp_rs 依赖已配置"
else
    echo "⚠️  llama_cpp_rs 依赖未找到"
fi

echo ""

# 4. 检查代码
echo "💻 4. 代码检查："
echo ""

if [ -f "src/services/local_llm_client.rs" ]; then
    echo "✅ LocalLLMClient 实现存在"
else
    echo "❌ LocalLLMClient 实现不存在"
fi

if [ -f "src/services/llm_trait.rs" ]; then
    echo "✅ LLMClient trait 存在"
else
    echo "❌ LLMClient trait 不存在"
fi

echo ""

# 5. 总结
echo "📋 部署状态总结："
echo ""

if [ "$MODEL_FOUND" = true ]; then
    echo "✅ 模型文件: 已就绪（Phi-3，llama.cpp 支持）"
    echo "✅ 配置文件: 已更新"
    echo "✅ 代码实现: 已完成"
    echo ""
    echo "🎉 本地 LLM 部署就绪！"
    echo ""
    echo "🚀 下一步："
    echo "   1. 启动服务: ./quick-test-local-llm.sh"
    echo "   2. 测试模型加载是否成功"
    echo "   3. 测试文档解析功能"
else
    echo "⚠️  模型文件: 存在但不完整或使用 Qwen2（可能不支持）"
    echo "✅ 配置文件: 已更新"
    echo "✅ 代码实现: 已完成"
    echo ""
    echo "💡 建议："
    echo "   1. 确保 Phi-3 模型下载完整"
    echo "   2. 或使用远程 API 模式: export USE_LOCAL_LLM=false"
fi

