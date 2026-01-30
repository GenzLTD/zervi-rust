#!/bin/bash
# 本地微调 LLM 模型（用于特定任务优化）
# 注意：微调需要 GPU 和大量资源

set -e

echo "🎓 LLM 模型微调设置"
echo "================================"
echo ""
echo "⚠️  注意：微调模型需要："
echo "   - GPU（推荐 16GB+ VRAM）"
echo "   - 大量训练数据"
echo "   - 较长的训练时间"
echo ""

read -p "是否继续设置微调环境? (y/N): " confirm
if [[ ! "$confirm" =~ ^[Yy]$ ]]; then
    echo "已取消"
    exit 0
fi

echo ""
echo "📦 检查微调依赖..."

# 检查 Python
if ! command -v python3 &> /dev/null; then
    echo "❌ 需要 Python 3"
    exit 1
fi

# 检查 CUDA/GPU
if command -v nvidia-smi &> /dev/null; then
    echo "✅ 检测到 NVIDIA GPU"
    nvidia-smi --query-gpu=name,memory.total --format=csv,noheader
else
    echo "⚠️  未检测到 NVIDIA GPU，微调将使用 CPU（非常慢）"
fi

echo ""
echo "📥 安装微调工具..."

# 安装必要的 Python 包
pip3 install --user \
    transformers \
    datasets \
    accelerate \
    peft \
    bitsandbytes \
    torch \
    torchvision \
    torchaudio \
    || {
    echo "❌ 安装失败"
    exit 1
}

echo ""
echo "✅ 依赖安装完成"
echo ""
echo "📋 微调选项："
echo ""
echo "1. LoRA 微调（推荐，资源需求较低）"
echo "2. 全参数微调（需要更多资源）"
echo "3. QLoRA 微调（量化 + LoRA，最节省资源）"
echo ""

read -p "请选择微调方法 (1-3) [默认: 1]: " method
method=${method:-1}

case $method in
    1)
        METHOD_NAME="LoRA"
        METHOD_DESC="低秩适应，适合资源有限的情况"
        ;;
    2)
        METHOD_NAME="Full Fine-tuning"
        METHOD_DESC="全参数微调，效果最好但需要大量资源"
        ;;
    3)
        METHOD_NAME="QLoRA"
        METHOD_DESC="量化 + LoRA，最节省资源"
        ;;
    *)
        echo "❌ 无效选择"
        exit 1
        ;;
esac

echo ""
echo "选择的方法: $METHOD_NAME"
echo "描述: $METHOD_DESC"
echo ""

# 创建微调脚本模板
FINETUNE_DIR="./finetune"
mkdir -p "$FINETUNE_DIR"

cat > "$FINETUNE_DIR/finetune_template.py" <<'PYTHON_EOF'
#!/usr/bin/env python3
"""
LLM 微调模板脚本
根据选择的微调方法调整参数
"""

import torch
from transformers import (
    AutoModelForCausalLM,
    AutoTokenizer,
    TrainingArguments,
    Trainer,
    DataCollatorForLanguageModeling
)
from datasets import load_dataset
from peft import LoraConfig, get_peft_model, TaskType

# 配置
MODEL_NAME = "Qwen/Qwen2.5-1.5B-Instruct"  # 基础模型
DATASET_PATH = "./data/train.jsonl"  # 训练数据路径
OUTPUT_DIR = "./models/finetuned"
METHOD = "lora"  # lora, qlora, full

def load_model_and_tokenizer():
    """加载模型和分词器"""
    tokenizer = AutoTokenizer.from_pretrained(MODEL_NAME)
    model = AutoModelForCausalLM.from_pretrained(
        MODEL_NAME,
        torch_dtype=torch.float16 if torch.cuda.is_available() else torch.float32,
        device_map="auto" if torch.cuda.is_available() else None,
    )
    return model, tokenizer

def setup_lora(model):
    """设置 LoRA"""
    lora_config = LoraConfig(
        task_type=TaskType.CAUSAL_LM,
        r=8,  # LoRA 秩
        lora_alpha=16,
        lora_dropout=0.1,
        target_modules=["q_proj", "v_proj"]  # 根据模型调整
    )
    model = get_peft_model(model, lora_config)
    return model

def prepare_dataset(tokenizer, dataset_path):
    """准备训练数据集"""
    # 这里需要根据实际数据格式调整
    # 示例：JSONL 格式，每行包含 "text" 字段
    dataset = load_dataset("json", data_files=dataset_path, split="train")
    
    def tokenize_function(examples):
        return tokenizer(
            examples["text"],
            truncation=True,
            max_length=512,
            padding="max_length"
        )
    
    tokenized_dataset = dataset.map(tokenize_function, batched=True)
    return tokenized_dataset

def main():
    print("🚀 开始微调...")
    
    # 加载模型
    print("📥 加载模型...")
    model, tokenizer = load_model_and_tokenizer()
    
    # 设置 LoRA（如果选择）
    if METHOD in ["lora", "qlora"]:
        print("🔧 设置 LoRA...")
        model = setup_lora(model)
    
    # 准备数据
    print("📊 准备训练数据...")
    dataset = prepare_dataset(tokenizer, DATASET_PATH)
    
    # 训练参数
    training_args = TrainingArguments(
        output_dir=OUTPUT_DIR,
        num_train_epochs=3,
        per_device_train_batch_size=4,
        gradient_accumulation_steps=4,
        warmup_steps=100,
        logging_steps=10,
        save_steps=500,
        fp16=torch.cuda.is_available(),
    )
    
    # 数据整理器
    data_collator = DataCollatorForLanguageModeling(
        tokenizer=tokenizer,
        mlm=False
    )
    
    # 训练器
    trainer = Trainer(
        model=model,
        args=training_args,
        train_dataset=dataset,
        data_collator=data_collator,
    )
    
    # 开始训练
    print("🎓 开始训练...")
    trainer.train()
    
    # 保存模型
    print("💾 保存模型...")
    trainer.save_model()
    tokenizer.save_pretrained(OUTPUT_DIR)
    
    print("✅ 微调完成！")
    print(f"模型保存在: {OUTPUT_DIR}")

if __name__ == "__main__":
    main()
PYTHON_EOF

chmod +x "$FINETUNE_DIR/finetune_template.py"

echo "✅ 微调环境设置完成"
echo ""
echo "📋 下一步："
echo ""
echo "1. 准备训练数据（JSONL 格式）"
echo "2. 编辑 $FINETUNE_DIR/finetune_template.py"
echo "3. 调整模型名称、数据路径和训练参数"
echo "4. 运行: python3 $FINETUNE_DIR/finetune_template.py"
echo ""
echo "💡 提示："
echo "   - 训练数据格式：每行一个 JSON 对象，包含 'text' 字段"
echo "   - 建议先用小数据集测试"
echo "   - 监控 GPU 使用情况：watch -n 1 nvidia-smi"
echo ""

