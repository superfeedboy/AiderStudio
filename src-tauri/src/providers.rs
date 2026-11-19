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
            let reasoning = is_reasoning_model(provider, &clean_model);
            let mut body = serde_json::json!({
                "model": clean_model,
                "messages": [
                    { "role": if reasoning { "developer" } else { "system" }, "content": system_prompt },
                    { "role": "user", "content": user_prompt }
                ]
            });
            if let Some(obj) = body.as_object_mut() {
                if reasoning {
                    obj.insert("max_completion_tokens".to_string(), Value::from(max_tokens.max(1024)));
                } else {
                    obj.insert("max_tokens".to_string(), Value::from(max_tokens));
                    obj.insert("temperature".to_string(), Value::from(temperature));
                }
            }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
    pub images: Vec<String>,
}

impl LlmMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            images,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LlmRequestOptions {
    pub max_tokens: usize,
    pub temperature: f32,
    pub enable_cache: bool,
}

impl Default for LlmRequestOptions {
    fn default() -> Self {
        Self {
            max_tokens: 8192,
            temperature: 0.2,
            enable_cache: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamDelta {
    Thinking(String),
    Content(String),
}

#[derive(Debug, Clone, Default)]
pub struct LlmStreamResult {
    pub content: String,
    pub thinking: Option<String>,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub cache_read_tokens: usize,
    pub finish_reason: Option<String>,
}

/// Tampon octet par octet pour découper des lignes sans jamais couper un caractère UTF-8 multi-octets.
#[derive(Debug, Default)]
pub struct ByteLineBuffer {
    buf: Vec<u8>,
}

impl ByteLineBuffer {
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        let mut start = 0usize;
        for (idx, &b) in self.buf.iter().enumerate() {
            if b == b'\n' {
                let mut end = idx;
                if end > start && self.buf[end - 1] == b'\r' {
                    end -= 1;
                }
                lines.push(String::from_utf8_lossy(&self.buf[start..end]).to_string());
                start = idx + 1;
            }
        }
        if start > 0 {
            self.buf.drain(0..start);
        }
        lines
    }

    pub fn flush(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let mut end = self.buf.len();
        if end > 0 && self.buf[end - 1] == b'\r' {
            end -= 1;
        }
        let s = String::from_utf8_lossy(&self.buf[..end]).to_string();
        self.buf.clear();
        Some(s)
    }
}

/// Séparateur incrémental `<think>...</think>` robuste aux balises coupées entre deux deltas.
#[derive(Debug, Default)]
pub struct ThinkStreamSplitter {
    buffer: String,
    in_think: bool,
}

impl ThinkStreamSplitter {
    fn trailing_tag_prefix_len(text: &str, tag: &str) -> usize {
        let max_check = text.len().min(tag.len().saturating_sub(1));
        for len in (1..=max_check).rev() {
            if text.ends_with(&tag[..len]) {
                return len;
            }
        }
        0
    }

    pub fn push(&mut self, chunk: &str) -> Vec<StreamDelta> {
        let mut out = Vec::new();
        if chunk.is_empty() {
            return out;
        }
        self.buffer.push_str(chunk);

        loop {
            if !self.in_think {
                if let Some(pos) = self.buffer.find("<think>") {
                    if pos > 0 {
                        out.push(StreamDelta::Content(self.buffer[..pos].to_string()));
                    }
                    self.buffer.drain(..pos + "<think>".len());
                    self.in_think = true;
                } else {
                    let keep = Self::trailing_tag_prefix_len(&self.buffer, "<think>");
                    let emit_len = self.buffer.len() - keep;
                    if emit_len > 0 {
                        out.push(StreamDelta::Content(self.buffer[..emit_len].to_string()));
                        self.buffer.drain(..emit_len);
                    }
                    break;
                }
            } else if let Some(pos) = self.buffer.find("</think>") {
                if pos > 0 {
                    out.push(StreamDelta::Thinking(self.buffer[..pos].to_string()));
                }
                self.buffer.drain(..pos + "</think>".len());
                self.in_think = false;
            } else {
                let keep = Self::trailing_tag_prefix_len(&self.buffer, "</think>");
                let emit_len = self.buffer.len() - keep;
                if emit_len > 0 {
                    out.push(StreamDelta::Thinking(self.buffer[..emit_len].to_string()));
                    self.buffer.drain(..emit_len);
                }
                break;
            }
        }
        out
    }

    pub fn finish(&mut self) -> Vec<StreamDelta> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let rem = std::mem::take(&mut self.buffer);
        if self.in_think {
            vec![StreamDelta::Thinking(rem)]
        } else {
            vec![StreamDelta::Content(rem)]
        }
    }
}

pub fn is_reasoning_model(provider: &str, clean_model: &str) -> bool {
    if provider != "openai" {
        return false;
    }
    let low = clean_model.to_lowercase();
    low.starts_with("o1")
        || low.starts_with("o3")
        || low.starts_with("o4")
        || low.starts_with("gpt-5")
}

fn detect_image_mime_and_b64(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("data:") {
        if let Some((mime, b64)) = rest.split_once(";base64,") {
            return (mime.to_string(), b64.to_string());
        }
    }
    let mime = if trimmed.starts_with("/9j/") {
        "image/jpeg"
    } else if trimmed.starts_with("iVBOR") {
        "image/png"
    } else if trimmed.starts_with("R0lGOD") {
        "image/gif"
    } else if trimmed.starts_with("UklGR") {
        "image/webp"
    } else {
        "image/png"
    };
    (mime.to_string(), trimmed.to_string())
}

fn normalize_finish_reason(raw: &str) -> String {
    match raw {
        "max_tokens" | "MAX_TOKENS" | "length" => "length".to_string(),
        other => other.to_string(),
    }
}

pub fn build_openai_stream_payload(
    provider: &str,
    clean_model: &str,
    messages: &[LlmMessage],
    options: &LlmRequestOptions,
    include_stream_options: bool,
) -> Value {
    let reasoning = is_reasoning_model(provider, clean_model);
    let mut api_msgs = Vec::new();

    for m in messages {
        let role = if m.role == "system" && reasoning {
            "developer"
        } else {
            m.role.as_str()
        };
        if m.images.is_empty() {
            api_msgs.push(serde_json::json!({
                "role": role,
                "content": m.content
            }));
        } else {
            let mut parts = vec![serde_json::json!({
                "type": "text",
                "text": m.content
            })];
            for img in &m.images {
                let (mime, b64) = detect_image_mime_and_b64(img);
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{mime};base64,{b64}")
                    }
                }));
            }
            api_msgs.push(serde_json::json!({
                "role": role,
                "content": parts
            }));
        }
    }

    let mut body = serde_json::json!({
        "model": clean_model,
        "stream": true,
        "messages": api_msgs
    });

    if let Some(obj) = body.as_object_mut() {
        if reasoning {
            obj.insert(
                "max_completion_tokens".to_string(),
                Value::from(options.max_tokens.max(1024)),
            );
        } else {
            obj.insert(
                "max_tokens".to_string(),
                Value::from(options.max_tokens.max(1024)),
            );
            obj.insert("temperature".to_string(), Value::from(options.temperature));
        }
        if include_stream_options && provider != "mistral" {
            obj.insert(
                "stream_options".to_string(),
                serde_json::json!({ "include_usage": true }),
            );
        }
    }

    body
}

