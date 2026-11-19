use crate::models::{AppSettings, ProviderConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn default_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            id: "gemini".to_string(),
            name: "Gemini".to_string(),
            api_key: String::new(),
            base_url: None,
            validated: false,
            available_models: vec![
                "gemini-2.5-pro".to_string(),
                "gemini-2.5-flash".to_string(),
                "gemini-2.0-flash".to_string(),
            ],
            daily_token_limit: 500_000,
        },
        ProviderConfig {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            api_key: String::new(),
            base_url: None,
            validated: false,
            available_models: vec![
                "gpt-4.1".to_string(),
                "gpt-4.1-mini".to_string(),
                "o4-mini".to_string(),
                "gpt-4o".to_string(),
            ],
            daily_token_limit: 500_000,
        },
        ProviderConfig {
            id: "anthropic".to_string(),
            name: "Anthropic (Claude)".to_string(),
            api_key: String::new(),
            base_url: None,
            validated: false,
            available_models: vec![
                "claude-sonnet-4-20250514".to_string(),
                "claude-3-7-sonnet-20250219".to_string(),
                "claude-3-5-haiku-20241022".to_string(),
            ],
            daily_token_limit: 500_000,
        },
        ProviderConfig {
            id: "deepseek".to_string(),
            name: "DeepSeek".to_string(),
            api_key: String::new(),
            base_url: None,
            validated: false,
            available_models: vec![
                "deepseek-chat".to_string(),
                "deepseek-reasoner".to_string(),
            ],
            daily_token_limit: 500_000,
        },
        ProviderConfig {
            id: "openrouter".to_string(),
            name: "OpenRouter".to_string(),
            api_key: String::new(),
            base_url: None,
            validated: false,
            available_models: vec![
                "anthropic/claude-sonnet-4".to_string(),
                "google/gemini-2.5-flash".to_string(),
                "deepseek/deepseek-chat-v3-0324".to_string(),
                "qwen/qwen-2.5-coder-32b-instruct".to_string(),
            ],
            daily_token_limit: 500_000,
        },
        ProviderConfig {
            id: "mistral".to_string(),
            name: "Mistral".to_string(),
            api_key: String::new(),
            base_url: None,
            validated: false,
            available_models: vec![
                "codestral-latest".to_string(),
                "devstral-small-2505".to_string(),
                "mistral-large-latest".to_string(),
            ],
            daily_token_limit: 500_000,
        },
    ]
}

pub fn merge_with_default_providers(existing: &[ProviderConfig]) -> Vec<ProviderConfig> {
    let mut merged = default_providers();
    for slot in &mut merged {
        if let Some(saved) = existing.iter().find(|p| p.id == slot.id) {
            slot.api_key = saved.api_key.clone();
            slot.base_url = saved.base_url.clone();
            slot.validated = saved.validated;
            if !saved.available_models.is_empty() {
                slot.available_models = saved.available_models.clone();
            }
            if saved.daily_token_limit > 0 {
                slot.daily_token_limit = saved.daily_token_limit;
            }
        }
    }
    merged
}

pub fn effective_ollama_num_ctx(settings: &AppSettings) -> usize {
    let ctx = if settings.context_window_tokens == 0 {
        8192
    } else {
        settings.context_window_tokens
    };
    ctx.clamp(2048, 131072)
}

pub fn resolve_model_context_limit(provider: &str, model: &str, settings: &AppSettings) -> usize {
    match provider {
        "ollama" => effective_ollama_num_ctx(settings),
        "gemini" => 1_000_000,
        "anthropic" => 200_000,
        "deepseek" => 64_000,
        "mistral" => {
            if model.to_lowercase().contains("codestral") {
                256_000
            } else {
                128_000
            }
        }
        "openai" | "openrouter" | _ => 128_000,
    }
}

pub fn find_provider_config<'a>(
    settings: &'a AppSettings,
    provider_id: &str,
) -> Option<&'a ProviderConfig> {
    settings.providers.iter().find(|p| p.id == provider_id)
}

