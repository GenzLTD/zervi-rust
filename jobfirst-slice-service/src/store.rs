use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::models::{
    ApplicationRecord, ApplicationStatus, ResumeRecord, SeedJob,
};

#[derive(Default)]
pub struct StoreData {
    pub jobs: Vec<SeedJob>,
    pub resumes: HashMap<String, ResumeRecord>,
    pub applications: HashMap<String, ApplicationRecord>,
}

#[derive(Clone)]
pub struct Store {
    inner: Arc<RwLock<StoreData>>,
}

impl Store {
    pub async fn new() -> Result<Self> {
        let jobs = load_seed_jobs()?;
        Ok(Self {
            inner: Arc::new(RwLock::new(StoreData {
                jobs,
                resumes: HashMap::new(),
                applications: HashMap::new(),
            })),
        })
    }

    pub async fn add_resume(&self, text: String, actor_sub: Option<String>) -> ResumeRecord {
        let id = format!("resume-{}", Uuid::new_v4());
        let record = ResumeRecord {
            resume_id: id.clone(),
            text,
            created_at: chrono::Utc::now().to_rfc3339(),
            actor_sub,
        };
        let mut g = self.inner.write().await;
        g.resumes.insert(id.clone(), record.clone());
        record
    }

    pub async fn get_resume(&self, resume_id: &str) -> Option<ResumeRecord> {
        self.inner.read().await.resumes.get(resume_id).cloned()
    }

    pub async fn jobs(&self) -> Vec<SeedJob> {
        self.inner.read().await.jobs.clone()
    }

    pub async fn create_applications(
        &self,
        resume_id: &str,
        job_ids: &[String],
        actor_sub: Option<String>,
    ) -> Result<Vec<ApplicationRecord>> {
        let mut g = self.inner.write().await;
        let mut out = Vec::new();
        for job_id in job_ids {
            let job = g
                .jobs
                .iter()
                .find(|j| j.id == *job_id)
                .with_context(|| format!("unknown job_id: {}", job_id))?;

            let app_id = format!("app-{}", Uuid::new_v4());
            let record = ApplicationRecord {
                application_id: app_id.clone(),
                resume_id: resume_id.to_string(),
                job_id: job.id.clone(),
                title: job.title.clone(),
                company_name: job.company_name.clone(),
                status: ApplicationStatus::Submitted,
                updated_at: chrono::Utc::now().to_rfc3339(),
                user_decision: None,
                actor_sub: actor_sub.clone(),
            };
            g.applications.insert(app_id, record.clone());
            out.push(record);
        }
        Ok(out)
    }

    pub async fn get_application(&self, application_id: &str) -> Option<ApplicationRecord> {
        self.inner
            .read()
            .await
            .applications
            .get(application_id)
            .cloned()
    }

    pub async fn inbox_for_resume(&self, resume_id: &str) -> Vec<ApplicationRecord> {
        let g = self.inner.read().await;
        let mut items: Vec<_> = g
            .applications
            .values()
            .filter(|a| a.resume_id == resume_id)
            .cloned()
            .collect();
        items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        items
    }

    pub async fn set_feedback(
        &self,
        application_id: &str,
        status: ApplicationStatus,
    ) -> Result<ApplicationRecord> {
        let mut g = self.inner.write().await;
        let app = g
            .applications
            .get_mut(application_id)
            .with_context(|| format!("unknown application_id: {}", application_id))?;
        app.status = status;
        app.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(app.clone())
    }

    pub async fn set_decision(
        &self,
        application_id: &str,
        action: &str,
    ) -> Result<ApplicationRecord> {
        let mut g = self.inner.write().await;
        let app = g
            .applications
            .get_mut(application_id)
            .with_context(|| format!("unknown application_id: {}", application_id))?;
        let (status, decision) = match action {
            "interview" => (ApplicationStatus::Interview, "interview"),
            "skip" => (ApplicationStatus::Skipped, "skip"),
            _ => anyhow::bail!("action must be interview or skip"),
        };
        app.status = status;
        app.user_decision = Some(decision.to_string());
        app.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(app.clone())
    }
}

fn load_seed_jobs() -> Result<Vec<SeedJob>> {
    let path = seed_jobs_path();
    let raw = fs::read_to_string(&path).with_context(|| format!("read seed jobs: {}", path.display()))?;
    let jobs: Vec<SeedJob> = serde_json::from_str(&raw)?;
    Ok(jobs)
}

fn seed_jobs_path() -> PathBuf {
    if let Ok(p) = std::env::var("JOBFIRST_SEED_JOBS") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/seed_jobs.json")
}