pub fn build_anthropic_stream_payload(
    clean_model: &str,
    messages: &[LlmMessage],
    options: &LlmRequestOptions,
) -> Value {
    let mut system_blocks = Vec::new();
    let mut api_msgs = Vec::new();
    let mut first_long_user_marked = false;

    for m in messages {
        if m.role == "system" {
            let mut block = serde_json::json!({
                "type": "text",
                "text": m.content
            });
            if options.enable_cache {
                block
                    .as_object_mut()
                    .unwrap()
                    .insert("cache_control".to_string(), serde_json::json!({ "type": "ephemeral" }));
            }
            system_blocks.push(block);
            continue;
        }

        let role = if m.role == "assistant" {
            "assistant"
        } else {
            "user"
        };

        if m.images.is_empty() && !(options.enable_cache && role == "user" && !first_long_user_marked && m.content.len() > 1200) {
            api_msgs.push(serde_json::json!({
                "role": role,
                "content": m.content
            }));
        } else {
            let mut parts = Vec::new();
            for img in &m.images {
                let (mime, b64) = detect_image_mime_and_b64(img);
                parts.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": mime,
                        "data": b64
                    }
                }));
            }
            let mut text_block = serde_json::json!({
                "type": "text",
                "text": m.content
            });
            if options.enable_cache && role == "user" && !first_long_user_marked && m.content.len() > 1200 {
                first_long_user_marked = true;
                text_block
                    .as_object_mut()
                    .unwrap()
                    .insert("cache_control".to_string(), serde_json::json!({ "type": "ephemeral" }));
            }
            parts.push(text_block);
            api_msgs.push(serde_json::json!({
                "role": role,
                "content": parts
            }));
        }
    }

    let mut body = serde_json::json!({
        "model": clean_model,
        "max_tokens": options.max_tokens.max(1024),
        "temperature": options.temperature,
        "stream": true,
        "messages": api_msgs
    });
    if !system_blocks.is_empty() {
        body.as_object_mut()
            .unwrap()
            .insert("system".to_string(), Value::Array(system_blocks));
    }
    body
}

