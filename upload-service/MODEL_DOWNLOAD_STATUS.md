# 模型下载状态

## 当前状态

正在下载 **Phi-3-mini-4k-instruct-q4.gguf** 模型（llama.cpp 支持）

### 模型信息
- **模型名称**: Phi-3-mini-4k-instruct-q4.gguf
- **仓库**: microsoft/Phi-3-mini-4k-instruct-gguf
- **大小**: 约 2.3GB
- **格式**: GGUF Q4 量化
- **架构**: Phi-3（llama.cpp 原生支持）

### 下载进度

检查下载状态：
```bash
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster/upload-service
./check-download.sh
```

查看实时日志：
```bash
tail -f /tmp/model_download.log
```

## 下载完成后

### 1. 验证下载
```bash
ls -lh models/Phi-3-mini-4k-instruct-q4.gguf
# 应该显示约 2.3GB 的文件
```

### 2. 更新配置（已自动更新）
- `src/config/mod.rs`: 默认模型路径已更新
- `quick-test-local-llm.sh`: 自动检测并使用 Phi-3 模型

### 3. 启动服务
```bash
./quick-test-local-llm.sh
```

## 为什么选择 Phi-3？

1. ✅ **llama.cpp 原生支持** - 不会出现架构不支持错误
2. ✅ **模型较小** - 约 2.3GB，适合本地运行
3. ✅ **性能优秀** - Microsoft 开发，指令跟随能力强
4. ✅ **中文支持** - 对中文任务有良好支持

## 替代方案

如果 Phi-3 下载失败，可以考虑：

1. **Mistral-7B-Instruct** (约 4.1GB)
   ```bash
   huggingface-cli download TheBloke/Mistral-7B-Instruct-v0.2-GGUF mistral-7b-instruct-v0.2.Q4_K_M.gguf --local-dir ./models
   ```

2. **Llama-2-7B-Chat** (约 4.1GB)
   ```bash
   huggingface-cli download TheBloke/Llama-2-7B-Chat-GGUF llama-2-7b-chat.Q4_K_M.gguf --local-dir ./models
   ```

## 注意事项

- 下载可能需要 10-30 分钟（取决于网络速度）
- 确保有足够的磁盘空间（至少 3GB）
- 下载完成后，模型文件约 2.3GB

