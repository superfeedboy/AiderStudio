use crate::git_diff;
use crate::mcp;
use crate::models::{
    AgentModelSlot, AgentTraceEvent, AgentTraceStep, AppSettings, AppStateData, ArtifactItem,
    AttachedFile, ChatMessage, FileChangeSummary, McpServerConfig, ModifiedFileDiff, Project,
    PromptStageEvent, SendPromptResult, StackStatus, TokenUsageEvent, ToolExecutionResult,
};
use crate::patcher;
use crate::providers::{self, LlmMessage, LlmRequestOptions, StreamDelta};
use crate::repo_map;
use crate::storage;
use chrono::Utc;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Option<Vec<OllamaModelTag>>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelTag {
    name: String,
}

pub fn list_project_files_internal(project_root: &Path) -> Vec<String> {
    repo_map::list_project_files(project_root, 1000)
}

pub const DEFAULT_AIDERIGNORE: &str = r#"# AiderStudio ignore pattern to preserve 8K context
node_modules/
dist/
build/
target/
.git/
.svn/
.hg/
coverage/
*.lock
package-lock.json
pnpm-lock.yaml
yarn.lock
Cargo.lock
*.log
*.sqlite
*.sqlite3
*.db
*.wasm
*.min.js
*.min.css
*.map
.env*
.DS_Store
Thumbs.db
tmp/
temp/
"#;

pub async fn ensure_aiderignore(root_path: &Path) -> Result<bool, String> {
    let ignore_path = root_path.join(".aiderignore");
    if !ignore_path.exists() {
        tokio::fs::write(&ignore_path, DEFAULT_AIDERIGNORE)
            .await
            .map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn has_aiderignore(root_path: &Path) -> bool {
    root_path.join(".aiderignore").exists()
}

#[derive(Debug, Clone)]
pub struct TestVerificationResult {
    pub ran: bool,
    pub command: String,
    pub success: bool,
    pub output_summary: String,
}

pub fn synthesize_error_log(raw_err: &str) -> String {
    if let Some(api_msg) = providers::extract_api_error_message(raw_err) {
        return api_msg;
    }

    let mut clean_lines = Vec::new();
    for line in raw_err.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.contains("Retrying in ") {
            continue;
        }
        if trimmed.contains("ERR_CONNECTION_REFUSED") {
            clean_lines.push(
                "Connexion locale refusée : le serveur ou service cible n'est pas actif."
                    .to_string(),
            );
            break;
        }
        if trimmed.contains("error TS")
            || trimmed.contains("error:")
            || trimmed.contains("FAILED")
            || trimmed.contains("503")
            || trimmed.contains("429")
            || trimmed.contains("404")
            || trimmed.contains("403")
            || trimmed.contains("401")
            || trimmed.contains("ServiceUnavailable")
            || trimmed.contains("RateLimit")
            || trimmed.contains("RESOURCE_EXHAUSTED")
            || trimmed.contains("UNAVAILABLE")
            || trimmed.contains("Quota")
            || trimmed.contains("NotFound")
            || trimmed.contains("AuthenticationError")
            || trimmed.contains("BadRequest")
            || trimmed.contains("Délai dépassé")
            || trimmed.contains("\"message\"")
            || trimmed.contains("\"status\"")
            || trimmed.contains("\"code\"")
        {
            clean_lines.push(trimmed.to_string());
        }
    }
    if clean_lines.is_empty() {
        truncate_chars(raw_err, 450)
    } else {
        clean_lines.into_iter().take(6).collect::<Vec<_>>().join("\n")
    }
}

fn record_trace_step(
    app: &AppHandle,
    conversation_id: &str,
    steps: &mut Vec<AgentTraceStep>,
    step: AgentTraceStep,
) {
    if let Some(existing) = steps.iter_mut().find(|s| s.role == step.role) {
        *existing = step.clone();
    } else {
        steps.push(step.clone());
    }
    let _ = app.emit(
        "aider:trace",
        AgentTraceEvent {
            conversation_id: conversation_id.to_string(),
            step,
        },
    );
}

pub async fn run_test_verifier(project_root: &Path) -> TestVerificationResult {
    // 1. Cargo.toml -> cargo check
    if project_root.join("Cargo.toml").exists() {
        let res = tokio::process::Command::new("cargo")
            .arg("check")
            .current_dir(project_root)
            .output()
            .await;
        if let Ok(out) = res {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let summary = if out.status.success() {
                "Compilation Cargo check validée avec succès (0 erreur).".to_string()
            } else {
                let err_lines: Vec<&str> = stderr
                    .lines()
                    .filter(|l| l.contains("error:") || l.contains("error["))
                    .take(6)
                    .collect();
                if err_lines.is_empty() {
                    truncate_chars(&stderr, 320)
                } else {
                    err_lines.join("\n")
                }
            };
            return TestVerificationResult {
                ran: true,
                command: "cargo check".to_string(),
                success: out.status.success(),
                output_summary: summary,
            };
        }
    }

    // 2. package.json -> check vue-tsc / type-check
    let pkg_json = project_root.join("package.json");
    if pkg_json.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&pkg_json).await {
            let has_vue_tsc = content.contains("vue-tsc");
            let has_type_check = content.contains("type-check");

            let cmd_name = if has_vue_tsc {
                "npx vue-tsc --noEmit"
            } else if has_type_check {
                "npm run type-check"
            } else {
                ""
            };

            if !cmd_name.is_empty() {
                let parts: Vec<&str> = cmd_name.split_whitespace().collect();
                #[cfg(not(target_os = "windows"))]
                let mut cmd = tokio::process::Command::new(parts[0]);
                #[cfg(target_os = "windows")]
                let mut cmd = {
                    let mut c = tokio::process::Command::new(format!("{}.cmd", parts[0]));
                    c.creation_flags(0x08000000);
                    c
                };
                cmd.args(&parts[1..]).current_dir(project_root);

                if let Ok(out) = cmd.output().await {
                    let combined = format!(
                        "{}\n{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let summary = if out.status.success() {
                        format!("Vérification TypeScript ({cmd_name}) validée sans erreur.")
                    } else {
                        let err_lines: Vec<&str> = combined
                            .lines()
                            .filter(|l| l.contains("error TS") || l.contains("error:"))
                            .take(6)
                            .collect();
                        if err_lines.is_empty() {
                            truncate_chars(&combined, 320)
                        } else {
                            err_lines.join("\n")
                        }
                    };
                    return TestVerificationResult {
                        ran: true,
                        command: cmd_name.to_string(),
                        success: out.status.success(),
                        output_summary: summary,
                    };
                }
            }
        }
    }

    TestVerificationResult {
        ran: false,
        command: String::new(),
        success: true,
        output_summary: "Aucun runner de test ou compilateur strict à exécuter (étape validée)."
            .to_string(),
    }
}

pub fn strip_code_blocks(text: &str) -> String {
    if let Ok(re) = Regex::new(r"```[^\n]*\n[\s\S]*?```") {
        re.replace_all(text, "").to_string()
    } else {
        text.to_string()
    }
}

pub async fn execute_local_tool_internal(
    project_root: &Path,
    name: &str,
    args: &serde_json::Value,
) -> Result<ToolExecutionResult, String> {
    match name {
        "write_file" | "create_file" => {
            let path_str = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Paramètre 'path' manquant")?;
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let clean_path = patcher::sanitize_rel_path(path_str)
                .ok_or_else(|| "Chemin de fichier invalide ou non autorisé".to_string())?;
            let target_path = project_root.join(&clean_path);
            if let Some(parent) = target_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| e.to_string())?;
            }

            let final_content =
                if content.contains("<<<<<<< SEARCH") && content.contains(">>>>>>> REPLACE") {
                    if target_path.exists() {
                        let orig = tokio::fs::read_to_string(&target_path)
                            .await
                            .unwrap_or_default();
                        let patched_res = patcher::parse_edit_blocks(content, &[clean_path.clone()], "")
                            .into_iter()
                            .fold(Ok(orig.clone()), |acc, blk| match (acc, blk) {
                                (Ok(cur), patcher::EditBlock::SearchReplace { search, replace, .. }) => {
                                    patcher::apply_search_replace_fuzzy(&cur, &search, &replace)
                                }
                                (res, _) => res,
                            });
                        patched_res.unwrap_or_else(|_| patcher::extract_clean_replace_code(content))
                    } else {
                        patcher::extract_clean_replace_code(content)
                    }
                } else {
                    content.to_string()
                };

            tokio::fs::write(&target_path, &final_content)
                .await
                .map_err(|e| e.to_string())?;
            Ok(ToolExecutionResult {
                tool: name.to_string(),
                success: true,
                message: format!(
                    "Fichier '{}' écrit avec succès ({} octets)",
                    clean_path,
                    final_content.len()
                ),
                path: Some(clean_path),
            })
        }
        "read_file" => {
            let path_str = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Paramètre 'path' manquant")?;
            let clean_path = path_str
                .trim()
                .trim_matches('`')
                .trim_start_matches("./")
                .trim()
                .replace('\\', "/");
            if clean_path.is_empty() || clean_path.contains("..") || clean_path.starts_with('/') {
                return Err("Chemin relatif non autorisé".to_string());
            }
            let target_path = project_root.join(&clean_path);
            let content = tokio::fs::read_to_string(&target_path)
                .await
                .map_err(|e| e.to_string())?;
            Ok(ToolExecutionResult {
                tool: name.to_string(),
                success: true,
                message: content,
                path: Some(clean_path),
            })
        }
        "search_code" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("Paramètre 'query' manquant")?;
            let path_filter = args.get("path_filter").and_then(|v| v.as_str());
            let out = repo_map::search_in_project(project_root, query, path_filter, 40)?;
            Ok(ToolExecutionResult {
                tool: name.to_string(),
                success: true,
                message: out,
                path: None,
            })
        }
        "run_command" => {
            let cmd_str = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("Paramètre 'command' manquant")?;

            #[cfg(target_os = "windows")]
            let mut cmd = {
                let mut c = tokio::process::Command::new("cmd");
                c.args(["/C", cmd_str]);
                c.creation_flags(0x08000000);
                c
            };

            #[cfg(not(target_os = "windows"))]
            let mut cmd = {
                let mut c = tokio::process::Command::new("sh");
                c.args(["-c", cmd_str]);
                c
            };

            cmd.current_dir(project_root);
            let output = tokio::time::timeout(std::time::Duration::from_secs(45), cmd.output())
                .await
                .map_err(|_| "Timeout lors de l'exécution de la commande (45s)".to_string())?
                .map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stderr.is_empty() {
                stdout.to_string()
            } else if stdout.is_empty() {
                stderr.to_string()
            } else {
                format!("{}\n{}", stdout, stderr)
            };
            Ok(ToolExecutionResult {
                tool: name.to_string(),
                success: output.status.success(),
                message: combined,
                path: None,
            })
        }
        "list_files" => {
            let files = list_project_files_internal(project_root);
            Ok(ToolExecutionResult {
                tool: name.to_string(),
                success: true,
                message: serde_json::to_string(&files).unwrap_or_default(),
                path: None,
            })
        }
        "delete_file" => {
            let path_str = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Paramètre 'path' manquant")?;
            let clean_path = path_str
                .trim()
                .trim_matches('`')
                .trim_start_matches("./")
                .trim()
                .replace('\\', "/");
            if clean_path.is_empty() || clean_path.contains("..") || clean_path.starts_with('/') {
                return Err("Chemin relatif non autorisé".to_string());
            }
            let target = project_root.join(&clean_path);
            if target.exists() {
                tokio::fs::remove_file(&target)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok(ToolExecutionResult {
                tool: name.to_string(),
                success: true,
                message: format!("Fichier '{}' supprimé", clean_path),
                path: Some(clean_path),
            })
        }
        other => Err(format!("Outil inconnu : {}", other)),
    }
}

pub fn resolve_binary(default_name: &str, custom: &str) -> Option<String> {
    let candidates: Vec<String> = if !custom.trim().is_empty() && custom.trim() != default_name {
        vec![custom.trim().to_string(), default_name.to_string()]
    } else {
        vec![default_name.to_string()]
    };

    for cand in &candidates {
        let p = Path::new(cand);
        if p.is_absolute() && p.exists() {
            return Some(cand.clone());
        }
    }

    let mut search_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(path_env) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_env) {
            search_dirs.push(dir);
        }
    }

    if let Some(home) = dirs::home_dir() {
        search_dirs.push(home.join(".local").join("bin"));
        search_dirs.push(home.join(".cargo").join("bin"));
        #[cfg(target_os = "macos")]
        {
            search_dirs.push(PathBuf::from("/opt/homebrew/bin"));
            search_dirs.push(PathBuf::from("/usr/local/bin"));
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(local) = dirs::data_local_dir() {
                search_dirs.push(local.join("Programs").join("Ollama"));
            }
        }
    }

    for cand in &candidates {
        let exe_names: Vec<String> = {
            #[cfg(target_os = "windows")]
            {
                if cand.ends_with(".exe") {
                    vec![cand.clone()]
                } else {
                    vec![format!("{}.exe", cand), format!("{}.cmd", cand), cand.clone()]
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                vec![cand.clone()]
            }
        };

        for dir in &search_dirs {
            for exe in &exe_names {
                let full = dir.join(exe);
                if full.is_file() {
                    return Some(full.to_string_lossy().to_string());
                }
            }
        }
    }

    None
}

pub async fn check_stack_status(settings: &AppSettings) -> StackStatus {
    let ollama_bin = resolve_binary("ollama", "ollama");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1200))
        .build()
        .ok();

    let mut ollama_running = false;
    let mut available_models = Vec::new();
    let base = settings.ollama_url.trim_end_matches('/');

    if let Some(c) = client {
        if let Ok(resp) = c.get(format!("{base}/api/tags")).send().await {
            if resp.status().is_success() {
                ollama_running = true;
                if let Ok(parsed) = resp.json::<OllamaTagsResponse>().await {
                    if let Some(models) = parsed.models {
                        available_models = models.into_iter().map(|m| m.name).collect();
                    }
                }
            }
        }
    }

    let target = settings.ollama_model.trim();
    let model_available = available_models
        .iter()
        .any(|m| m == target || m.starts_with(&format!("{target}:")));

    StackStatus {
        native_engine_ready: true,
        ollama_installed: ollama_bin.is_some(),
        ollama_running,
        model_available,
        available_models,
    }
}

pub async fn start_ollama_service(settings: &AppSettings) -> anyhow::Result<StackStatus> {
    let current = check_stack_status(settings).await;
    if current.ollama_running {
        return Ok(current);
    }

    let ollama_bin = resolve_binary("ollama", "ollama")
        .ok_or_else(|| anyhow::anyhow!("Binaire `ollama` introuvable dans le PATH."))?;

    Command::new(ollama_bin)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    for _ in 0..8 {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let next = check_stack_status(settings).await;
        if next.ollama_running {
            return Ok(next);
        }
    }

    Ok(check_stack_status(settings).await)
}

fn emit_stage(
    app: &AppHandle,
    conversation_id: &str,
    stage_index: u8,
    stage_label: &str,
    detail: &str,
    status: &str,
) {
    let _ = app.emit(
        "aider:stage",
        PromptStageEvent {
            conversation_id: conversation_id.to_string(),
            stage_index,
            stage_total: 5,
            stage_label: stage_label.to_string(),
            detail: detail.to_string(),
            status: status.to_string(),
        },
    );
}

fn extract_artifacts_from_markdown(text: &str) -> Vec<ArtifactItem> {
    let re = Regex::new(r"```([a-zA-Z0-9_+-]*)\n([\s\S]*?)```").expect("valid fence regex");
    let file_hint_re = Regex::new(r"[A-Za-z0-9_./-]+\.[A-Za-z0-9]{1,8}").expect("valid path regex");
    let now = Utc::now().to_rfc3339();
    let mut items = Vec::new();
    for (idx, cap) in re.captures_iter(text).enumerate() {
        let full_match = cap.get(0).unwrap();
        let before = &text[..full_match.start()];
        let prev_line = before
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .trim_start_matches('#')
            .trim()
            .trim_matches('`');

        let lang = cap
            .get(1)
            .map(|m| m.as_str().trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("text")
            .to_string();
        let code = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("").to_string();
        if code.len() < 12 {
            continue;
        }
        if code.contains("<<<<<<< SEARCH") || code.contains(">>>>>>> REPLACE") {
            continue;
        }
        if code.starts_with("{\"name\":") && code.contains("\"arguments\":") {
            continue;
        }
        let ext = match lang.as_str() {
            "rust" | "rs" => "rs",
            "typescript" | "ts" => "ts",
            "javascript" | "js" => "js",
            "vue" => "vue",
            "python" | "py" => "py",
            "json" => "json",
            "css" => "css",
            _ => "txt",
        };
        let inferred_path = file_hint_re
            .find(prev_line)
            .map(|m| m.as_str().trim_matches('`').to_string());
        let title = inferred_path
            .as_deref()
            .and_then(|p| Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| format!("snippet_{}.{}", idx + 1, ext));

        items.push(ArtifactItem {
            id: Uuid::new_v4().to_string(),
            title,
            language: lang,
            file_path: inferred_path,
            content: code,
            created_at: now.clone(),
        });
    }
    items
}

fn effective_num_ctx(settings: &AppSettings) -> usize {
    providers::effective_ollama_num_ctx(settings)
}

async fn record_external_api_usage(
    app: &AppHandle,
    state_lock: &Arc<Mutex<AppStateData>>,
    provider: &str,
    prompt_tokens: usize,
    completion_tokens: usize,
) {
    if provider == "ollama" || (prompt_tokens == 0 && completion_tokens == 0) {
        return;
    }
    let total = prompt_tokens + completion_tokens;
    let updated_usage = {
        let mut guard = state_lock.lock().await;
        let today = crate::models::today_date_string();
        if guard.api_usage.date != today {
            guard.api_usage.date = today;
            guard.api_usage.by_provider.clear();
        }
        let entry = guard
            .api_usage
            .by_provider
            .entry(provider.to_string())
            .or_default();
        entry.prompt_tokens += prompt_tokens;
        entry.completion_tokens += completion_tokens;
        entry.total_tokens += total;
        entry.calls += 1;
        let snapshot = guard.api_usage.clone();
        let _ = storage::save_state(&guard).await;
        snapshot
    };
    let _ = app.emit("aider:api_usage", updated_usage);
}

fn slot_for_role<'a>(role: &str, settings: &'a AppSettings) -> &'a AgentModelSlot {
    match role {
        "architect" => &settings.agent_routing.architect,
        "cartographer" => &settings.agent_routing.cartographer,
        "designer" => &settings.agent_routing.designer,
        "refactoring" => &settings.agent_routing.refactoring,
        "tester" => &settings.agent_routing.tester,
        "synthesizer" => &settings.agent_routing.synthesizer,
        _ => &settings.agent_routing.coder,
    }
}