pub fn build_gemini_stream_payload(
    messages: &[LlmMessage],
    options: &LlmRequestOptions,
) -> Value {
    let mut system_texts = Vec::new();
    let mut contents = Vec::new();

    for m in messages {
        if m.role == "system" {
            if !m.content.trim().is_empty() {
                system_texts.push(m.content.as_str());
            }
            continue;
        }
        let role = if m.role == "assistant" {
            "model"
        } else {
            "user"
        };
        let mut parts = vec![serde_json::json!({ "text": m.content })];
        for img in &m.images {
            let (mime, b64) = detect_image_mime_and_b64(img);
            parts.push(serde_json::json!({
                "inlineData": {
                    "mimeType": mime,
                    "data": b64
                }
            }));
        }
        contents.push(serde_json::json!({
            "role": role,
            "parts": parts
        }));
    }

    let mut body = serde_json::json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": options.max_tokens.max(2048),
            "temperature": options.temperature
        }
    });
    if !system_texts.is_empty() {
        body.as_object_mut().unwrap().insert(
            "systemInstruction".to_string(),
            serde_json::json!({
                "parts": [{ "text": system_texts.join("\n\n") }]
            }),
        );
    }
    body
}

#[derive(Debug, Default)]
struct StreamAccumulator {
    content: String,
    thinking: String,
    prompt_tokens: usize,
    completion_tokens: usize,
    cache_read_tokens: usize,
    finish_reason: Option<String>,
}

impl StreamAccumulator {
    fn emit<F: FnMut(StreamDelta)>(&mut self, delta: StreamDelta, on_delta: &mut F) {
        match &delta {
            StreamDelta::Thinking(t) if !t.is_empty() => {
                self.thinking.push_str(t);
                on_delta(delta);
            }
            StreamDelta::Content(c) if !c.is_empty() => {
                self.content.push_str(c);
                on_delta(delta);
            }
            _ => {}
        }
    }

    fn into_result(self) -> LlmStreamResult {
        let trimmed_think = self.thinking.trim().to_string();
        LlmStreamResult {
            content: self.content.trim().to_string(),
            thinking: if trimmed_think.is_empty() {
                None
            } else {
                Some(trimmed_think)
            },
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            cache_read_tokens: self.cache_read_tokens,
            finish_reason: self.finish_reason,
        }
    }
}

