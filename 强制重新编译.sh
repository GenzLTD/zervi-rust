#!/bin/bash

echo "🔧 强制重新编译 upload-service"
echo ""

cd /Users/szjason72/szbolent/Zervigo/zervigo.demo/prototypes/adirp-microservices/rust-cluster

# 1. 删除编译缓存中的 main.rs 相关文件
echo "1️⃣  清理 main.rs 的编译缓存..."
rm -rf target/release/deps/upload_service-*
rm -rf target/release/.fingerprint/upload-service-*
echo "✅ 缓存已清理"
echo ""

# 2. 触发 main.rs 重新编译
echo "2️⃣  修改 main.rs 触发重新编译..."
touch upload-service/src/main.rs
echo "✅ 已更新时间戳"
echo ""

# 3. 重新编译
echo "3️⃣  开始编译..."
cargo build --release -p upload-service

if [ $? -eq 0 ]; then
    echo ""
    echo "✅ 编译成功！"
    echo ""
    echo "可执行文件: target/release/upload-service"
    echo ""
else
    echo ""
    echo "❌ 编译失败，请检查错误信息"
    exit 1
fi

