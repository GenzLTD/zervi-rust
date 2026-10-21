//! X1：海外区从 LLM 候选里去掉 DeepSeek。滤完为空则停，不退回境内 provider。
//! 口径对齐 Vultr-Genzltd `config.py` 的 `is_overseas_deploy` / `safe_llm_provider_order`。

use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Settings {
    pub database_url: String,
    pub deploy_region: String,
    pub jwt_secret: String,
    pub supabase_jwt_secret: String,
    pub supabase_jwks_url: String,
    pub llm_provider_order: String,
    pub deepseek_api_key: String,
    pub deepseek_base_url: String,
    pub deepseek_model: String,
    pub openai_api_key: String,
    pub openai_base_url: String,
    pub openai_model: String,
    pub ollama_host: String,
    pub ollama_model: String,
    pub qcc_mcp_base_url: String,
}

impl Settings {
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/genzltd".into()),
            // 与 SG `is_overseas_deploy` 一致：未设置视为 US，即海外。
            deploy_region: std::env::var("DEPLOY_REGION").unwrap_or_else(|_| "US".into()),
            jwt_secret: std::env::var("JWT_SECRET").unwrap_or_default(),
            supabase_jwt_secret: std::env::var("SUPABASE_JWT_SECRET").unwrap_or_default(),
            supabase_jwks_url: std::env::var("SUPABASE_JWKS_URL").unwrap_or_default(),
            llm_provider_order: std::env::var("LLM_PROVIDER_ORDER")
                .unwrap_or_else(|_| "deepseek,ollama".into()),
            deepseek_api_key: std::env::var("DEEPSEEK_API_KEY").unwrap_or_default(),
            deepseek_base_url: std::env::var("DEEPSEEK_BASE_URL")
                .unwrap_or_else(|_| "https://api.deepseek.com/v1".into()),
            deepseek_model: std::env::var("DEEPSEEK_MODEL")
                .unwrap_or_else(|_| "deepseek-chat".into()),
            openai_api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            openai_base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            openai_model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-3.5-turbo".into()),
            ollama_host: std::env::var("OLLAMA_HOST")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".into()),
            ollama_model: std::env::var("OLLAMA_MODEL")
                .unwrap_or_else(|_| "qwen2.5-coder:1.5b".into()),
            qcc_mcp_base_url: std::env::var("QCC_MCP_BASE_URL")
                .unwrap_or_else(|_| "https://agent.qcc.com/mcp".into()),
        }
    }

    pub fn overseas(&self) -> bool {
        is_overseas_deploy(&self.deploy_region)
    }

    pub fn provider_order(&self) -> Vec<String> {
        safe_llm_provider_order(Some(&self.llm_provider_order), self.overseas())
    }
}

pub fn is_overseas_deploy(region: &str) -> bool {
    !matches!(
        region.trim().to_uppercase().as_str(),
        "CN" | "CHINA" | "CHN"
    )
}

/// 空顺序先落到 `deepseek`，再按区域过滤。海外区过滤后允许为空。
pub fn safe_llm_provider_order(order: Option<&str>, overseas: bool) -> Vec<String> {
    let mut providers: Vec<String> = order
        .unwrap_or("")
        .split(',')
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    if providers.is_empty() {
        providers.push("deepseek".into());
    }
    if overseas {
        providers.retain(|p| p != "deepseek");
    }
    providers
}

/// 海外区拒绝明文 HTTP 的企查查端点。HTTPS 或境内部署返回 None。
/// 口径对齐 `qcc_client.qcc_disabled_reason`：只看端点是不是明文 HTTP。
pub fn qcc_disabled_reason(base: &str, overseas: bool) -> Option<String> {
    let base = base.trim().trim_end_matches('/');
    if base.to_lowercase().starts_with("https://") {
        return None;
    }
    if overseas {
        return Some(format!(
            "overseas deploy refuses a plaintext HTTP QCC endpoint ({base}); personal data must not transit an unencrypted link"
        ));
    }
    None
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<String>,
    reasoning_content: Option<String>,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: Option<String>,
}

