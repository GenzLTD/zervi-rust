use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// 岗位信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDescription {
    /// 岗位名称
    pub title: Option<String>,
    /// 岗位职责列表
    pub responsibilities: Vec<String>,
    /// 任职要求列表
    pub requirements: Vec<String>,
    /// 工作地点
    pub location: Option<String>,
    /// 招聘人数
    pub headcount: Option<String>,
    /// 薪资范围
    pub salary_range: Option<String>,
    /// 学历要求
    pub education: Option<String>,
    /// 工作经验要求
    pub experience: Option<String>,
    /// 技能要求
    pub skills: Vec<String>,
    /// 所属部门/机构
    pub department: Option<String>,
    /// 原始文本（用于调试）
    pub raw_text: Option<String>,
}

lazy_static! {
    // 岗位名称模式
    static ref TITLE_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"(?:招聘)?岗位[名称]?[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"职位[名称]?[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"岗位说明[：:]?\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"(?i)position[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"(?i)job title[：:]\s*(.+?)(?:\n|$)").unwrap(),
    ];

    // 工作地点模式
    static ref LOCATION_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"工作地点[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"工作城市[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"地点[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"(?i)location[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"(?i)work place[：:]\s*(.+?)(?:\n|$)").unwrap(),
    ];

    // 招聘人数模式
    static ref HEADCOUNT_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"招聘人数[：:]\s*(.+?)(?:\n|人|$)").unwrap(),
        Regex::new(r"需求人数[：:]\s*(.+?)(?:\n|人|$)").unwrap(),
        Regex::new(r"人数[：:]\s*(.+?)(?:\n|人|$)").unwrap(),
        Regex::new(r"(?i)headcount[：:]\s*(.+?)(?:\n|$)").unwrap(),
    ];

    // 薪资模式
    static ref SALARY_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"薪资[范围待遇]?[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"薪酬[范围待遇]?[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"年薪[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"(\d+[kK万]\s*[-~至]\s*\d+[kK万])").unwrap(),
        Regex::new(r"(?i)salary[：:]\s*(.+?)(?:\n|$)").unwrap(),
    ];

    // 学历要求模式
    static ref EDUCATION_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"学历[要求]?[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"(本科|硕士|博士|大专)及?以上").unwrap(),
        Regex::new(r"(?i)(bachelor|master|phd|doctorate)").unwrap(),
    ];

    // 工作经验模式
    static ref EXPERIENCE_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"工作经[验历][：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"(\d+年)以?上.*?经验").unwrap(),
        Regex::new(r"(?i)(\d+\+?\s*years?).*?experience").unwrap(),
    ];

    // 部门模式
    static ref DEPARTMENT_PATTERNS: Vec<Regex> = vec![
        Regex::new(r"招聘机构[：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"所属[部门机构][：:]\s*(.+?)(?:\n|$)").unwrap(),
        Regex::new(r"(?i)department[：:]\s*(.+?)(?:\n|$)").unwrap(),
    ];
}

/// JD解析器
pub struct JDParser {
    /// 是否启用调试模式
    debug: bool,
}

impl JDParser {
    pub fn new() -> Self {
        Self { debug: false }
    }

