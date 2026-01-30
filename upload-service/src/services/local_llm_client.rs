use lazy_static::lazy_static;
use llama_cpp_rs::{
    options::{ModelOptions, PredictOptions},
    LLama,
};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex as StdMutex};
use std::thread;
use tracing;

// 全局 LLM 请求通道（用于在线程间传递请求）
lazy_static! {
    static ref LLM_REQUEST_CHANNEL: Arc<StdMutex<Option<(mpsc::Sender<LLMRequest>, thread::JoinHandle<()>)>>> =
        Arc::new(StdMutex::new(None));
}

// LLM 请求消息
enum LLMRequest {
    Predict {
        prompt: String,
        n_threads: usize,
        response_tx: mpsc::Sender<Result<String, anyhow::Error>>,
    },
}

/// 本地 LLM 客户端（使用 llama-cpp-rs）
/// 支持完全本地运行，无需外部 API 服务
///
/// 注意：LLama 内部包含 *mut c_void，不是 Send，因此使用专门的线程来处理所有请求
pub struct LocalLLMClient {
    // 模型路径（用于初始化）
    _model_path: String,
    // 上下文大小
    context_size: usize,
    // 线程数
    n_threads: usize,
}

// LocalLLMClient 是 Send 和 Sync 的，因为它不直接包含非 Send 类型

impl Clone for LocalLLMClient {
    fn clone(&self) -> Self {
        Self {
            _model_path: self._model_path.clone(),
            context_size: self.context_size,
            n_threads: self.n_threads,
        }
    }
}

impl LocalLLMClient {
    /// 创建新的本地 LLM 客户端
    ///
    /// # Arguments
    /// * `model_path` - GGUF 模型文件路径
    /// * `context_size` - 上下文窗口大小（默认 4096）
    /// * `n_threads` - 推理线程数（默认使用 CPU 核心数）
    pub fn new(
        model_path: &str,
        context_size: Option<usize>,
        n_threads: Option<usize>,
    ) -> Result<Self, anyhow::Error> {
        let context_size = context_size.unwrap_or(4096);
        let n_threads = n_threads.unwrap_or_else(|| num_cpus::get());

        tracing::info!(
            "🔄 正在加载本地 LLM 模型: {} (上下文: {}, 线程: {})",
            model_path,
            context_size,
            n_threads
        );

        // 解析路径（如果是相对路径，转换为绝对路径）
        let resolved_path = if Path::new(model_path).is_absolute() {
            Path::new(model_path).to_path_buf()
        } else {
            // 相对路径：尝试相对于当前工作目录
            let current_dir =
                std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
            let abs_path = current_dir.join(model_path);

            // 如果当前目录下不存在，尝试相对于 upload-service 目录
            if !abs_path.exists() {
                // 尝试查找 upload-service 目录
                let mut search_dir = current_dir.clone();
                let mut found = false;
                let mut resolved = abs_path.clone();

                while search_dir.parent().is_some() {
                    let candidate = search_dir.join("upload-service").join(model_path);
                    if candidate.exists() {
                        resolved = candidate;
                        found = true;
                        break;
                    }
                    search_dir = search_dir.parent().unwrap().to_path_buf();
                }

                if found {
                    resolved
                } else {
                    abs_path
                }
            } else {
                abs_path
            }
        };

        let model_path_str = resolved_path.to_string_lossy().to_string();

        // 检查模型文件是否存在
        if !resolved_path.exists() {
            anyhow::bail!(
                "模型文件不存在: {}\n\
                请下载 GGUF 格式的模型文件，例如：\n\
                - Qwen2.5-7B-Instruct-Q4_K_M.gguf\n\
                - ChatGLM3-6B-Q4_K_M.gguf\n\
                当前工作目录: {:?}\n\
                尝试的路径: {}",
                model_path,
                std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf()),
                model_path_str
            );
        }

        // 使用解析后的绝对路径
        let model_path = model_path_str;

        // 初始化全局 LLM 处理线程（如果尚未初始化）
        let mut channel_guard = LLM_REQUEST_CHANNEL
            .lock()
            .map_err(|e| anyhow::anyhow!("获取全局通道锁失败: {}", e))?;

