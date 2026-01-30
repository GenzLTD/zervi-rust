# ✅ 数据库连接配置修复完成

## 🔧 问题描述

服务启动时出现数据库连接错误：
```
Error: Connection Error: error returned from database: 1045 (28000): 
Access denied for user 'root'@'localhost' (using password: YES)
```

## 🔍 问题原因

数据库运行在 Docker 容器中，端口映射为 `3307->3306`，但配置中使用的是默认端口 `3306`。

- **Docker 容器**: `watchtower-mysql`
- **容器端口**: 3306 (内部)
- **主机端口**: 3307 (外部访问)
- **原配置**: `mysql://root:adirp_password@localhost/adirp_upload` (默认 3306)
- **正确配置**: `mysql://root:adirp_password@127.0.0.1:3307/adirp_upload`

## ✅ 修复内容

### 1. 更新默认配置 (`src/config/mod.rs`)

```rust
database_url: env::var("DATABASE_URL").unwrap_or_else(|_| {
    "mysql://root:adirp_password@127.0.0.1:3307/adirp_upload".to_string()
}),
```

### 2. 更新测试脚本

#### `quick-test-local-llm.sh`
- 添加 `DATABASE_URL` 环境变量
- 更新数据库检查逻辑，使用端口 3307

#### `test-with-model.sh`
- 添加 `DATABASE_URL` 环境变量（支持覆盖）

### 3. 数据库连接验证

```bash
# 测试连接
mysql -h127.0.0.1 -P3307 -uroot -padirp_password -e "SHOW DATABASES;"

# 确保数据库存在
mysql -h127.0.0.1 -P3307 -uroot -padirp_password -e "CREATE DATABASE IF NOT EXISTS adirp_upload;"
```

## 📋 当前配置

- **数据库主机**: `127.0.0.1`
- **数据库端口**: `3307` (Docker 容器映射端口)
- **数据库用户**: `root`
- **数据库密码**: `adirp_password`
- **数据库名称**: `adirp_upload`
- **连接字符串**: `mysql://root:adirp_password@127.0.0.1:3307/adirp_upload`

## 🚀 使用方法

### 方法 1: 使用环境变量（推荐）

```bash
export DATABASE_URL="mysql://root:adirp_password@127.0.0.1:3307/adirp_upload"
export USE_LOCAL_LLM=true
export LOCAL_LLM_MODEL_PATH=./models/qwen2.5-1.5b-instruct-q4_k_m.gguf

cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster
cargo run -p upload-service
```

### 方法 2: 使用测试脚本（自动配置）

```bash
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster/upload-service
./quick-test-local-llm.sh
```

脚本会自动：
- ✅ 检查模型文件
- ✅ 检查数据库连接（端口 3307）
- ✅ 设置所有必要的环境变量
- ✅ 启动服务

## ✅ 验证

运行以下命令验证数据库连接：

```bash
# 检查 Docker 容器状态
docker ps | grep mysql

# 测试数据库连接
mysql -h127.0.0.1 -P3307 -uroot -padirp_password -e "SELECT 'Connection OK' as Status;"

# 检查数据库是否存在
mysql -h127.0.0.1 -P3307 -uroot -padirp_password -e "SHOW DATABASES LIKE 'adirp_upload';"
```

## 📝 注意事项

1. **Docker 容器必须运行**: 确保 `watchtower-mysql` 容器正在运行
   ```bash
   docker ps | grep watchtower-mysql
   ```

2. **端口映射**: 如果 Docker 容器端口映射改变，需要相应更新配置

3. **环境变量优先级**: 如果设置了 `DATABASE_URL` 环境变量，会覆盖默认配置

4. **数据库初始化**: 首次运行时会自动创建数据库表结构（通过 SeaORM migrations）

## 🎉 修复完成

现在可以正常启动服务了：

```bash
./quick-test-local-llm.sh
```

服务应该能够成功连接到数据库并启动本地 LLM 功能。