    #[allow(dead_code)]
    pub fn with_debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }

    /// 解析JD文档
    pub fn parse(&self, text: &str) -> Result<JobDescription, anyhow::Error> {
        tracing::info!("📋 开始解析JD文档，文本长度: {}", text.len());

        let mut jd = JobDescription {
            title: None,
            responsibilities: Vec::new(),
            requirements: Vec::new(),
            location: None,
            headcount: None,
            salary_range: None,
            education: None,
            experience: None,
            skills: Vec::new(),
            department: None,
            raw_text: if self.debug {
                Some(text.to_string())
            } else {
                None
            },
        };

        // 1. 提取岗位名称
        jd.title = self
            .extract_with_patterns(text, &TITLE_PATTERNS)
            .map(|s| s.trim().to_string());

        // 2. 提取工作地点
        jd.location = self
            .extract_with_patterns(text, &LOCATION_PATTERNS)
            .map(|s| s.trim().to_string());

        // 3. 提取招聘人数
        jd.headcount = self
            .extract_with_patterns(text, &HEADCOUNT_PATTERNS)
            .map(|s| s.trim().to_string());

        // 4. 提取薪资
        jd.salary_range = self
            .extract_with_patterns(text, &SALARY_PATTERNS)
            .map(|s| s.trim().to_string());

        // 5. 提取学历
        jd.education = self
            .extract_with_patterns(text, &EDUCATION_PATTERNS)
            .map(|s| s.trim().to_string());

        // 6. 提取工作经验
        jd.experience = self
            .extract_with_patterns(text, &EXPERIENCE_PATTERNS)
            .map(|s| s.trim().to_string());

        // 7. 提取部门
        jd.department = self
            .extract_with_patterns(text, &DEPARTMENT_PATTERNS)
            .map(|s| s.trim().to_string());

        // 8. 提取岗位职责（分段提取）
        jd.responsibilities = self.extract_section_items(
            text,
            &[
                "岗位职责",
                "职位职责",
                "工作职责",
                "responsibilities",
                "duties",
            ],
            &[
                "任职要求",
                "任职资格",
                "岗位要求",
                "requirements",
                "qualifications",
            ],
        );

        // 9. 提取任职要求（分段提取）
        jd.requirements = self.extract_section_items(
            text,
            &[
                "任职要求",
                "任职资格",
                "岗位要求",
                "职位要求",
                "requirements",
                "qualifications",
            ],
            &["工作地点", "薪资", "福利", "location", "salary", "benefits"],
        );

        // 10. 从任职要求中提取技能
        jd.skills = self.extract_skills(&jd.requirements.join("\n"));

        tracing::info!("✅ JD解析完成:");
        tracing::info!("   岗位: {:?}", jd.title);
        tracing::info!("   职责数: {}", jd.responsibilities.len());
        tracing::info!("   要求数: {}", jd.requirements.len());
        tracing::info!("   技能数: {}", jd.skills.len());

        Ok(jd)
    }

    /// 使用正则模式提取
    fn extract_with_patterns(&self, text: &str, patterns: &[Regex]) -> Option<String> {
        for pattern in patterns {
            if let Some(caps) = pattern.captures(text) {
                if let Some(matched) = caps.get(1) {
                    let result = matched.as_str().trim();
                    if !result.is_empty() && result.len() < 200 {
                        return Some(result.to_string());
                    }
                }
            }
        }
        None
    }

    /// 提取章节内容（列表项）
    fn extract_section_items(
        &self,
        text: &str,
        start_markers: &[&str],
        end_markers: &[&str],
    ) -> Vec<String> {
        let text_lower = text.to_lowercase();

        // 找到章节开始位置
        let mut start_pos = None;
        for marker in start_markers {
            if let Some(pos) = text_lower.find(marker) {
                start_pos = Some(pos);
                break;
            }
        }

        if start_pos.is_none() {
            return Vec::new();
        }

        let start_pos = start_pos.unwrap();

        // 找到章节结束位置
        let mut end_pos = text.len();
        for marker in end_markers {
            if let Some(pos) = text_lower[start_pos..].find(marker) {
                end_pos = start_pos + pos;
                break;
            }
        }

        // 提取章节文本
        let section_text = &text[start_pos..end_pos];

        // 提取列表项
        let mut items = Vec::new();

        // 匹配模式：数字开头、字母开头、符号开头
        let item_patterns = vec![
            Regex::new(r"(?m)^\s*(\d+[、.．)])\s*(.+?)$").unwrap(), // 1. xxx
            Regex::new(r"(?m)^\s*([•·・●○◎◇◆■□▪▫])\s*(.+?)$").unwrap(), // • xxx
            Regex::new(r"(?m)^\s*([①②③④⑤⑥⑦⑧⑨⑩])\s*(.+?)$").unwrap(), // ① xxx
            Regex::new(r"(?m)^\s*([a-zA-Z][、.)])\s*(.+?)$").unwrap(), // a. xxx
        ];

        for pattern in &item_patterns {
            for caps in pattern.captures_iter(section_text) {
                if let Some(content) = caps.get(2) {
                    let item = content.as_str().trim();
                    if !item.is_empty() && item.len() > 5 && item.len() < 500 {
                        items.push(item.to_string());
                    }
                }
            }
        }

        // 如果没有找到列表项，尝试按句号分割
        if items.is_empty() {
            for line in section_text.lines().skip(1) {
                // 跳过标题行
                let line = line.trim();
                if !line.is_empty() && line.len() > 10 && line.len() < 500 {
                    items.push(line.to_string());
                }
            }
        }

        items
    }

    /// 提取技能关键词
    fn extract_skills(&self, text: &str) -> Vec<String> {
        let mut skills = Vec::new();

        // 常见技能关键词
        let skill_keywords = vec![
            // 编程语言
            "java",
            "python",
            "rust",
            "go",
            "golang",
            "c++",
            "javascript",
            "typescript",
            "scala",
            "kotlin",
            "swift",
            "ruby",
            "php",
            // 技术框架
            "spring",
            "django",
            "flask",
            "react",
            "vue",
            "angular",
            "node.js",
            // 数据库
            "mysql",
            "postgresql",
            "oracle",
            "mongodb",
            "redis",
            "elasticsearch",
            // 云计算
            "aws",
            "azure",
            "gcp",
            "docker",
            "kubernetes",
            "k8s",
            // 其他
            "linux",
            "git",
            "jenkins",
            "ci/cd",
            "微服务",
            "机器学习",
            "深度学习",
            "nlp",
            "大数据",
            "hadoop",
            "spark",
            "kafka",
        ];

        let text_lower = text.to_lowercase();
        for keyword in skill_keywords {
            if text_lower.contains(keyword) && !skills.contains(&keyword.to_string()) {
                skills.push(keyword.to_string());
            }
        }

        skills
    }
}

