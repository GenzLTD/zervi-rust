#!/bin/bash

echo "🧹 清理编译警告（0警告目标）"
echo ""

cd /Users/szjason72/szbolent/Zervigo/zervigo.demo/prototypes/adirp-microservices/rust-cluster

echo "1️⃣  运行 cargo fix 自动修复..."
cargo fix --bin upload-service --allow-dirty --allow-staged 2>&1 | tail -20

echo ""
echo "2️⃣  重新编译检查警告数..."
cargo build --release -p upload-service 2>&1 | tee /tmp/build.log | grep -c "warning"

echo ""
echo "3️⃣  显示剩余警告..."
grep "warning:" /tmp/build.log | head -20

echo ""
echo "完成状态:"
if grep -q "Finished" /tmp/build.log && [ $(grep -c "warning:" /tmp/build.log) -eq 0 ]; then
    echo "✅ 0警告编译成功！"
else
    WARN_COUNT=$(grep -c "warning:" /tmp/build.log)
    echo "⚠️  还有 $WARN_COUNT 个警告需要处理"
fi

