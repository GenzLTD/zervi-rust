#!/bin/bash
# 自动修复 llama_cpp_rs 构建脚本

set -e

echo "🔧 正在查找 llama_cpp_rs 构建脚本..."

# 查找 llama_cpp_rs 0.3.0 的构建脚本
BUILD_RS=$(find ~/.cargo/registry/src -name "build.rs" -path "*/llama_cpp_rs-0.3.0/*" 2>/dev/null | head -1)

if [ -z "$BUILD_RS" ]; then
    echo "❌ 未找到 llama_cpp_rs 0.3.0 的构建脚本"
    echo "💡 提示：请先运行 'cargo build' 让 Cargo 下载依赖"
    exit 1
fi

echo "✅ 找到构建脚本: $BUILD_RS"

# 检查是否已经修复过
if grep -q "cargo:rustc-link-lib=static=ggml" "$BUILD_RS"; then
    echo "✅ 构建脚本已经修复过，无需重复修复"
    exit 0
fi

echo "🔨 正在应用修复..."

# 备份原文件
cp "$BUILD_RS" "${BUILD_RS}.backup"
echo "📦 已备份原文件: ${BUILD_RS}.backup"

# 使用 sed 进行修复
# 删除硬编码的对象文件路径，替换为静态库链接
sed -i '' '/let ggml_obj = PathBuf::from(&out_path).join("llama.cpp\/ggml.o");/d' "$BUILD_RS"
sed -i '' '/cxx.object(ggml_obj);/d' "$BUILD_RS"

# 在 cxx.flag 之后添加静态库链接
sed -i '' '/for cxx_flag in cxx_flags.split_whitespace() {/,/}/ {
    /}/ {
        a\
\
    // 修复：使用静态库链接而不是硬编码的对象文件路径\
    println!("cargo:rustc-link-search=native={}", out_path.display());\
    println!("cargo:rustc-link-lib=static=ggml");
    }
}' "$BUILD_RS"

# 修复 Metal 对象文件查找逻辑
sed -i '' 's|let ggml_feature_obj =|// 查找实际生成的对象文件（可能带哈希前缀）\
        let out_dir = std::fs::read_dir(out_path).expect("Cannot read OUT_DIR");\
        let feature_pattern = format!("ggml-{}", ggml_type);\
        \
        for entry in out_dir {\
            if let Ok(entry) = entry {\
                let file_name = entry.file_name();\
                let name_str = file_name.to_string_lossy();\
                \
                if name_str.contains(&feature_pattern) \&\& name_str.ends_with(".o") {\
                    let ggml_feature_obj = entry.path();|' "$BUILD_RS"

sed -i '' 's|cxx.object(ggml_feature_obj);|cxx.object(\&ggml_feature_obj);\
                    break;\
                }\
            }\
        }|' "$BUILD_RS"

echo "✅ 修复完成！"
echo ""
echo "💡 提示："
echo "   - 每次运行 'cargo clean' 后需要重新运行此脚本"
echo "   - 或者直接运行 'cargo build' 会自动触发修复（如果已集成）"

