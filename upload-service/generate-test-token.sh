#!/bin/bash

# 生成测试用JWT Token
# 基于upload-service的JWT配置

SECRET_KEY=${JWT_SECRET:-"your-secret-key-change-in-production"}
USER_ID=${1:-1}
USERNAME=${2:-"test_user"}

echo "🔑 生成测试JWT Token"
echo "=================================="
echo "用户ID: $USER_ID"
echo "用户名: $USERNAME"
echo "密钥: ${SECRET_KEY:0:10}..."
echo ""

# 将值注入环境，Python脚本内读取
export WT_SECRET_VALUE="$SECRET_KEY"
export WT_USER_ID="$USER_ID"
export WT_USERNAME="$USERNAME"

# 使用Python生成JWT (大多数系统都有Python)
python3 - <<'EOF'
import jwt
import datetime
import sys
import os

secret = os.environ.get("WT_SECRET_VALUE")
user_id = int(os.environ.get("WT_USER_ID", "1"))
username = os.environ.get("WT_USERNAME", "test_user")

# 创建payload
payload = {
    "user_id": user_id,
    "username": username,
    "role": "user",
    "permissions": ["upload:read", "upload:write"],
    "exp": datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=24),
    "iat": datetime.datetime.now(datetime.timezone.utc),
    "iss": "upload-service"
}

# 生成token
token = jwt.encode(payload, secret, algorithm="HS256")

print("\n✅ Token已生成:")
print("-" * 60)
print(token)
print("-" * 60)
print("\n📋 使用方法:")
print(f"export TEST_TOKEN='{token}'")
print("")
print("curl -X POST http://localhost:8088/upload \\")
print(f"  -H 'Authorization: Bearer {token}' \\")
print("  -F 'file=@resume.pdf'")
print("")
print("💡 或者直接使用:")
print(f"TOKEN='{token}' ./test-upload-resume.sh")
EOF

echo ""
echo "=================================="
echo "💾 Token已保存到环境变量建议:"
echo ""
echo "# 复制下面的命令到你的终端:"
echo "export TEST_TOKEN='<上面显示的token>'"
echo ""
echo "# 然后测试:"
echo "curl -H \"Authorization: Bearer \$TEST_TOKEN\" http://localhost:8088/health"

