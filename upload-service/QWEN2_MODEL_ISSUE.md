# Qwen2 模型架构支持问题

## 问题描述

加载 Qwen2.5-1.5B-Instruct-Q4_K_M 模型时出现错误：
```
error loading model: unknown model architecture: 'qwen2'
llama_load_model_from_file: failed to load model
```

## 原因分析

`llama_cpp_rs` 0.3.0 版本使用的底层 `llama.cpp` 可能不支持 Qwen2 架构，或者需要更新版本的 `llama.cpp`。

## 解决方案

### 方案 1: 使用支持的模型（推荐）

使用 `llama.cpp` 已支持的模型架构：
- **Llama 2/3** 系列
- **Mistral** 系列
- **Phi** 系列
- **ChatGLM** 系列（如果支持）

### 方案 2: 更新 llama.cpp

如果 `llama_cpp_rs` 支持，可以尝试更新到最新版本的 `llama.cpp`，新版本可能已支持 Qwen2。

### 方案 3: 使用其他本地 LLM 库

考虑使用其他支持 Qwen2 的 Rust LLM 库，如：
- `candle` (HuggingFace 的 Rust ML 框架)
- `ort` (ONNX Runtime Rust 绑定)

## 临时解决方案

在修复模型架构支持之前，可以：
1. 暂时禁用本地 LLM，使用远程 API
2. 下载并使用支持的模型（如 Llama 2/3）

## 下一步

1. 检查 `llama_cpp_rs` 最新版本是否支持 Qwen2
2. 如果支持，更新依赖
3. 如果不支持，考虑切换到其他模型或库

