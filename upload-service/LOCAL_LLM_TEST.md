# 本地 LLM 测试指南

## ✅ 已完成的工作

1. **修复了 llama_cpp_rs 构建脚本问题**
   - 修复了 `ggml.o` 路径问题
   - 使用静态库链接替代硬编码对象文件路径

2. **完成了本地 LLM 客户端实现**
   - 使用专门的线程处理 LLM 请求（解决 Send 问题）
   - 实现了简历提取、诗人档案提取、质量评分功能
   - 支持 JSON 格式输出和自动提取

3. **集成了配置系统**
   - 支持通过环境变量切换本地/远程 LLM
   - 可配置模型路径、上下文大小、线程数

## 🚀 测试步骤

### 1. 下载模型文件

```bash
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster/upload-service
./download-model.sh
```

或者手动下载：
- 访问 https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF
- 下载 `Qwen2.5-7B-Instruct-Q4_K_M.gguf` 文件
- 放置到 `models/` 目录

### 2. 配置环境变量

```bash
export USE_LOCAL_LLM=true
export LOCAL_LLM_MODEL_PATH=./models/Qwen2.5-7B-Instruct-Q4_K_M.gguf
export LOCAL_LLM_CONTEXT_SIZE=4096
export LOCAL_LLM_N_THREADS=4  # 可选，不设置则使用 CPU 核心数
```

### 3. 启动服务

```bash
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster
cargo run -p upload-service
```

### 4. 验证

查看启动日志，应该看到：
```
🔒 使用本地 LLM（自主可控模式）
🔄 初始化本地 LLM 处理线程...
✅ 本地 LLM 模型加载完成（在专门线程中）
✅ 本地 LLM 处理线程已启动
   - 本地 LLM: ./models/Qwen2.5-7B-Instruct-Q4_K_M.gguf (上下文: 4096, 线程: 4) ✅ 自主可控
```

### 5. 测试上传

使用之前的 A/B 测试脚本：

```bash
cd /Users/szjason72/xiajun/genzltd/watchtower/scripts
export WATCHTOWER_AB_TOKEN="your-token"
python3 upload_ab_samples.py
```

## 📊 预期结果

1. **模型加载**：首次启动时，模型加载可能需要 10-30 秒
2. **推理速度**：本地推理可能比 API 慢，但可接受（取决于硬件）
3. **输出质量**：应该能够正确提取 JSON 格式的信息

## ⚠️ 注意事项

1. **内存要求**：7B 模型推理时可能需要 8-16GB RAM
2. **首次加载**：模型首次加载需要时间，请耐心等待
3. **并发处理**：所有请求都在同一个专门线程中串行处理，避免并发冲突
4. **模型文件**：确保模型文件路径正确，文件存在且可读

## 🔧 故障排除

### 问题 1：模型文件不存在

```
错误: 模型文件不存在: ./models/xxx.gguf
```

**解决**：运行 `./download-model.sh` 或手动下载模型文件

### 问题 2：模型加载失败

```
错误: 加载模型失败: ...
```

**解决**：
- 检查模型文件是否完整（文件大小是否正确）
- 检查文件权限
- 尝试使用其他模型文件

### 问题 3：推理失败

```
错误: 推理失败: ...
```

**解决**：
- 检查内存是否充足
- 尝试减少 `LOCAL_LLM_CONTEXT_SIZE`
- 检查日志中的详细错误信息

## 📈 性能优化建议

1. **使用量化模型**：Q4_K_M 是推荐的量化级别，平衡了质量和速度
2. **调整线程数**：根据 CPU 核心数调整 `LOCAL_LLM_N_THREADS`
3. **减少上下文大小**：如果不需要长上下文，可以减少 `LOCAL_LLM_CONTEXT_SIZE`
4. **使用 GPU**：如果支持，可以启用 GPU 加速（需要修改代码）

## 🎯 下一步

1. 下载并测试模型
2. 运行 A/B 测试，对比本地 LLM 和远程 API 的效果
3. 根据测试结果优化提示词和参数
4. 考虑性能优化（如模型量化、GPU 加速等）

