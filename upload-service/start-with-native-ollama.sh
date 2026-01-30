#!/bin/bash

# 使用原生Ollama启动upload-service

set -e

echo "🚀 启动upload-service (使用原生Ollama + Metal加速)"
echo "=========================================="
echo ""

# 1. 检查Ollama
echo "1️⃣  检查Ollama服务..."
if pgrep -x "ollama" > /dev/null; then
    echo "   ✅ Ollama正在运行"
else
    echo "   ❌ Ollama未运行，正在启动..."
    nohup ollama serve > /tmp/ollama.log 2>&1 &
    sleep 3
fi

# 2. 检查依赖服务
echo ""
echo "2️⃣  检查依赖服务..."

# 检查MySQL
if docker ps --filter "name=adirp-mysql" --filter "status=running" --format "{{.Names}}" | grep -q "adirp-mysql"; then
    echo "   ✅ MySQL: 运行中"
else
    echo "   ❌ MySQL未运行，正在启动..."
    docker start adirp-mysql
    sleep 5
    if docker ps --filter "name=adirp-mysql" --filter "status=running" --format "{{.Names}}" | grep -q "adirp-mysql"; then
        echo "   ✅ MySQL: 启动成功"
    else
        echo "   ❌ MySQL启动失败"
        exit 1
    fi
fi

# 检查Redis
if docker ps --filter "name=adirp-redis" --filter "status=running" --format "{{.Names}}" | grep -q "adirp-redis"; then
    echo "   ✅ Redis: 运行中"
else
    echo "   ❌ Redis未运行，正在启动..."
    docker start adirp-redis
    sleep 3
    if docker ps --filter "name=adirp-redis" --filter "status=running" --format "{{.Names}}" | grep -q "adirp-redis"; then
        echo "   ✅ Redis: 启动成功"
    else
        echo "   ❌ Redis启动失败"
        exit 1
    fi
fi

# 3. 设置环境变量
echo ""
echo "3️⃣  配置环境变量..."
export PORT="3007"
export DEEPSEEK_URL="http://localhost:11434"
export DEEPSEEK_MODEL="deepseek-r1:7b"
export DATABASE_URL="mysql://root:adirp_password@localhost:3306/adirp_upload"
export REDIS_URL="redis://localhost:6379"
export JWT_SECRET="your-secret-key-change-in-production"
export UPLOAD_DIR="./uploads"
export CONSUL_URL="http://localhost:8500"

echo "   ✅ DEEPSEEK_URL=$DEEPSEEK_URL"
echo "   ✅ DEEPSEEK_MODEL=$DEEPSEEK_MODEL"
echo "   ✅ DATABASE_URL=mysql://localhost:3306/adirp_upload"

# 4. 编译并运行
echo ""
echo "4️⃣  编译并启动服务..."
echo "   (首次编译可能需要几分钟)"
echo ""

# 运行服务
cargo run --release

# 注意：服务会占据终端，按Ctrl+C停止