pub fn resolve_agent_slot(role: &str, settings: &AppSettings) -> (String, String) {
    let target = settings.active_target.trim();
    if !target.is_empty() && target != "agent" {
        if let Some((prov, mdl)) = target.split_once(':') {
            let p = prov.trim();
            let m = mdl.trim();
            if !p.is_empty() && !m.is_empty() {
                if p == "ollama" || providers::find_provider_key(settings, p).is_some() {
                    return (p.to_string(), m.to_string());
                }
            }
        }
    }

    let slot = slot_for_role(role, settings);
    let p = slot.provider.trim();
    if p.is_empty() || p == "ollama" {
        let mdl = if slot.model.trim().is_empty() {
            settings.ollama_model.trim().to_string()
        } else {
            slot.model.trim().to_string()
        };
        return ("ollama".to_string(), mdl);
    }

    if providers::find_provider_key(settings, p).is_some() {
        let mdl = if !slot.model.trim().is_empty() {
            slot.model.trim().to_string()
        } else {
            providers::find_provider_config(settings, p)
                .and_then(|c| c.available_models.first().cloned())
                .unwrap_or_default()
        };
        if !mdl.is_empty() {
            return (p.to_string(), mdl);
        }
    }

    ("ollama".to_string(), settings.ollama_model.trim().to_string())
}

fn is_doc_focused_prompt(lower: &str) -> bool {
    let doc_markers = [
        "readme",
        "changelog",
        "contributing",
        "documentation",
        "doc technique",
        ".md",
    ];
    let explicit_ui_code = [
        "css",
        "scss",
        "tailwind",
        "composant",
        "component",
        "sidebar",
        "bouton",
        "page web",
        "interface",
    ];
    doc_markers.iter().any(|k| lower.contains(k))
        && !explicit_ui_code.iter().any(|k| lower.contains(k))
}

pub fn is_global_or_doc_task(user_prompt: &str, target_files: &[String]) -> bool {
    let has_doc_target = target_files.iter().any(|f| {
        let norm = f.trim().replace('\\', "/").to_lowercase();
        let base = norm.rsplit('/').next().unwrap_or(&norm);
        matches!(
            base,
            "readme.md" | "changelog.md" | "contributing.md" | "architecture.md"
        ) || norm.starts_with("docs/")
            || (!norm.contains('/') && norm.ends_with(".md"))
    });
    if has_doc_target {
        return true;
    }

    let lower = user_prompt.to_lowercase();
    let global_or_doc_kw = [
        "readme",
        "documentation",
        "changelog",
        "contributing",
        "guide d'installation",
        "guide d'utilisation",
        "architecture",
        "vue d'ensemble",
        "audit",
        "stack technique",
        "tout le projet",
        "ensemble du projet",
        "codebase",
        "inventaire",
    ];
    global_or_doc_kw.iter().any(|k| lower.contains(k))
}

pub fn classify_prompt_role(user_prompt: &str) -> (&'static str, bool) {
    let lower = user_prompt.to_lowercase();

    let test_kw = [
        "test",
        "vitest",
        "jest",
        "cargo test",
        "unitaire",
        "intégration",
        "integration",
        "couverture",
    ];
    let design_kw = [
        "design",
        "css",
        "style",
        "interface",
        "ui",
        "ux",
        "layout",
        "menu",
        "sidebar",
        "thème",
        "theme",
        "couleur",
        "color",
        "responsive",
        "moderne",
        "bouton",
        "animation",
        "tailwind",
        "hauteur",
        "largeur",
        "centré",
        "visuel",
        "look",
        "page web",
    ];
    let refac_kw = [
        "refactor",
        "nettoie",
        "factorise",
        "simplifie",
        "optimise",
        "découpe",
        "réorganise",
        "clean",
        "typage",
    ];
    let feature_kw = [
        "feature",
        "fonctionnalité",
        "endpoint",
        "implémente",
        "implement",
        "ajoute un ",
        "ajoute une ",
        "créé un ",
        "crée un ",
        "nouvelle fonctionnalité",
    ];

    let wants_tests = test_kw.iter().any(|k| lower.contains(k));
    if !is_doc_focused_prompt(&lower) && design_kw.iter().any(|k| lower.contains(k)) {
        return ("designer", wants_tests);
    }
    if refac_kw.iter().any(|k| lower.contains(k)) {
        return ("refactoring", wants_tests);
    }
    if wants_tests && !feature_kw.iter().any(|k| lower.contains(k)) {
        return ("tester", true);
    }
    ("coder", wants_tests)
}

pub fn augment_design_target_files(all_files: &[String], matched: &mut Vec<String>) {
    let has_css = matched
        .iter()
        .any(|f| f.ends_with(".css") || f.ends_with(".scss"));
    if !has_css && matched.len() < 3 {
        let css_candidates = [
            "src/style.css",
            "src/assets/main.css",
            "src/index.css",
            "src/app.css",
        ];
        for cand in css_candidates {
            if let Some(found) = all_files.iter().find(|f| f.eq_ignore_ascii_case(cand)) {
                if !matched.contains(found) {
                    matched.push(found.clone());
                    break;
                }
            }
        }
    }

    if matched.len() < 3 {
        for app_cand in ["src/App.vue", "src/App.tsx", "src/App.jsx"] {
            if let Some(found) = all_files.iter().find(|f| f.eq_ignore_ascii_case(app_cand)) {
                if !matched.contains(found) {
                    matched.push(found.clone());
                    break;
                }
            }
        }
    }
}

fn role_display_name(role: &str) -> &'static str {
    match role {
        "architect" => "1. Architecte",
        "cartographer" => "2. Cartographe",
        "designer" => "3. Designer",
        "refactoring" => "3. Refactoring",
        "tester" => "4. Tests",
        "synthesizer" => "5. Synthétiseur",
        _ => "3. Codeur",
    }
}

fn compact_conv_ctx_for_prompt(
    conv_ctx: &str,
    max_custom_chars: usize,
    max_recent_bullets: usize,
) -> String {
    let mut custom_lines = Vec::new();
    let mut bullet_lines = Vec::new();
    for raw in conv_ctx.lines() {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("• [") {
            bullet_lines.push(t.to_string());
        } else {
            custom_lines.push(t.to_string());
        }
    }

    let mut sections = Vec::new();
    if !custom_lines.is_empty() {
        sections.push(truncate_chars(&custom_lines.join("\n"), max_custom_chars));
    }
    if !bullet_lines.is_empty() && max_recent_bullets > 0 {
        let start = bullet_lines.len().saturating_sub(max_recent_bullets);
        sections.push(bullet_lines[start..].join("\n"));
    }
    sections.join("\n")
}

pub fn update_rolling_conversation_context(
    prev_conv_ctx: &str,
    new_entry: &str,
    max_bullets: usize,
) -> String {
    let mut custom_lines = Vec::new();
    let mut bullets = Vec::new();
    for raw in prev_conv_ctx.lines() {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("• [") {
            bullets.push(t.to_string());
        } else {
            custom_lines.push(t.to_string());
        }
    }

    let trimmed_entry = new_entry.trim();
    if !trimmed_entry.is_empty() {
        bullets.push(trimmed_entry.to_string());
    }
    if bullets.len() > max_bullets {
        let excess = bullets.len() - max_bullets;
        bullets.drain(0..excess);
    }

    let mut all_lines = custom_lines;
    all_lines.extend(bullets);
    let combined = all_lines.join("\n");
    if combined.chars().count() <= 2500 {
        return combined;
    }

    let mut final_custom = Vec::new();
    let mut final_bullets = Vec::new();
    for raw in combined.lines() {
        if raw.starts_with("• [") {
            final_bullets.push(raw.to_string());
        } else {
            final_custom.push(raw.to_string());
        }
    }
    while final_bullets.len() > 1 {
        let test = [final_custom.clone(), final_bullets.clone()]
            .concat()
            .join("\n");
        if test.chars().count() <= 2500 {
            return test;
        }
        final_bullets.remove(0);
    }
    truncate_chars(&[final_custom, final_bullets].concat().join("\n"), 2500)
}

pub fn list_source_files_for_cartographer(project_root: &Path) -> Vec<String> {
    repo_map::list_project_files(project_root, 1000)
}

fn match_candidate_in_files(
    candidate: &str,
    prompt_lower: &str,
    all_files: &[String],
) -> Option<String> {
    let cand = candidate.trim_start_matches('@').trim_start_matches("./").trim();
    if cand.is_empty() || cand.starts_with("http") || !cand.contains('.') || cand.ends_with('.') {
        return None;
    }
    let ext = cand.rsplit('.').next().unwrap_or("");
    if ext.is_empty() || ext.len() > 8 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }

    if let Some(found) = all_files.iter().find(|f| f.eq_ignore_ascii_case(cand)) {
        return Some(found.clone());
    }

    let cand_lower = cand.to_lowercase();
    let suffix = format!("/{cand_lower}");
    if let Some(found) = all_files.iter().find(|f| f.to_lowercase().ends_with(&suffix)) {
        return Some(found.clone());
    }

    let mut basename_matches: Vec<&String> = all_files
        .iter()
        .filter(|f| {
            let base = f.rsplit('/').next().unwrap_or(f.as_str());
            base.eq_ignore_ascii_case(cand)
        })
        .collect();

    if basename_matches.is_empty() {
        return None;
    }
    basename_matches.sort_by_key(|f| {
        let parent_mentioned = f
            .rsplit('/')
            .nth(1)
            .map(|dir| !dir.is_empty() && prompt_lower.contains(&dir.to_lowercase()))
            .unwrap_or(false);
        (!parent_mentioned, f.len())
    });
    Some(basename_matches[0].clone())
}

