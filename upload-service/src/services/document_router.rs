use serde::{Deserialize, Serialize};

// Result 类型是两个参数的，需要明确指定
type RouterResult<T> = std::result::Result<T, anyhow::Error>;

/// 解析引擎类型
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParserEngine {
    /// pyresparser - 快速路径（英文/结构化）
    PyResParser,
    /// MinerU - OCR路径（扫描件/复杂表格）
    MinerU,
    /// DeepSeek - 智能路径（JD/深度分析）
    DeepSeek,
    /// DeepSeek OCR - 图片路径（截图/拍照）
    DeepSeekOCR,
    /// 混合路径（多引擎协同）
    Hybrid,
}

/// 用户意图
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum UserIntent {
    /// 快速筛选（批量上传）
    QuickScreen,
    /// 深度分析（关键候选人）
    DeepAnalysis,
    /// OCR识别（图片简历）
    OCR,
    /// JD解析（职位描述）
    ParseJD,
    /// 默认
    Default,
}

/// 文档特征
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DocumentFeatures {
    /// 是否为图片
    pub is_image: bool,
    /// 是否为扫描件
    pub is_scanned: bool,
    /// 是否包含复杂表格
    pub has_complex_tables: bool,
    /// 是否为JD
    pub is_job_description: bool,
    /// 语言
    pub language: Language,
    /// 是否结构化
    pub is_structured: bool,
    /// 文件大小
    pub file_size: usize,
}

/// 语言类型
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum Language {
    English,
    Chinese,
    Mixed,
    Unknown,
}

/// 智能文档路由器
pub struct DocumentRouter {
    /// 启用的引擎
    pub enable_pyresparser: bool,
    pub enable_mineru: bool,
    pub enable_deepseek: bool,
    #[allow(dead_code)]
    pub enable_deepseek_ocr: bool,
}

impl DocumentRouter {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            enable_pyresparser: true,   // 默认启用
            enable_mineru: false,       // 可选启用
            enable_deepseek: true,      // 默认启用
            enable_deepseek_ocr: false, // 可选启用
        }
    }

    /// 智能路由 - 选择最佳解析引擎
    #[allow(dead_code)]
    pub async fn route(
        &self,
        file_data: &[u8],
        file_name: &str,
        mime_type: &str,
        user_intent: UserIntent,
    ) -> RouterResult<ParserEngine> {
        tracing::info!("🔀 智能路由: 文件={}, 类型={}", file_name, mime_type);

        // 1. 分析文档特征
        let features = self.analyze_features(file_data, file_name, mime_type).await;

        // 2. 根据特征智能选择
        let engine = self.select_engine(&features, user_intent);

        tracing::info!(
            "✅ 路由决策: {:?} (原因: {})",
            engine,
            self.get_reason(&engine, &features)
        );

        Ok(engine)
    }

    /// 分析文档特征
    #[allow(dead_code)]
    async fn analyze_features(
        &self,
        file_data: &[u8],
        file_name: &str,
        mime_type: &str,
    ) -> DocumentFeatures {
        let file_name_lower = file_name.to_lowercase();

        DocumentFeatures {
            // 检测是否为图片
            is_image: mime_type.starts_with("image/"),

            // 检测是否为扫描件（简单启发式：文件名包含"scan"）
            is_scanned: file_name_lower.contains("scan") || file_name_lower.contains("扫描"),

            // 检测是否包含复杂表格（文件较大且PDF）
            has_complex_tables: mime_type == "application/pdf" && file_data.len() > 200_000,

            // 检测是否为JD（文件名包含关键词）
            is_job_description: file_name_lower.contains("jd")
                || file_name_lower.contains("职位")
                || file_name_lower.contains("岗位")
                || file_name_lower.contains("job"),

            // 检测语言（简单：根据文件名）
            language: if file_name_lower.contains("resume") || file_name_lower.contains("cv") {
                Language::English
            } else if file_name_lower.contains("简历") {
                Language::Chinese
            } else {
                Language::Mixed
            },

            // 检测是否结构化（PDF/DOCX通常是结构化的）
            is_structured: matches!(
                mime_type,
                "application/pdf"
                    | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),

            file_size: file_data.len(),
        }
    }

    /// 选择解析引擎
    #[allow(dead_code)]
    fn select_engine(&self, features: &DocumentFeatures, user_intent: UserIntent) -> ParserEngine {
        // 优先级1: 图片 → DeepSeek OCR 或 MinerU
        if features.is_image {
            if self.enable_deepseek_ocr {
                return ParserEngine::DeepSeekOCR;
            } else if self.enable_mineru {
                return ParserEngine::MinerU;
            }
        }

        // 优先级2: JD职位描述 → DeepSeek
        if features.is_job_description && self.enable_deepseek {
            return ParserEngine::DeepSeek;
        }

        // 优先级3: 用户明确意图
        match user_intent {
            UserIntent::QuickScreen if self.enable_pyresparser => {
                return ParserEngine::PyResParser;
            }
            UserIntent::DeepAnalysis if self.enable_deepseek => {
                return ParserEngine::Hybrid;
            }
            UserIntent::OCR if self.enable_mineru => {
                return ParserEngine::MinerU;
            }
            UserIntent::ParseJD if self.enable_deepseek => {
                return ParserEngine::DeepSeek;
            }
            _ => {}
        }

        // 优先级4: 英文 + 结构化 → pyresparser（最快）
        if features.language == Language::English
            && features.is_structured
            && self.enable_pyresparser
        {
            return ParserEngine::PyResParser;
        }

        // 优先级5: 扫描件或复杂表格 → MinerU
        if (features.is_scanned || features.has_complex_tables) && self.enable_mineru {
            return ParserEngine::MinerU;
        }

        // 优先级6: 小文件优先快速路径
        if features.file_size < 100_000 && self.enable_pyresparser {
            return ParserEngine::PyResParser;
        }

        // 默认策略：根据启用情况选择
        if self.enable_pyresparser {
            ParserEngine::PyResParser // 优先快速
        } else if self.enable_mineru {
            ParserEngine::MinerU
        } else if self.enable_deepseek {
            ParserEngine::DeepSeek
        } else {
            ParserEngine::PyResParser // 兜底
        }
    }

    /// 获取选择原因（用于日志）
    #[allow(dead_code)]
    fn get_reason(&self, engine: &ParserEngine, features: &DocumentFeatures) -> String {
        match engine {
            ParserEngine::PyResParser => {
                if features.language == Language::English {
                    "英文简历，使用快速路径".to_string()
                } else if features.file_size < 100_000 {
                    "小文件，使用快速路径".to_string()
                } else {
                    "默认使用快速路径".to_string()
                }
            }
            ParserEngine::MinerU => {
                if features.is_scanned {
                    "扫描件，需要OCR".to_string()
                } else if features.has_complex_tables {
                    "复杂表格，需要高级解析".to_string()
                } else {
                    "使用MinerU解析".to_string()
                }
            }
            ParserEngine::DeepSeek => {
                if features.is_job_description {
                    "JD职位描述，需要智能分析".to_string()
                } else {
                    "需要深度理解的文档".to_string()
                }
            }
            ParserEngine::DeepSeekOCR => "图片文件，需要OCR识别".to_string(),
            ParserEngine::Hybrid => "混合策略，多引擎协同".to_string(),
        }
    }
}

