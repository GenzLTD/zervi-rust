use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 文档类型
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DocumentType {
    /// 简历文档
    Resume,
    /// 职位描述（JD）
    JobDescription,
    /// 古代诗人档案
    PoetryProfile,
    /// 其他文档
    Other,
}

/// 文档分类结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationResult {
    /// 文档类型
    pub doc_type: DocumentType,
    /// 置信度 (0.0-1.0)
    pub confidence: f64,
    /// 匹配的关键特征
    pub matched_features: Vec<String>,
}

/// 文档分类器
pub struct DocumentClassifier {
    /// 简历关键词权重
    resume_keywords: HashMap<String, f64>,
    /// JD关键词权重
    jd_keywords: HashMap<String, f64>,
    /// 诗人档案关键词权重
    poetry_keywords: HashMap<String, f64>,
}

impl DocumentClassifier {
    pub fn new() -> Self {
        let mut resume_keywords = HashMap::new();
        let mut jd_keywords = HashMap::new();
        let mut poetry_keywords = HashMap::new();

        // 简历关键词（中文）- 更高权重
        resume_keywords.insert("简历".to_string(), 3.0);
        resume_keywords.insert("个人简历".to_string(), 3.0);
        resume_keywords.insert("求职简历".to_string(), 3.0);
        resume_keywords.insert("教育背景".to_string(), 2.5);
        resume_keywords.insert("教育经历".to_string(), 2.5);
        resume_keywords.insert("工作经历".to_string(), 2.5);
        resume_keywords.insert("工作经验".to_string(), 2.5);
        resume_keywords.insert("项目经验".to_string(), 2.0);
        resume_keywords.insert("项目经历".to_string(), 2.0);
        resume_keywords.insert("个人技能".to_string(), 2.0);
        resume_keywords.insert("专业技能".to_string(), 2.0);
        resume_keywords.insert("获奖情况".to_string(), 1.5);
        resume_keywords.insert("自我评价".to_string(), 1.5);
        resume_keywords.insert("求职意向".to_string(), 2.0);
        resume_keywords.insert("期望薪资".to_string(), 1.5);
        resume_keywords.insert("到岗时间".to_string(), 1.0);

        // 简历关键词（英文）
        resume_keywords.insert("resume".to_string(), 3.0);
        resume_keywords.insert("curriculum vitae".to_string(), 3.0);
        resume_keywords.insert("cv".to_string(), 2.5);
        resume_keywords.insert("education".to_string(), 2.0);
        resume_keywords.insert("work experience".to_string(), 2.5);
        resume_keywords.insert("skills".to_string(), 2.0);
        resume_keywords.insert("projects".to_string(), 1.5);
        resume_keywords.insert("awards".to_string(), 1.0);
        resume_keywords.insert("certifications".to_string(), 1.5);

        // JD关键词（中文）- 更高权重
        jd_keywords.insert("岗位说明".to_string(), 3.0);
        jd_keywords.insert("岗位说明书".to_string(), 3.0);
        jd_keywords.insert("职位描述".to_string(), 3.0);
        jd_keywords.insert("招聘岗位".to_string(), 3.0);
        jd_keywords.insert("招聘需求".to_string(), 2.5);
        jd_keywords.insert("岗位职责".to_string(), 2.5);
        jd_keywords.insert("职位职责".to_string(), 2.5);
        jd_keywords.insert("任职要求".to_string(), 2.5);
        jd_keywords.insert("任职资格".to_string(), 2.5);
        jd_keywords.insert("岗位要求".to_string(), 2.5);
        jd_keywords.insert("招聘人数".to_string(), 2.0);
        jd_keywords.insert("工作地点".to_string(), 1.5);
        jd_keywords.insert("薪资待遇".to_string(), 1.5);
        jd_keywords.insert("薪资范围".to_string(), 1.5);
        jd_keywords.insert("福利待遇".to_string(), 1.0);
        jd_keywords.insert("公司介绍".to_string(), 1.0);
        jd_keywords.insert("招聘机构".to_string(), 1.5);

        // ✅【优化】Excel表格特征关键词（2025-11-13）
        jd_keywords.insert("岗位汇总".to_string(), 3.5);
        jd_keywords.insert("招聘需求表".to_string(), 3.5);
        jd_keywords.insert("岗位信息表".to_string(), 3.5);
        jd_keywords.insert("招聘计划".to_string(), 3.0);
        jd_keywords.insert("职位需求".to_string(), 3.0);
        jd_keywords.insert("人员编制".to_string(), 2.5);
        jd_keywords.insert("岗位编制".to_string(), 2.5);
        jd_keywords.insert("【表头】".to_string(), 2.0); // Excel表头标识
        jd_keywords.insert("【关键标识】岗位招聘表格".to_string(), 4.0); // Excel强特征

        // JD关键词（英文）
        jd_keywords.insert("job description".to_string(), 3.0);
        jd_keywords.insert("job posting".to_string(), 3.0);
        jd_keywords.insert("position description".to_string(), 3.0);
        jd_keywords.insert("responsibilities".to_string(), 2.5);
        jd_keywords.insert("requirements".to_string(), 2.5);
        jd_keywords.insert("qualifications".to_string(), 2.5);
        jd_keywords.insert("job requirements".to_string(), 2.5);
        jd_keywords.insert("we are looking for".to_string(), 2.0);
        jd_keywords.insert("we offer".to_string(), 1.5);
        jd_keywords.insert("salary range".to_string(), 1.5);
        jd_keywords.insert("company overview".to_string(), 1.0);

        // ✅【优化】JD核心动词（2025-11-13）
        jd_keywords.insert("负责".to_string(), 1.5);
        jd_keywords.insert("主导".to_string(), 1.5);
        jd_keywords.insert("协调".to_string(), 1.0);
        jd_keywords.insert("制定".to_string(), 1.0);
        jd_keywords.insert("实施".to_string(), 1.0);

        // 诗人档案关键词（中文）- 高权重
        poetry_keywords.insert("诗人".to_string(), 3.0);
        poetry_keywords.insert("唐代".to_string(), 2.5);
        poetry_keywords.insert("宋代".to_string(), 2.5);
        poetry_keywords.insert("元代".to_string(), 2.5);
        poetry_keywords.insert("明代".to_string(), 2.5);
        poetry_keywords.insert("清代".to_string(), 2.5);
        poetry_keywords.insert("全唐诗".to_string(), 3.0);
        poetry_keywords.insert("全宋词".to_string(), 3.0);
        poetry_keywords.insert("作品".to_string(), 2.0);
        poetry_keywords.insert("诗词".to_string(), 2.5);
        poetry_keywords.insert("诗句".to_string(), 2.0);
        poetry_keywords.insert("生平".to_string(), 2.5);
        poetry_keywords.insert("传记".to_string(), 2.5);
        poetry_keywords.insert("字号".to_string(), 2.0);
        poetry_keywords.insert("籍贯".to_string(), 2.0);
        poetry_keywords.insert("出生".to_string(), 1.5);
        poetry_keywords.insert("逝世".to_string(), 1.5);
        poetry_keywords.insert("进士".to_string(), 2.0);
        poetry_keywords.insert("及第".to_string(), 2.0);
        poetry_keywords.insert("游历".to_string(), 2.0);
        poetry_keywords.insert("隐居".to_string(), 1.5);
        poetry_keywords.insert("官职".to_string(), 1.5);
        poetry_keywords.insert("诗派".to_string(), 2.0);
        poetry_keywords.insert("风格".to_string(), 1.5);
        poetry_keywords.insert("代表作".to_string(), 2.0);
        poetry_keywords.insert("名句".to_string(), 1.5);
        poetry_keywords.insert("注释".to_string(), 1.5);
        poetry_keywords.insert("赏析".to_string(), 1.5);
        poetry_keywords.insert("出处".to_string(), 1.5);
        poetry_keywords.insert("句意".to_string(), 1.5);

        Self {
            resume_keywords,
            jd_keywords,
            poetry_keywords,
        }
    }