        if channel_guard.is_none() {
            tracing::info!("🔄 初始化本地 LLM 处理线程...");

            // 配置模型参数
            let mut model_opts = ModelOptions::default();
            model_opts.set_context(context_size as i32);
            model_opts.set_n_batch(512);
            model_opts.set_m_map(true); // 使用内存映射，减少内存占用
            model_opts.set_gpu_layers(0); // 0 = 仅 CPU，>0 使用 GPU 层数

            // 加载模型（在专门的线程中）
            let model_path_clone = model_path.to_string();
            let (request_tx, request_rx) = mpsc::channel::<LLMRequest>();

            let handle = thread::spawn(move || {
                // 在专门线程中加载模型
                // 先验证文件是否存在且可读
                if !std::path::Path::new(&model_path_clone).exists() {
                    tracing::error!("❌ 模型文件不存在: {}", model_path_clone);
                    return;
                }

                // 检查文件大小（应该大于100MB）
                if let Ok(metadata) = std::fs::metadata(&model_path_clone) {
                    let file_size = metadata.len();
                    tracing::info!(
                        "📦 模型文件大小: {} 字节 ({:.2} GB)",
                        file_size,
                        file_size as f64 / 1024.0 / 1024.0 / 1024.0
                    );
                    if file_size < 100_000_000 {
                        tracing::error!("❌ 模型文件太小（{} 字节），可能不完整", file_size);
                        return;
                    }
                }

                // 检查文件前几个字节（GGUF文件应该有magic number）
                if let Ok(mut file) = std::fs::File::open(&model_path_clone) {
                    use std::io::Read;
                    let mut header = [0u8; 4];
                    if file.read_exact(&mut header).is_ok() {
                        let magic = u32::from_le_bytes(header);
                        tracing::info!("🔍 文件magic number: 0x{:08x}", magic);
                        // GGUF文件的magic number应该是0x46554747 ("GGUF"的little-endian)
                        if magic != 0x46554747 {
                            tracing::warn!("⚠️  文件magic number不匹配，可能不是有效的GGUF文件");
                        }
                    }
                }

                let model = match LLama::new(model_path_clone.clone(), &model_opts) {
                    Ok(m) => {
                        tracing::info!("✅ 本地 LLM 模型加载完成（在专门线程中）");
                        Arc::new(StdMutex::new(m))
                    }
                    Err(e) => {
                        tracing::error!("❌ 加载模型失败: {}", e);
                        tracing::error!("   模型路径: {}", model_path_clone);
                        tracing::error!("   上下文大小: {}", context_size);
                        tracing::error!("   请检查：");
                        tracing::error!("   1. 模型文件是否完整（应该 > 3GB）");
                        tracing::error!("   2. 模型格式是否为GGUF");
                        tracing::error!("   3. llama.cpp版本是否支持该模型架构");
                        return; // 线程退出
                    }
                };

                // 处理请求循环
                while let Ok(request) = request_rx.recv() {
                    match request {
                        LLMRequest::Predict {
                            prompt,
                            n_threads,
                            response_tx,
                        } => {
                            let model_guard = match model.lock() {
                                Ok(guard) => guard,
                                Err(e) => {
                                    let _ = response_tx
                                        .send(Err(anyhow::anyhow!("获取模型锁失败: {}", e)));
                                    continue;
                                }
                            };

                            // 配置推理参数
                            let mut predict_opts = PredictOptions::default();
                            predict_opts.set_threads(n_threads as i32);
                            predict_opts.set_tokens(2048);
                            predict_opts.set_temperature(0.7);
                            predict_opts.set_top_k(40);
                            predict_opts.set_top_p(0.95);
                            predict_opts.set_penalty(1.1);
                            predict_opts.set_seed(-1);
                            predict_opts.stop_prompts = vec!["\n\n".to_string(), "```".to_string()];

                            // 执行推理
                            let result = model_guard
                                .predict(prompt, predict_opts)
                                .map_err(|e| anyhow::anyhow!("推理失败: {}", e));

                            let _ = response_tx.send(result);
                        }
                    }
                }
            });

            *channel_guard = Some((request_tx, handle));
            tracing::info!("✅ 本地 LLM 处理线程已启动");
        }