pub fn match_files_deterministically(
    user_prompt: &str,
    attachments: Option<&[AttachedFile]>,
    all_files: &[String],
) -> (Vec<String>, bool) {
    let mut matched: Vec<String> = Vec::new();
    let prompt_lower = user_prompt.to_lowercase();

    for raw_tok in user_prompt.split(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '[' | ']' | '"' | '\'' | '`')
    }) {
        let cleaned = raw_tok.trim_end_matches(|c: char| matches!(c, '.' | ':' | '?' | '!'));
        if let Some(found) = match_candidate_in_files(cleaned, &prompt_lower, all_files) {
            if !matched.contains(&found) {
                matched.push(found);
                if matched.len() >= 3 {
                    break;
                }
            }
        }
    }

    if matched.len() < 3 {
        if let Some(att_list) = attachments {
            for att in att_list {
                if !att.is_image {
                    if let Some(found) =
                        match_candidate_in_files(&att.name, &prompt_lower, all_files)
                    {
                        if !matched.contains(&found) {
                            matched.push(found);
                            if matched.len() >= 3 {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    if !matched.is_empty() {
        return (matched, true);
    }

    let mut add_if_exists = |cand: &str| {
        if matched.len() >= 2 {
            return;
        }
        if let Some(found) = all_files.iter().find(|f| f.eq_ignore_ascii_case(cand)) {
            if !matched.contains(found) {
                matched.push(found.clone());
            }
        }
    };

    if prompt_lower.contains("vite") {
        if all_files.iter().any(|f| f.eq_ignore_ascii_case("vite.config.ts")) {
            add_if_exists("vite.config.ts");
        } else {
            add_if_exists("vite.config.js");
        }
        if ["script", "start", "démarre", "demarre", "serveur", "server", "build", "dev"]
            .iter()
            .any(|kw| prompt_lower.contains(kw))
        {
            add_if_exists("package.json");
        }
    }
    if prompt_lower.contains("tauri") {
        add_if_exists("src-tauri/tauri.conf.json");
    }
    if prompt_lower.contains("tsconfig") {
        add_if_exists("tsconfig.json");
    }
    if prompt_lower.contains("cargo") {
        add_if_exists("src-tauri/Cargo.toml");
    }
    if prompt_lower.contains("package.json")
        || prompt_lower.contains("dépendance")
        || prompt_lower.contains("dependance")
        || prompt_lower.contains("dependencies")
    {
        add_if_exists("package.json");
    }

    (matched, false)
}

fn filter_files_by_size_budget(
    project_root: &Path,
    candidates: Vec<String>,
    max_files: usize,
    max_single_bytes: u64,
    max_total_bytes: u64,
) -> Vec<String> {
    let mut kept = Vec::new();
    let mut total_bytes = 0u64;

    for rel in candidates {
        if kept.len() >= max_files {
            break;
        }
        if rel.contains("..") {
            continue;
        }
        let full = project_root.join(&rel);
        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let size = meta.len();
        if size > max_single_bytes || (total_bytes > 0 && total_bytes + size > max_total_bytes) {
            continue;
        }
        total_bytes += size;
        kept.push(rel);
    }

    kept
}

pub fn collect_structural_context_files(
    project_root: &Path,
    all_files: &[String],
    target_files: &[String],
    is_cloud_exec: bool,
    is_global_task: bool,
    exec_num_ctx: usize,
) -> Vec<String> {
    if !is_global_task && !(is_cloud_exec && target_files.is_empty()) {
        return Vec::new();
    }

    let (max_files, max_single_bytes, max_total_bytes) = if is_cloud_exec {
        (10usize, 32_768u64, 98_304u64)
    } else if exec_num_ctx >= 16_384 {
        (4usize, 12_288u64, 24_576u64)
    } else {
        (3usize, 6_144u64, 10_240u64)
    };

    let mut candidates: Vec<String> = Vec::new();
    let push_if_new = |cand: &str, out: &mut Vec<String>| {
        if target_files.iter().any(|t| t.eq_ignore_ascii_case(cand)) {
            return;
        }
        if !out.iter().any(|existing| existing.eq_ignore_ascii_case(cand)) {
            out.push(cand.to_string());
        }
    };

    let manifests = [
        "package.json",
        "Cargo.toml",
        "src-tauri/Cargo.toml",
        "src-tauri/tauri.conf.json",
        "pyproject.toml",
        "requirements.txt",
        "go.mod",
        "vite.config.ts",
        "tsconfig.json",
        "README.md",
        "src/App.vue",
        "src/App.tsx",
        "src/main.ts",
        "src-tauri/src/lib.rs",
        "src-tauri/src/models.rs",
    ];
    for name in manifests {
        if let Some(found) = all_files.iter().find(|f| f.eq_ignore_ascii_case(name)) {
            push_if_new(found, &mut candidates);
        }
    }

    filter_files_by_size_budget(
        project_root,
        candidates,
        max_files,
        max_single_bytes,
        max_total_bytes,
    )
}

#[derive(Debug, Clone)]
struct CartographerPlan {
    target_files: Vec<String>,
    read_only_files: Vec<String>,
    explicit_match: bool,
    is_global_task: bool,
    strategy_note: String,
    cartographer_tokens: Option<(usize, usize)>,
}

async fn run_cartographer(
    project_root: &Path,
    all_files: &[String],
    user_prompt: &str,
    attachments: Option<&[AttachedFile]>,
    settings: &AppSettings,
    carto_provider: &str,
    carto_model: &str,
    exec_provider: &str,
    ollama_running: bool,
    is_design_task: bool,
) -> CartographerPlan {
    if all_files.is_empty() {
        let is_global_task = is_global_or_doc_task(user_prompt, &[]);
        return CartographerPlan {
            target_files: Vec::new(),
            read_only_files: Vec::new(),
            explicit_match: false,
            is_global_task,
            strategy_note: "Projet sans fichiers indexés".to_string(),
            cartographer_tokens: None,
        };
    }

    let is_cloud_exec = exec_provider != "ollama";
    let exec_num_ctx = effective_num_ctx(settings);
    let (max_target_files, max_single_bytes, max_total_bytes) = if is_cloud_exec {
        (6usize, 65_536u64, 163_840u64)
    } else if exec_num_ctx >= 16_384 {
        (4usize, 32_768u64, 57_344u64)
    } else {
        (3usize, 20_480u64, 28_672u64)
    };
    let doc_focused_prompt = is_doc_focused_prompt(&user_prompt.to_lowercase());

    let (mut tier1, explicit) = match_files_deterministically(user_prompt, attachments, all_files);
    if is_design_task && !tier1.is_empty() && !doc_focused_prompt {
        augment_design_target_files(all_files, &mut tier1);
    }
    let budgeted_tier1 = filter_files_by_size_budget(
        project_root,
        tier1,
        max_target_files,
        max_single_bytes,
        max_total_bytes,
    );

    let (target_files, explicit_match, base_note, cartographer_tokens) =
        if !budgeted_tier1.is_empty() {
            let note = if explicit {
                "0 tok déterministe (mention directe)".to_string()
            } else {
                "0 tok déterministe (mot-clé outillage)".to_string()
            };
            (budgeted_tier1, explicit, note, None)
        } else {
            let can_call_llm = if carto_provider == "ollama" {
                ollama_running
            } else {
                providers::find_provider_key(settings, carto_provider).is_some()
            };

            if !can_call_llm || all_files.len() < 2 {
                let mut fallback = Vec::new();
                if is_design_task && !doc_focused_prompt {
                    augment_design_target_files(all_files, &mut fallback);
                }
                let budgeted = filter_files_by_size_budget(
                    project_root,
                    fallback,
                    max_target_files,
                    max_single_bytes,
                    max_total_bytes,
                );
                (
                    budgeted,
                    false,
                    "Ciblage auto · repo-map native".to_string(),
                    None,
                )
            } else if all_files.len() > 400 {
                let mut fallback = Vec::new();
                if is_design_task && !doc_focused_prompt {
                    augment_design_target_files(all_files, &mut fallback);
                }
                let budgeted = filter_files_by_size_budget(
                    project_root,
                    fallback,
                    max_target_files,
                    max_single_bytes,
                    max_total_bytes,
                );
                (
                    budgeted,
                    false,
                    "Dépôt volumineux (>400 fichiers) · repo-map native".to_string(),
                    None,
                )
            } else {
                let mut prioritized = Vec::new();
                let mut others = Vec::new();
                for f in all_files {
                    if f.starts_with("src/") || f.starts_with("src-tauri/src/") || !f.contains('/') {
                        prioritized.push(f.clone());
                    } else {
                        others.push(f.clone());
                    }
                }
                prioritized.extend(others);
                let compact_limit = if is_cloud_exec { 200 } else { 100 };
                let compact_files: Vec<String> =
                    prioritized.into_iter().take(compact_limit).collect();

                let sys = if is_cloud_exec {
                    "Tu es un routeur de fichiers. À partir de la liste des fichiers du projet et de la demande utilisateur, retourne UNIQUEMENT un tableau JSON contenant au maximum 6 chemins relatifs exacts de fichiers nécessaires pour répondre (ex: [\"src/App.vue\"] ou []). Aucun texte ni explication."
                } else {
                    "Tu es un routeur de fichiers. À partir de la liste des fichiers du projet et de la demande utilisateur, retourne UNIQUEMENT un tableau JSON contenant au maximum 3 chemins relatifs exacts de fichiers nécessaires pour répondre (ex: [\"src/App.vue\"] ou []). Aucun texte ni explication."
                };
                let user = format!(
                    "Fichiers:\n{}\n\nDemande:\n{}",
                    compact_files.join("\n"),
                    truncate_chars(user_prompt.trim(), 400)
                );

                match providers::call_provider_oneshot(
                    carto_provider,
                    carto_model,
                    settings,
                    sys,
                    &user,
                    120,
                    0.0,
                )
                .await
                {
                    Ok((reply, p_tok, c_tok)) => {
                        let mut routed = Vec::new();
                        if let (Some(start), Some(end)) = (reply.find('['), reply.rfind(']')) {
                            if start < end {
                                if let Ok(arr) =
                                    serde_json::from_str::<Vec<String>>(&reply[start..=end])
                                {
                                    for cand in arr {
                                        let clean = cand.trim().trim_start_matches("./");
                                        if let Some(canon) =
                                            all_files.iter().find(|f| f.eq_ignore_ascii_case(clean))
                                        {
                                            if !routed.contains(canon) {
                                                routed.push(canon.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if is_design_task && !doc_focused_prompt {
                            augment_design_target_files(all_files, &mut routed);
                        }
                        let budgeted = filter_files_by_size_budget(
                            project_root,
                            routed,
                            max_target_files,
                            max_single_bytes,
                            max_total_bytes,
                        );
                        let note = if budgeted.is_empty() {
                            "Routeur IA (aucun fichier ciblé)".to_string()
                        } else {
                            format!("Routeur IA ({carto_provider})")
                        };
                        (budgeted, false, note, Some((p_tok, c_tok)))
                    }
                    Err(_) => {
                        let mut fallback = Vec::new();
                        if is_design_task && !doc_focused_prompt {
                            augment_design_target_files(all_files, &mut fallback);
                        }
                        let budgeted = filter_files_by_size_budget(
                            project_root,
                            fallback,
                            max_target_files,
                            max_single_bytes,
                            max_total_bytes,
                        );
                        (
                            budgeted,
                            false,
                            "Repli repo-map native".to_string(),
                            None,
                        )
                    }
                }
            }
        };

    let is_global_task = is_global_or_doc_task(user_prompt, &target_files);
    let read_only_files = collect_structural_context_files(
        project_root,
        all_files,
        &target_files,
        is_cloud_exec,
        is_global_task,
        exec_num_ctx,
    );
    let strategy_note = if read_only_files.is_empty() {
        base_note
    } else {
        format!("{base_note} + {} réf. lecture seule", read_only_files.len())
    };

    CartographerPlan {
        target_files,
        read_only_files,
        explicit_match,
        is_global_task,
        strategy_note,
        cartographer_tokens,
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionStrategy {
    pub edit_format: String,
    pub map_tokens: usize,
    pub max_turns: usize,
    pub max_file_lines: usize,
}

pub fn choose_execution_strategy(
    project_root: &Path,
    target_files: &[String],
    explicit_match: bool,
    provider: &str,
    settings: &AppSettings,
) -> ExecutionStrategy {
    let mut max_file_lines = 0usize;
    let mut total_file_lines = 0usize;

    for rel in target_files {
        if let Ok(content) = std::fs::read_to_string(project_root.join(rel)) {
            let lines = content.lines().count();
            max_file_lines = max_file_lines.max(lines);
            total_file_lines += lines;
        }
    }

    let edit_format = match settings.aider_edit_format.trim() {
        "whole" => "whole".to_string(),
        "diff" => "diff".to_string(),
        _ => {
            if provider != "ollama" {
                if !target_files.is_empty() && max_file_lines > 120 {
                    "diff".to_string()
                } else {
                    "whole".to_string()
                }
            } else if target_files.is_empty() || (max_file_lines <= 250 && total_file_lines <= 350)
            {
                "whole".to_string()
            } else {
                "diff".to_string()
            }
        }
    };

    let num_ctx = effective_num_ctx(settings);
    let map_tokens = if provider != "ollama" {
        if target_files.is_empty() {
            2048
        } else if target_files.len() == 1 && explicit_match {
            1024
        } else {
            1536
        }
    } else if num_ctx >= 16_384 {
        if target_files.is_empty() {
            1024
        } else if target_files.len() == 1 && explicit_match {
            256
        } else {
            512
        }
    } else if target_files.len() == 1 && explicit_match {
        0
    } else if !target_files.is_empty() {
        192
    } else {
        384
    };

    let max_turns = if provider != "ollama" {
        5
    } else if num_ctx >= 16_384 {
        3
    } else {
        2
    };

    ExecutionStrategy {
        edit_format,
        map_tokens,
        max_turns,
        max_file_lines,
    }
}

#[derive(Debug, Clone)]
struct ParsedToolCall {
    name: String,
    arguments: serde_json::Value,
}

fn extract_tool_calls(text: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    let xml_re = Regex::new(r"<tool_call>([\s\S]*?)</tool_call>").expect("valid xml regex");
    for cap in xml_re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(m.as_str().trim()) {
                let name = val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    let arguments = val
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    calls.push(ParsedToolCall { name, arguments });
                }
            }
        }
    }

    let md_re =
        Regex::new(r"```(?:json:)?tool_call\r?\n([\s\S]*?)```").expect("valid md tool regex");
    for cap in md_re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(m.as_str().trim()) {
                let name = val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    let arguments = val
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    calls.push(ParsedToolCall { name, arguments });
                }
            }
        }
    }

    calls
}

fn has_unclosed_fence_or_patch(text: &str) -> bool {
    let fence_count = text
        .lines()
        .filter(|l| l.trim_start().starts_with("```"))
        .count();
    if fence_count % 2 == 1 {
        return true;
    }
    let search_count = text.matches("<<<<<<< SEARCH").count();
    let replace_count = text.matches(">>>>>>> REPLACE").count();
    search_count > replace_count
}

fn build_native_system_prompt(
    edit_format: &str,
    active_mcp: &[&McpServerConfig],
    mcp_schemas_note: &str,
) -> String {
    let edit_rules = if edit_format == "diff" {
        "FORMAT D'ÉDITION (`diff`) :\n\
        Pour modifier un fichier existant, écris son chemin relatif exact (sans backtick) suivi d'un bloc SEARCH/REPLACE :\n\
        ### chemin/du/fichier.ext\n\
        ```langage\n\
        <<<<<<< SEARCH\n\
        lignes exactes existantes\n\
        =======\n\
        nouvelles lignes de remplacement\n\
        >>>>>>> REPLACE\n\
        ```\n\
        Pour créer un nouveau fichier inexistant, écris le contenu complet dans le bloc précédé de `### chemin/du/fichier.ext`."
    } else {
        "FORMAT D'ÉDITION (`whole`) :\n\
        Pour créer ou modifier un fichier, écris son chemin relatif exact (sans backtick) suivi du contenu COMPLET et opérationnel du fichier (sans TODO ni ellipsis) :\n\
        ### chemin/du/fichier.ext\n\
        ```langage\n\
        // Contenu complet...\n\
        ```\n\
        Si un fichier est marqué [TRONQUÉ], utilise exclusivement des blocs `<<<<<<< SEARCH / ======= / >>>>>>> REPLACE`."
    };

    let mcp_section = if active_mcp.is_empty() {
        String::new()
    } else {
        let names = active_mcp
            .iter()
            .map(|s| format!("`{}`", s.name))
            .collect::<Vec<_>>()
            .join(", ");
        let mut s = format!(
            "\n\nSERVEURS MCP ACTIFS ({names}) :\n\
            Tu peux interroger un serveur MCP via :\n\
            <tool_call>{{\"name\":\"mcp_call\",\"arguments\":{{\"server\":\"nom_serveur\",\"tool\":\"nom_outil\",\"arguments\":{{}}}}}}</tool_call>"
        );
        if !mcp_schemas_note.trim().is_empty() {
            s.push_str("\n");
            s.push_str(mcp_schemas_note.trim());
        }
        s
    };

    format!(
        "Tu es le moteur de code d'AiderStudio. Réponds TOUJOURS ET EXCLUSIVEMENT EN FRANÇAIS de façon concise et technique.\n\n\
        {edit_rules}\n\n\
        STYLE DE RÉPONSE :\n\
        - Les blocs d'édition de fichiers sont appliqués automatiquement sur le disque puis masqués de la conversation (le diff détaillé apparaît dans un menu dédié).\n\
        - N'écris JAMAIS de phrase d'introduction annonçant un bloc de code (« Voici les modifications nécessaires : », « Voici le code mis à jour : », « Modifiez le fichier comme suit : »).\n\
        - Émets directement les blocs d'édition, puis conclus par un bref résumé autonome au passé composé expliquant concrètement ce qui a été généré ou modifié dans chaque fichier en le citant par un lien Markdown cliquable `[NomFichier](chemin/relatif/NomFichier)` (ex. `[App.vue](src/App.vue)`).\n\
        - Si la demande utilisateur comporte plusieurs tâches ou points distincts, présente ce résumé en liste à puces (`- ...`), une puce par demande traitée avec les fichiers cliquables concernés.\n\
        - N'ajoute JAMAIS de section `### Fichiers modifiés` en fin de message.\n\n\
        OUTILS DISPONIBLES (uniquement si une information manque) :\n\
        <tool_call>{{\"name\":\"read_file\",\"arguments\":{{\"path\":\"src/...\"}}}}</tool_call>\n\
        <tool_call>{{\"name\":\"search_code\",\"arguments\":{{\"query\":\"motif\",\"path_filter\":\"src/\"}}}}</tool_call>\n\
        <tool_call>{{\"name\":\"list_files\",\"arguments\":{{}}}}</tool_call>\n\
        <tool_call>{{\"name\":\"delete_file\",\"arguments\":{{\"path\":\"...\"}}}}</tool_call>\n\
        <tool_call>{{\"name\":\"run_command\",\"arguments\":{{\"command\":\"...\"}}}}</tool_call>\n\
        Les commandes shell à afficher simplement à l'utilisateur vont dans un bloc ```bash ... ``` standard (hors <tool_call>).{mcp_section}"
    )
}

#[allow(clippy::too_many_arguments)]
async fn build_native_initial_user_message(
    project: &Project,
    project_root: &Path,
    conv_ctx: &str,
    user_prompt: &str,
    attachments: Option<&[AttachedFile]>,
    image_base64_list: Vec<String>,
    architect_spec: Option<&str>,
    all_project_files: &[String],
    target_files: &[String],
    read_only_files: &[String],
    repo_map_text: Option<&str>,
    is_cloud: bool,
) -> (LlmMessage, HashSet<String>, HashSet<String>) {
    let mut parts = Vec::new();
    let mut truncated_files = HashSet::new();
    let mut preloaded_files = HashSet::new();

    // Préfixe stable (favorable au Prompt Caching)
    if !project.global_context.trim().is_empty() {
        let limit = if is_cloud { 2000 } else { 1000 };
        parts.push(format!(
            "[Contexte global du projet]\n{}",
            truncate_chars(project.global_context.trim(), limit)
        ));
    }

    if !project.shared_directories.is_empty() {
        parts.push(format!(
            "[Dossiers partagés]\n{}",
            project
                .shared_directories
                .iter()
                .map(|d| format!("- {d}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if is_global_or_doc_task(user_prompt, target_files) && !all_project_files.is_empty() {
        let max_tree = if is_cloud { 60usize } else { 25usize };
        let shown: Vec<&str> = all_project_files.iter().take(max_tree).map(String::as_str).collect();
        let mut tree = shown.join("\n  ");
        if all_project_files.len() > max_tree {
            tree.push_str(&format!(
                "\n  ... (+{} autres fichiers)",
                all_project_files.len() - max_tree
            ));
        }
        parts.push(format!(
            "[Aperçu du dépôt ({} fichiers)]\n  {}",
            all_project_files.len(),
            tree
        ));
    }

    if let Some(rmap) = repo_map_text {
        if !rmap.trim().is_empty() {
            parts.push(format!("[Repo-Map du projet]\n{}", rmap.trim()));
        }
    }

    if !read_only_files.is_empty() {
        let mut ref_sections = Vec::new();
        let mut total_chars = 0usize;
        let max_total = if is_cloud { 45_000 } else { 6_000 };
        let clip_each = if is_cloud { 8_000 } else { 2_500 };
        for rel in read_only_files {
            if total_chars >= max_total {
                break;
            }
            if let Ok(content) = tokio::fs::read_to_string(project_root.join(rel)).await {
                let clipped = truncate_chars(&content, clip_each);
                total_chars += clipped.chars().count();
                ref_sections.push(format!("### {rel}\n```\n{clipped}\n```"));
                preloaded_files.insert(rel.clone());
            }
        }
        if !ref_sections.is_empty() {
            parts.push(format!(
                "[Fichiers de référence (lecture seule)]\n{}",
                ref_sections.join("\n\n")
            ));
        }
    }

    if !target_files.is_empty() {
        let max_targets = if is_cloud { 6 } else { 3 };
        let clip_chars = if is_cloud { 24_000 } else { 12_000 };
        let mut file_sections = Vec::new();
        for rel in target_files.iter().take(max_targets) {
            if let Ok(content) = tokio::fs::read_to_string(project_root.join(rel)).await {
                let char_len = content.chars().count();
                if char_len > clip_chars {
                    truncated_files.insert(rel.clone());
                    let clipped = truncate_chars(&content, clip_chars);
                    file_sections.push(format!(
                        "### {rel} [TRONQUÉ : utilisez SEARCH/REPLACE]\n```\n{clipped}\n```"
                    ));
                } else {
                    file_sections.push(format!("### {rel}\n```\n{content}\n```"));
                }
                preloaded_files.insert(rel.clone());
            }
        }
        if !file_sections.is_empty() {
            parts.push(format!(
                "[Fichiers cibles éditables]\n{}",
                file_sections.join("\n\n")
            ));
        }
    }

    // Suffixe dynamique
    let compact_conv = if is_cloud {
        compact_conv_ctx_for_prompt(conv_ctx, 800, 4)
    } else {
        compact_conv_ctx_for_prompt(conv_ctx, 400, 2)
    };
    if !compact_conv.is_empty() {
        parts.push(format!("[Objectif discussion]\n{compact_conv}"));
    }
    if let Some(spec) = architect_spec {
        if !spec.trim().is_empty() {
            parts.push(format!("[Plan Architecte]\n{}", spec.trim()));
        }
    }
    if let Some(att_list) = attachments {
        for att in att_list {
            if !att.is_image {
                if let Some(content) = &att.content {
                    parts.push(format!("[Fichier joint: {}]\n{}", att.name, content));
                }
            }
        }
    }

    parts.push(format!("[Demande utilisateur]\n{}", user_prompt.trim()));
    let text = parts.join("\n\n");
    let msg = if image_base64_list.is_empty() {
        LlmMessage::user(text)
    } else {
        LlmMessage::user_with_images(text, image_base64_list)
    };
    (msg, truncated_files, preloaded_files)
}

struct NativeLoopResult {
    narrative: String,
    raw_last_response: String,
    thinking: Option<String>,
    tool_results: Vec<ToolExecutionResult>,
    applied_files: Vec<String>,
    prompt_tokens: usize,
    completion_tokens: usize,
    turns_run: usize,
    had_repairs: bool,
    effective_edit_format: String,
}

#[allow(clippy::too_many_arguments)]
async fn execute_native_agent_loop(
    app: &AppHandle,
    state_lock: &Arc<Mutex<AppStateData>>,
    conversation_id: &str,
    project: &Project,
    conv_ctx: &str,
    user_prompt: &str,
    attachments: Option<&[AttachedFile]>,
    image_base64_list: Vec<String>,
    architect_spec: Option<&str>,
    provider: &str,
    model: &str,
    role: &str,
    settings: &AppSettings,
    all_project_files: &[String],
    target_files: &[String],
    read_only_files: &[String],
    strategy: &ExecutionStrategy,
    mcp_servers: &[McpServerConfig],
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<NativeLoopResult> {
    if is_cancelled(cancel_rx) {
        anyhow::bail!("CANCELLED_BY_USER");
    }

    let project_root = Path::new(&project.root_path);
    let is_cloud = provider != "ollama";
    let num_ctx = effective_num_ctx(settings);

    let active_mcp: Vec<&McpServerConfig> = mcp_servers
        .iter()
        .filter(|s| {
            s.enabled
                && (s.project_id.is_none() || s.project_id.as_deref() == Some(project.id.as_str()))
        })
        .collect();

    // Pré-découverte des schémas MCP uniquement si @mcp: est mentionné dans le prompt
    let mut mcp_schemas_note = String::new();
    let lower_prompt = user_prompt.to_lowercase();
    if lower_prompt.contains("@mcp:") && !active_mcp.is_empty() {
        let mut schema_lines = Vec::new();
        for srv in &active_mcp {
            let tag = format!("@mcp:{}", srv.name.to_lowercase());
            let should_probe = lower_prompt.contains(&tag) || active_mcp.len() == 1;
            if !should_probe {
                continue;
            }
            let fut = mcp::list_tools_for_server(srv, project_root);
            let res = tokio::select! {
                biased;
                _ = wait_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                r = tokio::time::timeout(std::time::Duration::from_secs(6), fut) => r,
            };
            if let Ok(Ok(tools)) = res {
                for t in tools.into_iter().take(8) {
                    schema_lines.push(format!(
                        "- Serveur `{}` · outil `{}` : {} (args: {})",
                        srv.name,
                        t.name,
                        truncate_chars(&t.description, 100),
                        truncate_chars(&t.input_schema.to_string(), 180)
                    ));
                }
            }
        }
        if !schema_lines.is_empty() {
            mcp_schemas_note = format!("Outils MCP détectés :\n{}", schema_lines.join("\n"));
        }
    }

    let repo_map_text = if strategy.map_tokens > 0 {
        repo_map::build_repo_map(
            project_root,
            all_project_files,
            target_files,
            read_only_files,
            user_prompt,
            strategy.map_tokens,
        )
    } else {
        None
    };

    let (initial_user_msg, truncated_files, mut already_read) = build_native_initial_user_message(
        project,
        project_root,
        conv_ctx,
        user_prompt,
        attachments,
        image_base64_list,
        architect_spec,
        all_project_files,
        target_files,
        read_only_files,
        repo_map_text.as_deref(),
        is_cloud,
    )
    .await;

    let effective_edit_format = if !truncated_files.is_empty() && strategy.edit_format == "whole" {
        "diff".to_string()
    } else {
        strategy.edit_format.clone()
    };

    let system_prompt =
        build_native_system_prompt(&effective_edit_format, &active_mcp, &mcp_schemas_note);
    let mut messages = vec![LlmMessage::system(system_prompt), initial_user_msg];

    let tool_output_budget = if is_cloud {
        12_000usize
    } else if num_ctx >= 16_384 {
        4_000usize
    } else {
        1_800usize
    };

    let max_output_tokens = if is_cloud {
        8192usize
    } else if num_ctx >= 16_384 {
        6144usize
    } else {
        4096usize
    };

    let ctx_limit = providers::resolve_model_context_limit(provider, model, settings);
    let subagent_label = format!("{} ({})", role_display_name(role), effective_edit_format);

    let mut total_prompt_tokens = 0usize;
    let mut total_completion_tokens = 0usize;
    let mut all_thinking = String::new();
    let mut all_tool_results: Vec<ToolExecutionResult> = Vec::new();
    let mut all_applied_files: Vec<String> = Vec::new();
    let mut seen_applied = HashSet::new();
    let mut last_raw_response = String::new();
    let mut last_non_empty_narrative = String::new();
    let mut pending_continuation: Option<String> = None;
    let mut turns_run = 0usize;
    let mut had_repairs = false;

    for turn in 0..strategy.max_turns {
        if is_cancelled(cancel_rx) {
            anyhow::bail!("CANCELLED_BY_USER");
        }
        turns_run = turn + 1;

        let stream_start = format!(
            "[Tour {}/{}] Appel {}:{} (format {})...",
            turn + 1,
            strategy.max_turns,
            provider,
            model,
            effective_edit_format
        );
        let _ = app.emit("aider:stream", (conversation_id, &stream_start));

        let mut turn_live_content = String::new();
        let mut turn_live_thinking = all_thinking.clone();
        if !turn_live_thinking.is_empty() {
            turn_live_thinking.push_str("\n\n");
        }

        let opts = LlmRequestOptions {
            max_tokens: max_output_tokens,
            temperature: 0.2,
            enable_cache: true,
        };

        let res = providers::call_provider_streaming(
            provider,
            model,
            settings,
            &messages,
            &opts,
            cancel_rx,
            |delta| match delta {
                StreamDelta::Thinking(t) => {
                    turn_live_thinking.push_str(&t);
                    let _ = app.emit("aider:thinking", (conversation_id, turn_live_thinking.trim()));
                }
                StreamDelta::Content(c) => {
                    turn_live_content.push_str(&c);
                    let _ = app.emit("aider:live_chunk", (conversation_id, &turn_live_content));
                }
            },
        )
        .await?;

        if let Some(th) = &res.thinking {
            if !th.trim().is_empty() {
                if !all_thinking.is_empty() {
                    all_thinking.push_str("\n\n");
                }
                all_thinking.push_str(th.trim());
            }
        }

        total_prompt_tokens += res.prompt_tokens;
        total_completion_tokens += res.completion_tokens;

        let turn_tot = res.prompt_tokens + res.completion_tokens;
        if turn_tot > 0 {
            record_external_api_usage(
                app,
                state_lock,
                provider,
                res.prompt_tokens,
                res.completion_tokens,
            )
            .await;
            let _ = app.emit(
                "aider:tokens",
                TokenUsageEvent {
                    conversation_id: conversation_id.to_string(),
                    subagent: subagent_label.clone(),
                    prompt_tokens: res.prompt_tokens,
                    completion_tokens: res.completion_tokens,
                    total_tokens: turn_tot,
                    role: role.to_string(),
                    provider: provider.to_string(),
                    model: model.to_string(),
                    context_limit: ctx_limit,
                },
            );
        }

        let cache_note = if res.cache_read_tokens > 0 {
            format!(", cache: {} tok", res.cache_read_tokens)
        } else {
            String::new()
        };
        let stream_done = format!(
            "[Tour {}/{}] Réponse reçue ({} in / {} out{}).",
            turn + 1,
            strategy.max_turns,
            res.prompt_tokens,
            res.completion_tokens,
            cache_note
        );
        let _ = app.emit("aider:stream", (conversation_id, &stream_done));

        let effective_response = if let Some(prev) = pending_continuation.take() {
            format!("{prev}{}", res.content)
        } else {
            res.content.clone()
        };

        last_raw_response = effective_response.clone();
        messages.push(LlmMessage::assistant(res.content.clone()));

        // 1. Exécuter les <tool_call> (locaux + mcp_call)
        let tool_calls = extract_tool_calls(&res.content);
        let mut tool_outputs = Vec::new();
        let mut wrote_via_tool = false;

        for tc in tool_calls {
            if is_cancelled(cancel_rx) {
                anyhow::bail!("CANCELLED_BY_USER");
            }
            if tc.name == "mcp_call" {
                let server_ref = tc
                    .arguments
                    .get("server")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let tool_name = tc
                    .arguments
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let tool_args = tc
                    .arguments
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));

                let matched_srv = active_mcp
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(server_ref) || s.id == server_ref)
                    .copied();

                if let Some(srv) = matched_srv {
                    let msg_line = format!("[MCP] {} -> {}...", srv.name, tool_name);
                    let _ = app.emit("aider:stream", (conversation_id, &msg_line));
                    let fut = mcp::call_tool_for_server(srv, project_root, tool_name, &tool_args);
                    let call_res = tokio::select! {
                        biased;
                        _ = wait_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
                        r = fut => r,
                    };
                    match call_res {
                        Ok(out) => {
                            let clipped = truncate_chars(&out, tool_output_budget);
                            all_tool_results.push(ToolExecutionResult {
                                tool: format!("mcp:{}:{}", srv.name, tool_name),
                                success: true,
                                message: format!("MCP {}::{} exécuté.", srv.name, tool_name),
                                path: None,
                            });
                            tool_outputs.push(format!(
                                "[MCP {}::{}]\n{}",
                                srv.name, tool_name, clipped
                            ));
                        }
                        Err(e) => {
                            all_tool_results.push(ToolExecutionResult {
                                tool: format!("mcp:{}:{}", srv.name, tool_name),
                                success: false,
                                message: format!("Erreur MCP {}::{} : {}", srv.name, tool_name, e),
                                path: None,
                            });
                            tool_outputs.push(format!(
                                "[Erreur MCP {}::{}] {}",
                                srv.name, tool_name, e
                            ));
                        }
                    }
                } else {
                    tool_outputs.push(format!(
                        "[Erreur MCP] Serveur '{server_ref}' introuvable ou inactif."
                    ));
                }
                continue;
            }

            if tc.name == "read_file" {
                if let Some(raw_p) = tc.arguments.get("path").and_then(|v| v.as_str()) {
                    let norm_p = raw_p
                        .trim()
                        .trim_matches('`')
                        .trim_start_matches("./")
                        .replace('\\', "/");
                    if already_read.contains(&norm_p) {
                        tool_outputs.push(format!(
                            "[read_file: {norm_p}] Déjà présent dans le contexte ci-dessus."
                        ));
                        continue;
                    }
                }
            }

            let log_line = format!("[Outil] {}...", tc.name);
            let _ = app.emit("aider:stream", (conversation_id, &log_line));
            match execute_local_tool_internal(project_root, &tc.name, &tc.arguments).await {
                Ok(tr) => {
                    if tc.name == "write_file" || tc.name == "create_file" {
                        wrote_via_tool = true;
                        if let Some(p) = &tr.path {
                            already_read.remove(p);
                            if seen_applied.insert(p.clone()) {
                                all_applied_files.push(p.clone());
                            }
                        }
                        all_tool_results.push(tr.clone());
                        tool_outputs.push(format!("[{}] {}", tc.name, tr.message));
                    } else if tc.name == "read_file" {
                        if let Some(p) = &tr.path {
                            already_read.insert(p.clone());
                        }
                        let clipped = truncate_chars(&tr.message, tool_output_budget);
                        all_tool_results.push(ToolExecutionResult {
                            tool: tr.tool.clone(),
                            success: tr.success,
                            message: format!(
                                "Lecture '{}' ({} octets)",
                                tr.path.as_deref().unwrap_or(""),
                                tr.message.len()
                            ),
                            path: tr.path.clone(),
                        });
                        tool_outputs.push(format!(
                            "[read_file: {}]\n{}",
                            tr.path.as_deref().unwrap_or(""),
                            clipped
                        ));
                    } else {
                        let clipped = truncate_chars(&tr.message, tool_output_budget);
                        all_tool_results.push(ToolExecutionResult {
                            tool: tr.tool.clone(),
                            success: tr.success,
                            message: if tc.name == "run_command" {
                                format!(
                                    "Commande exécutée (succès={})",
                                    tr.success
                                )
                            } else {
                                format!("Outil '{}' exécuté", tc.name)
                            },
                            path: tr.path.clone(),
                        });
                        tool_outputs.push(format!("[{}]\n{}", tc.name, clipped));
                    }
                }
                Err(e) => {
                    all_tool_results.push(ToolExecutionResult {
                        tool: tc.name.clone(),
                        success: false,
                        message: format!("Erreur {} : {}", tc.name, e),
                        path: None,
                    });
                    tool_outputs.push(format!("[Erreur {}] {}", tc.name, e));
                }
            }
        }

        // 2. Appliquer les blocs de fichiers / SEARCH-REPLACE
        let edit_outcome = patcher::apply_edits_from_response(
            project_root,
            &effective_response,
            target_files,
            &truncated_files,
            user_prompt,
        )
        .await;

        for f in &edit_outcome.applied_files {
            already_read.remove(f);
            if seen_applied.insert(f.clone()) {
                all_applied_files.push(f.clone());
            }
            let patch_line = format!("[Patch] Appliqué sur `{f}`.");
            let _ = app.emit("aider:stream", (conversation_id, &patch_line));
        }
        all_tool_results.extend(edit_outcome.results.clone());

        let turn_cleaned =
            patcher::strip_edit_blocks_for_display(&effective_response, &all_applied_files);
        if !turn_cleaned.trim().is_empty() {
            last_non_empty_narrative = turn_cleaned;
        }

        // 3. Décision de boucle
        if turn + 1 >= strategy.max_turns {
            break;
        }

        if res.finish_reason.as_deref() == Some("length")
            && has_unclosed_fence_or_patch(&effective_response)
        {
            pending_continuation = Some(effective_response);
            let _ = app.emit(
                "aider:stream",
                (conversation_id, "[Continuation] Réponse tronquée par max_tokens, poursuite..."),
            );
            messages.push(LlmMessage::user(
                "Ta réponse a été interrompue par la limite de tokens. Continue exactement là où tu t'es arrêté pour terminer le bloc de code en cours sans répéter ce qui a déjà été écrit.",
            ));
            continue;
        }

        if !edit_outcome.failures.is_empty() {
            had_repairs = true;
            let mut fail_notes = Vec::new();
            for fail in &edit_outcome.failures {
                let _ = app.emit(
                    "aider:stream",
                    (
                        conversation_id,
                        &format!("[Auto-réparation] Échec patch `{}` : {}", fail.path, fail.reason),
                    ),
                );
                let mut item = format!("- Fichier `{}` : {}", fail.path, fail.reason);
                if let (Some(ln), Some(closest)) = (fail.closest_line, &fail.closest_match) {
                    item.push_str(&format!(
                        "\n  Extrait réel autour de la ligne {} :\n```\n{}\n```",
                        ln, closest
                    ));
                }
                fail_notes.push(item);
            }
            messages.push(LlmMessage::user(format!(
                "Un ou plusieurs blocs d'édition ont échoué :\n{}\n\nRéémets UNIQUEMENT les blocs `<<<<<<< SEARCH / ======= / >>>>>>> REPLACE` corrigés en recopiant fidèlement les lignes réelles ci-dessus.",
                fail_notes.join("\n")
            )));
            continue;
        }

        if !tool_outputs.is_empty() && edit_outcome.applied_files.is_empty() && !wrote_via_tool {
            messages.push(LlmMessage::user(format!(
                "[Résultats des outils]\n{}\n\nPoursuis maintenant avec ces informations pour répondre précisément ou appliquer les modifications de fichiers attendues.",
                tool_outputs.join("\n\n")
            )));
            continue;
        }

        break;
    }

    let final_narrative = if last_non_empty_narrative.is_empty() {
        patcher::strip_edit_blocks_for_display(&last_raw_response, &all_applied_files)
    } else {
        last_non_empty_narrative
    };

    Ok(NativeLoopResult {
        narrative: final_narrative,
        raw_last_response: last_raw_response,
        thinking: if all_thinking.trim().is_empty() {
            None
        } else {
            Some(all_thinking)
        },
        tool_results: all_tool_results,
        applied_files: all_applied_files,
        prompt_tokens: total_prompt_tokens,
        completion_tokens: total_completion_tokens,
        turns_run,
        had_repairs,
        effective_edit_format,
    })
}

fn build_compact_diff_summary(modified_files: &[ModifiedFileDiff], max_chars: usize) -> String {
    let mut parts = Vec::new();
    for f in modified_files {
        let mut file_lines = vec![format!(
            "Fichier {} (+{} / -{}, ~{} mod):",
            f.path, f.added, f.removed, f.modified
        )];
        for dl in f.lines.iter().filter(|l| l.kind != "ctx").take(14) {
            let prefix = match dl.kind.as_str() {
                "add" => "+",
                "del" => "-",
                "mod" => "~",
                _ => " ",
            };
            file_lines.push(format!("  {} {}", prefix, truncate_chars(&dl.content, 110)));
        }
        parts.push(file_lines.join("\n"));
    }
    truncate_chars(&parts.join("\n"), max_chars)
}

fn build_files_summary(modified_files: &[ModifiedFileDiff]) -> Vec<FileChangeSummary> {
    modified_files
        .iter()
        .map(|f| FileChangeSummary {
            path: f.path.clone(),
            added: f.added,
            removed: f.removed,
            modified: f.modified,
        })
        .collect()
}

fn push_unique_paths(dest: &mut Vec<String>, paths: &[String]) {
    for p in paths {
        let trimmed = p.trim();
        if !trimmed.is_empty() && !dest.iter().any(|existing| existing == trimmed) {
            dest.push(trimmed.to_string());
        }
    }
}

fn has_multiple_requests(user_prompt: &str) -> bool {
    let norm = user_prompt.replace("\r\n", "\n");
    let trimmed = norm.trim();
    if trimmed.is_empty() {
        return false;
    }

    // 1. Items numérotés (ex. "1. ... 2. ..." ou "1) ... 2) ...")
    if let Ok(num_re) = Regex::new(r"(?m)(?:^|\s)\d+[.)]\s+\S") {
        if num_re.find_iter(trimmed).count() >= 2 {
            return true;
        }
    }

    // 2. Puces multiples en début de ligne
    let bullet_count = trimmed
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("- ") || t.starts_with("* ") || t.starts_with("• ")
        })
        .count();
    if bullet_count >= 2 {
        return true;
    }

    // 3. Plusieurs phrases/lignes contenant un marqueur explicite de demande distincte
    let non_empty_lines = trimmed.lines().filter(|l| !l.trim().is_empty()).count();
    let sentence_breaks = trimmed
        .matches(". ")
        .count()
        + trimmed.matches("! ").count()
        + trimmed.matches("? ").count()
        + trimmed.matches(".\n").count()
        + trimmed.matches("; ").count();

    if non_empty_lines >= 2 || sentence_breaks >= 1 {
        let low = trimmed.to_lowercase();
        let connectors = [
            "par ailleurs",
            "également",
            "en plus",
            "autre point",
            "deuxième",
            "2ème",
            "troisième",
            "3ème",
            "second point",
            "et aussi",
            "aussi,",
            ". puis ",
            ", puis ",
            "; puis ",
            "\npuis ",
            "enfin,",
        ];
        if connectors.iter().any(|c| low.contains(c)) {
            return true;
        }
    }

    false
}

fn file_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn narrative_needs_structured_synthesis(
    narrative: &str,
    turn_applied_files: &[String],
    user_prompt: &str,
    extra_stage_edits: bool,
) -> bool {
    if turn_applied_files.is_empty() {
        return false;
    }
    if extra_stage_edits {
        return true;
    }

    let trimmed = narrative.trim();
    if trimmed.is_empty() {
        return true;
    }

    let low_narr = trimmed.to_lowercase();
    let all_files_mentioned = turn_applied_files.iter().all(|path| {
        let low_path = path.to_lowercase();
        let low_base = file_basename(path).to_lowercase();
        low_narr.contains(&low_path) || low_narr.contains(&low_base)
    });
    if !all_files_mentioned {
        return true;
    }

    if has_multiple_requests(user_prompt) {
        let has_list = trimmed.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("- ")
                || t.starts_with("* ")
                || t.starts_with("• ")
                || Regex::new(r"^\d+[.)]\s+")
                    .map(|re| re.is_match(t))
                    .unwrap_or(false)
        });
        if !has_list {
            return true;
        }
    }

    false
}

fn resolve_exact_or_unique_path<'a>(
    raw_token: &str,
    sorted_paths: &[&'a str],
    unique_bases: &std::collections::HashMap<&'a str, &'a str>,
) -> Option<(&'a str, String)> {
    let mut candidate = raw_token
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\''))
        .replace('\\', "/");
    while let Some(rest) = candidate.strip_prefix("./") {
        candidate = rest.to_string();
    }

    let mut line_fragment = String::new();
    if let Some(hash_idx) = candidate.rfind("#L") {
        let num_part = &candidate[hash_idx + 2..];
        if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit()) {
            line_fragment = format!("#L{num_part}");
            candidate.truncate(hash_idx);
        }
    } else if let Some(colon_idx) = candidate.rfind(':') {
        let num_part = &candidate[colon_idx + 1..];
        if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit()) {
            line_fragment = format!("#L{num_part}");
            candidate.truncate(colon_idx);
        }
    }

    if let Some(&exact) = sorted_paths
        .iter()
        .find(|&&p| p == candidate || p.eq_ignore_ascii_case(&candidate))
    {
        return Some((exact, line_fragment));
    }
    if let Some(&target) = unique_bases.get(candidate.as_str()) {
        return Some((target, line_fragment));
    }
    None
}

fn linkify_plain_segment(
    seg: &str,
    lookup_re: &Option<Regex>,
    lookup_map: &std::collections::HashMap<&str, &str>,
) -> String {
    let Some(re) = lookup_re else {
        return seg.to_string();
    };
    let mut out = String::with_capacity(seg.len() + 32);
    let mut cursor = 0usize;

    for m in re.find_iter(seg) {
        if m.start() < cursor {
            continue;
        }
        let prev_ok = seg[..m.start()]
            .chars()
            .next_back()
            .map(|c| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '/' | '\\' | '-')))
            .unwrap_or(true);
        if !prev_ok {
            continue;
        }

        let after_slice = &seg[m.end()..];
        let next_ch = after_slice.chars().next();
        let next_ok = match next_ch {
            None => true,
            Some('.') => after_slice
                .chars()
                .nth(1)
                .map(|c2| !c2.is_alphanumeric())
                .unwrap_or(true),
            Some(c) => !(c.is_alphanumeric() || matches!(c, '_' | '/' | '\\' | '-')),
        };
        if !next_ok {
            continue;
        }

        if let Some(&target_path) = lookup_map.get(m.as_str()) {
            let base = file_basename(target_path);
            out.push_str(&seg[cursor..m.start()]);
            out.push_str(&format!("[{base}]({target_path})"));
            cursor = m.end();
        }
    }

    out.push_str(&seg[cursor..]);
    out
}

fn linkify_inline_prose(
    chunk: &str,
    sorted_paths: &[&str],
    unique_bases: &std::collections::HashMap<&str, &str>,
    lookup_re: &Option<Regex>,
    lookup_map: &std::collections::HashMap<&str, &str>,
) -> String {
    let code_span_re = Regex::new(r"`([^`\n]+)`").expect("valid inline code regex");
    let mut out = String::with_capacity(chunk.len() + 32);
    let mut last = 0usize;

    for cap in code_span_re.captures_iter(chunk) {
        let Some(full) = cap.get(0) else { continue };
        let Some(inner) = cap.get(1) else { continue };

        if full.start() > last {
            out.push_str(&linkify_plain_segment(
                &chunk[last..full.start()],
                lookup_re,
                lookup_map,
            ));
        }

        if let Some((target_path, frag)) =
            resolve_exact_or_unique_path(inner.as_str(), sorted_paths, unique_bases)
        {
            let base = file_basename(target_path);
            if frag.is_empty() {
                out.push_str(&format!("[{base}]({target_path})"));
            } else {
                out.push_str(&format!("[{base}{frag}]({target_path}{frag})"));
            }
        } else {
            out.push_str(full.as_str());
        }

        last = full.end();
    }

    if last < chunk.len() {
        out.push_str(&linkify_plain_segment(&chunk[last..], lookup_re, lookup_map));
    }

    out
}

fn linkify_file_mentions(text: &str, files: &[ModifiedFileDiff]) -> String {
    if text.trim().is_empty() || files.is_empty() {
        return text.to_string();
    }

    let mut seen_paths = HashSet::new();
    let mut sorted_paths: Vec<&str> = files
        .iter()
        .map(|f| f.path.trim())
        .filter(|p| !p.is_empty() && seen_paths.insert(*p))
        .collect();
    if sorted_paths.is_empty() {
        return text.to_string();
    }
    sorted_paths.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    let mut base_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for &p in &sorted_paths {
        *base_counts.entry(file_basename(p)).or_insert(0) += 1;
    }
    let mut unique_bases: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for &p in &sorted_paths {
        let base = file_basename(p);
        if base_counts.get(base).copied() == Some(1) {
            unique_bases.insert(base, p);
        }
    }

    let mut lookup_map: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    let mut tokens: Vec<&str> = Vec::new();
    for &p in &sorted_paths {
        lookup_map.insert(p, p);
        tokens.push(p);
    }
    for (&base, &full_p) in &unique_bases {
        if !lookup_map.contains_key(base) {
            lookup_map.insert(base, full_p);
            tokens.push(base);
        }
    }
    tokens.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    let lookup_re = if tokens.is_empty() {
        None
    } else {
        let escaped: Vec<String> = tokens.iter().map(|t| regex::escape(t)).collect();
        Regex::new(&escaped.join("|")).ok()
    };

    // Préserver intacts les blocs de code clôturés, les liens Markdown [label](href) et les URLs
    let skip_re = Regex::new(r"```[\s\S]*?(?:```|$)|\[[^\]]+\]\([^)\s]+\)|https?://[^\s)]+")
        .expect("valid skip regex");

    let mut result = String::with_capacity(text.len() + 64);
    let mut last = 0usize;
    for m in skip_re.find_iter(text) {
        if m.start() > last {
            result.push_str(&linkify_inline_prose(
                &text[last..m.start()],
                &sorted_paths,
                &unique_bases,
                &lookup_re,
                &lookup_map,
            ));
        }
        result.push_str(m.as_str());
        last = m.end();
    }
    if last < text.len() {
        result.push_str(&linkify_inline_prose(
            &text[last..],
            &sorted_paths,
            &unique_bases,
            &lookup_re,
            &lookup_map,
        ));
    }

    result
}

fn append_modified_files_summary(
    narrative: &str,
    modified_files: &[ModifiedFileDiff],
    turn_applied_files: &[String],
) -> String {
    let trimmed = narrative.trim();
    if !trimmed.is_empty() {
        return linkify_file_mentions(trimmed, modified_files);
    }

    if modified_files.is_empty() && turn_applied_files.is_empty() {
        return "Traitement terminé sans modification de fichier.".to_string();
    }

    let fallback_paths: Vec<String> = if !turn_applied_files.is_empty() {
        turn_applied_files.to_vec()
    } else {
        modified_files.iter().map(|f| f.path.clone()).collect()
    };

    if fallback_paths.len() == 1 {
        let p = &fallback_paths[0];
        let base = file_basename(p);
        return format!("Modifications appliquées dans [{base}]({p}).");
    }

    let bullets: Vec<String> = fallback_paths
        .iter()
        .map(|p| {
            let base = file_basename(p);
            format!("- Mise à jour appliquée dans [{base}]({p}).")
        })
        .collect();
    bullets.join("\n")
}

#[allow(clippy::too_many_arguments)]
async fn synthesize_final_response(
    app: &AppHandle,
    state_lock: &Arc<Mutex<AppStateData>>,
    conversation_id: &str,
    user_prompt: &str,
    cleaned_narrative: &str,
    engine_ran: bool,
    ollama_running: bool,
    synth_provider: &str,
    synth_model: &str,
    settings: &AppSettings,
    modified_files: &[ModifiedFileDiff],
    turn_applied_files: &[String],
    extra_stage_edits: bool,
    verifier_note: &str,
) -> (String, usize) {
    let can_call_synth = if synth_provider == "ollama" {
        ollama_running
    } else {
        providers::find_provider_key(settings, synth_provider).is_some()
    };

    let needs_llm_synthesis = engine_ran
        && can_call_synth
        && (cleaned_narrative.trim().is_empty()
            || cleaned_narrative.chars().count() > 900
            || narrative_needs_structured_synthesis(
                cleaned_narrative,
                turn_applied_files,
                user_prompt,
                extra_stage_edits,
            ));

    let mut synth_tokens = 0usize;
    let core_narrative = if needs_llm_synthesis {
        let scoped_diffs: Vec<ModifiedFileDiff> = if !turn_applied_files.is_empty() {
            let scoped: Vec<ModifiedFileDiff> = modified_files
                .iter()
                .filter(|f| turn_applied_files.iter().any(|p| p == &f.path))
                .cloned()
                .collect();
            if scoped.is_empty() {
                modified_files.to_vec()
            } else {
                scoped
            }
        } else {
            modified_files.to_vec()
        };

        let diff_summary = if scoped_diffs.is_empty() {
            "Aucune modification appliquée sur le disque.".to_string()
        } else {
            build_compact_diff_summary(&scoped_diffs, 1400)
        };
        let files_names: Vec<String> = scoped_diffs.iter().map(|f| f.path.clone()).collect();
        let multi_req = has_multiple_requests(user_prompt);
        let synth_sys = "Tu es l'Architecte Synthétiseur d'AiderStudio. Rédige en français un retour clair et concis sur ce qui a réellement été généré ou modifié dans chaque fichier.\n\
        RÈGLES STRICTES :\n\
        1. Cite chaque fichier concerné avec un lien Markdown cliquable exact `[NomFichier](chemin/relatif/NomFichier)` (ex. `[App.vue](src/App.vue)`).\n\
        2. Si la demande contient plusieurs points/tâches distincts (ou plusieurs fichiers modifiés pour des raisons distinctes), structure obligatoirement la réponse en bullet points (`- ...`), avec une puce par demande traitée expliquant ce qui a changé et dans quel(s) fichier(s).\n\
        3. Sinon, rédige 1 à 3 phrases ou puces concrètes décrivant ce qui a été généré/modifié.\n\
        4. INTERDIT : blocs de code complets, marqueurs SEARCH/REPLACE, section `### Fichiers modifiés` (déjà affichée dans le dropdown dédié), excuses ou blabla.";
        let synth_user = format!(
            "Demande utilisateur{} :\n{}\n\nFichiers concernés sur ce tour : {}\nExtrait diff réel :\n{}\nVérification : {}\nExplication brute du modèle : {}",
            if multi_req { " (MULTI-DEMANDES DÉTECTÉES -> bullet points obligatoires)" } else { "" },
            truncate_chars(user_prompt.trim(), 420),
            if files_names.is_empty() {
                "aucun".to_string()
            } else {
                files_names.join(", ")
            },
            diff_summary,
            if verifier_note.is_empty() { "N/A" } else { verifier_note },
            truncate_chars(cleaned_narrative, 800)
        );

        match providers::call_provider_oneshot(
            synth_provider,
            synth_model,
            settings,
            synth_sys,
            &synth_user,
            420,
            0.1,
        )
        .await
        {
            Ok((summary, p_tok, c_tok)) => {
                synth_tokens = p_tok + c_tok;
                if synth_tokens > 0 {
                    record_external_api_usage(app, state_lock, synth_provider, p_tok, c_tok).await;
                    let _ = app.emit(
                        "aider:tokens",
                        TokenUsageEvent {
                            conversation_id: conversation_id.to_string(),
                            subagent: "5. Synthétiseur".to_string(),
                            prompt_tokens: p_tok,
                            completion_tokens: c_tok,
                            total_tokens: synth_tokens,
                            role: "synthesizer".to_string(),
                            provider: synth_provider.to_string(),
                            model: synth_model.to_string(),
                            context_limit: providers::resolve_model_context_limit(
                                synth_provider,
                                synth_model,
                                settings,
                            ),
                        },
                    );
                }
                if summary.trim().is_empty() {
                    cleaned_narrative.to_string()
                } else {
                    summary
                }
            }
            Err(_) => cleaned_narrative.to_string(),
        }
    } else {
        cleaned_narrative.to_string()
    };

    let final_markdown =
        append_modified_files_summary(&core_narrative, modified_files, turn_applied_files);
    (final_markdown, synth_tokens)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let truncated: String = input.chars().take(max_chars).collect();
    format!("{truncated}…")
}

fn is_cancelled(cancel_rx: &tokio::sync::watch::Receiver<bool>) -> bool {
    *cancel_rx.borrow()
}

async fn wait_cancelled(cancel_rx: &mut tokio::sync::watch::Receiver<bool>) {
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

pub fn clean_generated_title(raw: &str) -> Option<String> {
    let first_line = raw
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())?;

    let mut cleaned = first_line.trim_start_matches('#').trim();
    for prefix in [
        "Titre :",
        "Titre:",
        "Title :",
        "Title:",
        "Sujet :",
        "Sujet:",
    ] {
        if let Some(rest) = cleaned.strip_prefix(prefix) {
            cleaned = rest.trim();
        }
    }

    cleaned = cleaned.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '«' | '»' | '*' | '.' | '!' | '?' | ':' | ';')
    });
    if cleaned.is_empty() {
        None
    } else {
        Some(truncate_chars(cleaned, 46))
    }
}