/// 路由配置
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// pyresparser 服务地址
    pub pyresparser_url: String,
    /// MinerU 服务地址
    pub mineru_url: Option<String>,
    /// DeepSeek 服务地址
    pub deepseek_url: String,
    /// 是否启用混合策略
    pub enable_hybrid: bool,
    /// 快速模式阈值（文件大小）
    pub quick_mode_threshold: usize,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            pyresparser_url: "http://localhost:5001".to_string(),
            mineru_url: None, // 可选
            deepseek_url: "http://localhost:11434".to_string(),
            enable_hybrid: true,
            quick_mode_threshold: 100_000, // 100KB
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_image() {
        let mut router = DocumentRouter::new();
        router.enable_mineru = true;
        let features = DocumentFeatures {
            is_image: true,
            is_scanned: false,
            has_complex_tables: false,
            is_job_description: false,
            language: Language::Chinese,
            is_structured: false,
            file_size: 50000,
        };

        let engine = router.select_engine(&features, UserIntent::Default);
        assert!(matches!(
            engine,
            ParserEngine::MinerU | ParserEngine::DeepSeekOCR
        ));
    }

    #[test]
    fn test_route_english_resume() {
        let router = DocumentRouter::new();
        let features = DocumentFeatures {
            is_image: false,
            is_scanned: false,
            has_complex_tables: false,
            is_job_description: false,
            language: Language::English,
            is_structured: true,
            file_size: 50000,
        };

        let engine = router.select_engine(&features, UserIntent::Default);
        assert!(matches!(engine, ParserEngine::PyResParser));
    }

    #[test]
    fn test_route_jd() {
        let router = DocumentRouter::new();
        let features = DocumentFeatures {
            is_image: false,
            is_scanned: false,
            has_complex_tables: false,
            is_job_description: true,
            language: Language::Chinese,
            is_structured: false,
            file_size: 10000,
        };

        let engine = router.select_engine(&features, UserIntent::ParseJD);
        assert!(matches!(engine, ParserEngine::DeepSeek));
    }
}