        Ok(Self {
            _model_path: model_path.to_string(),
            context_size,
            n_threads,
        })
    }

    /// 提取简历信息
    pub async fn extract_resume_info(
        &self,
        document_text: &str,
    ) -> Result<super::deepseek_client::ExtractedResumeInfo, anyhow::Error> {
        tracing::info!(
            "🤖 使用本地LLM提取简历信息，文本长度: {}",
            document_text.len()
        );

        let prompt = Self::build_extraction_prompt(document_text);
        let response = self.generate_json(&prompt).await?;

        // 解析JSON响应
        let info: super::deepseek_client::ExtractedResumeInfo = serde_json::from_str(&response)
            .map_err(|e| {
                tracing::error!("本地LLM返回格式错误: {}", e);
                tracing::debug!("原始响应: {}", response);
                anyhow::anyhow!("解析LLM响应失败: {}", e)
            })?;

        tracing::info!(
            "✅ 本地LLM提取成功: 姓名={:?}, 技能数={}",
            info.name,
            info.skills.len()
        );

        Ok(info)
    }

    /// 提取诗人档案信息
    pub async fn extract_poetry_profile(
        &self,
        document_text: &str,
    ) -> Result<super::deepseek_client::ExtractedPoetryProfile, anyhow::Error> {
        tracing::info!(
            "🤖 使用本地LLM提取诗人档案信息，文本长度: {}",
            document_text.len()
        );

        let prompt = Self::build_poetry_extraction_prompt(document_text);
        let response = self.generate_json(&prompt).await?;

        // 解析JSON响应
        let profile: super::deepseek_client::ExtractedPoetryProfile =
            serde_json::from_str(&response).map_err(|e| {
                tracing::error!("本地LLM返回格式错误: {}", e);
                tracing::debug!("原始响应: {}", response);
                anyhow::anyhow!("解析LLM响应失败: {}", e)
            })?;

        tracing::info!(
            "✅ 本地LLM提取成功: 诗人={:?}, 作品数={}, 地点数={}",
            profile.name,
            profile.works.len(),
            profile.locations.len()
        );

        Ok(profile)
    }

    /// 质量评分
    pub async fn score_quality(
        &self,
        extracted_info: &super::deepseek_client::ExtractedResumeInfo,
        document_text: &str,
    ) -> Result<super::deepseek_client::QualityScore, anyhow::Error> {
        tracing::info!("📊 使用本地LLM进行质量评分");

        let prompt = Self::build_quality_prompt(extracted_info, document_text);
        let response = self.generate_json(&prompt).await?;

        let score: super::deepseek_client::QualityScore =
            serde_json::from_str(&response).map_err(|e| {
                tracing::error!("本地LLM评分格式错误: {}", e);
                anyhow::anyhow!("解析评分响应失败: {}", e)
            })?;

        tracing::info!("✅ 本地LLM质量评分完成: {}/100", score.overall_score);

        Ok(score)
    }

    /// 生成 JSON 格式的响应
    async fn generate_json(&self, prompt: &str) -> Result<String, anyhow::Error> {
        // 构建完整提示词，强调 JSON 格式要求
        let full_prompt = format!(
            "你是一个专业的信息提取助手。请严格按照JSON格式返回结果，不要包含任何其他文本、解释或markdown代码块标记。\n\n{}",
            prompt
        );

        // 通过全局通道发送请求到专门的 LLM 处理线程
        let request_tx = {
            let channel_guard = LLM_REQUEST_CHANNEL
                .lock()
                .map_err(|e| anyhow::anyhow!("获取全局通道锁失败: {}", e))?;

            let (tx, _handle) = channel_guard
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("LLM 处理线程未初始化"))?;

            // 克隆 Sender（它是 Send 的）
            tx.clone()
        }; // 在这里释放锁

        // 创建响应通道
        let (response_tx, response_rx) = mpsc::channel();

        // 发送请求
        request_tx
            .send(LLMRequest::Predict {
                prompt: full_prompt,
                n_threads: self.n_threads,
                response_tx,
            })
            .map_err(|e| anyhow::anyhow!("发送请求失败: {}", e))?;

        // 在 tokio blocking thread 中等待结果
        let response = tokio::task::spawn_blocking(move || {
            response_rx
                .recv()
                .unwrap_or_else(|e| Err(anyhow::anyhow!("接收结果失败: {}", e)))
        })
        .await??;

        tracing::info!("📝 本地 LLM 生成完成: 输出长度: {}", response.len());

        // 提取 JSON 部分
        let json_text = Self::extract_json_from_response(&response);

        // 验证 JSON 格式
        if serde_json::from_str::<serde_json::Value>(&json_text).is_err() {
            tracing::warn!("⚠️ 提取的 JSON 格式可能有问题，原始输出: {}", response);
            // 尝试修复常见的 JSON 格式问题
            let fixed_json = Self::fix_json_format(&json_text);
            if serde_json::from_str::<serde_json::Value>(&fixed_json).is_ok() {
                return Ok(fixed_json);
            }
        }

        Ok(json_text)
    }

    /// 从响应文本中提取 JSON 部分
    fn extract_json_from_response(text: &str) -> String {
        // 方法1: 查找第一个 { 到最后一个 } 之间的内容
        if let Some(start) = text.find('{') {
            if let Some(end) = text.rfind('}') {
                let json_candidate = &text[start..=end];
                // 验证是否是有效的 JSON
                if serde_json::from_str::<serde_json::Value>(json_candidate).is_ok() {
                    return json_candidate.to_string();
                }
            }
        }

        // 方法2: 移除 markdown 代码块标记
        let cleaned = text
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        // 再次尝试提取 JSON
        if let Some(start) = cleaned.find('{') {
            if let Some(end) = cleaned.rfind('}') {
                return cleaned[start..=end].to_string();
            }
        }

        // 如果都失败，返回清理后的文本
        cleaned.to_string()
    }

    /// 修复常见的 JSON 格式问题
    fn fix_json_format(json: &str) -> String {
        let mut fixed = json.to_string();

        // 移除尾随逗号
        fixed = regex::Regex::new(r",\s*}")
            .unwrap()
            .replace_all(&fixed, "}")
            .to_string();
        fixed = regex::Regex::new(r",\s*]")
            .unwrap()
            .replace_all(&fixed, "]")
            .to_string();

        // 确保字符串被正确引用
        // 这里可以添加更多修复逻辑

        fixed
    }

    /// 构建简历提取提示词
    fn build_extraction_prompt(text: &str) -> String {
        let text_preview = if text.len() > 8000 {
            &text[..8000]
        } else {
            text
        };

        format!(
            r#"请从以下简历文本中提取关键信息，以JSON格式返回。

要求：
1. 严格按照JSON格式返回
2. 如果某些信息未找到，使用null
3. 技能列表提取所有相关技能
4. 工作经验按时间倒序

返回格式：
{{
  "name": "姓名",
  "phone": "手机号",
  "email": "邮箱",
  "location": "所在城市",
  "education": [
    {{
      "school": "学校名称",
      "major": "专业",
      "degree": "学历(本科/硕士/博士)",
      "start_date": "2018-09",
      "end_date": "2022-06"
    }}
  ],
  "work_experience": [
    {{
      "company": "公司名称",
      "position": "职位",
      "description": "工作描述",
      "start_date": "2022-07",
      "end_date": "2024-10"
    }}
  ],
  "skills": ["技能1", "技能2", "技能3"],
  "summary": "个人简介摘要"
}}

简历文本：
{}
"#,
            text_preview
        )
    }

    /// 构建诗人档案提取提示词
    fn build_poetry_extraction_prompt(text: &str) -> String {
        let text_preview = if text.len() > 8000 {
            &text[..8000]
        } else {
            text
        };

        format!(
            r#"请从以下古代诗人档案文本中提取关键信息，以JSON格式返回。

要求：
1. 严格按照JSON格式返回
2. 如果某些信息未找到，使用null或空数组
3. 作品列表提取所有提到的诗词作品
4. 地点列表提取所有提到的游历、居住、出生、逝世地点
5. 事件列表提取重要的生平事件（如及第、任职、隐居等）
6. 时间信息尽量提取具体年份或朝代

返回格式：
{{
  "name": "诗人姓名",
  "courtesy_name": "字号",
  "pseudonym": "别号",
  "dynasty": "朝代（如：唐代、宋代）",
  "birth_year": "出生年份",
  "death_year": "逝世年份",
  "birthplace": "出生地/籍贯",
  "deathplace": "逝世地",
  "biography": "生平简介",
  "official_positions": ["官职1", "官职2"],
  "literary_style": "文学风格/诗派",
  "works": [
    {{
      "title": "作品标题",
      "excerpt": "名句/片段",
      "category": "作品类型（如：五言诗、七言绝句）"
    }}
  ],
  "locations": [
    {{
      "name": "地点名称",
      "type": "地点类型（如：出生地、游历地、隐居地）",
      "description": "相关描述"
    }}
  ],
  "events": [
    {{
      "year": "年份（如：天宝初年）",
      "event": "事件描述",
      "type": "事件类型（如：及第、任职、游历、隐居）"
    }}
  ],
  "famous_quotes": ["名句1", "名句2"],
  "extraction_confidence": 0.85
}}

诗人档案文本：
{}
"#,
            text_preview
        )
    }

    /// 构建质量评分提示词
    fn build_quality_prompt(
        info: &super::deepseek_client::ExtractedResumeInfo,
        text: &str,
    ) -> String {
        let text_preview = if text.len() > 3000 {
            &text[..3000]
        } else {
            text
        };

        format!(
            r#"请评估以下简历的质量，返回JSON格式评分。

评分维度：
1. overall_score: 总分 (0-100)
2. completeness: 完整性 (0-100) - 信息是否齐全
3. clarity: 清晰度 (0-100) - 表达是否清晰
4. professionalism: 专业性 (0-100) - 内容是否专业

已提取信息：
{}

简历文本摘要：
{}

返回格式：
{{
  "overall_score": 85,
  "completeness": 90,
  "clarity": 80,
  "professionalism": 85,
  "highlights": ["亮点1", "亮点2"],
  "suggestions": ["建议1", "建议2"]
}}
"#,
            serde_json::to_string_pretty(info).unwrap_or_default(),
            text_preview
        )
    }
}