    /// 分类文档
    pub fn classify(&self, text: &str, filename: &str) -> ClassificationResult {
        // 转换为小写以便匹配
        let text_lower = text.to_lowercase();
        let filename_lower = filename.to_lowercase();

        // 计算简历、JD和诗人档案的得分
        let (resume_score, resume_features) =
            self.calculate_score(&text_lower, &self.resume_keywords);
        let (jd_score, jd_features) = self.calculate_score(&text_lower, &self.jd_keywords);
        let (poetry_score, poetry_features) =
            self.calculate_score(&text_lower, &self.poetry_keywords);

        // 文件名加权
        let filename_resume_bonus =
            self.check_filename_keywords(&filename_lower, &["简历", "resume", "cv", "curriculum"]);
        let filename_jd_bonus = self.check_filename_keywords(
            &filename_lower,
            &["岗位", "职位", "jd", "job", "position", "招聘", "说明书"],
        );
        let filename_poetry_bonus = self.check_filename_keywords(
            &filename_lower,
            &["诗人", "诗词", "唐诗", "宋词", "全唐诗", "全宋词"],
        );

        let final_resume_score = resume_score + filename_resume_bonus * 2.0;
        let final_jd_score = jd_score + filename_jd_bonus * 2.0;
        let final_poetry_score = poetry_score + filename_poetry_bonus * 2.0;

        tracing::info!("📊 文档分类得分 - 文件: {}", filename);
        tracing::info!(
            "   简历得分: {:.2} (内容: {:.2} + 文件名: {:.2})",
            final_resume_score,
            resume_score,
            filename_resume_bonus * 2.0
        );
        tracing::info!(
            "   JD得分: {:.2} (内容: {:.2} + 文件名: {:.2})",
            final_jd_score,
            jd_score,
            filename_jd_bonus * 2.0
        );
        tracing::info!(
            "   诗人档案得分: {:.2} (内容: {:.2} + 文件名: {:.2})",
            final_poetry_score,
            poetry_score,
            filename_poetry_bonus * 2.0
        );

        // 决策逻辑
        let total_score = final_resume_score + final_jd_score + final_poetry_score;

        if total_score < 1.0 {
            // 得分太低，无法判断
            return ClassificationResult {
                doc_type: DocumentType::Other,
                confidence: 0.3,
                matched_features: vec!["未检测到明显特征".to_string()],
            };
        }

        // 判断类型（优先匹配得分最高的）
        let max_score = final_resume_score
            .max(final_jd_score)
            .max(final_poetry_score);

        if max_score < 1.0 {
            // 所有得分都太低
            return ClassificationResult {
                doc_type: DocumentType::Other,
                confidence: 0.3,
                matched_features: vec!["未检测到明显特征".to_string()],
            };
        }

        if final_poetry_score == max_score {
            let confidence = final_poetry_score / total_score.max(1.0);
            ClassificationResult {
                doc_type: DocumentType::PoetryProfile,
                confidence: confidence.min(1.0),
                matched_features: poetry_features,
            }
        } else if final_resume_score == max_score {
            let confidence = final_resume_score / total_score.max(1.0);
            ClassificationResult {
                doc_type: DocumentType::Resume,
                confidence: confidence.min(1.0),
                matched_features: resume_features,
            }
        } else if final_jd_score == max_score {
            let confidence = final_jd_score / total_score.max(1.0);
            ClassificationResult {
                doc_type: DocumentType::JobDescription,
                confidence: confidence.min(1.0),
                matched_features: jd_features,
            }
        } else {
            // 得分相同，看文件名
            if filename_poetry_bonus > filename_resume_bonus.max(filename_jd_bonus) {
                ClassificationResult {
                    doc_type: DocumentType::PoetryProfile,
                    confidence: 0.6,
                    matched_features: vec!["基于文件名判断".to_string()],
                }
            } else if filename_jd_bonus > filename_resume_bonus {
                ClassificationResult {
                    doc_type: DocumentType::JobDescription,
                    confidence: 0.6,
                    matched_features: vec!["基于文件名判断".to_string()],
                }
            } else {
                ClassificationResult {
                    doc_type: DocumentType::Resume,
                    confidence: 0.6,
                    matched_features: vec!["基于文件名判断".to_string()],
                }
            }
        }
    }