impl Default for JDParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_jd() {
        let text = r#"
            招聘岗位：Rust高级工程师
            工作地点：北京
            招聘人数：2人
            薪资待遇：30K-50K
            
            岗位职责：
            1. 负责后端服务开发
            2. 负责系统架构设计
            3. 负责性能优化
            
            任职要求：
            1. 计算机相关专业本科及以上学历
            2. 5年以上工作经验
            3. 精通Rust语言
            4. 熟悉微服务架构
            5. 熟悉MySQL、Redis等数据库
        "#;

        let parser = JDParser::new();
        let result = parser.parse(text).unwrap();

        assert_eq!(result.title.as_deref(), Some("Rust高级工程师"));
        assert_eq!(result.location.as_deref(), Some("北京"));
        assert_eq!(result.headcount.as_deref(), Some("2"));
        assert!(result.responsibilities.len() >= 3);
        assert!(result.requirements.len() >= 3);
        assert!(result.skills.contains(&"rust".to_string()));
        assert!(result.skills.contains(&"mysql".to_string()));
    }

    #[test]
    fn test_parse_buildingbank_jd() {
        let text = r#"
            中国建设银行总行2025年度专业人才社会招聘岗位说明书
            
            序号：1
            招聘岗位：模型验证评估专家
            招聘机构：总行风险管理部、风险计量中心
            工作地点：北京、上海
            招聘人数：1
            
            岗位职责：
            1. 负责牵头线上业务风控模型的验证和评估；
            2. 负责制定和推动线上业务风险计量模型/策略的有关政策，阈值标准及模型规范；
            3. 负责建立线上业务准入额度等规则类模型分级分类审核机制；
            
            任职要求：
            1. 具有计量经济学、金融工程、数学、计算机等专业硕士及以上学历；
            2. 具有8年以上全球系统重要性银行、知名银行或互联网金融机构从事风险计量和模型管理经验；
            3. 熟悉线上业务风控规则策略监测指标和阈值体系；
        "#;

        let parser = JDParser::new();
        let result = parser.parse(text).unwrap();

        println!("解析结果: {:#?}", result);

        assert!(result.title.is_some());
        assert!(result.responsibilities.len() >= 2);
        assert!(result.requirements.len() >= 2);
        assert_eq!(result.location.as_deref(), Some("北京、上海"));
    }
}
