# 本地 LLM 迁移指南

## 概述

为了实现自主可控，我们将从依赖外部 DeepSeek API 服务迁移到使用本地 LLM 模型。

## 方案选择

### 推荐方案：llama-cpp-rs

**优点：**
- ✅ 纯 Rust 实现，无外部依赖
- ✅ 支持 GGUF 格式模型（量化模型，体积小）
- ✅ 性能优秀，支持多线程推理
- ✅ 社区成熟，文档完善
- ✅ 支持中文模型（Qwen、ChatGLM 等）

**模型推荐：**
- **Qwen2.5-7B-Instruct-Q4_K_M** (约 4.5GB) - 中文能力强，适合提取任务
- **ChatGLM3-6B-Q4_K_M** (约 3.8GB) - 中文对话模型
- **Qwen2.5-1.5B-Instruct-Q4_K_M** (约 1GB) - 轻量级，速度快

## 实施步骤

### 1. 添加依赖

在 `Cargo.toml` 中添加：

```toml
[dependencies]
# 本地 LLM 推理（使用 llama-cpp-rs）
llm-chain-llama-cpp = "0.2"
# 或者使用更底层的 llama-cpp-sys
# llama-cpp-sys = { version = "0.1", features = ["metal", "cuda"] }
```

### 2. 下载模型

```bash
# 创建模型目录
mkdir -p models

# 下载 Qwen2.5-7B-Instruct 量化模型（推荐）
# 可以从 HuggingFace 或 ModelScope 下载 GGUF 格式模型
# 例如：https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF
```

### 3. 更新配置

在 `.env` 或环境变量中配置：

```bash
# 使用本地 LLM
USE_LOCAL_LLM=true
LOCAL_LLM_MODEL_PATH=./models/qwen2.5-7b-instruct-q4_k_m.gguf
LOCAL_LLM_CONTEXT_SIZE=4096
LOCAL_LLM_N_THREADS=4

# 禁用外部 DeepSeek（可选）
ENABLE_DEEPSEEK=false
```

### 4. 实现本地推理

在 `local_llm_client.rs` 中实现 `generate_json` 方法：

```rust
use llm_chain_llama_cpp::{LlamaModel, LlamaParams, LlamaContext};

async fn generate_json(&self, prompt: &str) -> Result<String, anyhow::Error> {
    // 加载模型（可以缓存模型实例）
    let params = LlamaParams::default()
        .with_n_ctx(self.context_size)
        .with_n_threads(self.n_threads.unwrap_or(4));
    
    let model = LlamaModel::load_from_file(&self.model_path, &params)?;
    let ctx = LlamaContext::new(&model, params)?;
    
    // 构建完整提示词（包含 JSON 格式要求）
    let full_prompt = format!(
        "你是一个专业的信息提取助手。请严格按照JSON格式返回结果，不要包含任何其他文本。\n\n{}",
        prompt
    );
    
    // 执行推理
    let response = ctx.completion(&full_prompt, None)?;
    
    // 提取 JSON 部分（可能需要清理）
    let json_text = extract_json_from_response(&response);
    
    Ok(json_text)
}
```

### 5. 更新 DocumentProcessor

修改 `document_processor.rs`，支持本地 LLM 和远程 API 两种模式：

```rust
pub enum LLMBackend {
    Remote(Arc<DeepSeekClient>),
    Local(Arc<LocalLLMClient>),
}

impl DocumentProcessor {
    pub fn new_with_local_llm(
        model_path: String,
        // ... 其他参数
    ) -> Self {
        let local_llm = Arc::new(LocalLLMClient::new(
            model_path,
            Some(4096),
            Some(4),
        ));
        // ...
    }
}
```

## 性能优化

1. **模型缓存**：在应用启动时加载模型，避免每次请求都重新加载
2. **批量处理**：对于批量任务，复用同一个模型实例
3. **量化模型**：使用 Q4_K_M 或 Q5_K_M 量化模型，在保持精度的同时减少内存占用
4. **GPU 加速**：如果支持，启用 Metal（macOS）或 CUDA（Linux/Windows）

## 测试

1. 下载测试模型
2. 运行单元测试验证 JSON 提取
3. 对比本地 LLM 和远程 API 的提取质量
4. 性能测试（延迟、吞吐量）

## 回退方案

如果本地 LLM 性能不满足要求，可以：
- 保留 DeepSeek 作为备选
- 使用配置开关在本地/远程之间切换
- 实现混合模式（本地优先，失败时回退到远程）

## 注意事项

1. **模型文件大小**：7B 模型约 4-5GB，确保有足够磁盘空间
2. **内存占用**：推理时可能需要 8-16GB RAM
3. **首次加载**：模型首次加载可能需要 10-30 秒
4. **并发控制**：本地 LLM 推理是 CPU/GPU 密集型，需要控制并发数