    /// 计算关键词得分
    fn calculate_score(&self, text: &str, keywords: &HashMap<String, f64>) -> (f64, Vec<String>) {
        let mut total_score = 0.0;
        let mut matched_features = Vec::new();

        for (keyword, weight) in keywords.iter() {
            // 计算关键词出现次数
            let count = text.matches(keyword.as_str()).count() as f64;
            if count > 0.0 {
                // 得分 = 权重 * sqrt(出现次数)，避免单个关键词过度影响
                let score = weight * count.sqrt();
                total_score += score;
                matched_features.push(format!("{} ({}次, {:.1}分)", keyword, count, score));
            }
        }

        // 只保留得分最高的前5个特征
        matched_features.sort_by(|a, b| {
            let score_a = a
                .split("分)")
                .next()
                .and_then(|s| s.rsplit(',').next())
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(0.0);
            let score_b = b
                .split("分)")
                .next()
                .and_then(|s| s.rsplit(',').next())
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(0.0);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matched_features.truncate(5);

        (total_score, matched_features)
    }

    /// 检查文件名关键词
    fn check_filename_keywords(&self, filename: &str, keywords: &[&str]) -> f64 {
        let mut score = 0.0;
        for keyword in keywords {
            if filename.contains(keyword) {
                score += 1.0;
            }
        }
        score
    }

    /// 快速分类（仅基于文件名，用于预判）
    #[allow(dead_code)]
    pub fn quick_classify(&self, filename: &str) -> DocumentType {
        let filename_lower = filename.to_lowercase();

        // JD关键词
        if filename_lower.contains("岗位")
            || filename_lower.contains("职位")
            || filename_lower.contains("jd")
            || filename_lower.contains("job description")
            || filename_lower.contains("招聘")
            || filename_lower.contains("说明书")
        {
            return DocumentType::JobDescription;
        }

        // 简历关键词
        if filename_lower.contains("简历")
            || filename_lower.contains("resume")
            || filename_lower.contains("cv")
        {
            return DocumentType::Resume;
        }

        // 默认当做简历处理（向后兼容）
        DocumentType::Resume
    }
}

impl Default for DocumentClassifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_resume() {
        let classifier = DocumentClassifier::new();
        let text = r#"
            个人简历
            姓名：张三
            教育背景：北京大学
            工作经历：阿里巴巴工程师
            专业技能：Rust, Python
        "#;

        let result = classifier.classify(text, "张三的简历.pdf");
        assert_eq!(result.doc_type, DocumentType::Resume);
        assert!(result.confidence > 0.7);
    }

    #[test]
    fn test_classify_jd() {
        let classifier = DocumentClassifier::new();
        let text = r#"
            岗位说明书
            招聘岗位：Rust工程师
            岗位职责：负责后端开发
            任职要求：3年以上工作经验
            招聘人数：2人
        "#;

        let result = classifier.classify(text, "Rust工程师岗位说明.pdf");
        assert_eq!(result.doc_type, DocumentType::JobDescription);
        assert!(result.confidence > 0.7);
    }

    #[test]
    fn test_quick_classify() {
        let classifier = DocumentClassifier::new();

        assert_eq!(
            classifier.quick_classify("张三简历.pdf"),
            DocumentType::Resume
        );
        assert_eq!(
            classifier.quick_classify("工程师岗位说明书.pdf"),
            DocumentType::JobDescription
        );
        assert_eq!(
            classifier.quick_classify("resume_zhangsan.pdf"),
            DocumentType::Resume
        );
        assert_eq!(
            classifier.quick_classify("JD_engineer.pdf"),
            DocumentType::JobDescription
        );
    }

    #[test]
    fn test_classify_buildingbank_jd() {
        let classifier = DocumentClassifier::new();
        let text = r#"
            中国建设银行总行2025年度专业人才社会招聘岗位说明书
            
            序号 招聘岗位 招聘机构 岗位职责 任职要求 工作地点 招聘人数
            1 模型验证评估专家 总行风险管理部、风险计量中心
            岗位职责：
            1. 负责牵头线上业务风控模型的验证和评估
            2. 负责制定和推动线上业务风险计量模型
            任职要求：
            1. 具有计量经济学、金融工程、数学、计算机等专业硕士及以上学历
            2. 具有8年以上全球系统重要性银行工作经验
        "#;

        let result = classifier.classify(text, "建设银行岗位说明书.pdf");
        println!("分类结果: {:?}", result);
        assert_eq!(result.doc_type, DocumentType::JobDescription);
        assert!(
            result.confidence > 0.75,
            "置信度太低: {}",
            result.confidence
        );
    }
}