pub fn find_provider_key(settings: &AppSettings, provider_id: &str) -> Option<String> {
    find_provider_config(settings, provider_id).and_then(|p| {
        let trimmed = p.api_key.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn clean_model_name(provider: &str, model: &str) -> String {
    let trimmed = model.trim();
    if provider == "ollama" {
        return trimmed
            .strip_prefix("ollama_chat/")
            .or_else(|| trimmed.strip_prefix("ollama/"))
            .unwrap_or(trimmed)
            .to_string();
    }
    let prefix = format!("{provider}/");
    trimmed.strip_prefix(&prefix).unwrap_or(trimmed).to_string()
}

pub fn aider_model_and_env(
    provider: &str,
    model: &str,
    settings: &AppSettings,
) -> Result<(String, Vec<(String, String)>), String> {
    let clean = clean_model_name(provider, model);
    if clean.is_empty() {
        return Err(format!("Aucun modèle sélectionné pour '{provider}'."));
    }

    if provider == "ollama" {
        return Ok((
            format!("ollama_chat/{clean}"),
            vec![(
                "OLLAMA_API_BASE".to_string(),
                settings.ollama_url.trim_end_matches('/').to_string(),
            )],
        ));
    }

    let key = find_provider_key(settings, provider).ok_or_else(|| {
        format!("Clé API manquante pour le fournisseur '{provider}'. Configurez-la dans Paramètres → Fournisseurs.")
    })?;

    let mut env_vars = Vec::new();
    let aider_model = match provider {
        "gemini" => {
            env_vars.push(("GEMINI_API_KEY".to_string(), key));
            format!("gemini/{clean}")
        }
        "openai" => {
            env_vars.push(("OPENAI_API_KEY".to_string(), key));
            if let Some(cfg) = find_provider_config(settings, "openai") {
                if let Some(base) = &cfg.base_url {
                    let b = base.trim().trim_end_matches('/');
                    if !b.is_empty() {
                        env_vars.push(("OPENAI_API_BASE".to_string(), b.to_string()));
                    }
                }
            }
            format!("openai/{clean}")
        }
        "anthropic" => {
            env_vars.push(("ANTHROPIC_API_KEY".to_string(), key));
            format!("anthropic/{clean}")
        }
        "deepseek" => {
            env_vars.push(("DEEPSEEK_API_KEY".to_string(), key));
            format!("deepseek/{clean}")
        }
        "openrouter" => {
            env_vars.push(("OPENROUTER_API_KEY".to_string(), key));
            format!("openrouter/{clean}")
        }
        "mistral" => {
            env_vars.push(("MISTRAL_API_KEY".to_string(), key));
            format!("mistral/{clean}")
        }
        other => return Err(format!("Fournisseur inconnu : {other}")),
    };

    Ok((aider_model, env_vars))
}

fn default_models_for(provider_id: &str) -> Vec<String> {
    default_providers()
        .into_iter()
        .find(|p| p.id == provider_id)
        .map(|p| p.available_models)
        .unwrap_or_default()
}

pub async fn validate_provider_api_key(
    provider_id: &str,
    api_key: &str,
    base_url: Option<&str>,
) -> Result<Vec<String>, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Veuillez saisir une clé API.".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| format!("Client HTTP indisponible : {e}"))?;

    let custom_base = base_url
        .map(|b| b.trim().trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty());

    let models = match provider_id {
        "gemini" => {
            let base = custom_base
                .as_deref()
                .unwrap_or("https://generativelanguage.googleapis.com/v1beta");
            let url = format!("{base}/models?key={key}");
            let resp = client
                .get(url)
                .send()
                .await
                .map_err(|e| format!("Erreur réseau Gemini : {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Clé API Gemini refusée (HTTP {})", resp.status().as_u16()));
            }
            let json: Value = resp
                .json()
                .await
                .map_err(|e| format!("Réponse Gemini invalide : {e}"))?;
            let mut list: Vec<String> = json
                .get("models")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let methods_ok = item
                                .get("supportedGenerationMethods")
                                .and_then(|m| m.as_array())
                                .map(|methods| {
                                    methods.iter().any(|x| x.as_str() == Some("generateContent"))
                                })
                                .unwrap_or(false);
                            if !methods_ok {
                                return None;
                            }
                            let raw_name = item.get("name")?.as_str()?;
                            let name = raw_name.strip_prefix("models/").unwrap_or(raw_name);
                            let low = name.to_lowercase();
                            let excluded = [
                                "embedding",
                                "tts",
                                "image",
                                "audio",
                                "live",
                                "aqa",
                                "vision",
                                "robotics",
                                "computer-use",
                                "veo",
                                "imagen",
                            ];
                            if low.starts_with("gemini-")
                                && !excluded.iter().any(|k| low.contains(k))
                            {
                                Some(name.to_string())
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            list.sort();
            list.reverse();
            list.truncate(25);
            list
        }
        "openai" => {
            let base = custom_base
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let url = format!("{base}/models");
            let resp = client
                .get(url)
                .bearer_auth(key)
                .send()
                .await
                .map_err(|e| format!("Erreur réseau OpenAI : {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Clé API OpenAI refusée (HTTP {})", resp.status().as_u16()));
            }
            let json: Value = resp
                .json()
                .await
                .map_err(|e| format!("Réponse OpenAI invalide : {e}"))?;
            let excluded_kw = [
                "audio",
                "realtime",
                "transcribe",
                "tts",
                "whisper",
                "image",
                "search",
                "embedding",
                "moderation",
            ];
            let mut list: Vec<String> = json
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let id = item.get("id")?.as_str()?;
                            let lower = id.to_lowercase();
                            let allowed_prefix = lower.starts_with("gpt-4")
                                || lower.starts_with("gpt-5")
                                || lower.starts_with("o1")
                                || lower.starts_with("o3")
                                || lower.starts_with("o4");
                            if allowed_prefix && !excluded_kw.iter().any(|k| lower.contains(k)) {
                                Some(id.to_string())
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            list.sort();
            list.reverse();
            list.truncate(25);
            list
        }
        "anthropic" => {
            let base = custom_base
                .as_deref()
                .unwrap_or("https://api.anthropic.com/v1");
            let url = format!("{base}/models");
            let resp = client
                .get(url)
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await
                .map_err(|e| format!("Erreur réseau Anthropic : {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Clé API Anthropic refusée (HTTP {})", resp.status().as_u16()));
            }
            let json: Value = resp
                .json()
                .await
                .map_err(|e| format!("Réponse Anthropic invalide : {e}"))?;
            let mut list: Vec<String> = json
                .get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.get("id")?.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            list.truncate(20);
            list
        }
        "deepseek" => {
            let base = custom_base
                .as_deref()
                .unwrap_or("https://api.deepseek.com/v1");
            let url = format!("{base}/models");
            let resp = client
                .get(url)
                .bearer_auth(key)
                .send()
                .await
                .map_err(|e| format!("Erreur réseau DeepSeek : {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Clé API DeepSeek refusée (HTTP {})", resp.status().as_u16()));
            }
            let json: Value = resp
                .json()
                .await
                .map_err(|e| format!("Réponse DeepSeek invalide : {e}"))?;
            json.get("data")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.get("id")?.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        }
        "openrouter" => {
            let base = custom_base
                .as_deref()
                .unwrap_or("https://openrouter.ai/api/v1");
            // OpenRouter `/models` is public, so verify the key via `/auth/key` first
            let auth_resp = client
                .get(format!("{base}/auth/key"))
                .bearer_auth(key)
                .send()
                .await
                .map_err(|e| format!("Erreur réseau OpenRouter : {e}"))?;
            if !auth_resp.status().is_success() {
                return Err(format!(
                    "Clé API OpenRouter refusée (HTTP {})",
                    auth_resp.status().as_u16()
                ));
            }
            let models_resp = client
                .get(format!("{base}/models"))
                .bearer_auth(key)
                .send()
                .await
                .map_err(|e| format!("Erreur récupération modèles OpenRouter : {e}"))?;
            let mut prioritized = Vec::new();
            let mut others = Vec::new();
            if models_resp.status().is_success() {
                if let Ok(json) = models_resp.json::<Value>().await {
                    if let Some(arr) = json.get("data").and_then(|v| v.as_array()) {
                        for item in arr {
                            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                                let lower = id.to_lowercase();
                                let priority = lower.starts_with("anthropic/claude")
                                    || lower.starts_with("google/gemini")
                                    || lower.starts_with("openai/gpt")
                                    || lower.starts_with("deepseek/")
                                    || lower.starts_with("qwen/")
                                    || lower.ends_with(":free");
                                if priority {
                                    prioritized.push(id.to_string());
                                } else {
                                    others.push(id.to_string());
                                }
                            }
                        }
                    }
                }
            }
            prioritized.extend(others);
            prioritized.truncate(30);
            prioritized
        }
        "mistral" => {
            let base = custom_base
                .as_deref()
                .unwrap_or("https://api.mistral.ai/v1");
            let url = format!("{base}/models");
            let resp = client
                .get(url)
                .bearer_auth(key)
                .send()
                .await
                .map_err(|e| format!("Erreur réseau Mistral : {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Clé API Mistral refusée (HTTP {})", resp.status().as_u16()));
            }
            let json: Value = resp
                .json()
                .await
                .map_err(|e| format!("Réponse Mistral invalide : {e}"))?;
            let mut prioritized = Vec::new();
            let mut others = Vec::new();
            if let Some(arr) = json.get("data").and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                        let lower = id.to_lowercase();
                        if lower.contains("codestral")
                            || lower.contains("devstral")
                            || lower.starts_with("mistral-")
                        {
                            prioritized.push(id.to_string());
                        } else {
                            others.push(id.to_string());
                        }
                    }
                }
            }
            prioritized.extend(others);
            prioritized.truncate(25);
            prioritized
        }
        other => return Err(format!("Fournisseur inconnu : {other}")),
    };

    if models.is_empty() {
        Ok(default_models_for(provider_id))
    } else {
        Ok(models)
    }
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
struct OllamaReq<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<OllamaMsg<'a>>,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaMsg<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct OllamaResp {
    message: Option<OllamaRespMsg>,
    prompt_eval_count: Option<usize>,
    eval_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct OllamaRespMsg {
    content: Option<String>,
}