fn key_usable(key: &str) -> bool {
    let k = key.trim();
    !k.is_empty() && !matches!(k.to_lowercase().as_str(), "skip" | "your-deepseek-api-key" | "your-openai-api-key")
}

/// 按 X1 过滤后的顺序调用。没有任何可用 provider 时返回 None（不回退 DeepSeek）。
pub async fn call_llm(settings: &Settings, prompt: &str, temperature: f32) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .ok()?;
    for provider in settings.provider_order() {
        let result = match provider.as_str() {
            "deepseek" if key_usable(&settings.deepseek_api_key) => {
                post_chat(
                    &client,
                    &format!(
                        "{}/chat/completions",
                        settings.deepseek_base_url.trim_end_matches('/')
                    ),
                    &settings.deepseek_api_key,
                    &settings.deepseek_model,
                    prompt,
                    temperature,
                )
                .await
            }
            "openai" if key_usable(&settings.openai_api_key) => {
                post_chat(
                    &client,
                    &format!(
                        "{}/chat/completions",
                        settings.openai_base_url.trim_end_matches('/')
                    ),
                    &settings.openai_api_key,
                    &settings.openai_model,
                    prompt,
                    temperature,
                )
                .await
            }
            "ollama" => post_ollama(&client, settings, prompt, temperature).await,
            _ => None,
        };
        if result.is_some() {
            return result;
        }
    }
    None
}

async fn post_chat(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    model: &str,
    prompt: &str,
    temperature: f32,
) -> Option<String> {
    let resp = client
        .post(url)
        .bearer_auth(key)
        .json(&json!({
            "model": model,
            "temperature": temperature,
            "messages": [{"role": "user", "content": prompt}],
        }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: ChatResponse = resp.json().await.ok()?;
    let message = body.choices.into_iter().next()?.message;
    let content = message.content.unwrap_or_default();
    if !content.is_empty() {
        return Some(content);
    }
    let reasoning = message.reasoning_content.unwrap_or_default();
    if reasoning.is_empty() {
        None
    } else {
        Some(reasoning)
    }
}

async fn post_ollama(
    client: &reqwest::Client,
    settings: &Settings,
    prompt: &str,
    temperature: f32,
) -> Option<String> {
    let model = if settings.ollama_model.trim().is_empty() {
        "qwen2.5-coder:1.5b"
    } else {
        settings.ollama_model.trim()
    };
    let url = format!(
        "{}/api/generate",
        settings.ollama_host.trim().trim_end_matches('/')
    );
    let resp = client
        .post(url)
        .json(&json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "options": { "temperature": temperature },
        }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: OllamaResponse = resp.json().await.ok()?;
    let text = body.response.unwrap_or_default();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overseas_strips_deepseek_and_does_not_fallback() {
        assert_eq!(
            safe_llm_provider_order(Some("deepseek,openai"), true),
            vec!["openai".to_string()]
        );
        assert!(safe_llm_provider_order(Some("deepseek"), true).is_empty());
        assert!(safe_llm_provider_order(Some(""), true).is_empty());
        assert_eq!(
            safe_llm_provider_order(Some("deepseek,openai"), false),
            vec!["deepseek".to_string(), "openai".to_string()]
        );
    }

    #[test]
    fn unset_region_is_overseas() {
        assert!(is_overseas_deploy("US"));
        assert!(is_overseas_deploy(""));
        assert!(!is_overseas_deploy("CN"));
        assert!(!is_overseas_deploy("china"));
    }

    #[test]
    fn qcc_plaintext_blocked_only_overseas() {
        assert!(qcc_disabled_reason("https://agent.qcc.com/mcp", true).is_none());
        assert!(qcc_disabled_reason("http://1.14.202.161:8998/mcp", false).is_none());
        let reason = qcc_disabled_reason("http://1.14.202.161:8998/mcp/", true)
            .expect("plaintext overseas");
        assert!(reason.contains("plaintext HTTP"));
        assert!(reason.contains("http://1.14.202.161:8998/mcp"));
    }
}
