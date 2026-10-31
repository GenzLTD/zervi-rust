//! 库内职位匹配：先对 documents 里的 job 计分，再用 LLM 给理由。
//! 幻觉职位（对不上库内 job_id）丢弃，不作为功能。

use serde_json::{json, Value};

const TOP_K: usize = 8;

pub fn job_id(job: &Value) -> Option<String> {
    job.get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn job_title(job: &Value) -> String {
    job.get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "未命名职位".into())
}

pub fn job_blob(job: &Value) -> String {
    let jd = str_field(job, "jd");
    let desc = str_field(job, "description").or_else(|| str_field(job, "desc"));
    let req = str_field(job, "requirements");
    [
        str_field(job, "title").unwrap_or_default(),
        str_field(job, "company").unwrap_or_default(),
        jd.unwrap_or_default(),
        desc.unwrap_or_default(),
        req.unwrap_or_default(),
    ]
    .join("\n")
}

fn str_field(job: &Value, key: &str) -> Option<String> {
    job.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 对库内职位做词面计分，返回已按分数降序、带 job_id 的候选。
pub fn rank_library_jobs(resume_text: &str, jobs: &[Value]) -> Vec<Value> {
    let resume = resume_text.to_lowercase();
    let resume_tokens = tokens(&resume);
    let mut scored: Vec<(f64, Value)> = jobs
        .iter()
        .filter_map(|job| {
            let id = job_id(job)?;
            let title = job_title(job);
            let blob = job_blob(job).to_lowercase();
            let score = lexical_score(&resume, &resume_tokens, &title.to_lowercase(), &blob);
            Some((
                score,
                json!({
                    "job_id": id,
                    "title": title,
                    "company": str_field(job, "company").unwrap_or_default(),
                    "score": (score * 1000.0).round() / 1000.0,
                    "reasons": lexical_reasons(&resume, &title.to_lowercase()),
                    "gaps": [],
                }),
            ))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(TOP_K);
    scored.into_iter().map(|(_, row)| row).collect()
}

pub fn pin_to_catalog(catalog_ids: &[String], llm_matches: &[Value], fallback: &[Value]) -> Vec<Value> {
    let mut by_id: std::collections::BTreeMap<String, Value> = fallback
        .iter()
        .filter_map(|m| {
            let id = m.get("job_id").and_then(|v| v.as_str())?.to_string();
            Some((id, m.clone()))
        })
        .collect();
    for raw in llm_matches {
        let id = raw
            .get("job_id")
            .or_else(|| raw.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let Some(id) = id else { continue };
        if !catalog_ids.iter().any(|c| c == &id) {
            continue;
        }
        let mut row = by_id.remove(&id).unwrap_or_else(|| {
            json!({
                "job_id": id,
                "title": raw.get("title").cloned().unwrap_or(json!("未命名职位")),
                "score": 0.0,
                "reasons": [],
                "gaps": [],
            })
        });
        if let Some(title) = raw.get("title").and_then(|v| v.as_str()) {
            if !title.trim().is_empty() {
                row["title"] = json!(title.trim());
            }
        }
        if let Some(score) = as_f64(raw.get("score")).or_else(|| as_f64(raw.get("match_score"))) {
            row["score"] = json!(score);
        }
        if let Some(reasons) = string_list(raw.get("reasons")) {
            if !reasons.is_empty() {
                row["reasons"] = json!(reasons);
            }
        }
        if let Some(gaps) = string_list(raw.get("gaps")) {
            row["gaps"] = json!(gaps);
        }
        by_id.insert(id, row);
    }
    let mut out: Vec<Value> = by_id.into_values().collect();
    out.sort_by(|a, b| {
        let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(TOP_K);
    out
}

pub fn catalog_prompt(resume_text: &str, ranked: &[Value], jobs: &[Value]) -> String {
    let catalog: Vec<Value> = ranked
        .iter()
        .filter_map(|m| {
            let id = m.get("job_id").and_then(|v| v.as_str())?;
            let job = jobs.iter().find(|j| job_id(j).as_deref() == Some(id))?;
            Some(json!({
                "job_id": id,
                "title": job_title(job),
                "company": str_field(job, "company").unwrap_or_default(),
                "blob": job_blob(job).chars().take(800).collect::<String>(),
            }))
        })
        .collect();
    format!(
        "你是职位匹配引擎。只能从下面的职位目录里挑，job_id 必须原样使用，禁止编造目录外的公司或职位。\n\
         只输出一个 JSON 对象，不要解释，不要 markdown。结构：\n\
         {{\"matches\":[{{\"job_id\":\"\",\"title\":\"\",\"score\":0.0,\"reasons\":[\"\"],\"gaps\":[\"\"]}}]}}\n\
         score 为 0 到 1。reasons / gaps 服务于人决策，不要写已投递。\n\n\
         职位目录：\n{}\n\n简历：\n{}",
        Value::Array(catalog),
        resume_text.chars().take(4000).collect::<String>()
    )
}

fn lexical_score(resume: &str, resume_tokens: &[String], title: &str, blob: &str) -> f64 {
    let mut score: f64 = 0.0;
    if !title.is_empty() && resume.contains(title) {
        score += 0.45;
    }
    let job_tokens = tokens(blob);
    if job_tokens.is_empty() {
        return score.min(1.0);
    }
    let hits = job_tokens
        .iter()
        .filter(|t| resume_tokens.iter().any(|r| r == *t) || resume.contains(t.as_str()))
        .count();
    score += 0.55 * (hits as f64 / job_tokens.len() as f64);
    score.min(1.0)
}

fn lexical_reasons(resume: &str, title: &str) -> Vec<String> {
    if !title.is_empty() && resume.contains(title) {
        vec![format!("简历中出现职位「{title}」")]
    } else {
        vec!["按库内职位词面重合排序，供人工点开详情".into()]
    }
}

fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut ascii = String::new();
    let mut cjk = String::new();
    let flush_ascii = |buf: &mut String, out: &mut Vec<String>| {
        if buf.len() > 2 {
            out.push(std::mem::take(buf));
        } else {
            buf.clear();
        }
    };
    let flush_cjk = |buf: &mut String, out: &mut Vec<String>| {
        if buf.chars().count() >= 2 {
            let chars: Vec<char> = buf.chars().collect();
            for w in chars.windows(2) {
                out.push(w.iter().collect());
            }
        }
        buf.clear();
    };
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk, &mut out);
            ascii.push(ch.to_ascii_lowercase());
        } else if is_cjk(ch) {
            flush_ascii(&mut ascii, &mut out);
            cjk.push(ch);
        } else {
            flush_ascii(&mut ascii, &mut out);
            flush_cjk(&mut cjk, &mut out);
        }
    }
    flush_ascii(&mut ascii, &mut out);
    flush_cjk(&mut cjk, &mut out);
    out
}

fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
    )
}