fn extract_kv_string(haystack: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let idx = haystack.find(&needle)?;
    let after_key = &haystack[idx + needle.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let rest = &after_colon[1..];
    let mut out = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            let trimmed = out.trim();
            return if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        } else {
            out.push(ch);
        }
    }
    None
}

fn extract_kv_number(haystack: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\"");
    let idx = haystack.find(&needle)?;
    let after_key = &haystack[idx + needle.len()..];
    let colon = after_key.find(':')?;
    let digits: String = after_key[colon + 1..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse::<u64>().ok()
}

pub fn extract_api_error_message(raw: &str) -> Option<String> {
    let normalized = raw
        .replace("\\n", "\n")
        .replace("\\\"", "\"")
        .replace("\\'", "'");

    if let (Some(start), Some(end)) = (normalized.find('{'), normalized.rfind('}')) {
        if start < end {
            if let Ok(val) = serde_json::from_str::<Value>(&normalized[start..=end]) {
                let err_obj = val.get("error").unwrap_or(&val);
                let msg = err_obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let code = err_obj.get("code").and_then(|v| v.as_u64());
                let status = err_obj
                    .get("status")
                    .or_else(|| err_obj.get("type"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                if let Some(m) = msg {
                    return Some(match (code, status) {
                        (Some(c), Some(st)) => format!("HTTP {c} {st} : {m}"),
                        (Some(c), None) => format!("HTTP {c} : {m}"),
                        (None, Some(st)) => format!("{st} : {m}"),
                        (None, None) => m,
                    });
                }
            }
        }
    }

    if let Some(m) = extract_kv_string(&normalized, "message") {
        let code = extract_kv_number(&normalized, "code");
        let status = extract_kv_string(&normalized, "status");
        return Some(match (code, status) {
            (Some(c), Some(st)) => format!("HTTP {c} {st} : {m}"),
            (Some(c), None) => format!("HTTP {c} : {m}"),
            (None, Some(st)) => format!("{st} : {m}"),
            (None, None) => m,
        });
    }

    if normalized.contains("ServiceUnavailableError") || normalized.contains("UNAVAILABLE") {
        return Some("Service indisponible (HTTP 503 : surcharge ou modèle indisponible côté fournisseur)".to_string());
    }
    if normalized.contains("RateLimitError") || normalized.contains("RESOURCE_EXHAUSTED") {
        return Some("Quota / limite de requêtes dépassée (HTTP 429)".to_string());
    }
    if normalized.contains("NotFoundError") {
        return Some("Modèle ou endpoint introuvable (HTTP 404)".to_string());
    }
    if normalized.contains("AuthenticationError") {
        return Some("Clé API invalide ou refusée (HTTP 401/403)".to_string());
    }

    None
}

fn extract_gemini_text(json: &Value) -> String {
    let parts = match json
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        Some(p) => p,
        None => return String::new(),
    };

    let filtered = parts
        .iter()
        .filter(|p| p.get("thought").and_then(|v| v.as_bool()) != Some(true))
        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("");

    if !filtered.trim().is_empty() {
        return filtered.trim().to_string();
    }

    parts
        .iter()
        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

pub async fn call_provider_oneshot(
    provider: &str,
    model: &str,
    settings: &AppSettings,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: usize,
    temperature: f32,
) -> anyhow::Result<(String, usize, usize)> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()?;

    let clean_model = clean_model_name(provider, model);
    if clean_model.is_empty() {
        anyhow::bail!("Modèle vide pour le fournisseur {provider}");
    }

    if provider == "ollama" {
        let url = format!("{}/api/chat", settings.ollama_url.trim_end_matches('/'));
        let payload = OllamaReq {
            model: &clean_model,
            stream: false,
            messages: vec![
                OllamaMsg {
                    role: "system",
                    content: system_prompt,
                },
                OllamaMsg {
                    role: "user",
                    content: user_prompt,
                },
            ],
            options: OllamaOptions {
                num_ctx: Some(effective_ollama_num_ctx(settings)),
                num_predict: Some(max_tokens),
                temperature: Some(temperature),
            },
        };
        let resp = client.post(url).json(&payload).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Ollama HTTP {}", resp.status());
        }
        let parsed: OllamaResp = resp.json().await?;
        let text = parsed
            .message
            .and_then(|m| m.content)
            .unwrap_or_default()
            .trim()
            .to_string();
        return Ok((
            text,
            parsed.prompt_eval_count.unwrap_or(0),
            parsed.eval_count.unwrap_or(0),
        ));
    }

    let key = find_provider_key(settings, provider)
        .ok_or_else(|| anyhow::anyhow!("Clé API absente pour '{provider}'"))?;
    let custom_base = find_provider_config(settings, provider)
        .and_then(|c| c.base_url.as_deref())
        .map(|b| b.trim().trim_end_matches('/'))
        .filter(|b| !b.is_empty());

    match provider {
        "gemini" => {
            let base = custom_base.unwrap_or("https://generativelanguage.googleapis.com/v1beta");
            let url = format!("{base}/models/{clean_model}:generateContent?key={key}");
            let body = serde_json::json!({
                "systemInstruction": {
                    "parts": [{ "text": system_prompt }]
                },
                "contents": [{
                    "role": "user",
                    "parts": [{ "text": user_prompt }]
                }],
                "generationConfig": {
                    "maxOutputTokens": max_tokens.max(2048),
                    "temperature": temperature
                }
            });
            let resp = client.post(url).json(&body).send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let err_txt = resp.text().await.unwrap_or_default();
                let clean = extract_api_error_message(&err_txt).unwrap_or(err_txt);
                anyhow::bail!("Gemini HTTP {} : {}", status, clean);
            }
            let json: Value = resp.json().await?;
            let text = extract_gemini_text(&json);
            let p_tok = json
                .get("usageMetadata")
                .and_then(|u| u.get("promptTokenCount"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let c_tok = json
                .get("usageMetadata")
                .and_then(|u| u.get("candidatesTokenCount"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            Ok((text, p_tok, c_tok))
        }
        "anthropic" => {
            let base = custom_base.unwrap_or("https://api.anthropic.com/v1");
            let url = format!("{base}/messages");
            let body = serde_json::json!({
                "model": clean_model,
                "max_tokens": max_tokens,
                "temperature": temperature,
                "system": system_prompt,
                "messages": [{
                    "role": "user",
                    "content": user_prompt
                }]
            });
            let resp = client
                .post(url)
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let err_txt = resp.text().await.unwrap_or_default();
                let clean = extract_api_error_message(&err_txt).unwrap_or(err_txt);
                anyhow::bail!("Anthropic HTTP {} : {}", status, clean);
            }
            let json: Value = resp.json().await?;
            let text = json
                .get("content")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            let p_tok = json
                .get("usage")
                .and_then(|u| u.get("input_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let c_tok = json
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            Ok((text, p_tok, c_tok))
        }
        "openai" | "deepseek" | "openrouter" | "mistral" => {
            let default_base = match provider {
                "openai" => "https://api.openai.com/v1",
                "deepseek" => "https://api.deepseek.com/v1",
                "openrouter" => "https://openrouter.ai/api/v1",
                "mistral" => "https://api.mistral.ai/v1",
                _ => "https://api.openai.com/v1",
            };
            let base = custom_base.unwrap_or(default_base);
            let url = format!("{base}/chat/completions");
            let body = serde_json::json!({
                "model": clean_model,
                "max_tokens": max_tokens,
                "temperature": temperature,
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": user_prompt }
                ]
            });
            let mut req = client.post(url).bearer_auth(key).json(&body);
            if provider == "openrouter" {
                req = req
                    .header("HTTP-Referer", "https://aiderstudio.local")
                    .header("X-Title", "AiderStudio");
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let err_txt = resp.text().await.unwrap_or_default();
                let clean = extract_api_error_message(&err_txt).unwrap_or(err_txt);
                anyhow::bail!("{provider} HTTP {status} : {clean}");
            }
            let json: Value = resp.json().await?;
            let text = json
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            let p_tok = json
                .get("usage")
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let c_tok = json
                .get("usage")
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            Ok((text, p_tok, c_tok))
        }
        other => anyhow::bail!("Fournisseur inconnu : {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with_keys() -> AppSettings {
        let mut s = AppSettings::default();
        s.ollama_url = "http://192.168.1.10:11434/".to_string();
        for p in &mut s.providers {
            p.api_key = format!("test-key-{}", p.id);
            p.validated = true;
        }
        s
    }

    #[test]
    fn test_aider_model_and_env_all_providers() {
        let s = settings_with_keys();

        let (m_ollama, env_ollama) = aider_model_and_env("ollama", "qwen2.5-coder:7b", &s).unwrap();
        assert_eq!(m_ollama, "ollama_chat/qwen2.5-coder:7b");
        assert_eq!(env_ollama, vec![("OLLAMA_API_BASE".into(), "http://192.168.1.10:11434".into())]);

        let (m_gem, env_gem) = aider_model_and_env("gemini", "gemini/gemini-2.5-pro", &s).unwrap();
        assert_eq!(m_gem, "gemini/gemini-2.5-pro");
        assert_eq!(env_gem, vec![("GEMINI_API_KEY".into(), "test-key-gemini".into())]);

        let (m_oai, env_oai) = aider_model_and_env("openai", "gpt-4.1", &s).unwrap();
        assert_eq!(m_oai, "openai/gpt-4.1");
        assert_eq!(env_oai[0], ("OPENAI_API_KEY".into(), "test-key-openai".into()));

        let (m_ant, env_ant) = aider_model_and_env("anthropic", "claude-sonnet-4-20250514", &s).unwrap();
        assert_eq!(m_ant, "anthropic/claude-sonnet-4-20250514");
        assert_eq!(env_ant, vec![("ANTHROPIC_API_KEY".into(), "test-key-anthropic".into())]);

        let (m_ds, env_ds) = aider_model_and_env("deepseek", "deepseek-chat", &s).unwrap();
        assert_eq!(m_ds, "deepseek/deepseek-chat");
        assert_eq!(env_ds, vec![("DEEPSEEK_API_KEY".into(), "test-key-deepseek".into())]);

        let (m_or, env_or) = aider_model_and_env("openrouter", "anthropic/claude-sonnet-4", &s).unwrap();
        assert_eq!(m_or, "openrouter/anthropic/claude-sonnet-4");
        assert_eq!(env_or, vec![("OPENROUTER_API_KEY".into(), "test-key-openrouter".into())]);

        let (m_mis, env_mis) = aider_model_and_env("mistral", "codestral-latest", &s).unwrap();
        assert_eq!(m_mis, "mistral/codestral-latest");
        assert_eq!(env_mis, vec![("MISTRAL_API_KEY".into(), "test-key-mistral".into())]);

        let empty_key_settings = AppSettings::default();
        assert!(aider_model_and_env("gemini", "gemini-2.5-pro", &empty_key_settings).is_err());
    }

    #[test]
    fn test_resolve_model_context_limit() {
        let mut s = AppSettings::default();
        s.context_window_tokens = 16384;
        assert_eq!(resolve_model_context_limit("ollama", "qwen2.5-coder:7b", &s), 16384);
        assert_eq!(resolve_model_context_limit("gemini", "gemini-2.5-pro", &s), 1_000_000);
        assert_eq!(resolve_model_context_limit("anthropic", "claude-sonnet-4", &s), 200_000);
        assert_eq!(resolve_model_context_limit("openai", "gpt-4.1", &s), 128_000);
        assert_eq!(resolve_model_context_limit("deepseek", "deepseek-chat", &s), 64_000);
        assert_eq!(resolve_model_context_limit("mistral", "codestral-latest", &s), 256_000);
    }

    #[test]
    fn test_merge_with_default_providers() {
        let saved = vec![ProviderConfig {
            id: "gemini".to_string(),
            name: "Gemini".to_string(),
            api_key: "AIza-secret".to_string(),
            base_url: None,
            validated: true,
            available_models: vec!["gemini-2.5-pro".to_string()],
            daily_token_limit: 900_000,
        }];
        let merged = merge_with_default_providers(&saved);
        assert_eq!(merged.len(), 6);
        let gem = merged.iter().find(|p| p.id == "gemini").unwrap();
        assert_eq!(gem.api_key, "AIza-secret");
        assert!(gem.validated);
        assert_eq!(gem.daily_token_limit, 900_000);
        assert!(merged.iter().any(|p| p.id == "openrouter" && !p.validated));
    }

    #[test]
    fn test_extract_gemini_text_skips_thought_parts() {
        let json = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "thought": true, "text": "Internal reasoning chain..." },
                        { "text": "{\"role\":\"designer\",\"runTests\":false,\"spec\":\"Ajuster CSS\"}" }
                    ]
                }
            }]
        });
        assert_eq!(
            extract_gemini_text(&json),
            "{\"role\":\"designer\",\"runTests\":false,\"spec\":\"Ajuster CSS\"}"
        );

        let fallback_only = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "thought": true, "text": "Only thought text" }
                    ]
                }
            }]
        });
        assert_eq!(extract_gemini_text(&fallback_only), "Only thought text");
    }

    #[test]
    fn test_extract_api_error_message_from_litellm_bytes() {
        let full = r#"litellm.ServiceUnavailableError: Vertex_ai_betaException - b'{\n "error": {\n "code": 503, "message": "This model is currently experiencing high demand.", "status": "UNAVAILABLE"\n }\n}'"#;
        assert_eq!(
            extract_api_error_message(full).as_deref(),
            Some("HTTP 503 UNAVAILABLE : This model is currently experiencing high demand.")
        );

        let truncated = r#"Retrying in 0.2 seconds... litellm.ServiceUnavailableError: Vertex_ai_betaException - b'{\n "error": {\n"#;
        let msg = extract_api_error_message(truncated).unwrap();
        assert!(msg.contains("503"));
    }
}
