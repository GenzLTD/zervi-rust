# llama_cpp_rs 构建脚本修复

## 问题描述

`llama_cpp_rs` 0.3.0 在编译时出现错误：
```
ar: /Users/.../out/llama.cpp/ggml.o: No such file or directory
```

**根本原因：**
- `cc::Build` 编译生成的对象文件名带有哈希前缀（如 `0d792ea23c98b07a-ggml.o`）
- 构建脚本硬编码了 `llama.cpp/ggml.o` 路径，导致找不到文件

## 修复方案

修改 `compile_llama` 函数，使用静态库链接而不是硬编码的对象文件路径：

### 修复前（第 133-135 行）：
```rust
let ggml_obj = PathBuf::from(&out_path).join("llama.cpp/ggml.o");
cxx.object(ggml_obj);
```

### 修复后：
```rust
// 使用静态库链接而不是硬编码的对象文件路径
println!("cargo:rustc-link-search=native={}", out_path.display());
println!("cargo:rustc-link-lib=static=ggml");
```

## 应用修复

### 方法 1：手动修改（当前使用）

修复文件位置：
```
~/.cargo/registry/src/index.crates.io-*/llama_cpp_rs-0.3.0/build.rs
```

**注意：** 每次 `cargo clean` 或更新依赖后，需要重新应用修复。

### 方法 2：使用 Cargo Patch（推荐）

在 `Cargo.toml` 中添加：

```toml
[patch.crates-io]
llama_cpp_rs = { path = "../patches/llama_cpp_rs-0.3.0" }
```

但这需要 fork 整个 crate。

### 方法 3：等待上游修复

向 `llama_cpp_rs` 项目提交 PR 或 issue。

## 验证修复

运行以下命令验证编译成功：

```bash
cd /Users/szjason72/xiajun/adirp-ctyun/rust-cluster
cargo clean -p llama_cpp_rs
cargo build -p upload-service
```

应该看到：
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in XX.XXs
```

## Patch 文件

Patch 文件已保存在：
```
patches/llama_cpp_rs-0.3.0/build.rs.patch
```

可以使用以下命令应用：
```bash
cd ~/.cargo/registry/src/index.crates.io-*/llama_cpp_rs-0.3.0
patch -p0 < /path/to/build.rs.patch
```

## 注意事项

1. **每次清理后需要重新应用**：`cargo clean` 会删除修改
2. **依赖更新后需要重新应用**：更新 `llama_cpp_rs` 版本后需要重新修复
3. **Metal 功能支持**：修复后的代码也支持查找带哈希前缀的 Metal 对象文件

## 下一步

现在 `llama_cpp_rs` 可以正常编译，可以继续完成本地 LLM 集成了！