fn pick_title_provider_model(settings: &AppSettings, ollama_running: bool) -> Option<(String, String)> {
    for role in ["synthesizer", "architect"] {
        let (prov, mdl) = resolve_agent_slot(role, settings);
        if mdl.trim().is_empty() {
            continue;
        }
        if prov == "ollama" {
            if ollama_running {
                return Some((prov, mdl));
            }
        } else if providers::find_provider_key(settings, &prov).is_some() {
            return Some((prov, mdl));
        }
    }

    if let Some(cfg) = settings
        .providers
        .iter()
        .find(|p| p.validated && !p.api_key.trim().is_empty() && !p.available_models.is_empty())
    {
        return Some((cfg.id.clone(), cfg.available_models[0].clone()));
    }

    if ollama_running && !settings.ollama_model.trim().is_empty() {
        return Some(("ollama".to_string(), settings.ollama_model.trim().to_string()));
    }

    None
}

async fn generate_conversation_title(
    settings: &AppSettings,
    ollama_running: bool,
    user_prompt: &str,
) -> Option<String> {
    let (prov, mdl) = pick_title_provider_model(settings, ollama_running)?;
    let sys = "Tu génères un titre très court en français (3 à 6 mots maximum) résumant le sujet d'une discussion technique. Réponds UNIQUEMENT par le titre brut : sans guillemets, sans point final, sans préfixe.";
    let prompt_excerpt = truncate_chars(user_prompt.trim(), 500);
    let fut = providers::call_provider_oneshot(&prov, &mdl, settings, sys, &prompt_excerpt, 48, 0.2);
    match tokio::time::timeout(std::time::Duration::from_secs(15), fut).await {
        Ok(Ok((reply, _, _))) => clean_generated_title(&reply),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn finish_cancelled_pipeline(
    app: &AppHandle,
    state_lock: &Arc<Mutex<AppStateData>>,
    conversation_id: &str,
    project_root: &str,
    agent_trace: &mut Vec<AgentTraceStep>,
) -> anyhow::Result<SendPromptResult> {
    for step in agent_trace.iter_mut() {
        if step.status == "running" {
            step.status = "error".to_string();
            step.summary = "Interrompu par l'utilisateur".to_string();
            let _ = app.emit(
                "aider:trace",
                AgentTraceEvent {
                    conversation_id: conversation_id.to_string(),
                    step: step.clone(),
                },
            );
        }
    }

    git_diff::sanitize_backtick_paths(project_root).await;
    let (modified_files, lines_added, lines_removed) =
        git_diff::collect_project_diffs(project_root).await;

    let result = {
        let mut guard = state_lock.lock().await;
        let now = Utc::now().to_rfc3339();
        let conv_idx = guard
            .conversations
            .iter()
            .position(|c| c.id == conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Conversation introuvable"))?;

        guard.conversations[conv_idx].messages.push(ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".to_string(),
            content: "⏹️ **Traitement interrompu par l'utilisateur.**".to_string(),
            thinking: None,
            attachments: None,
            created_at: now.clone(),
            stage_summary: Some("Interrompu".to_string()),
            files_changed: modified_files.len(),
            lines_added,
            lines_removed,
            files_summary: build_files_summary(&modified_files),
            agent_trace: agent_trace.clone(),
        });
        guard.conversations[conv_idx].modified_files = modified_files;
        guard.conversations[conv_idx].lines_added = lines_added;
        guard.conversations[conv_idx].lines_removed = lines_removed;
        guard.conversations[conv_idx].updated_at = now.clone();

        let proj_id = guard.conversations[conv_idx].project_id.clone();
        let proj_idx = guard
            .projects
            .iter()
            .position(|p| p.id == proj_id)
            .ok_or_else(|| anyhow::anyhow!("Projet introuvable"))?;
        guard.projects[proj_idx].updated_at = now;

        let updated_conv = guard.conversations[conv_idx].clone();
        let updated_proj = guard.projects[proj_idx].clone();
        let _ = storage::save_state(&guard).await;

        SendPromptResult {
            conversation: updated_conv,
            project: updated_proj,
        }
    };

    emit_stage(
        app,
        conversation_id,
        5,
        "Interrompu",
        "Traitement arrêté à la demande de l'utilisateur.",
        "error",
    );
    Ok(result)
}

pub async fn run_prompt_pipeline(
    app: &AppHandle,
    state_lock: &Arc<Mutex<AppStateData>>,
    conversation_id: String,
    user_prompt: String,
    attachments: Option<Vec<AttachedFile>>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<SendPromptResult> {
    let (settings, project, conv_ctx, mcp_servers, user_msg_time, is_first_message, initial_auto_title) = {
        let mut guard = state_lock.lock().await;
        let conv_idx = guard
            .conversations
            .iter()
            .position(|c| c.id == conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Conversation introuvable"))?;

        let now = Utc::now().to_rfc3339();
        let is_first_message = guard.conversations[conv_idx].messages.is_empty();
        let initial_auto_title = if is_first_message {
            let t = truncate_chars(user_prompt.trim(), 38);
            guard.conversations[conv_idx].title = t.clone();
            t
        } else {
            guard.conversations[conv_idx].title.clone()
        };
        guard.conversations[conv_idx].messages.push(ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "user".to_string(),
            content: user_prompt.clone(),
            thinking: None,
            attachments: attachments.clone(),
            created_at: now.clone(),
            stage_summary: None,
            files_changed: 0,
            lines_added: 0,
            lines_removed: 0,
            files_summary: Vec::new(),
            agent_trace: Vec::new(),
        });
        guard.conversations[conv_idx].updated_at = now.clone();

        let project_id = guard.conversations[conv_idx].project_id.clone();
        let project = guard
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Projet introuvable"))?;

        let conv_ctx = guard.conversations[conv_idx].conversation_context.clone();
        let settings = guard.settings.clone();
        let mcp_servers = guard.mcp_servers.clone();
        let _ = storage::save_state(&guard).await;
        (
            settings,
            project,
            conv_ctx,
            mcp_servers,
            now,
            is_first_message,
            initial_auto_title,
        )
    };

    let mut image_base64_list = Vec::new();
    if let Some(att_list) = &attachments {
        for att in att_list {
            if att.is_image {
                if let Some(data_url) = &att.data_url {
                    let b64 = if let Some(idx) = data_url.find(";base64,") {
                        &data_url[idx + 8..]
                    } else {
                        data_url.as_str()
                    };
                    image_base64_list.push(b64.to_string());
                }
            }
        }
    }

    let project_root_path = PathBuf::from(&project.root_path);
    if project_root_path.exists() {
        if !project_root_path.join(".git").exists() {
            let _ = tokio::process::Command::new("git")
                .args(["init"])
                .current_dir(&project_root_path)
                .output()
                .await;
        }
        let _ = ensure_aiderignore(&project_root_path).await;
    }

    let mut stack = check_stack_status(&settings).await;
    if !stack.ollama_running && settings.auto_start_ollama && stack.ollama_installed {
        if let Ok(updated) = start_ollama_service(&settings).await {
            stack = updated;
        }
    }

    let mut agent_trace: Vec<AgentTraceStep> = Vec::new();
    if is_cancelled(&cancel_rx) {
        return finish_cancelled_pipeline(
            app,
            state_lock,
            &conversation_id,
            &project.root_path,
            &mut agent_trace,
        )
        .await;
    }

    let mut title_handle = if is_first_message && !user_prompt.trim().is_empty() {
        let t_app = app.clone();
        let t_state = Arc::clone(state_lock);
        let t_conv_id = conversation_id.clone();
        let t_settings = settings.clone();
        let t_prompt = user_prompt.clone();
        let t_init_title = initial_auto_title.clone();
        let t_ollama_running = stack.ollama_running;
        Some(tokio::spawn(async move {
            if let Some(new_title) =
                generate_conversation_title(&t_settings, t_ollama_running, &t_prompt).await
            {
                let mut should_emit = false;
                {
                    let mut guard = t_state.lock().await;
                    if let Some(conv) = guard.conversations.iter_mut().find(|c| c.id == t_conv_id) {
                        if conv.title == t_init_title && conv.title != new_title {
                            conv.title = new_title.clone();
                            should_emit = true;
                            let _ = storage::save_state(&guard).await;
                        }
                    }
                }
                if should_emit {
                    let _ = t_app.emit("aider:conversation_titled", (&t_conv_id, &new_title));
                }
            }
        }))
    } else {
        None
    };

    // SUB-AGENT 1/5: Architecte & Orchestrateur
    let (arch_prov, arch_model) = resolve_agent_slot("architect", &settings);
    let (mut exec_role, mut run_tests_after) = classify_prompt_role(&user_prompt);
    let (base_exec_prov, base_exec_model) = resolve_agent_slot(exec_role, &settings);
    let is_agent_mode =
        settings.active_target.trim().is_empty() || settings.active_target.trim() == "agent";

    emit_stage(
        app,
        &conversation_id,
        1,
        "1. Architecte & Orchestrateur",
        &format!(
            "Analyse de la demande et routage ({} · {}).",
            arch_prov, arch_model
        ),
        "running",
    );
    record_trace_step(
        app,
        &conversation_id,
        &mut agent_trace,
        AgentTraceStep {
            role: "architect".to_string(),
            role_label: "Architecte".to_string(),
            provider: arch_prov.clone(),
            model: arch_model.clone(),
            summary: "Analyse de la demande et préparation du cadrage...".to_string(),
            status: "running".to_string(),
            tokens: 0,
        },
    );

    let mut architect_spec: Option<String> = None;
    let mut architect_llm_ran = false;
    let mut architect_err: Option<String> = None;
    let mut arch_tokens_used = 0usize;

    if is_agent_mode
        && (arch_prov != base_exec_prov || arch_model != base_exec_model)
        && (arch_prov != "ollama" || stack.ollama_running)
    {
        let arch_sys = "Tu es l'Architecte logiciel d'AiderStudio. Analyse la demande utilisateur et décide du sous-agent d'exécution le plus adapté.\nSi la demande contient plusieurs tâches ou points distincts, décompose `spec` en les énumérant point par point pour qu'aucun ne soit oublié.\nRetourne UNIQUEMENT un objet JSON valide, sans texte ni markdown autour :\n{\"role\":\"designer|coder|refactoring|tester\",\"runTests\":false,\"spec\":\"3 à 6 consignes techniques précises et concises en français pour guider l'exécution\"}";
        let arch_user = format!(
            "[Contexte projet]\n{}\n\n[Session récente]\n{}\n\n[Demande utilisateur]\n{}",
            truncate_chars(project.global_context.trim(), 500),
            truncate_chars(conv_ctx.trim(), 500),
            truncate_chars(user_prompt.trim(), 900)
        );
        let arch_call_res = tokio::select! {
            biased;
            _ = wait_cancelled(&mut cancel_rx) => {
                if let Some(h) = title_handle.take() {
                    h.abort();
                }
                return finish_cancelled_pipeline(
                    app,
                    state_lock,
                    &conversation_id,
                    &project.root_path,
                    &mut agent_trace,
                )
                .await;
            }
            res = providers::call_provider_oneshot(
                &arch_prov,
                &arch_model,
                &settings,
                arch_sys,
                &arch_user,
                450,
                0.2,
            ) => res,
        };
        match arch_call_res {
            Ok((reply, p_tok, c_tok)) => {
                if let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) {
                    if start < end {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&reply[start..=end]) {
                            if let Some(r) = val.get("role").and_then(|v| v.as_str()) {
                                exec_role = match r.trim() {
                                    "designer" => "designer",
                                    "coder" => "coder",
                                    "refactoring" => "refactoring",
                                    "tester" => "tester",
                                    _ => exec_role,
                                };
                            }
                            if val.get("runTests").and_then(|v| v.as_bool()) == Some(true) {
                                run_tests_after = true;
                            }
                            if let Some(spec) = val.get("spec").and_then(|v| v.as_str()) {
                                let clean_spec = spec.trim();
                                if !clean_spec.is_empty() {
                                    architect_spec = Some(truncate_chars(clean_spec, 900));
                                }
                            }
                        }
                    }
                }
                architect_llm_ran = true;
                arch_tokens_used = p_tok + c_tok;
                record_external_api_usage(app, state_lock, &arch_prov, p_tok, c_tok).await;
                let _ = app.emit(
                    "aider:tokens",
                    TokenUsageEvent {
                        conversation_id: conversation_id.clone(),
                        subagent: "1. Architecte".to_string(),
                        prompt_tokens: p_tok,
                        completion_tokens: c_tok,
                        total_tokens: arch_tokens_used,
                        role: "architect".to_string(),
                        provider: arch_prov.clone(),
                        model: arch_model.clone(),
                        context_limit: providers::resolve_model_context_limit(
                            &arch_prov,
                            &arch_model,
                            &settings,
                        ),
                    },
                );
            }
            Err(e) => {
                architect_err = Some(synthesize_error_log(&e.to_string()));
            }
        }
    }

    if !architect_llm_ran {
        let att_chars: usize = attachments
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter(|a| !a.is_image)
            .filter_map(|a| a.content.as_ref())
            .map(|c| c.len())
            .sum();
        let approx_chars = project.global_context.len().min(2000)
            + conv_ctx.len().min(800)
            + user_prompt.len()
            + att_chars
            + 160;
        let init_prompt_tokens = (approx_chars as f64 / 3.8).ceil() as usize;
        arch_tokens_used = init_prompt_tokens;
        let _ = app.emit(
            "aider:tokens",
            TokenUsageEvent {
                conversation_id: conversation_id.clone(),
                subagent: "1. Architecte & Contexte".to_string(),
                prompt_tokens: init_prompt_tokens,
                completion_tokens: 0,
                total_tokens: init_prompt_tokens,
                role: "architect".to_string(),
                provider: arch_prov.clone(),
                model: arch_model.clone(),
                context_limit: providers::resolve_model_context_limit(
                    &arch_prov,
                    &arch_model,
                    &settings,
                ),
            },
        );
    }

    let (arch_status, arch_summary) = if architect_llm_ran {
        let spec_note = architect_spec
            .as_deref()
            .map(|s| format!(" · {}", truncate_chars(s.replace('\n', " ").trim(), 120)))
            .unwrap_or_default();
        (
            "done".to_string(),
            format!(
                "Routé vers **{}** · Plan technique généré{}",
                role_display_name(exec_role),
                spec_note
            ),
        )
    } else if let Some(ref err_msg) = architect_err {
        (
            "fallback".to_string(),
            format!(
                "Repli cadrage déterministe (vers **{}**) · `{}`",
                role_display_name(exec_role),
                truncate_chars(&err_msg.replace('`', "'"), 110)
            ),
        )
    } else {
        (
            "done".to_string(),
            format!(
                "Cadrage contexte et routage vers **{}**",
                role_display_name(exec_role)
            ),
        )
    };
    record_trace_step(
        app,
        &conversation_id,
        &mut agent_trace,
        AgentTraceStep {
            role: "architect".to_string(),
            role_label: "Architecte".to_string(),
            provider: arch_prov.clone(),
            model: arch_model.clone(),
            summary: arch_summary,
            status: arch_status,
            tokens: arch_tokens_used,
        },
    );

    let (exec_prov, exec_model) = resolve_agent_slot(exec_role, &settings);

    // SUB-AGENT 2/5: Cartographe & Analyseur (toujours actif)
    let (carto_prov, carto_model) = resolve_agent_slot("cartographer", &settings);
    emit_stage(
        app,
        &conversation_id,
        2,
        "2. Cartographe & Analyseur",
        &format!(
            "Sélection des fichiers cibles ({} · {}).",
            carto_prov, carto_model
        ),
        "running",
    );
    record_trace_step(
        app,
        &conversation_id,
        &mut agent_trace,
        AgentTraceStep {
            role: "cartographer".to_string(),
            role_label: "Cartographe".to_string(),
            provider: carto_prov.clone(),
            model: carto_model.clone(),
            summary: "Identification des fichiers cibles...".to_string(),
            status: "running".to_string(),
            tokens: 0,
        },
    );

    let all_project_files = list_source_files_for_cartographer(&project_root_path);
    let carto = tokio::select! {
        biased;
        _ = wait_cancelled(&mut cancel_rx) => {
            if let Some(h) = title_handle.take() {
                h.abort();
            }
            return finish_cancelled_pipeline(
                app,
                state_lock,
                &conversation_id,
                &project.root_path,
                &mut agent_trace,
            )
            .await;
        }
        p = run_cartographer(
            &project_root_path,
            &all_project_files,
            &user_prompt,
            attachments.as_deref(),
            &settings,
            &carto_prov,
            &carto_model,
            &exec_prov,
            stack.ollama_running,
            exec_role == "designer",
        ) => p,
    };

    if let Some((p_tok, c_tok)) = carto.cartographer_tokens {
        if p_tok + c_tok > 0 {
            record_external_api_usage(app, state_lock, &carto_prov, p_tok, c_tok).await;
            let _ = app.emit(
                "aider:tokens",
                TokenUsageEvent {
                    conversation_id: conversation_id.clone(),
                    subagent: "2. Cartographe (Ciblage)".to_string(),
                    prompt_tokens: p_tok,
                    completion_tokens: c_tok,
                    total_tokens: p_tok + c_tok,
                    role: "cartographer".to_string(),
                    provider: carto_prov.clone(),
                    model: carto_model.clone(),
                    context_limit: providers::resolve_model_context_limit(
                        &carto_prov,
                        &carto_model,
                        &settings,
                    ),
                },
            );
        }
    }

    let target_detail = if carto.target_files.is_empty() {
        carto.strategy_note.clone()
    } else {
        format!(
            "Ciblage : {} ({})",
            carto.target_files.join(", "),
            carto.strategy_note
        )
    };
    emit_stage(
        app,
        &conversation_id,
        2,
        "2. Cartographe & Analyseur",
        &target_detail,
        "running",
    );

    let carto_tokens_used = carto.cartographer_tokens.map(|(p, c)| p + c).unwrap_or(0);
    let carto_summary = if carto.target_files.is_empty() {
        format!("Découverte automatique ({})", carto.strategy_note)
    } else {
        let file_tags: Vec<String> = carto
            .target_files
            .iter()
            .map(|f| format!("`{f}`"))
            .collect();
        format!("Ciblé : {} ({})", file_tags.join(", "), carto.strategy_note)
    };
    record_trace_step(
        app,
        &conversation_id,
        &mut agent_trace,
        AgentTraceStep {
            role: "cartographer".to_string(),
            role_label: "Cartographe".to_string(),
            provider: carto_prov.clone(),
            model: carto_model.clone(),
            summary: carto_summary,
            status: "done".to_string(),
            tokens: carto_tokens_used,
        },
    );

    // SUB-AGENT 3/5: Exécution (Moteur natif multi-tours unifié)
    let strategy = choose_execution_strategy(
        &project_root_path,
        &carto.target_files,
        carto.explicit_match,
        &exec_prov,
        &settings,
    );

    let stage3_label = format!(
        "3. {} ({} · {})",
        role_display_name(exec_role),
        exec_prov,
        exec_model
    );
    let ref_suffix = if carto.read_only_files.is_empty() {
        String::new()
    } else {
        format!(" (+{} réf.)", carto.read_only_files.len())
    };
    let global_suffix = if carto.is_global_task {
        " · vue globale"
    } else {
        ""
    };
    let stage3_detail = format!(
        "Moteur natif · {} · map {} tok · {} fichier(s){}{} sur {}:{}",
        strategy.edit_format,
        strategy.map_tokens,
        carto.target_files.len(),
        ref_suffix,
        global_suffix,
        exec_prov,
        exec_model
    );
    emit_stage(
        app,
        &conversation_id,
        3,
        &stage3_label,
        &stage3_detail,
        "running",
    );
    record_trace_step(
        app,
        &conversation_id,
        &mut agent_trace,
        AgentTraceStep {
            role: exec_role.to_string(),
            role_label: role_display_name(exec_role).to_string(),
            provider: exec_prov.clone(),
            model: exec_model.clone(),
            summary: stage3_detail.clone(),
            status: "running".to_string(),
            tokens: 0,
        },
    );

    let can_run_exec = (exec_prov != "ollama"
        && providers::find_provider_key(&settings, &exec_prov).is_some())
        || (exec_prov == "ollama" && stack.ollama_running);

    let mut engine_ran = false;
    let mut exec_tokens_used = 0usize;
    let mut exec_status = "done".to_string();
    let exec_note: String;
    let mut extracted_thinking: Option<String> = None;
    let mut tool_results: Vec<ToolExecutionResult> = Vec::new();
    let mut had_repairs = false;
    let mut raw_exec_output = String::new();
    let mut turn_applied_files: Vec<String> = Vec::new();
    let mut extra_stage_edits = false;

    let (cleaned_narrative, stage_note) = if can_run_exec {
        git_diff::sanitize_corrupted_diff_files(&project.root_path).await;
        match execute_native_agent_loop(
            app,
            state_lock,
            &conversation_id,
            &project,
            &conv_ctx,
            &user_prompt,
            attachments.as_deref(),
            image_base64_list,
            architect_spec.as_deref(),
            &exec_prov,
            &exec_model,
            exec_role,
            &settings,
            &all_project_files,
            &carto.target_files,
            &carto.read_only_files,
            &strategy,
            &mcp_servers,
            &mut cancel_rx,
        )
        .await
        {
            Ok(loop_res) => {
                engine_ran = true;
                exec_tokens_used = loop_res.prompt_tokens + loop_res.completion_tokens;
                extracted_thinking = loop_res.thinking;
                tool_results = loop_res.tool_results;
                had_repairs = loop_res.had_repairs;
                raw_exec_output = loop_res.raw_last_response;
                push_unique_paths(&mut turn_applied_files, &loop_res.applied_files);
                exec_note = format!(
                    "Moteur natif (format `{}`, max {} l., {} tour(s){})",
                    loop_res.effective_edit_format,
                    strategy.max_file_lines,
                    loop_res.turns_run,
                    if had_repairs { ", auto-réparé" } else { "" }
                );
                let out = if loop_res.narrative.trim().is_empty()
                    && !loop_res.applied_files.is_empty()
                {
                    // Laisser vide pour que Stage 5 synthétise un résumé concret basé sur le diff
                    String::new()
                } else if loop_res.narrative.trim().is_empty() {
                    format!(
                        "Le modèle `{exec_model}` ({exec_prov}) n'a retourné aucun contenu."
                    )
                } else {
                    loop_res.narrative
                };
                (
                    out,
                    format!(
                        "{} · {} ({}:{})",
                        role_display_name(exec_role),
                        loop_res.effective_edit_format,
                        exec_prov,
                        exec_model
                    ),
                )
            }
            Err(err) => {
                if is_cancelled(&cancel_rx) || err.to_string().contains("CANCELLED_BY_USER") {
                    if let Some(h) = title_handle.take() {
                        h.abort();
                    }
                    return finish_cancelled_pipeline(
                        app,
                        state_lock,
                        &conversation_id,
                        &project.root_path,
                        &mut agent_trace,
                    )
                    .await;
                }
                let clean_err = synthesize_error_log(&err.to_string());
                exec_status = "error".to_string();
                exec_note = clean_err.clone();
                (
                    format!(
                        "⚠️ Erreur sur `{exec_prov}:{exec_model}` :\n`{clean_err}`\n\nVérifiez le modèle sélectionné, vos quotas API ou qu'Ollama est bien actif."
                    ),
                    format!("{exec_prov} erreur"),
                )
            }
        }
    } else {
        exec_status = "error".to_string();
        exec_note = "Ollama arrêté et aucune clé API externe configurée".to_string();
        let diag = format!(
            "### Stack locale non démarrée\n\n- **Moteur natif Rust** : Prêt\n- **Ollama** : {}\n- **Modèle cible** : `{}:{}`\n\nLancez Ollama (`ollama serve` puis `ollama pull {}`), ou sélectionnez un fournisseur API externe validé dans les paramètres pour exécuter ce prompt sur `{}`.",
            if stack.ollama_installed {
                "Installé mais arrêté"
            } else {
                "Non détecté"
            },
            exec_prov,
            exec_model,
            settings.ollama_model,
            project.root_path
        );
        (diag, "Diagnostic stack".to_string())
    };

    record_trace_step(
        app,
        &conversation_id,
        &mut agent_trace,
        AgentTraceStep {
            role: exec_role.to_string(),
            role_label: role_display_name(exec_role).to_string(),
            provider: exec_prov.clone(),
            model: exec_model.clone(),
            summary: exec_note.clone(),
            status: exec_status.clone(),
            tokens: exec_tokens_used,
        },
    );

    if is_cancelled(&cancel_rx) {
        if let Some(h) = title_handle.take() {
            h.abort();
        }
        return finish_cancelled_pipeline(
            app,
            state_lock,
            &conversation_id,
            &project.root_path,
            &mut agent_trace,
        )
        .await;
    }

    // SUB-AGENT 4/5: Testeur & Vérifieur + Auto-fix build (1 tour max)
    emit_stage(
        app,
        &conversation_id,
        4,
        "4. Testeur & Vérifieur",
        "Validation de la syntaxe, compilation et vérifications automatisées.",
        "running",
    );

    let mut tester_note = String::new();
    let mut tester_ran = false;
    let mut tester_tokens_used = 0usize;
    let (test_prov, test_model) = resolve_agent_slot("tester", &settings);
    let mut tester_status = "done".to_string();
    let mut tester_summary = String::new();

    if run_tests_after && exec_role != "tester" && engine_ran && is_agent_mode {
        let (pre_diffs, _, _) = git_diff::collect_project_diffs(&project.root_path).await;
        let changed_paths: Vec<String> = pre_diffs.into_iter().map(|d| d.path).take(3).collect();
        let can_run_tester = !changed_paths.is_empty()
            && (test_prov != "ollama" || stack.ollama_running)
            && (test_prov == "ollama"
                || providers::find_provider_key(&settings, &test_prov).is_some());
        if can_run_tester {
            emit_stage(
                app,
                &conversation_id,
                4,
                "4. Testeur & Vérifieur",
                &format!(
                    "Sous-agent Testeur ({} · {}) sur {}...",
                    test_prov,
                    test_model,
                    changed_paths.join(", ")
                ),
                "running",
            );
            record_trace_step(
                app,
                &conversation_id,
                &mut agent_trace,
                AgentTraceStep {
                    role: "tester".to_string(),
                    role_label: "Testeur".to_string(),
                    provider: test_prov.clone(),
                    model: test_model.clone(),
                    summary: format!(
                        "Rédaction des tests pour `{}`...",
                        changed_paths.join(", ")
                    ),
                    status: "running".to_string(),
                    tokens: 0,
                },
            );
            let test_prompt = format!(
                "À la suite de la demande : « {} »\nLes fichiers suivants viennent d'être modifiés : {}.\nÉcris ou complète uniquement les tests unitaires/d'intégration pertinents pour vérifier ces changements sans modifier la logique métier existante. Réponds brièvement en français.",
                truncate_chars(user_prompt.trim(), 300),
                changed_paths.join(", ")
            );
            let mut test_strategy = choose_execution_strategy(
                &project_root_path,
                &changed_paths,
                true,
                &test_prov,
                &settings,
            );
            test_strategy.max_turns = 2;

            match execute_native_agent_loop(
                app,
                state_lock,
                &conversation_id,
                &project,
                &conv_ctx,
                &test_prompt,
                None,
                Vec::new(),
                None,
                &test_prov,
                &test_model,
                "tester",
                &settings,
                &all_project_files,
                &changed_paths,
                &[],
                &test_strategy,
                &mcp_servers,
                &mut cancel_rx,
            )
            .await
            {
                Ok(t_res) => {
                    tester_ran = true;
                    tester_tokens_used = t_res.prompt_tokens + t_res.completion_tokens;
                    tool_results.extend(t_res.tool_results);
                    if !t_res.applied_files.is_empty() {
                        push_unique_paths(&mut turn_applied_files, &t_res.applied_files);
                        extra_stage_edits = true;
                    }
                    tester_summary = format!(
                        "Tests générés/mis à jour pour `{}`",
                        changed_paths.join(", ")
                    );
                    tester_note = format!(
                        "• 🧪 **Sous-agent Testeur ({} · {})** : tests générés/mis à jour pour `{}`.",
                        test_prov,
                        test_model,
                        changed_paths.join(", ")
                    );
                }
                Err(e) => {
                    if is_cancelled(&cancel_rx) || e.to_string().contains("CANCELLED_BY_USER") {
                        if let Some(h) = title_handle.take() {
                            h.abort();
                        }
                        return finish_cancelled_pipeline(
                            app,
                            state_lock,
                            &conversation_id,
                            &project.root_path,
                            &mut agent_trace,
                        )
                        .await;
                    }
                    tester_ran = true;
                    tester_status = "error".to_string();
                    tester_summary = format!(
                        "Échec rédaction tests : `{}`",
                        synthesize_error_log(&e.to_string())
                    );
                }
            }
        }
    }

    let mut test_res = tokio::select! {
        biased;
        _ = wait_cancelled(&mut cancel_rx) => {
            if let Some(h) = title_handle.take() {
                h.abort();
            }
            return finish_cancelled_pipeline(
                app,
                state_lock,
                &conversation_id,
                &project.root_path,
                &mut agent_trace,
            )
            .await;
        }
        res = run_test_verifier(&project_root_path) => res,
    };

    // Auto-fix compilation (1 tour max) si des fichiers ont été modifiés et la compilation échoue
    let (pre_fix_diffs, _, _) = git_diff::collect_project_diffs(&project.root_path).await;
    if !pre_fix_diffs.is_empty()
        && test_res.ran
        && !test_res.success
        && !is_cancelled(&cancel_rx)
        && is_agent_mode
        && can_run_exec
    {
        let broken_paths: Vec<String> = pre_fix_diffs.into_iter().map(|d| d.path).take(3).collect();
        let _ = app.emit(
            "aider:stream",
            (
                &conversation_id,
                &format!(
                    "[Auto-fix] Erreur détectée par `{}`, tentative de correction ciblée...",
                    test_res.command
                ),
            ),
        );
        emit_stage(
            app,
            &conversation_id,
            4,
            "4. Testeur & Vérifieur",
            &format!("Auto-correction sur `{}`...", test_res.command),
            "running",
        );

        let fix_prompt = format!(
            "La vérification `{}` a échoué après nos modifications avec les erreurs suivantes :\n```\n{}\n```\nCorrige uniquement ces erreurs dans les fichiers cibles avec des blocs `SEARCH/REPLACE` précis.",
            test_res.command, test_res.output_summary
        );
        let fix_strategy = ExecutionStrategy {
            edit_format: "diff".to_string(),
            map_tokens: 0,
            max_turns: 1,
            max_file_lines: 0,
        };

        if let Ok(fix_res) = execute_native_agent_loop(
            app,
            state_lock,
            &conversation_id,
            &project,
            "",
            &fix_prompt,
            None,
            Vec::new(),
            None,
            &exec_prov,
            &exec_model,
            exec_role,
            &settings,
            &all_project_files,
            &broken_paths,
            &[],
            &fix_strategy,
            &mcp_servers,
            &mut cancel_rx,
        )
        .await
        {
            exec_tokens_used += fix_res.prompt_tokens + fix_res.completion_tokens;
            tool_results.extend(fix_res.tool_results);
            if !fix_res.applied_files.is_empty() {
                push_unique_paths(&mut turn_applied_files, &fix_res.applied_files);
                extra_stage_edits = true;
                had_repairs = true;
                test_res = run_test_verifier(&project_root_path).await;
            }
        }
    }

    let mut verifier_note = String::new();
    if test_res.ran {
        if test_res.success {
            verifier_note = format!(
                "• ✅ **Vérifieur ({})** : {}",
                test_res.command, test_res.output_summary
            );
            let verif_short = format!("✅ `{}` validé", test_res.command);
            if tester_ran {
                tester_summary = format!("{tester_summary} · {verif_short}");
            } else {
                record_trace_step(
                    app,
                    &conversation_id,
                    &mut agent_trace,
                    AgentTraceStep {
                        role: "tester".to_string(),
                        role_label: "Vérifieur".to_string(),
                        provider: "native".to_string(),
                        model: test_res.command.clone(),
                        summary: format!(
                            "{verif_short} · {}",
                            truncate_chars(&test_res.output_summary, 90)
                        ),
                        status: "done".to_string(),
                        tokens: 0,
                    },
                );
            }
        } else {
            verifier_note = format!(
                "• ⚠️ **Vérifieur ({})** : Compilation en erreur :\n```\n{}\n```",
                test_res.command, test_res.output_summary
            );
            let verif_short = format!("⚠️ `{}` en erreur", test_res.command);
            if tester_ran {
                tester_status = "error".to_string();
                tester_summary = format!("{tester_summary} · {verif_short}");
            } else {
                record_trace_step(
                    app,
                    &conversation_id,
                    &mut agent_trace,
                    AgentTraceStep {
                        role: "tester".to_string(),
                        role_label: "Vérifieur".to_string(),
                        provider: "native".to_string(),
                        model: test_res.command.clone(),
                        summary: format!(
                            "{verif_short} · {}",
                            truncate_chars(&test_res.output_summary.replace('\n', " "), 90)
                        ),
                        status: "error".to_string(),
                        tokens: 0,
                    },
                );
            }
        }
    }
    if tester_ran {
        record_trace_step(
            app,
            &conversation_id,
            &mut agent_trace,
            AgentTraceStep {
                role: "tester".to_string(),
                role_label: "Testeur".to_string(),
                provider: test_prov.clone(),
                model: test_model.clone(),
                summary: tester_summary,
                status: tester_status,
                tokens: tester_tokens_used,
            },
        );
    }

    // SUB-AGENT 5/5: Synthétiseur & Mémoire Globale
    let (synth_prov, synth_model) = resolve_agent_slot("synthesizer", &settings);
    emit_stage(
        app,
        &conversation_id,
        5,
        "5. Synthétiseur & Mémoire Globale",
        &format!(
            "Calcul du diff Git et synthèse ({} · {}).",
            synth_prov, synth_model
        ),
        "running",
    );
    record_trace_step(
        app,
        &conversation_id,
        &mut agent_trace,
        AgentTraceStep {
            role: "synthesizer".to_string(),
            role_label: "Synthétiseur".to_string(),
            provider: synth_prov.clone(),
            model: synth_model.clone(),
            summary: "Calcul du diff Git et préparation de la synthèse...".to_string(),
            status: "running".to_string(),
            tokens: 0,
        },
    );

    git_diff::sanitize_backtick_paths(&project.root_path).await;
    let (modified_files, lines_added, lines_removed) =
        git_diff::collect_project_diffs(&project.root_path).await;

    if !modified_files.is_empty() && exec_status != "error" {
        let enriched_summary = format!(
            "{} · **{} fichier(s)** modifié(s) (`+{}` / `-{}`)",
            exec_note,
            modified_files.len(),
            lines_added,
            lines_removed
        );
        record_trace_step(
            app,
            &conversation_id,
            &mut agent_trace,
            AgentTraceStep {
                role: exec_role.to_string(),
                role_label: role_display_name(exec_role).to_string(),
                provider: exec_prov.clone(),
                model: exec_model.clone(),
                summary: enriched_summary,
                status: exec_status.clone(),
                tokens: exec_tokens_used,
            },
        );
    }

    let (synthesized_text, synth_tokens) = tokio::select! {
        biased;
        _ = wait_cancelled(&mut cancel_rx) => {
            if let Some(h) = title_handle.take() {
                h.abort();
            }
            return finish_cancelled_pipeline(
                app,
                state_lock,
                &conversation_id,
                &project.root_path,
                &mut agent_trace,
            )
            .await;
        }
        res = synthesize_final_response(
            app,
            state_lock,
            &conversation_id,
            &user_prompt,
            &cleaned_narrative,
            engine_ran,
            stack.ollama_running,
            &synth_prov,
            &synth_model,
            &settings,
            &modified_files,
            &turn_applied_files,
            extra_stage_edits,
            &verifier_note,
        ) => res,
    };

    let synth_summary = if synth_tokens > 0 {
        format!(
            "Synthèse finale (**{} fichier(s)** modifié(s)) et mise à jour mémoire",
            modified_files.len()
        )
    } else {
        format!(
            "Mise en forme finale (**{} fichier(s)** modifié(s)) sans appel LLM",
            modified_files.len()
        )
    };
    record_trace_step(
        app,
        &conversation_id,
        &mut agent_trace,
        AgentTraceStep {
            role: "synthesizer".to_string(),
            role_label: "Synthétiseur".to_string(),
            provider: synth_prov.clone(),
            model: synth_model.clone(),
            summary: synth_summary,
            status: "done".to_string(),
            tokens: synth_tokens,
        },
    );

    let new_artifacts = extract_artifacts_from_markdown(&cleaned_narrative);

    let mut thinking_sections: Vec<String> = Vec::new();
    if let Some(t) = extracted_thinking {
        thinking_sections.push(t);
    }
    if !tool_results.is_empty() || !tester_note.is_empty() || !verifier_note.is_empty() || had_repairs {
        let mut notes = Vec::new();
        for r in &tool_results {
            notes.push(format!("• 🛠️ {}", r.message));
        }
        if had_repairs {
            notes.push("• 🔄 Auto-réparation / correction appliquée avec succès.".to_string());
        }
        if !tester_note.is_empty() {
            notes.push(tester_note);
        }
        if !verifier_note.is_empty() {
            notes.push(verifier_note);
        }
        thinking_sections.push(format!(
            "### Contrôles & Outils exécutés en local\n{}",
            notes.join("\n")
        ));
    }
    if engine_ran && had_repairs && !raw_exec_output.trim().is_empty() {
        thinking_sections.push(format!(
            "### Détail brut du dernier tour moteur\n{}",
            truncate_chars(raw_exec_output.trim(), 2500)
        ));
    }
    let thinking_with_tools = if thinking_sections.is_empty() {
        None
    } else {
        Some(thinking_sections.join("\n\n"))
    };

    if let Some(h) = title_handle.take() {
        let _ = tokio::time::timeout(std::time::Duration::from_millis(1500), h).await;
    }

    let result = {
        let mut guard = state_lock.lock().await;
        let now = Utc::now().to_rfc3339();

        let conv_idx = guard
            .conversations
            .iter()
            .position(|c| c.id == conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Conversation introuvable"))?;

        guard.conversations[conv_idx].messages.push(ChatMessage {
            id: Uuid::new_v4().to_string(),
            role: "assistant".to_string(),
            content: synthesized_text.clone(),
            thinking: thinking_with_tools,
            attachments: None,
            created_at: now.clone(),
            stage_summary: Some(stage_note),
            files_changed: modified_files.len(),
            lines_added,
            lines_removed,
            files_summary: build_files_summary(&modified_files),
            agent_trace,
        });

        for art in new_artifacts {
            guard.conversations[conv_idx].artifacts.push(art);
        }
        guard.conversations[conv_idx].modified_files = modified_files.clone();
        guard.conversations[conv_idx].lines_added = lines_added;
        guard.conversations[conv_idx].lines_removed = lines_removed;
        guard.conversations[conv_idx].updated_at = now.clone();

        let text_without_code = strip_code_blocks(&synthesized_text);
        let summary_snippet = truncate_chars(text_without_code.replace('\n', " ").trim(), 180);
        let conv_entry = format!(
            "• [{}] Demande: {} → Résumé: {}",
            user_msg_time.get(..16).unwrap_or(&user_msg_time),
            truncate_chars(user_prompt.trim(), 90),
            summary_snippet
        );
        let prev_conv_ctx = guard.conversations[conv_idx].conversation_context.clone();
        guard.conversations[conv_idx].conversation_context =
            update_rolling_conversation_context(&prev_conv_ctx, &conv_entry, 6);

        let proj_id = guard.conversations[conv_idx].project_id.clone();
        let proj_idx = guard
            .projects
            .iter()
            .position(|p| p.id == proj_id)
            .ok_or_else(|| anyhow::anyhow!("Projet introuvable"))?;
        guard.projects[proj_idx].updated_at = now;

        let updated_conv = guard.conversations[conv_idx].clone();
        let updated_proj = guard.projects[proj_idx].clone();
        let _ = storage::save_state(&guard).await;

        SendPromptResult {
            conversation: updated_conv,
            project: updated_proj,
        }
    };

    emit_stage(
        app,
        &conversation_id,
        5,
        "Prêt",
        "Réponse synthétisée et contexte synchronisé.",
        "done",
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aiderstudio_test_{}_{}", name, Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_match_files_deterministically() {
        let all_files = vec![
            "package.json".to_string(),
            "vite.config.js".to_string(),
            "src/App.vue".to_string(),
            "src/stores/studio.ts".to_string(),
            "src/components/LeHud.vue".to_string(),
        ];

        let (m1, exp1) = match_files_deterministically(
            "Corrige le problème de `vite.config.js`.",
            None,
            &all_files,
        );
        assert!(exp1);
        assert_eq!(m1, vec!["vite.config.js"]);

        let (m2, exp2) = match_files_deterministically(
            "Mets à jour studio.ts et LeHud.vue pour afficher le pic.",
            None,
            &all_files,
        );
        assert!(exp2);
        assert_eq!(m2, vec!["src/stores/studio.ts", "src/components/LeHud.vue"]);

        let (m3, exp3) = match_files_deterministically(
            "Vérifie pourquoi Vite ne démarre pas le serveur web",
            None,
            &all_files,
        );
        assert!(!exp3);
        assert_eq!(m3, vec!["vite.config.js", "package.json"]);
    }

    #[test]
    fn test_choose_execution_strategy_adaptive() {
        let dir = make_temp_dir("strategy");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("vite.config.js"), "export default {}\n").unwrap();
        let big_file = "const x = 1;\n".repeat(280);
        std::fs::write(dir.join("src/Large.ts"), big_file).unwrap();
        let medium_file = "const y = 2;\n".repeat(150);
        std::fs::write(dir.join("src/Medium.ts"), medium_file).unwrap();

        let settings = AppSettings::default();

        // Single small explicit file on Ollama 8K -> whole + map 0 + 2 turns
        let s1 = choose_execution_strategy(
            &dir,
            &["vite.config.js".to_string()],
            true,
            "ollama",
            &settings,
        );
        assert_eq!(s1.edit_format, "whole");
        assert_eq!(s1.map_tokens, 0);
        assert_eq!(s1.max_turns, 2);

        // Large single explicit file -> diff + map 0
        let s2 = choose_execution_strategy(
            &dir,
            &["src/Large.ts".to_string()],
            true,
            "ollama",
            &settings,
        );
        assert_eq!(s2.edit_format, "diff");
        assert_eq!(s2.map_tokens, 0);

        // Non-explicit match -> diff + map 192
        let s2b = choose_execution_strategy(
            &dir,
            &["src/Large.ts".to_string()],
            false,
            "ollama",
            &settings,
        );
        assert_eq!(s2b.edit_format, "diff");
        assert_eq!(s2b.map_tokens, 192);

        // Ollama 16K -> 3 turns + map 256 for single explicit
        let mut settings_16k = settings.clone();
        settings_16k.context_window_tokens = 16_384;
        let s_16k = choose_execution_strategy(
            &dir,
            &["vite.config.js".to_string()],
            true,
            "ollama",
            &settings_16k,
        );
        assert_eq!(s_16k.max_turns, 3);
        assert_eq!(s_16k.map_tokens, 256);

        // Cloud provider on 150-line file (> 120 threshold) -> diff + 5 turns + map 1024
        let s_cloud = choose_execution_strategy(
            &dir,
            &["src/Medium.ts".to_string()],
            true,
            "gemini",
            &settings,
        );
        assert_eq!(s_cloud.edit_format, "diff");
        assert_eq!(s_cloud.max_turns, 5);
        assert_eq!(s_cloud.map_tokens, 1024);

        // 2 small files -> whole + map 192
        std::fs::write(dir.join("package.json"), "{}\n").unwrap();
        let s3 = choose_execution_strategy(
            &dir,
            &["vite.config.js".to_string(), "package.json".to_string()],
            true,
            "ollama",
            &settings,
        );
        assert_eq!(s3.edit_format, "whole");
        assert_eq!(s3.map_tokens, 192);

        // Manual override -> diff
        let mut s_diff_override = settings.clone();
        s_diff_override.aider_edit_format = "diff".to_string();
        let s4 = choose_execution_strategy(
            &dir,
            &["vite.config.js".to_string()],
            true,
            "ollama",
            &s_diff_override,
        );
        assert_eq!(s4.edit_format, "diff");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_classify_prompt_role() {
        assert_eq!(
            classify_prompt_role("Crée un design moderne avec menu à gauche 100% hauteur"),
            ("designer", false)
        );
        assert_eq!(
            classify_prompt_role("Prépare un README.md avec architecture du projet"),
            ("coder", false)
        );
        assert_eq!(
            classify_prompt_role("Factorise et nettoie ce module TypeScript"),
            ("refactoring", false)
        );
        assert_eq!(
            classify_prompt_role("Écris des tests unitaires Vitest pour le store"),
            ("tester", true)
        );
        assert_eq!(
            classify_prompt_role("Implémente un endpoint avec tests unitaires"),
            ("coder", true)
        );
    }

    #[test]
    fn test_designer_auto_includes_global_css() {
        let all_files = vec![
            "src/components/Sidebar.vue".to_string(),
            "src/style.css".to_string(),
            "src/App.vue".to_string(),
            "package.json".to_string(),
        ];
        let mut matched = vec!["src/components/Sidebar.vue".to_string()];
        augment_design_target_files(&all_files, &mut matched);
        assert!(matched.contains(&"src/style.css".to_string()));
        assert!(matched.contains(&"src/App.vue".to_string()));
        assert!(matched.len() <= 3);
    }

    #[test]
    fn test_resolve_agent_slot_override_vs_agent() {
        let mut settings = AppSettings::default();
        if let Some(gem) = settings.providers.iter_mut().find(|p| p.id == "gemini") {
            gem.api_key = "test-gem-key".to_string();
            gem.validated = true;
            gem.available_models = vec!["gemini-2.5-flash".to_string()];
        }
        settings.agent_routing.designer.provider = "gemini".to_string();
        settings.agent_routing.designer.model = "gemini-2.5-flash".to_string();
        settings.active_target = "agent".to_string();

        assert_eq!(
            resolve_agent_slot("designer", &settings),
            ("gemini".to_string(), "gemini-2.5-flash".to_string())
        );
        assert_eq!(
            resolve_agent_slot("coder", &settings),
            ("ollama".to_string(), settings.ollama_model.clone())
        );

        settings.active_target = "ollama:qwen2.5-coder:7b".to_string();
        assert_eq!(
            resolve_agent_slot("designer", &settings),
            ("ollama".to_string(), "qwen2.5-coder:7b".to_string())
        );
    }

    #[test]
    fn test_extract_tool_calls_and_unclosed_check() {
        let txt = r#"Analyse en cours.
<tool_call>{"name":"search_code","arguments":{"query":"run_prompt"}}</tool_call>
```json:tool_call
{"name":"read_file","arguments":{"path":"src/App.vue"}}
```"#;
        let calls = extract_tool_calls(txt);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "search_code");
        assert_eq!(calls[1].name, "read_file");

        assert!(!has_unclosed_fence_or_patch("```ts\nconst a = 1;\n```"));
        assert!(has_unclosed_fence_or_patch("```ts\nconst a = 1;"));
        assert!(has_unclosed_fence_or_patch("<<<<<<< SEARCH\ncode\n======="));
    }

    #[test]
    fn test_build_native_system_prompt_diff_vs_whole_and_mcp() {
        let srv = McpServerConfig {
            id: "1".to_string(),
            name: "sqlite-db".to_string(),
            server_type: Some("sqlite".to_string()),
            command: "npx".to_string(),
            args: vec![],
            env: Default::default(),
            enabled: true,
            project_id: None,
        };
        let p_diff = build_native_system_prompt("diff", &[&srv], "Outil `read_query`");
        assert!(p_diff.contains("SEARCH/REPLACE"));
        assert!(p_diff.contains("mcp_call"));
        assert!(p_diff.contains("sqlite-db"));
        assert!(p_diff.contains("read_query"));

        let p_whole = build_native_system_prompt("whole", &[], "");
        assert!(p_whole.contains("FORMAT D'ÉDITION (`whole`)"));
        assert!(!p_whole.contains("mcp_call"));
    }

    #[test]
    fn test_extract_artifacts_ignores_search_replace_diff() {
        let md = r#"Explication.

```javascript
<<<<<<< SEARCH
const a = 1;
=======
const a = 2;
>>>>>>> REPLACE
```

```ts
export function cleanHelper(x: number): number {
  return x + 1;
}
```"#;
        let arts = extract_artifacts_from_markdown(md);
        assert_eq!(arts.len(), 1);
        assert_eq!(arts[0].language, "ts");
        assert!(!arts[0].content.contains("<<<<<<< SEARCH"));
    }

    #[test]
    fn test_update_rolling_conversation_context_fifo() {
        let initial = "Objectif : Migrer vers Vite 6 sans casser Tauri.";
        let mut ctx = initial.to_string();
        for i in 1..=8 {
            let entry = format!("• [2026-11-18T22:0{i}] Demande: étape {i} → Résumé: fait {i}");
            ctx = update_rolling_conversation_context(&ctx, &entry, 6);
        }

        assert!(ctx.contains("Objectif : Migrer vers Vite 6 sans casser Tauri."));
        assert!(!ctx.contains("étape 1"));
        assert!(!ctx.contains("étape 2"));
        assert!(ctx.contains("étape 3"));
        assert!(ctx.contains("étape 8"));
    }

    #[test]
    fn test_clean_generated_title() {
        assert_eq!(
            clean_generated_title("\"Refactoring du menu latéral.\"\nAutre ligne").as_deref(),
            Some("Refactoring du menu latéral")
        );
        assert_eq!(
            clean_generated_title("## Titre : « Optimisation cache Vite! »").as_deref(),
            Some("Optimisation cache Vite")
        );
        assert_eq!(clean_generated_title("   \n\"\"  "), None);
    }

    #[test]
    fn test_list_project_files_includes_github_workflows_but_skips_git() {
        let dir = make_temp_dir("github_idx");
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        std::fs::create_dir_all(dir.join(".git/objects")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join(".github/workflows/build.yml"), "name: CI\n").unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1\n").unwrap();
        std::fs::write(dir.join("src/App.vue"), "<template></template>\n").unwrap();

        let files = list_project_files_internal(&dir);
        assert!(files.contains(&".github/workflows/build.yml".to_string()));
        assert!(files.contains(&"src/App.vue".to_string()));
        assert!(!files.iter().any(|f| f.starts_with(".git/")));
        assert!(!files.iter().any(|f| f == ".env"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_append_modified_files_summary_no_markdown_duplication() {
        let diffs = vec![
            ModifiedFileDiff {
                path: "src/App.vue".to_string(),
                added: 12,
                removed: 2,
                modified: 5,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/new.ts".to_string(),
                added: 10,
                removed: 0,
                modified: 0,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/old.ts".to_string(),
                added: 0,
                removed: 8,
                modified: 0,
                lines: Vec::new(),
            },
        ];

        let with_text = append_modified_files_summary(
            "  Titre MinouAI ajouté dans `src/App.vue`.  ",
            &diffs,
            &["src/App.vue".to_string()],
        );
        assert_eq!(with_text, "Titre MinouAI ajouté dans [App.vue](src/App.vue).");
        assert!(!with_text.contains("### Fichiers modifiés"));
        assert!(!with_text.contains("### Fichiers créés"));
        assert!(!with_text.contains("### Fichiers supprimés"));

        let empty_one = append_modified_files_summary("   ", &diffs, &["src/App.vue".to_string()]);
        assert_eq!(empty_one, "Modifications appliquées dans [App.vue](src/App.vue).");

        let empty_multi = append_modified_files_summary("   ", &diffs, &[]);
        assert!(empty_multi.contains("- Mise à jour appliquée dans [App.vue](src/App.vue)."));
        assert!(empty_multi.contains("- Mise à jour appliquée dans [new.ts](src/new.ts)."));
        assert!(!empty_multi.contains("### Fichiers"));

        let empty_no_diffs = append_modified_files_summary("", &[], &[]);
        assert_eq!(
            empty_no_diffs,
            "Traitement terminé sans modification de fichier."
        );
    }

    #[test]
    fn test_has_multiple_requests_detection() {
        assert!(has_multiple_requests("1. Remplace l'image par 🙀.\n2. Ajoute le titre MinouAI."));
        assert!(has_multiple_requests("1) Corriger le bug 2) Mettre à jour le style"));
        assert!(has_multiple_requests("- Ajouter le bouton\n- Centrer le titre"));
        assert!(has_multiple_requests(
            "Change le logo dans App.vue.\nÉgalement, ajoute le titre en blanc."
        ));
        assert!(has_multiple_requests(
            "Mettre à jour le header. Par ailleurs, ajouter les tests unitaires."
        ));
        assert!(!has_multiple_requests("Ajoute l'emote 🙀 et affiche le titre MinouAI au centre."));
        assert!(!has_multiple_requests("Rends le bouton aussi grand que le conteneur."));
    }

    #[test]
    fn test_linkify_file_mentions_preserves_code_and_existing_links() {
        let diffs = vec![
            ModifiedFileDiff {
                path: "src/App.vue".to_string(),
                added: 4,
                removed: 1,
                modified: 0,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/components/LeHud.vue".to_string(),
                added: 2,
                removed: 0,
                modified: 1,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/a/index.ts".to_string(),
                added: 1,
                removed: 0,
                modified: 0,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/b/index.ts".to_string(),
                added: 1,
                removed: 0,
                modified: 0,
                lines: Vec::new(),
            },
        ];

        let input = r#"Modifications effectuées :
- Dans `src/App.vue`, l'image est remplacée par l'emote.
- Dans LeHud.vue et src/a/index.ts, le compteur est ajusté sans toucher à index.ts ambigu.
- Lien déjà présent : [App.vue](src/App.vue) et URL https://example.com/src/App.vue.

```ts
// ne pas toucher à src/App.vue ici
const p = "LeHud.vue";
```"#;

        let out = linkify_file_mentions(input, &diffs);
        assert!(out.contains("Dans [App.vue](src/App.vue), l'image est remplacée"));
        assert!(out.contains("Dans [LeHud.vue](src/components/LeHud.vue) et [index.ts](src/a/index.ts), le compteur est ajusté sans toucher à index.ts ambigu."));
        assert!(out.contains("Lien déjà présent : [App.vue](src/App.vue)"));
        assert!(out.contains("URL https://example.com/src/App.vue."));
        assert!(out.contains("// ne pas toucher à src/App.vue ici\nconst p = \"LeHud.vue\";"));
    }

    #[test]
    fn test_narrative_needs_structured_synthesis() {
        let files = vec!["src/App.vue".to_string(), "src/styles/main.css".to_string()];

        assert!(!narrative_needs_structured_synthesis("", &[], "Bonjour", false));
        assert!(narrative_needs_structured_synthesis(
            "J'ai modifié App.vue pour ajouter le titre.",
            &files,
            "Mets à jour App.vue et main.css",
            false
        ));
        assert!(narrative_needs_structured_synthesis(
            "J'ai modifié App.vue et main.css comme demandé.",
            &files,
            "1. Modifie App.vue\n2. Modifie main.css",
            false
        ));
        assert!(!narrative_needs_structured_synthesis(
            "- Ajout du titre dans [App.vue](src/App.vue)\n- Ajustement du contraste dans [main.css](src/styles/main.css)",
            &files,
            "1. Modifie App.vue\n2. Modifie main.css",
            false
        ));
        assert!(narrative_needs_structured_synthesis(
            "- Tout est dans [App.vue](src/App.vue) et [main.css](src/styles/main.css)",
            &files,
            "1. Modifie App.vue\n2. Modifie main.css",
            true
        ));
    }
}
