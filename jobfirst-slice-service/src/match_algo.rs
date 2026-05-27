use crate::models::{MatchItem, SeedJob};

fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

pub fn score_resume_against_job(resume_text: &str, job: &SeedJob) -> (f32, String) {
    let resume_tokens = tokenize(resume_text);
    let mut hits = 0u32;
    let mut matched: Vec<&str> = Vec::new();

    for kw in &job.keywords {
        let k = kw.to_lowercase();
        if resume_text.to_lowercase().contains(&k) {
            hits += 1;
            matched.push(kw.as_str());
            continue;
        }
        for t in &resume_tokens {
            if t.contains(&k) || k.contains(t.as_str()) {
                hits += 1;
                matched.push(kw.as_str());
                break;
            }
        }
    }

    let denom = job.keywords.len().max(1) as f32;
    let score = (hits as f32 / denom).min(1.0);
    let preview = if matched.is_empty() {
        "简历与 JD 关键词重叠较少".to_string()
    } else {
        format!("关键词命中：{}", matched.join("、"))
    };
    (score, preview)
}

pub fn rank_jobs(resume_text: &str, jobs: &[SeedJob], top_n: usize) -> Vec<MatchItem> {
    let mut scored: Vec<MatchItem> = jobs
        .iter()
        .map(|job| {
            let (score, reason_preview) = score_resume_against_job(resume_text, job);
            MatchItem {
                job_id: job.id.clone(),
                title: job.title.clone(),
                company_name: job.company_name.clone(),
                score,
                reason_preview,
            }
        })
        .collect();

    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_n);
    scored
}