fn as_f64(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    v.as_f64()
        .or_else(|| v.as_i64().map(|n| n as f64))
        .or_else(|| v.as_u64().map(|n| n as f64))
}

fn string_list(v: Option<&Value>) -> Option<Vec<String>> {
    let arr = v?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_library_job_and_requires_id() {
        let jobs = vec![
            json!({"id": "demo-1", "title": "高级 Rust 后端工程师", "desc": "支付网关 Rust 微服务"}),
            json!({"title": "没有 id 的幻觉职位", "company": "Phantom"}),
        ];
        let ranked = rank_library_jobs(
            "3年 Rust 后端，负责支付网关核心链路。期望高级 Rust 后端工程师。",
            &jobs,
        );
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0]["job_id"], "demo-1");
        assert!(ranked[0]["score"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn drops_hallucinated_llm_jobs() {
        let catalog = vec!["demo-1".to_string()];
        let fallback = vec![json!({
            "job_id": "demo-1",
            "title": "高级 Rust 后端工程师",
            "score": 0.4,
            "reasons": ["词面"],
            "gaps": []
        })];
        let llm = vec![
            json!({"job_id": "demo-1", "reasons": ["库内职位"], "gaps": ["补量化"]}),
            json!({"job_id": "phantom-99", "title": "编造的公司", "score": 0.99}),
        ];
        let pinned = pin_to_catalog(&catalog, &llm, &fallback);
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0]["job_id"], "demo-1");
        assert_eq!(pinned[0]["reasons"][0], "库内职位");
        assert_eq!(pinned[0]["gaps"][0], "补量化");
    }

    #[test]
    fn empty_catalog_is_empty_matches() {
        let ranked = rank_library_jobs("足够长的简历文本用于匹配计算。", &[]);
        assert!(ranked.is_empty());
    }
}