fn process_openai_sse_data<F: FnMut(StreamDelta)>(
    data: &str,
    splitter: &mut ThinkStreamSplitter,
    acc: &mut StreamAccumulator,
    on_delta: &mut F,
) -> anyhow::Result<bool> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    if trimmed == "[DONE]" {
        for d in splitter.finish() {
            acc.emit(d, on_delta);
        }
        return Ok(true);
    }
    let Ok(json) = serde_json::from_str::<Value>(trimmed) else {
        return Ok(false);
    };
    if let Some(err) = json.get("error") {
        let msg = extract_api_error_message(&err.to_string()).unwrap_or_else(|| err.to_string());
        anyhow::bail!("{msg}");
    }
    if let Some(usage) = json.get("usage").filter(|u| !u.is_null()) {
        if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
            acc.prompt_tokens = pt as usize;
        }
        if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
            acc.completion_tokens = ct as usize;
        }
        if let Some(cached) = usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_u64())
        {
            acc.cache_read_tokens = cached as usize;
        }
    }
    if let Some(choice) = json.get("choices").and_then(|c| c.get(0)) {
        if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            if !fr.is_empty() {
                acc.finish_reason = Some(normalize_finish_reason(fr));
            }
        }
        if let Some(delta) = choice.get("delta") {
            if let Some(rc) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(|v| v.as_str())
            {
                if !rc.is_empty() {
                    acc.emit(StreamDelta::Thinking(rc.to_string()), on_delta);
                }
            }
            if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
                for d in splitter.push(content) {
                    acc.emit(d, on_delta);
                }
            }
        }
    }
    Ok(false)
}

fn process_anthropic_sse_data<F: FnMut(StreamDelta)>(
    data: &str,
    splitter: &mut ThinkStreamSplitter,
    acc: &mut StreamAccumulator,
    on_delta: &mut F,
) -> anyhow::Result<()> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let Ok(json) = serde_json::from_str::<Value>(trimmed) else {
        return Ok(());
    };
    let ev_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match ev_type {
        "message_start" => {
            if let Some(usage) = json.get("message").and_then(|m| m.get("usage")) {
                let inp = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let cache_create = usage
                    .get("cache_creation_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                acc.prompt_tokens = inp + cache_create + cache_read;
                acc.cache_read_tokens = cache_read;
            }
        }
        "content_block_delta" => {
            if let Some(delta) = json.get("delta") {
                let dtype = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if dtype == "thinking_delta" {
                    if let Some(t) = delta.get("thinking").and_then(|v| v.as_str()) {
                        acc.emit(StreamDelta::Thinking(t.to_string()), on_delta);
                    }
                } else if dtype == "text_delta" {
                    if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                        for d in splitter.push(t) {
                            acc.emit(d, on_delta);
                        }
                    }
                }
            }
        }
        "message_delta" => {
            if let Some(sr) = json
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str())
            {
                if !sr.is_empty() {
                    acc.finish_reason = Some(normalize_finish_reason(sr));
                }
            }
            if let Some(out) = json
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_u64())
            {
                acc.completion_tokens = out as usize;
            }
        }
        "message_stop" => {
            for d in splitter.finish() {
                acc.emit(d, on_delta);
            }
        }
        "error" => {
            let msg = extract_api_error_message(&json.to_string())
                .unwrap_or_else(|| json.to_string());
            anyhow::bail!("Anthropic stream error : {msg}");
        }
        _ => {}
    }
    Ok(())
}

fn process_gemini_sse_data<F: FnMut(StreamDelta)>(
    data: &str,
    splitter: &mut ThinkStreamSplitter,
    acc: &mut StreamAccumulator,
    on_delta: &mut F,
) -> anyhow::Result<()> {
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(());
    }
    let Ok(json) = serde_json::from_str::<Value>(trimmed) else {
        return Ok(());
    };
    if let Some(err) = json.get("error") {
        let msg = extract_api_error_message(&err.to_string()).unwrap_or_else(|| err.to_string());
        anyhow::bail!("Gemini stream error : {msg}");
    }
    if let Some(usage) = json.get("usageMetadata") {
        if let Some(pt) = usage.get("promptTokenCount").and_then(|v| v.as_u64()) {
            acc.prompt_tokens = pt as usize;
        }
        if let Some(ct) = usage.get("candidatesTokenCount").and_then(|v| v.as_u64()) {
            acc.completion_tokens = ct as usize;
        }
        if let Some(cached) = usage.get("cachedContentTokenCount").and_then(|v| v.as_u64()) {
            acc.cache_read_tokens = cached as usize;
        }
    }
    if let Some(cand) = json.get("candidates").and_then(|c| c.get(0)) {
        if let Some(fr) = cand.get("finishReason").and_then(|v| v.as_str()) {
            if !fr.is_empty() {
                acc.finish_reason = Some(normalize_finish_reason(fr));
            }
        }
        if let Some(parts) = cand
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
        {
            for part in parts {
                let Some(text) = part.get("text").and_then(|t| t.as_str()) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                if part.get("thought").and_then(|v| v.as_bool()) == Some(true) {
                    acc.emit(StreamDelta::Thinking(text.to_string()), on_delta);
                } else {
                    for d in splitter.push(text) {
                        acc.emit(d, on_delta);
                    }
                }
            }
        }
    }
    Ok(())
}

