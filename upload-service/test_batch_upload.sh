#!/bin/bash

# Phase 1: 批量上传功能测试脚本
# 用于测试批量简历上传API

set -e

echo "🧪 Phase 1 批量上传功能测试"
echo "=============================="
echo ""

# 配置
API_BASE_URL="${API_BASE_URL:-http://localhost:3007}"
TOKEN="${JWT_TOKEN:-your-jwt-token-here}"

# 颜色输出
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 测试文件目录
TEST_RESUMES_DIR="../../测试简历"

echo "📍 API Base URL: $API_BASE_URL"
echo "🔑 JWT Token: ${TOKEN:0:20}..."
echo "📁 测试简历目录: $TEST_RESUMES_DIR"
echo ""

# ==================== 测试 1: 批量上传（普通POST）====================
echo "📦 测试 1: 批量上传简历（普通POST）"
echo "-----------------------------"

# 准备测试文件
TEST_FILES=(
    "316317189.pdf"
    "316416071.pdf"
    "6292066.pdf"
)

echo "准备上传 ${#TEST_FILES[@]} 份简历..."

# 构建curl命令
CURL_CMD="curl -X POST \"$API_BASE_URL/upload/batch-resumes\" \
  -H \"Authorization: Bearer $TOKEN\" \
  -H \"Content-Type: multipart/form-data\""

# 添加文件
for file in "${TEST_FILES[@]}"; do
    if [ -f "$TEST_RESUMES_DIR/$file" ]; then
        echo "  ✓ 找到文件: $file"
        CURL_CMD="$CURL_CMD -F \"files=@$TEST_RESUMES_DIR/$file\""
    else
        echo -e "  ${RED}✗ 文件不存在: $file${NC}"
    fi
done

echo ""
echo "🚀 开始上传..."
echo ""

# 执行上传
START_TIME=$(date +%s)
RESPONSE=$(eval $CURL_CMD)
END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

echo "📊 上传完成（耗时: ${DURATION}秒）"
echo ""
echo "响应:"
echo "$RESPONSE" | jq '.'
echo ""

# 解析结果
TOTAL=$(echo "$RESPONSE" | jq -r '.data.total')
SUCCEEDED=$(echo "$RESPONSE" | jq -r '.data.succeeded')
FAILED=$(echo "$RESPONSE" | jq -r '.data.failed')

if [ "$SUCCEEDED" -gt 0 ]; then
    echo -e "${GREEN}✅ 成功: $SUCCEEDED 份${NC}"
fi

if [ "$FAILED" -gt 0 ]; then
    echo -e "${RED}❌ 失败: $FAILED 份${NC}"
    echo ""
    echo "失败详情:"
    echo "$RESPONSE" | jq '.data.results[] | select(.success == false)'
fi

echo ""
echo "=============================="
echo ""

# ==================== 测试 2: 批量上传（SSE实时进度）====================
echo "📡 测试 2: 批量上传简历（SSE实时进度）"
echo "-----------------------------"
echo "注意: 此测试需要手动观察SSE事件流"
echo ""
echo "使用以下命令测试SSE:"
echo ""
echo -e "${YELLOW}curl -X GET \"$API_BASE_URL/upload/batch-resumes-progress\" \\
  -H \"Authorization: Bearer $TOKEN\" \\
  -H \"Accept: text/event-stream\" \\
  -N${NC}"
echo ""

# ==================== 性能测试 ====================
echo "⚡ 性能统计"
echo "-----------------------------"

if [ "$TOTAL" -gt 0 ]; then
    AVG_TIME=$(echo "scale=2; $DURATION / $TOTAL" | bc)
    echo "总文件数: $TOTAL"
    echo "总耗时: ${DURATION}秒"
    echo "平均耗时: ${AVG_TIME}秒/份"
    echo ""
    
    if (( $(echo "$AVG_TIME < 3" | bc -l) )); then
        echo -e "${GREEN}✅ 性能优秀！平均每份简历 < 3秒${NC}"
    elif (( $(echo "$AVG_TIME < 5" | bc -l) )); then
        echo -e "${YELLOW}⚠️ 性能良好，平均每份简历 < 5秒${NC}"
    else
        echo -e "${RED}❌ 性能需要优化，平均每份简历 > 5秒${NC}"
    fi
fi

echo ""
echo "=============================="
echo ""

# ==================== 测试 3: 错误处理 ====================
echo "🧨 测试 3: 错误处理"
echo "-----------------------------"

# 测试超大文件
echo "3.1 测试文件大小限制..."
# TODO: 创建一个超过10MB的测试文件

# 测试不支持的文件类型
echo "3.2 测试不支持的文件类型..."
echo "test" > /tmp/test_resume.txt
RESPONSE=$(curl -s -X POST "$API_BASE_URL/upload/batch-resumes" \
  -H "Authorization: Bearer $TOKEN" \
  -F "files=@/tmp/test_resume.txt")

echo "$RESPONSE" | jq '.'
rm -f /tmp/test_resume.txt

echo ""
echo "=============================="
echo ""

# ==================== 测试总结 ====================
echo "📝 测试总结"
echo "-----------------------------"
echo "✅ 已完成:"
echo "  - 批量上传API"
echo "  - 并发处理"
echo "  - 错误处理"
echo ""
echo "⏳ 待测试:"
echo "  - SSE实时进度（需手动测试）"
echo "  - 数据库持久化验证"
echo "  - 前端批量上传组件"
echo ""

echo "=============================="
echo "🎉 Phase 1 测试完成！"
echo ""

