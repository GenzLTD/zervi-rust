use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedJob {
    pub id: String,
    pub title: String,
    pub company_name: String,
    pub company_credit_code: Option<String>,
    pub jd_text: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResumeRecord {
    pub resume_id: String,
    pub text: String,
    pub created_at: String,
    /// 终端身份：zervi-auth `sub` 原样（GFCC 1.3.0 `actor_sub`）。
    /// `None` 只应出现在显式放行匿名的开发模式；终端的正常路径必有值。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_sub: Option<String>,
}

/// job_match 回写到 identity 的辅助痕迹。人不能自封 evidence。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CapabilityTrace {
    pub label: String,
    pub source_port: String,
    pub weight: String,
    pub observed_at: String,
    pub actor_sub: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchItem {
    pub job_id: String,
    pub title: String,
    pub company_name: String,
    pub score: f32,
    pub reason_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationStatus {
    Submitted,
    Replied,
    Invited,
    Rejected,
    Interview,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplicationRecord {
    pub application_id: String,
    pub resume_id: String,
    pub job_id: String,
    pub title: String,
    pub company_name: String,
    pub status: ApplicationStatus,
    pub updated_at: String,
    pub user_decision: Option<String>,
    /// 归属身份，随简历继承。用于拒绝跨 actor 的收件箱读取与决策。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_sub: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PostResumeBody {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct PostApplyBody {
    pub resume_id: String,
    pub job_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct DecisionBody {
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub struct MockFeedbackBody {
    pub application_id: String,
    pub status: String,
}