async fn wait_stream_cancelled(cancel_rx: &mut tokio::sync::watch::Receiver<bool>) {
    if *cancel_rx.borrow() {
        return;
    }
    while cancel_rx.changed().await.is_ok() {
        if *cancel_rx.borrow() {
            return;
        }
    }
    std::future::pending::<()>().await;
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504 | 529)
}

/// Appel unifié multi-tours en streaming SSE / NDJSON pour les 7 fournisseurs avec retry et annulation.
pub async fn call_provider_streaming<F>(
    provider: &str,
    model: &str,
    settings: &AppSettings,
    messages: &[LlmMessage],
    options: &LlmRequestOptions,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
    mut on_delta: F,
) -> anyhow::Result<LlmStreamResult>
where
    F: FnMut(StreamDelta),
{
    if *cancel_rx.borrow() {
        anyhow::bail!("CANCELLED_BY_USER");
    }

    let clean_model = clean_model_name(provider, model);
    if clean_model.is_empty() {
        anyhow::bail!("Modèle vide pour le fournisseur {provider}");
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()?;

    if provider == "ollama" {
        let url = format!("{}/api/chat", settings.ollama_url.trim_end_matches('/'));
        let api_msgs: Vec<Value> = messages
            .iter()
            .map(|m| {
                let mut obj = serde_json::json!({
                    "role": m.role,
                    "content": m.content
                });
                if !m.images.is_empty() {
                    let b64_list: Vec<String> = m
                        .images
                        .iter()
                        .map(|img| detect_image_mime_and_b64(img).1)
                        .collect();
                    obj.as_object_mut()
                        .unwrap()
                        .insert("images".to_string(), serde_json::json!(b64_list));
                }
                obj
            })
            .collect();
        let payload = serde_json::json!({
            "model": clean_model,
            "stream": true,
            "messages": api_msgs,
            "options": {
                "num_ctx": effective_ollama_num_ctx(settings),
                "num_predict": options.max_tokens.max(1024),
                "temperature": options.temperature
            }
        });

        let mut resp = tokio::select! {
            biased;
            _ = wait_stream_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
            res = client.post(url).json(&payload).send() => res?,
        };
        if !resp.status().is_success() {
            let st = resp.status();
            let err_txt = resp.text().await.unwrap_or_default();
            let clean = extract_api_error_message(&err_txt).unwrap_or(err_txt);
            anyhow::bail!("Ollama HTTP {st} : {clean}");
        }

        let mut line_buf = ByteLineBuffer::default();
        let mut splitter = ThinkStreamSplitter::default();
        let mut acc = StreamAccumulator::default();

        loop {
            let chunk_opt = tokio::select! {
                biased;
                _ = wait_stream_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                res = resp.chunk() => res?,
            };
            let Some(chunk_bytes) = chunk_opt else {
                break;
            };
            for line in line_buf.push_bytes(&chunk_bytes) {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                if let Ok(parsed) = serde_json::from_str::<Value>(t) {
                    if let Some(msg) = parsed.get("message") {
                        if let Some(th) = msg.get("thinking").and_then(|v| v.as_str()) {
                            if !th.is_empty() {
                                acc.emit(StreamDelta::Thinking(th.to_string()), &mut on_delta);
                            }
                        }
                        if let Some(delta) = msg.get("content").and_then(|v| v.as_str()) {
                            for d in splitter.push(delta) {
                                acc.emit(d, &mut on_delta);
                            }
                        }
                    }
                    if parsed.get("done").and_then(|v| v.as_bool()) == Some(true) {
                        if let Some(pt) = parsed.get("prompt_eval_count").and_then(|v| v.as_u64()) {
                            acc.prompt_tokens = pt as usize;
                        }
                        if let Some(ct) = parsed.get("eval_count").and_then(|v| v.as_u64()) {
                            acc.completion_tokens = ct as usize;
                        }
                        if let Some(dr) = parsed.get("done_reason").and_then(|v| v.as_str()) {
                            if !dr.is_empty() {
                                acc.finish_reason = Some(normalize_finish_reason(dr));
                            }
                        }
                    }
                }
            }
        }
        if let Some(last) = line_buf.flush() {
            let t = last.trim();
            if let Ok(parsed) = serde_json::from_str::<Value>(t) {
                if let Some(delta) = parsed
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|v| v.as_str())
                {
                    for d in splitter.push(delta) {
                        acc.emit(d, &mut on_delta);
                    }
                }
            }
        }
        for d in splitter.finish() {
            acc.emit(d, &mut on_delta);
        }
        return Ok(acc.into_result());
    }

    let key = find_provider_key(settings, provider)
        .ok_or_else(|| anyhow::anyhow!("Clé API absente pour '{provider}'"))?;
    let custom_base = find_provider_config(settings, provider)
        .and_then(|c| c.base_url.as_deref())
        .map(|b| b.trim().trim_end_matches('/'))
        .filter(|b| !b.is_empty());

    let mut include_stream_options = true;
    let backoff_secs = [1u64, 2u64, 4u64];

    for attempt in 0..3 {
        if *cancel_rx.borrow() {
            anyhow::bail!("CANCELLED_BY_USER");
        }

        let resp_res = match provider {
            "gemini" => {
                let base =
                    custom_base.unwrap_or("https://generativelanguage.googleapis.com/v1beta");
                let url = format!("{base}/models/{clean_model}:streamGenerateContent?alt=sse&key={key}");
                let body = build_gemini_stream_payload(messages, options);
                tokio::select! {
                    biased;
                    _ = wait_stream_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                    res = client.post(url).json(&body).send() => res,
                }
            }
            "anthropic" => {
                let base = custom_base.unwrap_or("https://api.anthropic.com/v1");
                let url = format!("{base}/messages");
                let body = build_anthropic_stream_payload(&clean_model, messages, options);
                tokio::select! {
                    biased;
                    _ = wait_stream_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                    res = client
                        .post(url)
                        .header("x-api-key", &key)
                        .header("anthropic-version", "2023-06-01")
                        .json(&body)
                        .send() => res,
                }
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
                let body = build_openai_stream_payload(
                    provider,
                    &clean_model,
                    messages,
                    options,
                    include_stream_options,
                );
                let mut req = client.post(url).bearer_auth(&key).json(&body);
                if provider == "openrouter" {
                    req = req
                        .header("HTTP-Referer", "https://aiderstudio.local")
                        .header("X-Title", "AiderStudio");
                }
                tokio::select! {
                    biased;
                    _ = wait_stream_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                    res = req.send() => res,
                }
            }
            other => anyhow::bail!("Fournisseur inconnu : {other}"),
        };

        let mut resp = match resp_res {
            Ok(r) => r,
            Err(e) => {
                if attempt < 2 && (e.is_timeout() || e.is_connect() || e.is_request()) {
                    let delay = std::time::Duration::from_secs(backoff_secs[attempt]);
                    tokio::select! {
                        biased;
                        _ = wait_stream_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                        _ = tokio::time::sleep(delay) => {}
                    }
                    continue;
                }
                return Err(e.into());
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let code = status.as_u16();
            let err_txt = resp.text().await.unwrap_or_default();

            // Repli transparent si le fournisseur OpenAI-compatible refuse `stream_options`
            if code == 400
                && include_stream_options
                && err_txt.to_lowercase().contains("stream_options")
            {
                include_stream_options = false;
                continue;
            }

            if is_retryable_status(code) && attempt < 2 {
                let delay = std::time::Duration::from_secs(backoff_secs[attempt]);
                tokio::select! {
                    biased;
                    _ = wait_stream_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                    _ = tokio::time::sleep(delay) => {}
                }
                continue;
            }

            let clean = extract_api_error_message(&err_txt).unwrap_or(err_txt);
            anyhow::bail!("{provider} HTTP {status} : {clean}");
        }

        let mut line_buf = ByteLineBuffer::default();
        let mut splitter = ThinkStreamSplitter::default();
        let mut acc = StreamAccumulator::default();
        let mut sse_done = false;

        loop {
            let chunk_opt = tokio::select! {
                biased;
                _ = wait_stream_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                res = resp.chunk() => res?,
            };
            let Some(chunk_bytes) = chunk_opt else {
                break;
            };

            for line in line_buf.push_bytes(&chunk_bytes) {
                if let Some(data) = line.strip_prefix("data:") {
                    let payload = data.trim_start();
                    match provider {
                        "gemini" => process_gemini_sse_data(
                            payload,
                            &mut splitter,
                            &mut acc,
                            &mut on_delta,
                        )?,
                        "anthropic" => process_anthropic_sse_data(
                            payload,
                            &mut splitter,
                            &mut acc,
                            &mut on_delta,
                        )?,
                        _ => {
                            if process_openai_sse_data(
                                payload,
                                &mut splitter,
                                &mut acc,
                                &mut on_delta,
                            )? {
                                sse_done = true;
                            }
                        }
                    }
                }
            }
            if sse_done {
                break;
            }
        }

        if !sse_done {
            if let Some(last) = line_buf.flush() {
                if let Some(data) = last.strip_prefix("data:") {
                    let payload = data.trim_start();
                    match provider {
                        "gemini" => process_gemini_sse_data(
                            payload,
                            &mut splitter,
                            &mut acc,
                            &mut on_delta,
                        )?,
                        "anthropic" => process_anthropic_sse_data(
                            payload,
                            &mut splitter,
                            &mut acc,
                            &mut on_delta,
                        )?,
                        _ => {
                            let _ = process_openai_sse_data(
                                payload,
                                &mut splitter,
                                &mut acc,
                                &mut on_delta,
                            )?;
                        }
                    }
                }
            }
        }

        for d in splitter.finish() {
            acc.emit(d, &mut on_delta);
        }

        return Ok(acc.into_result());
    }

    anyhow::bail!("Échec de l'appel en streaming vers {provider}:{model} après tentatives.")
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
    fn test_think_stream_splitter_handles_split_tags() {
        let mut splitter = ThinkStreamSplitter::default();
        let mut deltas = Vec::new();
        for chunk in ["Bonjour ", "<thi", "nk>Réflexion", " étape 1</thi", "nk>Code prêt."] {
            deltas.extend(splitter.push(chunk));
        }
        deltas.extend(splitter.finish());

        let mut content = String::new();
        let mut thinking = String::new();
        for d in deltas {
            match d {
                StreamDelta::Thinking(t) => thinking.push_str(&t),
                StreamDelta::Content(c) => content.push_str(&c),
            }
        }
        assert_eq!(thinking, "Réflexion étape 1");
        assert_eq!(content, "Bonjour Code prêt.");
    }

    #[test]
    fn test_byte_line_buffer_splits_utf8_safely() {
        let mut buf = ByteLineBuffer::default();
        let raw = "data: café crème\ndata: résumé\n".as_bytes();
        // Coupe pile au milieu du 'é' (2 octets UTF-8: 0xC3 0xA9)
        let split_at = 10;
        let l1 = buf.push_bytes(&raw[..split_at]);
        assert!(l1.is_empty());
        let l2 = buf.push_bytes(&raw[split_at..]);
        assert_eq!(l2, vec!["data: café crème", "data: résumé"]);
    }

    #[test]
    fn test_openai_payload_for_reasoning_models() {
        let msgs = vec![LlmMessage::system("sys"), LlmMessage::user("hi")];
        let opts = LlmRequestOptions {
            max_tokens: 4096,
            temperature: 0.2,
            enable_cache: true,
        };
        let o4_payload = build_openai_stream_payload("openai", "o4-mini", &msgs, &opts, true);
        assert!(o4_payload.get("max_completion_tokens").is_some());
        assert!(o4_payload.get("temperature").is_none());
        assert_eq!(o4_payload["messages"][0]["role"], "developer");

        let gpt4_payload = build_openai_stream_payload("openai", "gpt-4.1", &msgs, &opts, true);
        assert!(gpt4_payload.get("max_tokens").is_some());
        assert!(gpt4_payload.get("temperature").is_some());
        assert_eq!(gpt4_payload["messages"][0]["role"], "system");
    }

    #[test]
    fn test_sse_parsers_openai_anthropic_gemini() {
        let _s = settings_with_keys();

        // OpenAI / DeepSeek reasoning_content
        let mut sp1 = ThinkStreamSplitter::default();
        let mut acc1 = StreamAccumulator::default();
        let mut out1 = Vec::new();
        process_openai_sse_data(
            r#"{"choices":[{"delta":{"reasoning_content":"Analyse..."}}]}"#,
            &mut sp1,
            &mut acc1,
            &mut |d| out1.push(d),
        )
        .unwrap();
        process_openai_sse_data(
            r#"{"choices":[{"delta":{"content":"Ok."},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":4}}"#,
            &mut sp1,
            &mut acc1,
            &mut |d| out1.push(d),
        )
        .unwrap();
        assert!(process_openai_sse_data("[DONE]", &mut sp1, &mut acc1, &mut |d| out1.push(d)).unwrap());
        let res1 = acc1.into_result();
        assert_eq!(res1.thinking.as_deref(), Some("Analyse..."));
        assert_eq!(res1.content, "Ok.");
        assert_eq!(res1.prompt_tokens, 12);
        assert_eq!(res1.completion_tokens, 4);

        // Anthropic thinking + text + cache usage
        let mut sp2 = ThinkStreamSplitter::default();
        let mut acc2 = StreamAccumulator::default();
        process_anthropic_sse_data(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":50}}}"#,
            &mut sp2,
            &mut acc2,
            &mut |_| {},
        )
        .unwrap();
        process_anthropic_sse_data(
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"Plan"}}"#,
            &mut sp2,
            &mut acc2,
            &mut |_| {},
        )
        .unwrap();
        process_anthropic_sse_data(
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Fait."}}"#,
            &mut sp2,
            &mut acc2,
            &mut |_| {},
        )
        .unwrap();
        process_anthropic_sse_data(
            r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":7}}"#,
            &mut sp2,
            &mut acc2,
            &mut |_| {},
        )
        .unwrap();
        let res2 = acc2.into_result();
        assert_eq!(res2.prompt_tokens, 60);
        assert_eq!(res2.cache_read_tokens, 50);
        assert_eq!(res2.completion_tokens, 7);
        assert_eq!(res2.finish_reason.as_deref(), Some("length"));
        assert_eq!(res2.thinking.as_deref(), Some("Plan"));
        assert_eq!(res2.content, "Fait.");

        // Gemini thought + text
        let mut sp3 = ThinkStreamSplitter::default();
        let mut acc3 = StreamAccumulator::default();
        process_gemini_sse_data(
            r#"{"candidates":[{"content":{"parts":[{"thought":true,"text":"Check"},{"text":"Réponse"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":5,"cachedContentTokenCount":8}}"#,
            &mut sp3,
            &mut acc3,
            &mut |_| {},
        )
        .unwrap();
        let res3 = acc3.into_result();
        assert_eq!(res3.thinking.as_deref(), Some("Check"));
        assert_eq!(res3.content, "Réponse");
        assert_eq!(res3.prompt_tokens, 20);
        assert_eq!(res3.completion_tokens, 5);
        assert_eq!(res3.cache_read_tokens, 8);
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
