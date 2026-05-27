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
