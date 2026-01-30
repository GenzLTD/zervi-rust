# 本地 LLM 模型设置指南

本指南介绍如何在本地设置 LLM 模型用于 `upload-service` 的测试和训练。

## 方案一：下载预训练模型（推荐）

这是最快、最简单的方法，适合大多数测试场景。

### 快速开始

```bash
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster/upload-service
./setup-local-model.sh
```

脚本会引导您：
1. 选择模型（1.5B、3B、7B 等）
2. 自动下载模型文件
3. 配置环境变量

### 推荐的模型

| 模型 | 大小 | 适用场景 | 推荐度 |
|------|------|----------|--------|
| Qwen2.5-1.5B-Instruct-Q4_K_M | ~1GB | 快速测试、开发 | ⭐⭐⭐⭐⭐ |
| Qwen2.5-3B-Instruct-Q4_K_M | ~2GB | 平衡性能和资源 | ⭐⭐⭐⭐ |
| Qwen2.5-7B-Instruct-Q4_K_M | ~4.5GB | 生产环境、高质量 | ⭐⭐⭐ |
| ChatGLM3-6B-Q4_K_M | ~3.8GB | 中文优化 | ⭐⭐⭐⭐ |

### 手动下载

如果脚本无法自动下载，可以手动下载：

```bash
# 使用 huggingface-cli
pip3 install huggingface_hub
huggingface-cli download Qwen/Qwen2.5-1.5B-Instruct-GGUF \
    qwen2.5-1.5b-instruct-q4_k_m.gguf \
    --local-dir ./models

# 或使用 hf 命令（新版本）
hf download Qwen/Qwen2.5-1.5B-Instruct-GGUF \
    qwen2.5-1.5b-instruct-q4_k_m.gguf \
    --local-dir ./models
```

### 配置使用

下载完成后，设置环境变量：

```bash
export USE_LOCAL_LLM=true
export LOCAL_LLM_MODEL_PATH=./models/qwen2.5-1.5b-instruct-q4_k_m.gguf
export LOCAL_LLM_CONTEXT_SIZE=4096
export LOCAL_LLM_N_THREADS=4
```

或修改 `.env` 文件：

```env
USE_LOCAL_LLM=true
LOCAL_LLM_MODEL_PATH=./models/qwen2.5-1.5b-instruct-q4_k_m.gguf
LOCAL_LLM_CONTEXT_SIZE=4096
LOCAL_LLM_N_THREADS=4
```

### 测试模型

```bash
./test-with-model.sh ./models/qwen2.5-1.5b-instruct-q4_k_m.gguf
```

## 方案二：本地微调模型（高级）

如果您需要针对特定任务优化模型，可以进行微调。

### 前置要求

- GPU（推荐 16GB+ VRAM，如 NVIDIA RTX 3090/4090）
- 大量训练数据（至少数千条样本）
- 足够的存储空间（模型 + 数据 + 检查点）

### 设置微调环境

```bash
./setup-model-finetune.sh
```

### 微调方法

1. **LoRA（推荐）**
   - 资源需求：中等
   - 训练时间：较短
   - 效果：良好

2. **QLoRA（最节省资源）**
   - 资源需求：低
   - 训练时间：中等
   - 效果：良好

3. **全参数微调**
   - 资源需求：高
   - 训练时间：长
   - 效果：最好

### 准备训练数据

训练数据应为 JSONL 格式，每行一个 JSON 对象：

```json
{"text": "这是训练样本1的文本内容..."}
{"text": "这是训练样本2的文本内容..."}
```

对于文档解析任务，可以准备如下格式：

```json
{"text": "简历文本内容...", "extracted": {"name": "张三", "education": "本科", ...}}
```

### 运行微调

1. 编辑 `finetune/finetune_template.py`
2. 设置模型名称、数据路径、训练参数
3. 运行：

```bash
python3 finetune/finetune_template.py
```

### 使用微调后的模型

微调完成后，模型会保存在 `./models/finetuned/`。需要转换为 GGUF 格式才能在 `upload-service` 中使用：

```bash
# 使用 llama.cpp 转换工具
python3 -m llama_cpp.convert \
    --outfile ./models/finetuned.gguf \
    --outtype q4_k_m \
    ./models/finetuned
```

## 性能优化

### CPU 优化

- 使用量化模型（Q4_K_M、Q5_K_M）
- 调整线程数：`LOCAL_LLM_N_THREADS=4`（根据 CPU 核心数）
- 减小上下文大小：`LOCAL_LLM_CONTEXT_SIZE=2048`

### GPU 加速（如果可用）

在 `local_llm_client.rs` 中启用 GPU：

```rust
model_opts.set_gpu_layers(35); // 使用 GPU 层数
```

需要编译时启用 `metal` feature（macOS）或 `cuda` feature（Linux/NVIDIA）：

```bash
cargo build --release --features metal  # macOS
cargo build --release --features cuda   # Linux/NVIDIA
```

## 常见问题

### Q: 模型下载失败？

A: 检查网络连接，或使用代理：
```bash
export HF_ENDPOINT=https://hf-mirror.com  # 使用镜像
```

### Q: 模型加载失败？

A: 检查：
1. 模型文件是否完整（文件大小）
2. 模型路径是否正确
3. 是否有足够的 RAM（7B 模型需要 ~8GB RAM）

### Q: 推理速度慢？

A: 
1. 使用更小的模型（1.5B 或 3B）
2. 减小上下文大小
3. 使用量化模型（Q4_K_M）
4. 启用 GPU 加速（如果可用）

### Q: 如何选择模型大小？

A:
- **开发/测试**：1.5B 或 3B
- **生产环境**：7B 或更大
- **资源受限**：1.5B + Q4_K_M 量化

## 下一步

1. ✅ 下载模型：运行 `./setup-local-model.sh`
2. ✅ 测试模型：运行 `./test-with-model.sh`
3. ✅ 集成到服务：启动 `upload-service` 并测试文档解析
4. ✅ A/B 测试：对比本地 LLM 和远程 API 的效果

