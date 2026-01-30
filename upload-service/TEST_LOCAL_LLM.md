# 🧪 测试本地 LLM 功能

## ✅ 准备工作已完成

1. ✅ **模型下载完成**: Llama-2-7B-Chat (3.80 GB)
2. ✅ **配置文件已更新**: 默认使用 Llama-2 模型
3. ✅ **代码实现完成**: LocalLLMClient 已集成
4. ✅ **依赖已配置**: llama_cpp_rs 已添加

## 🚀 开始测试

### 方法 1: 使用快速测试脚本（推荐）

```bash
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster/upload-service
./quick-test-local-llm.sh
```

### 方法 2: 手动启动

```bash
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster/upload-service

# 设置环境变量
export USE_LOCAL_LLM=true
export LOCAL_LLM_MODEL_PATH="$(pwd)/models/llama-2-7b-chat.Q4_K_M.gguf"
export LOCAL_LLM_CONTEXT_SIZE=4096
export LOCAL_LLM_N_THREADS=4
export DATABASE_URL="mysql://root:adirp_password@127.0.0.1:3307/adirp_upload"

# 启动服务
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster
cargo run -p upload-service
```

## 📊 预期输出

### ✅ 成功启动的标志

1. **模型加载成功**:
   ```
   🔄 正在加载本地 LLM 模型: .../llama-2-7b-chat.Q4_K_M.gguf
   ✅ 本地 LLM 模型加载完成（在专门线程中）
   ```

2. **服务启动成功**:
   ```
   ✅ Document processor initialized (智能路由系统)
      - Local LLM: .../llama-2-7b-chat.Q4_K_M.gguf ✅ 自主可控
   ```

3. **服务监听端口**:
   ```
   🚀 Server running on http://0.0.0.0:3007
   ```

### ❌ 如果出现错误

#### 错误 1: 模型架构不支持
```
❌ error loading model: unknown model architecture: 'qwen2'
```
**解决**: 已切换到 Llama-2 模型，不应该再出现此错误。

#### 错误 2: 模型文件不存在
```
❌ 本地 LLM 初始化失败: 模型文件不存在
```
**解决**: 检查 `LOCAL_LLM_MODEL_PATH` 环境变量，确保路径正确。

#### 错误 3: 数据库连接失败
```
❌ Error: Connection Error: ... Access denied
```
**解决**: 确保 Docker 容器 `watchtower-mysql` 正在运行，端口为 3307。

## 🧪 功能测试

### 1. 测试文档上传

```bash
# 生成测试 token
./generate-test-token.sh
export TEST_TOKEN="<生成的token>"

# 上传测试文档
curl -X POST http://localhost:3007/upload \
  -H "Authorization: Bearer $TEST_TOKEN" \
  -F "file=@test_resume.pdf"
```

### 2. 测试诗歌档案解析

```bash
# 上传诗人档案文档
curl -X POST http://localhost:3007/upload \
  -H "Authorization: Bearer $TEST_TOKEN" \
  -F "file=@poet_profile.docx"
```

### 3. 验证本地 LLM 推理

查看日志，应该看到：
```
📝 本地 LLM 生成完成: 输出长度: ...
⏱️ LLM extraction took: ... (应该比远程 API 慢，但完全本地)
```

## 📈 性能对比

| 指标 | 远程 DeepSeek API | 本地 Llama-2 LLM |
|------|------------------|------------------|
| 响应时间 | ~1-2 秒 | ~2-5 秒 |
| 数据安全 | ❌ 数据发送到外部 | ✅ 数据不出本地 |
| 成本 | 💰 API 调用费用 | ✅ 无 API 费用 |
| 离线运行 | ❌ 需要网络 | ✅ 完全离线 |
| 自主可控 | ❌ 依赖外部服务 | ✅ 完全自主 |

## 🎯 测试检查清单

- [ ] 模型加载成功（无架构错误）
- [ ] 服务启动成功（监听 3007 端口）
- [ ] 文档上传功能正常
- [ ] 信息提取功能正常（使用本地 LLM）
- [ ] 诗歌档案解析功能正常
- [ ] 日志显示使用本地 LLM（而非远程 API）

## 🐛 问题排查

如果遇到问题，检查：

1. **模型文件**: `ls -lh models/llama-2-7b-chat.Q4_K_M.gguf`
2. **环境变量**: `echo $USE_LOCAL_LLM $LOCAL_LLM_MODEL_PATH`
3. **数据库**: `docker ps | grep mysql`
4. **Consul**: `curl http://localhost:8500/v1/status/leader`
5. **日志**: 查看服务启动日志中的错误信息

---

**准备好测试了吗？运行 `./quick-test-local-llm.sh` 开始吧！** 🚀

