#!/bin/bash
# 停止 upload-service 服务

set -e

PORT=3007
SERVICE_NAME="upload-service"

echo "🛑 停止 $SERVICE_NAME 服务..."
echo ""

# 方法1: 通过端口查找并停止
if lsof -ti:$PORT > /dev/null 2>&1; then
    PIDS=$(lsof -ti:$PORT)
    echo "找到占用端口 $PORT 的进程: $PIDS"
    echo "$PIDS" | xargs kill -9 2>/dev/null || true
    sleep 1
    
    if lsof -ti:$PORT > /dev/null 2>&1; then
        echo "❌ 无法停止进程，请手动处理"
        exit 1
    else
        echo "✅ 已停止占用端口 $PORT 的进程"
    fi
else
    echo "✅ 端口 $PORT 未被占用"
fi

# 方法2: 通过进程名查找并停止
PIDS=$(pgrep -f "$SERVICE_NAME" 2>/dev/null || true)
if [ -n "$PIDS" ]; then
    echo "找到 $SERVICE_NAME 进程: $PIDS"
    echo "$PIDS" | xargs kill -9 2>/dev/null || true
    sleep 1
    echo "✅ 已停止 $SERVICE_NAME 进程"
else
    echo "✅ 未找到运行中的 $SERVICE_NAME 进程"
fi

echo ""
echo "✅ 服务已停止"

