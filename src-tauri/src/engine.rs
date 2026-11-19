use crate::git_diff;
use crate::models::{
    AgentModelSlot, AgentTraceEvent, AgentTraceStep, AppSettings, AppStateData, ArtifactItem,
    AttachedFile, ChatMessage, ModifiedFileDiff, PromptStageEvent, SendPromptResult, StackStatus,
    TokenUsageEvent, ToolExecutionResult,
};
use crate::providers;
use crate::storage;
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
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

#[derive(Debug, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<usize>,
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<OllamaChatMsg<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize)]
struct OllamaChatMsg<'a> {
    role: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    images: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OllamaChatMsgOut {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    message: Option<OllamaChatMsgOut>,
    #[allow(dead_code)]
    done: Option<bool>,
    prompt_eval_count: Option<usize>,
    eval_count: Option<usize>,
}

pub fn list_project_files_internal(project_root: &Path) -> Vec<String> {
    let mut results = Vec::new();
    if !project_root.exists() {
        return results;
    }

    fn walk(dir: &Path, root: &Path, list: &mut Vec<String>) {
        if list.len() >= 1000 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.')
                || name_str == "node_modules"
                || name_str == "target"
                || name_str == "dist"
                || name_str == "build"
                || name_str == "coverage"
                || name_str == ".git"
            {
                continue;
            }
            if path.is_dir() {
                walk(&path, root, list);
            } else if path.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    list.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }

    walk(project_root, project_root, &mut results);
    results.sort();
    results
}

pub const DEFAULT_AIDERIGNORE: &str = r#"# AiderStudio / Aider ignore pattern to preserve 8K context
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
        if trimmed.is_empty()
            || trimmed.starts_with("Error navigating to")
            || trimmed.starts_with("Call log:")
            || trimmed.starts_with("navigating to")
            || trimmed.starts_with("Scraping http")
            || trimmed.starts_with("Failed to retrieve content")
        {
            continue;
        }
        if trimmed.contains("ERR_CONNECTION_REFUSED") {
            clean_lines.push("Connexion locale refusée : le serveur ou service cible n'est pas actif.".to_string());
            break;
        }
        if trimmed.contains("Retrying in ") {
            continue;
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
            || trimmed.contains("Appels répétés")
            || trimmed.contains("litellm.")
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
                    .take(4)
                    .collect();
                if err_lines.is_empty() {
                    truncate_chars(&stderr, 220)
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
                            .take(4)
                            .collect();
                        if err_lines.is_empty() {
                            truncate_chars(&combined, 220)
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
        output_summary: "Aucun runner de test ou compilateur strict à exécuter (étape validée).".to_string(),
    }
}

pub fn apply_search_replace_patch(original: &str, patch: &str) -> Option<String> {
    let re = Regex::new(r"<<<<<<< SEARCH\r?\n([\s\S]*?)\r?\n=======\r?\n([\s\S]*?)\r?\n>>>>>>> REPLACE").ok()?;
    let mut modified = original.to_string();
    let mut applied_count = 0;

    for cap in re.captures_iter(patch) {
        let search = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let replace = cap.get(2).map(|m| m.as_str()).unwrap_or("");

        if search.is_empty() {
            if modified.is_empty() {
                modified = replace.to_string();
                applied_count += 1;
            }
        } else if modified.contains(search) {
            modified = modified.replacen(search, replace, 1);
            applied_count += 1;
        } else {
            let search_trimmed = search.trim();
            if !search_trimmed.is_empty() && modified.contains(search_trimmed) {
                modified = modified.replacen(search_trimmed, replace.trim(), 1);
                applied_count += 1;
            }
        }
    }

    if applied_count > 0 {
        Some(modified)
    } else {
        None
    }
}

pub fn extract_clean_replace_code(patch: &str) -> String {
    if let Ok(re) = Regex::new(r"<<<<<<< SEARCH\r?\n[\s\S]*?\r?\n=======\r?\n([\s\S]*?)\r?\n>>>>>>> REPLACE") {
        let mut parts = Vec::new();
        for cap in re.captures_iter(patch) {
            if let Some(m) = cap.get(1) {
                parts.push(m.as_str().trim_matches('\r'));
            }
        }
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }

    patch
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.starts_with("<<<<<<< SEARCH")
                && !t.starts_with("=======")
                && !t.starts_with(">>>>>>> REPLACE")
        })
        .collect::<Vec<_>>()
        .join("\n")
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
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let clean_path = path_str.trim().trim_matches('`').trim();
            if clean_path.contains("..") {
                return Err("Chemin relatif non autorisé (contient '..')".to_string());
            }
            let target_path = project_root.join(clean_path);
            if let Some(parent) = target_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| e.to_string())?;
            }

            // CRITICAL PROTECTION: Never write raw SEARCH/REPLACE diff markers into files.
            // If the content is a diff, apply it to the existing file or extract clean replacement code.
            let final_content = if content.contains("<<<<<<< SEARCH") && content.contains(">>>>>>> REPLACE") {
                if target_path.exists() {
                    let orig = tokio::fs::read_to_string(&target_path).await.unwrap_or_default();
                    apply_search_replace_patch(&orig, content)
                        .unwrap_or_else(|| extract_clean_replace_code(content))
                } else {
                    extract_clean_replace_code(content)
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
                path: Some(clean_path.to_string()),
            })
        }
        "read_file" => {
            let path_str = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Paramètre 'path' manquant")?;
            let clean_path = path_str.trim().trim_matches('`').trim();
            if clean_path.contains("..") {
                return Err("Chemin relatif non autorisé (contient '..')".to_string());
            }
            let target_path = project_root.join(clean_path);
            let content = tokio::fs::read_to_string(&target_path)
                .await
                .map_err(|e| e.to_string())?;
            Ok(ToolExecutionResult {
                tool: name.to_string(),
                success: true,
                message: content,
                path: Some(clean_path.to_string()),
            })
        }
        "run_command" => {
            let cmd_str = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("Paramètre 'command' manquant")?;

            #[cfg(target_os = "windows")]
            let mut cmd = tokio::process::Command::new("cmd");
            #[cfg(target_os = "windows")]
            cmd.args(["/C", cmd_str]);

            #[cfg(not(target_os = "windows"))]
            let mut cmd = tokio::process::Command::new("sh");
            #[cfg(not(target_os = "windows"))]
            cmd.args(["-c", cmd_str]);

            cmd.current_dir(project_root);
            let output = cmd.output().await.map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stderr.is_empty() {
                stdout.to_string()
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
            let clean_path = path_str.trim().trim_matches('`').trim();
            if clean_path.contains("..") {
                return Err("Chemin relatif non autorisé".to_string());
            }
            let target = project_root.join(clean_path);
            if target.exists() {
                tokio::fs::remove_file(&target)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok(ToolExecutionResult {
                tool: name.to_string(),
                success: true,
                message: format!("Fichier '{}' supprimé", clean_path),
                path: Some(clean_path.to_string()),
            })
        }
        other => Err(format!("Outil inconnu : {}", other)),
    }
}

pub async fn apply_tools_and_file_blocks(
    project_root: &Path,
    assistant_text: &str,
    user_prompt: &str,
    aider_ran: bool,
) -> Vec<ToolExecutionResult> {
    let mut results = Vec::new();
    let mut processed_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Explicit XML tool calls: <tool_call>{...}</tool_call>
    let xml_re = Regex::new(r"<tool_call>([\s\S]*?)</tool_call>").expect("valid xml regex");
    for cap in xml_re.captures_iter(assistant_text) {
        if let Some(json_str) = cap.get(1) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str.as_str().trim()) {
                let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = val
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                if let Ok(res) = execute_local_tool_internal(project_root, name, &args).await {
                    if let Some(p) = &res.path {
                        processed_paths.insert(p.clone());
                    }
                    results.push(res);
                }
            }
        }
    }

    // 2. Markdown tool calls: ```json:tool_call ... ```
    let md_tool_re =
        Regex::new(r"```(?:json:)?tool_call\n([\s\S]*?)```").expect("valid tool call regex");
    for cap in md_tool_re.captures_iter(assistant_text) {
        if let Some(json_str) = cap.get(1) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str.as_str().trim()) {
                let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = val
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                if let Ok(res) = execute_local_tool_internal(project_root, name, &args).await {
                    if let Some(p) = &res.path {
                        processed_paths.insert(p.clone());
                    }
                    results.push(res);
                }
            }
        }
    }

    // 3. Heuristic Code Fence Extractor: ONLY executed when Aider CLI did not run.
    // When Aider runs, Aider already applies the edits directly to files on disk.
    // Parsing Aider's stdout diffs/explanations and blindly overwriting files would corrupt them with diff syntax!
    if !aider_ran {
        let path_re = Regex::new(r#"(?:^|[\s"'\*#`:])([a-zA-Z0-9_\-./]+\.[a-zA-Z0-9]{1,8})(?:[\s"'\*`:]|$)"#)
            .expect("valid path regex");
        let any_fence_re = Regex::new(r"```([^\n]*)\n([\s\S]*?)```").expect("valid fence regex");

        for cap in any_fence_re.captures_iter(assistant_text) {
            let full_match = cap.get(0).unwrap();
            let info = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let code = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            if code.trim().is_empty() {
                continue;
            }

            let mut detected_path: Option<String> = None;

            // A) Path in info string: ```html:index.html or ```src/App.vue
            if info.contains(':') {
                let candidate = info.split(':').last().unwrap_or("").trim().trim_matches('`').trim_matches('\'').trim_matches('"');
                if path_re.is_match(candidate) {
                    detected_path = Some(candidate.to_string());
                }
            } else if path_re.is_match(info) && info.contains('.') {
                detected_path = Some(info.trim_matches('`').trim_matches('\'').trim_matches('"').to_string());
            }

            // B) Path in preceding lines (up to 4 lines back)
            if detected_path.is_none() {
                let before_fence = &assistant_text[..full_match.start()];
                let non_empty_lines: Vec<&str> = before_fence
                    .lines()
                    .rev()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .take(4)
                    .collect();

                for line in non_empty_lines {
                    // Ignore lines that look like shell commands or markdown headers without path
                    if line.starts_with("$ ") || line.starts_with("npx ") || line.starts_with("npm ") || line.starts_with("cd ") {
                        continue;
                    }
                    if let Some(p_cap) = path_re.captures(line) {
                        if let Some(m) = p_cap.get(1) {
                            let candidate = m.as_str().trim().trim_matches('`').trim_matches('\'').trim_matches('"').to_string();
                            // Verify candidate has an extension and isn't just a domain or general word
                            if candidate.contains('.') && !candidate.ends_with('.') && !candidate.starts_with("http") {
                                detected_path = Some(candidate);
                                break;
                            }
                        }
                    }
                }
            }

            // C) Path in the first line of the code block itself (e.g. `// src/App.js` or `/* styles.css */` or `<!-- index.html -->`)
            if detected_path.is_none() {
                let first_line = code.lines().next().unwrap_or("").trim();
                if first_line.starts_with("//") || first_line.starts_with("/*") || first_line.starts_with('#') || first_line.starts_with("<!--") {
                    if let Some(p_cap) = path_re.captures(first_line) {
                        if let Some(m) = p_cap.get(1) {
                            let candidate = m.as_str().trim().trim_matches('`').trim_matches('\'').trim_matches('"').to_string();
                            if candidate.contains('.') && !candidate.ends_with('.') {
                                detected_path = Some(candidate);
                            }
                        }
                    }
                }
            }

            // D) Heuristic fallback based on prompt and code content
            if detected_path.is_none() {
                let lower_prompt = user_prompt.to_lowercase();
                let is_html = code.contains("<!DOCTYPE html>") || code.contains("<html");
                let is_css = (code.contains('{') && code.contains('}')) && (code.contains("margin:") || code.contains("padding:") || code.contains("color:") || code.contains("font-family:"));
                let is_js = code.contains("function ") || code.contains("const ") || code.contains("let ") || code.contains("import ");

                if is_html && (lower_prompt.contains("index.html") || lower_prompt.contains("site web") || lower_prompt.contains("page html") || lower_prompt.contains("vitrine") || lower_prompt.contains("site")) {
                    if !processed_paths.contains("index.html") {
                        detected_path = Some("index.html".to_string());
                    }
                } else if is_css && (lower_prompt.contains("css") || lower_prompt.contains("style")) {
                    if !processed_paths.contains("style.css") && !processed_paths.contains("styles.css") {
                        detected_path = Some("styles.css".to_string());
                    }
                } else if is_js && (lower_prompt.contains("script.js") || lower_prompt.contains("app.js")) {
                    if !processed_paths.contains("app.js") && !processed_paths.contains("script.js") {
                        detected_path = Some("app.js".to_string());
                    }
                }
            }

            // If a path was successfully detected and not already written in this turn:
            if let Some(raw_path) = detected_path {
                let clean_path = raw_path
                    .trim()
                    .trim_matches('`')
                    .trim_matches('\'')
                    .trim_matches('"')
                    .trim_start_matches("./")
                    .to_string();

                // Avoid writing unwanted files like package.json or node_modules or command paths
                if !clean_path.is_empty()
                    && !clean_path.contains("..")
                    && !clean_path.starts_with("http")
                    && !processed_paths.contains(&clean_path)
                {
                    // If code contains SEARCH/REPLACE diff markers, apply as patch or extract clean code, never write raw diff!
                    let target = project_root.join(&clean_path);
                    if code.contains("<<<<<<< SEARCH") && code.contains(">>>>>>> REPLACE") {
                        if target.exists() {
                            if let Ok(orig) = tokio::fs::read_to_string(&target).await {
                                if let Some(patched) = apply_search_replace_patch(&orig, code) {
                                    if tokio::fs::write(&target, patched).await.is_ok() {
                                        processed_paths.insert(clean_path.clone());
                                        results.push(ToolExecutionResult {
                                            tool: "patch_file".to_string(),
                                            success: true,
                                            message: format!("Patch appliqué avec succès à '{}'", clean_path),
                                            path: Some(clean_path),
                                        });
                                    }
                                }
                            }
                        }
                    } else {
                        let write_args = serde_json::json!({
                            "path": clean_path,
                            "content": code,
                        });
                        if let Ok(res) = execute_local_tool_internal(project_root, "write_file", &write_args).await {
                            processed_paths.insert(clean_path);
                            results.push(res);
                        }
                    }
                }
            }
        }
    }

    // After writing files, track changes in Git repository
    if !results.is_empty() && project_root.exists() {
        let _ = tokio::process::Command::new("git")
            .args(["add", "-A", "--", "."])
            .current_dir(project_root)
            .output()
            .await;
    }

    results
}

pub fn extract_thinking_and_content(raw: &str) -> (Option<String>, String) {
    let think_start_tag = "<think>";
    let think_end_tag = "</think>";

    if let Some(start_pos) = raw.find(think_start_tag) {
        let after_start = &raw[start_pos + think_start_tag.len()..];
        if let Some(end_pos) = after_start.find(think_end_tag) {
            let thinking = after_start[..end_pos].trim().to_string();
            let before = &raw[..start_pos];
            let after = &after_start[end_pos + think_end_tag.len()..];
            let content = format!("{}{}", before, after).trim().to_string();
            return (
                if thinking.is_empty() {
                    None
                } else {
                    Some(thinking)
                },
                content,
            );
        } else {
            let thinking = after_start.trim().to_string();
            let before = raw[..start_pos].trim().to_string();
            return (
                if thinking.is_empty() {
                    None
                } else {
                    Some(thinking)
                },
                before,
            );
        }
    }

    (None, raw.trim().to_string())
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
            search_dirs.push(home.join("Library/Python/3.12/bin"));
            search_dirs.push(home.join("Library/Python/3.11/bin"));
            search_dirs.push(home.join("Library/Python/3.10/bin"));
            search_dirs.push(home.join("Library/Python/3.9/bin"));
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(local) = dirs::data_local_dir() {
                search_dirs.push(local.join("Programs").join("Ollama"));
                search_dirs.push(local.join("Programs").join("Python").join("Python312").join("Scripts"));
                search_dirs.push(local.join("Programs").join("Python").join("Python311").join("Scripts"));
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
    let aider_bin = resolve_binary("aider", &settings.aider_path);
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
        aider_installed: aider_bin.is_some(),
        aider_bin,
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
        // Exclude raw SEARCH/REPLACE diff blocks from becoming artifacts
        if code.contains("<<<<<<< SEARCH") || code.contains(">>>>>>> REPLACE") {
            continue;
        }
        // Exclude internal tool call JSON strings from becoming artifacts
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

fn build_augmented_prompt(
    global_ctx: &str,
    conv_ctx: &str,
    user_prompt: &str,
    attachments: Option<&[AttachedFile]>,
    shared_dirs: &[String],
) -> String {
    let mut parts = Vec::new();
    if !global_ctx.trim().is_empty() {
        parts.push(format!("[Contexte global du projet]\n{}", global_ctx.trim()));
    }
    if !shared_dirs.is_empty() {
        parts.push(format!(
            "[Dossiers partagés et répertoires de ressources mis à disposition de l'agent]\n{}",
            shared_dirs.iter().map(|d| format!("- {}", d)).collect::<Vec<_>>().join("\n")
        ));
    }
    if !conv_ctx.trim().is_empty() {
        parts.push(format!("[Objectif & Contexte spécifique de cette discussion]\n{}", conv_ctx.trim()));
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

    parts.push(
        "[CONSIGNE STRICTE ET ABSOLUE : LANGUE FRANÇAISE]\n\
        Tu réponds TOUJOURS ET EXCLUSIVEMENT EN FRANÇAIS. Ne formule JAMAIS tes explications, résumés, plans d'action ou réflexions en anglais. Tout le texte explicatif DOIT être en français.\n\n\
        [Création et Édition de Fichiers en Local]\n\
        Tu peux créer et modifier directement les fichiers du projet sur le disque. Pour chaque fichier à créer ou modifier, utilise OBLIGATOIREMENT un bloc de code complet précédé du chemin de fichier relatif au projet :\n\
        ### chemin/du/fichier.ext\n\
        ```langage\n\
        // Contenu complet du fichier...\n\
        ```\n\n\
        [Commandes Terminal pour l'Utilisateur]\n\
        Si tu veux indiquer à l'utilisateur des commandes shell à exécuter (ex: npm install, npm run dev, cargo run), écris-les UNIQUEMENT dans un bloc de code bash standard :\n\
        ```bash\n\
        npm install\n\
        npm run dev\n\
        ```\n\
        Ne génère JAMAIS de JSON brut de tool_call comme {\"name\":\"run_command\"...} dans ta réponse visible !\n\
        Si tu souhaites expliciter ta réflexion pas à pas, place impérativement ta pensée entre les balises <think> et </think>.\n\
        - Fournis toujours du code COMPLET et directement opérationnel (pas de 'TODO' ou de code tronqué).\n\
        - N'ajoute JAMAIS de backtick (`) autour des chemins de fichiers dans les en-têtes d'édition (écris src/App.vue et NON `src/App.vue`)."
            .to_string(),
    );
    parts.push(format!("[Demande utilisateur]\n{}", user_prompt.trim()));
    parts.join("\n\n")
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
    if design_kw.iter().any(|k| lower.contains(k)) {
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

fn build_aider_prompt(
    global_ctx: &str,
    conv_ctx: &str,
    user_prompt: &str,
    attachments: Option<&[AttachedFile]>,
    shared_dirs: &[String],
    architect_spec: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if !global_ctx.trim().is_empty() {
        parts.push(format!(
            "[Contexte global]\n{}",
            truncate_chars(global_ctx.trim(), 1000)
        ));
    }
    if !shared_dirs.is_empty() {
        parts.push(format!(
            "[Dossiers partagés]\n{}",
            shared_dirs
                .iter()
                .map(|d| format!("- {}", d))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let compact_conv = compact_conv_ctx_for_prompt(conv_ctx, 400, 2);
    if !compact_conv.is_empty() {
        parts.push(format!("[Objectif discussion]\n{}", compact_conv));
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
    parts.push("[Consigne] Réponds en français de façon très concise (2 à 3 phrases max pour expliquer la modification).".to_string());
    parts.push(format!("[Demande]\n{}", user_prompt.trim()));
    parts.join("\n\n")
}

pub fn list_source_files_for_cartographer(project_root: &Path) -> Vec<String> {
    let all = list_project_files_internal(project_root);
    let excluded_names = [
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "cargo.lock",
        ".ds_store",
        "thumbs.db",
    ];
    let excluded_suffixes = [
        ".min.js", ".min.css", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".woff",
        ".woff2", ".ttf", ".eot", ".pdf", ".mov", ".mp4", ".mp3", ".zip", ".wasm", ".sqlite",
        ".sqlite3", ".db", ".map",
    ];

    all.into_iter()
        .filter(|rel| {
            let lower = rel.to_lowercase();
            let base = lower.rsplit('/').next().unwrap_or(&lower);
            !excluded_names.contains(&base)
                && !excluded_suffixes.iter().any(|ext| lower.ends_with(ext))
        })
        .collect()
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

    // 1. Exact relative path match
    if let Some(found) = all_files.iter().find(|f| f.eq_ignore_ascii_case(cand)) {
        return Some(found.clone());
    }

    // 2. Path suffix match
    let cand_lower = cand.to_lowercase();
    let suffix = format!("/{cand_lower}");
    if let Some(found) = all_files.iter().find(|f| f.to_lowercase().ends_with(&suffix)) {
        return Some(found.clone());
    }

    // 3. Exact basename match (disambiguate using parent directory if mentioned in prompt, else shortest path)
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

    // Fallback: safe toolchain config keyword hints (only if the file actually exists in all_files)
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

#[derive(Debug, Clone)]
struct CartographerPlan {
    target_files: Vec<String>,
    explicit_match: bool,
    strategy_note: String,
    cartographer_tokens: Option<(usize, usize)>,
}

async fn run_cartographer(
    project_root: &Path,
    user_prompt: &str,
    attachments: Option<&[AttachedFile]>,
    settings: &AppSettings,
    carto_provider: &str,
    carto_model: &str,
    ollama_running: bool,
    is_design_task: bool,
) -> CartographerPlan {
    let all_files = list_source_files_for_cartographer(project_root);
    if all_files.is_empty() {
        return CartographerPlan {
            target_files: Vec::new(),
            explicit_match: false,
            strategy_note: "Projet sans fichiers indexés".to_string(),
            cartographer_tokens: None,
        };
    }

    let (mut tier1, explicit) = match_files_deterministically(user_prompt, attachments, &all_files);
    if is_design_task && !tier1.is_empty() {
        augment_design_target_files(&all_files, &mut tier1);
    }
    let budgeted_tier1 = filter_files_by_size_budget(project_root, tier1, 3, 20_480, 28_672);
    if !budgeted_tier1.is_empty() {
        return CartographerPlan {
            target_files: budgeted_tier1,
            explicit_match: explicit,
            strategy_note: if explicit {
                "0 tok déterministe (mention directe)".to_string()
            } else {
                "0 tok déterministe (mot-clé outillage)".to_string()
            },
            cartographer_tokens: None,
        };
    }

    let can_call_llm = if carto_provider == "ollama" {
        ollama_running
    } else {
        providers::find_provider_key(settings, carto_provider).is_some()
    };

    if !can_call_llm || all_files.len() < 2 {
        let mut fallback = Vec::new();
        if is_design_task {
            augment_design_target_files(&all_files, &mut fallback);
        }
        let budgeted = filter_files_by_size_budget(project_root, fallback, 3, 20_480, 28_672);
        return CartographerPlan {
            target_files: budgeted,
            explicit_match: false,
            strategy_note: "Ciblage auto CSS / repo-map Aider".to_string(),
            cartographer_tokens: None,
        };
    }

    if all_files.len() > 400 {
        let mut fallback = Vec::new();
        if is_design_task {
            augment_design_target_files(&all_files, &mut fallback);
        }
        let budgeted = filter_files_by_size_budget(project_root, fallback, 3, 20_480, 28_672);
        return CartographerPlan {
            target_files: budgeted,
            explicit_match: false,
            strategy_note: "Dépôt volumineux (>400 fichiers) · repo-map Aider".to_string(),
            cartographer_tokens: None,
        };
    }

    let mut prioritized = Vec::new();
    let mut others = Vec::new();
    for f in &all_files {
        if f.starts_with("src/") || f.starts_with("src-tauri/src/") || !f.contains('/') {
            prioritized.push(f.clone());
        } else {
            others.push(f.clone());
        }
    }
    prioritized.extend(others);
    let compact_files: Vec<String> = prioritized.into_iter().take(100).collect();

    let sys = "Tu es un routeur de fichiers. À partir de la liste des fichiers du projet et de la demande utilisateur, retourne UNIQUEMENT un tableau JSON contenant au maximum 3 chemins relatifs exacts de fichiers nécessaires pour répondre (ex: [\"src/App.vue\"] ou []). Aucun texte ni explication.";
    let user = format!(
        "Fichiers:\n{}\n\nDemande:\n{}",
        compact_files.join("\n"),
        truncate_chars(user_prompt.trim(), 400)
    );

    match providers::call_provider_oneshot(carto_provider, carto_model, settings, sys, &user, 100, 0.0).await {
        Ok((reply, p_tok, c_tok)) => {
            let mut routed = Vec::new();
            if let (Some(start), Some(end)) = (reply.find('['), reply.rfind(']')) {
                if start < end {
                    if let Ok(arr) = serde_json::from_str::<Vec<String>>(&reply[start..=end]) {
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
            if is_design_task {
                augment_design_target_files(&all_files, &mut routed);
            }
            let budgeted = filter_files_by_size_budget(project_root, routed, 3, 20_480, 28_672);
            let note = if budgeted.is_empty() {
                "Routeur IA (aucun fichier ciblé)".to_string()
            } else {
                format!("Routeur IA ({carto_provider})")
            };
            CartographerPlan {
                target_files: budgeted,
                explicit_match: false,
                strategy_note: note,
                cartographer_tokens: Some((p_tok, c_tok)),
            }
        }
        Err(_) => {
            let mut fallback = Vec::new();
            if is_design_task {
                augment_design_target_files(&all_files, &mut fallback);
            }
            let budgeted = filter_files_by_size_budget(project_root, fallback, 3, 20_480, 28_672);
            CartographerPlan {
                target_files: budgeted,
                explicit_match: false,
                strategy_note: "Repli repo-map Aider".to_string(),
                cartographer_tokens: None,
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiderStrategy {
    pub edit_format: String,
    pub map_tokens: usize,
    pub max_chat_history_tokens: usize,
    pub max_file_lines: usize,
}

pub fn choose_aider_strategy(
    project_root: &Path,
    target_files: &[String],
    explicit_match: bool,
    provider: &str,
    settings: &AppSettings,
) -> AiderStrategy {
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
            } else if target_files.is_empty() || (max_file_lines <= 250 && total_file_lines <= 350) {
                "whole".to_string()
            } else {
                "diff".to_string()
            }
        }
    };

    let map_tokens = if target_files.len() == 1 && explicit_match {
        0
    } else if !target_files.is_empty() {
        192
    } else {
        384
    };

    let max_chat_history_tokens = if provider == "ollama" && effective_num_ctx(settings) <= 8192 {
        1024
    } else {
        2048
    };

    AiderStrategy {
        edit_format,
        map_tokens,
        max_chat_history_tokens,
        max_file_lines,
    }
}

fn write_aider_model_settings(model_arg: &str, num_ctx: usize) -> Option<PathBuf> {
    let base_dir = storage::state_file_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    if std::fs::create_dir_all(&base_dir).is_err() {
        return None;
    }
    let settings_path = base_dir.join("aider_model_settings.yml");
    let escaped = model_arg.replace('"', "\\\"");
    let yaml = format!(
        "- name: \"{escaped}\"\n  extra_params:\n    num_ctx: {num_ctx}\n"
    );
    if std::fs::write(&settings_path, yaml).is_ok() {
        Some(settings_path)
    } else {
        None
    }
}

fn is_aider_noise_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    let prefixes = [
        "Tokens:",
        "Applied edit to",
        "Added ",
        "Wrote ",
        "Removed ",
        "Commit ",
        "These files should be added",
        "Create new file?",
        "Allow edits to",
        "Run shell command",
        "Repo-map:",
        "Aider v",
        "Main model:",
        "Weak model:",
        "Editor model:",
        "Model: ollama_chat",
        "Git repo:",
        "Note:",
        "Warning:",
        "You can use ",
        "Use /help",
        "See: https://aider.chat",
        "Scraping http",
        "Error navigating to",
        "Page.goto:",
        "Call log:",
        "navigating to",
        "Failed to retrieve content",
        "Protocol error",
        "Repo-map can't include",
        "The LLM did not conform to the edit format",
        "SearchReplaceNoExactMatch",
        "Did you mean to match",
        "The SEARCH section must exactly match",
        "Veuillez appliquer ces modifications",
        "Je suis désolé pour la confusion",
        "Voici les SEARCH/REPLACE",
    ];
    if prefixes.iter().any(|p| t.starts_with(p)) {
        return true;
    }
    if t.contains("SEARCH/REPLACE block") {
        return true;
    }
    if t.starts_with("Veuillez ajouter ") && t.contains("au chat") {
        return true;
    }
    if t.starts_with("Only ") && t.contains("edit block") {
        return true;
    }
    // Standalone backticked filename line like `` `src/App.vue ``
    if t.starts_with('`') && !t.ends_with('`') && !t.contains(' ') {
        return true;
    }
    false
}

pub fn clean_aider_output_for_display(
    raw_output: &str,
    aider_ran: bool,
    modified_files: &[ModifiedFileDiff],
) -> (String, bool) {
    let trimmed = raw_output.trim();
    let had_retries_or_errors = trimmed.contains("SEARCH/REPLACE block failed")
        || trimmed.contains("SearchReplaceNoExactMatch")
        || trimmed.contains("The LLM did not conform to the edit format")
        || trimmed.contains("Veuillez ajouter ")
        || trimmed.contains("Did you mean to match");

    if trimmed.starts_with("⚠️") || trimmed.starts_with("### Stack locale") {
        return (trimmed.to_string(), had_retries_or_errors);
    }

    let modified_paths: Vec<String> = modified_files
        .iter()
        .map(|f| f.path.trim().to_string())
        .collect();
    let shell_langs = ["bash", "sh", "shell", "zsh", "console", "cmd", "powershell"];

    let mut out_lines: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut in_raw_diff = false;
    let mut skip_next_fence = false;
    let mut fence_header = String::new();
    let mut fence_lang = String::new();
    let mut fence_body: Vec<String> = Vec::new();

    for line in trimmed.lines() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") {
            if !in_fence {
                in_fence = true;
                fence_header = line.trim_end().to_string();
                fence_lang = trimmed_start
                    .trim_start_matches('`')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_matches(':')
                    .to_lowercase();
                fence_body.clear();
            } else {
                in_fence = false;
                let joined = fence_body.join("\n");
                let has_diff_markers =
                    joined.contains("<<<<<<< SEARCH") || joined.contains(">>>>>>> REPLACE");
                let is_shell = shell_langs.contains(&fence_lang.as_str());
                let prev_non_empty = out_lines
                    .iter()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| l.trim().trim_matches('#').trim().trim_matches('`').to_string())
                    .unwrap_or_default();
                let prev_is_file = modified_paths.iter().any(|p| p == &prev_non_empty)
                    || (!prev_non_empty.is_empty()
                        && !prev_non_empty.contains(' ')
                        && prev_non_empty.contains('.'));

                let drop_redundant_code = aider_ran
                    && !modified_files.is_empty()
                    && !is_shell
                    && (prev_is_file || fence_body.len() >= 5);

                if !skip_next_fence && !has_diff_markers && !drop_redundant_code && !joined.trim().is_empty() {
                    out_lines.push(fence_header.clone());
                    out_lines.extend(fence_body.iter().cloned());
                    out_lines.push("```".to_string());
                }
                skip_next_fence = false;
                fence_body.clear();
            }
            continue;
        }

        if in_fence {
            fence_body.push(line.to_string());
            continue;
        }

        let t = line.trim();
        if t.starts_with("<<<<<<< SEARCH") {
            in_raw_diff = true;
            continue;
        }
        if in_raw_diff {
            if t.starts_with(">>>>>>> REPLACE") {
                in_raw_diff = false;
            }
            continue;
        }

        if t.starts_with("Did you mean to match")
            || t.starts_with("SearchReplaceNoExactMatch")
            || t.contains("SEARCH/REPLACE block failed")
        {
            skip_next_fence = true;
            continue;
        }

        if is_aider_noise_line(line) {
            continue;
        }

        let unbackticked = t.trim_matches('`');
        if modified_paths.iter().any(|p| p == unbackticked) {
            continue;
        }

        let mut normalized = line.trim_end().to_string();
        if let Some(rest) = normalized.strip_prefix("### ") {
            let clean_heading = rest.trim().trim_matches('`').trim();
            normalized = format!("### {clean_heading}");
        }

        if normalized.is_empty() {
            if !out_lines.last().map(|l| l.is_empty()).unwrap_or(true) {
                out_lines.push(String::new());
            }
        } else {
            out_lines.push(normalized);
        }
    }

    // Deduplicate identical consecutive paragraphs
    let raw_joined = out_lines.join("\n");
    let mut dedup_paras: Vec<String> = Vec::new();
    for para in raw_joined.split("\n\n") {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        if dedup_paras.last().map(|prev| prev.as_str()) != Some(p) {
            dedup_paras.push(p.to_string());
        }
    }

    (dedup_paras.join("\n\n"), had_retries_or_errors)
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

fn append_modified_files_summary(narrative: &str, modified_files: &[ModifiedFileDiff]) -> String {
    if modified_files.is_empty() {
        return if narrative.is_empty() {
            "Traitement terminé sans modification de fichier.".to_string()
        } else {
            narrative.to_string()
        };
    }

    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut deleted = Vec::new();

    for f in modified_files {
        if f.modified == 0 && f.removed == 0 && f.added > 0 {
            created.push(format!("- `{}` (+{} lignes)", f.path, f.added));
        } else if f.added == 0 && f.modified == 0 && f.removed > 0 {
            deleted.push(format!("- `{}` (-{} lignes)", f.path, f.removed));
        } else {
            updated.push(format!(
                "- `{}` (~{} modifiées, +{} / -{})",
                f.path, f.modified, f.added, f.removed
            ));
        }
    }

    let mut sections = Vec::new();
    if !narrative.is_empty() {
        sections.push(narrative.to_string());
    } else {
        sections.push("Modifications appliquées sur le projet.".to_string());
    }
    if !narrative.contains("### Fichiers ") {
        if !created.is_empty() {
            sections.push(format!("### Fichiers créés\n{}", created.join("\n")));
        }
        if !updated.is_empty() {
            sections.push(format!("### Fichiers modifiés\n{}", updated.join("\n")));
        }
        if !deleted.is_empty() {
            sections.push(format!("### Fichiers supprimés\n{}", deleted.join("\n")));
        }
    }

    sections.join("\n\n")
}

#[allow(clippy::too_many_arguments)]
async fn synthesize_final_response(
    app: &AppHandle,
    state_lock: &Arc<Mutex<AppStateData>>,
    conversation_id: &str,
    user_prompt: &str,
    raw_output: &str,
    aider_ran: bool,
    ollama_running: bool,
    synth_provider: &str,
    synth_model: &str,
    settings: &AppSettings,
    modified_files: &[ModifiedFileDiff],
    verifier_note: &str,
) -> (String, String, bool, usize) {
    let (cleaned_narrative, had_retries_or_errors) =
        clean_aider_output_for_display(raw_output, aider_ran, modified_files);

    let can_call_synth = if synth_provider == "ollama" {
        ollama_running
    } else {
        providers::find_provider_key(settings, synth_provider).is_some()
    };

    let needs_llm_synthesis = aider_ran
        && can_call_synth
        && (had_retries_or_errors
            || cleaned_narrative.is_empty()
            || cleaned_narrative.chars().count() > 650);

    let mut synth_tokens = 0usize;
    let core_narrative = if needs_llm_synthesis {
        let diff_summary = if modified_files.is_empty() {
            "Aucune modification appliquée sur le disque.".to_string()
        } else {
            build_compact_diff_summary(modified_files, 900)
        };
        let files_names: Vec<String> = modified_files.iter().map(|f| f.path.clone()).collect();
        let synth_sys = "Tu es l'Architecte Synthétiseur d'AiderStudio. Résume en français, en 2 à 4 puces claires et concises, la cause identifiée et les modifications réellement appliquées (prêt à copier en message de commit). INTERDIT : blocs de code complets, marqueurs SEARCH/REPLACE, excuses ou blabla.";
        let synth_user = format!(
            "Demande : {}\nFichiers modifiés : {}\nExtrait diff :\n{}\nVérification : {}\nNote brute : {}",
            truncate_chars(user_prompt.trim(), 220),
            if files_names.is_empty() {
                "aucun".to_string()
            } else {
                files_names.join(", ")
            },
            diff_summary,
            if verifier_note.is_empty() { "N/A" } else { verifier_note },
            truncate_chars(&cleaned_narrative, 700)
        );

        match providers::call_provider_oneshot(
            synth_provider,
            synth_model,
            settings,
            synth_sys,
            &synth_user,
            220,
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
                if summary.is_empty() {
                    cleaned_narrative.clone()
                } else {
                    summary
                }
            }
            Err(_) => cleaned_narrative.clone(),
        }
    } else {
        cleaned_narrative.clone()
    };

    let final_markdown = append_modified_files_summary(&core_narrative, modified_files);
    (final_markdown, cleaned_narrative, had_retries_or_errors, synth_tokens)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let truncated: String = input.chars().take(max_chars).collect();
    format!("{truncated}…")
}

fn tail_chars(input: &str, max_chars: usize) -> String {
    let count = input.chars().count();
    if count <= max_chars {
        return input.to_string();
    }
    let skipped: String = input.chars().skip(count - max_chars).collect();
    format!("…\n{skipped}")
}

fn parse_aider_token_count(line: &str, marker: &str) -> usize {
    for part in line.split(',') {
        if part.contains(marker) {
            let num_str = part.trim().trim_start_matches("Tokens:").trim();
            let first_word = num_str.split_whitespace().next().unwrap_or("");
            if let Some(val) = first_word.strip_suffix('k') {
                if let Ok(f) = val.parse::<f32>() {
                    return (f * 1000.0) as usize;
                }
            } else if let Ok(n) = first_word.parse::<usize>() {
                return n;
            }
        }
    }
    0
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

enum AiderWaitOutcome {
    Finished(std::process::ExitStatus),
    TimedOut,
    RetryAborted,
    Cancelled,
}

#[allow(clippy::too_many_arguments)]
async fn execute_aider_cli(
    app: &AppHandle,
    conversation_id: &str,
    aider_bin: &str,
    project_root: &str,
    provider: &str,
    model: &str,
    role: &str,
    settings: &AppSettings,
    prompt: &str,
    target_files: &[String],
    strategy: &AiderStrategy,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<(String, usize, usize)> {
    if is_cancelled(cancel_rx) {
        anyhow::bail!("CANCELLED_BY_USER");
    }
    let (model_arg, env_vars) = providers::aider_model_and_env(provider, model, settings)
        .map_err(|e| anyhow::anyhow!(e))?;

    let mut args = vec![
        "--model".to_string(),
        model_arg.clone(),
        "--yes-always".to_string(),
        "--no-pretty".to_string(),
        "--no-auto-commits".to_string(),
        "--no-show-model-warnings".to_string(),
        "--no-check-update".to_string(),
        "--no-detect-urls".to_string(),
        "--no-suggest-shell-commands".to_string(),
        "--no-auto-lint".to_string(),
        "--no-auto-test".to_string(),
        "--no-restore-chat-history".to_string(),
        "--max-chat-history-tokens".to_string(),
        strategy.max_chat_history_tokens.to_string(),
        "--edit-format".to_string(),
        strategy.edit_format.clone(),
        "--map-tokens".to_string(),
        strategy.map_tokens.to_string(),
    ];

    if provider == "ollama" {
        if let Some(settings_path) =
            write_aider_model_settings(&model_arg, effective_num_ctx(settings))
        {
            args.push("--model-settings-file".to_string());
            args.push(settings_path.to_string_lossy().to_string());
        }
    }

    args.push("--chat-language".to_string());
    args.push("fr".to_string());

    for file in target_files {
        args.push("--file".to_string());
        args.push(file.clone());
    }

    args.push("--message".to_string());
    args.push(prompt.to_string());

    let mut cmd = Command::new(aider_bin);
    cmd.current_dir(project_root)
        .args(&args)
        .env("AIDER_ANALYTICS", "false")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env_vars {
        cmd.env(k, v);
    }

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    let mut child = cmd.spawn()?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let collected = Arc::new(Mutex::new(String::new()));
    let c_out = Arc::clone(&collected);
    let app_out = app.clone();
    let conv_id_out = conversation_id.to_string();
    let role_str = role.to_string();
    let provider_str = provider.to_string();
    let model_str = model.to_string();
    let ctx_limit = providers::resolve_model_context_limit(provider, model, settings);
    let subagent_label = format!("{} ({})", role_display_name(role), strategy.edit_format);
    let billed_prompt = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let billed_completion = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let bp_task = Arc::clone(&billed_prompt);
    let bc_task = Arc::clone(&billed_completion);
    let retry_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let abort_notify = Arc::new(tokio::sync::Notify::new());
    let retry_out = Arc::clone(&retry_hits);
    let notify_out = Arc::clone(&abort_notify);

    let out_task = tokio::spawn(async move {
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("Retrying in ") {
                    let count = retry_out.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    if count >= 6 {
                        notify_out.notify_one();
                    }
                }
                let mut guard = c_out.lock().await;
                guard.push_str(&line);
                guard.push('\n');
                drop(guard);
                if line.trim().starts_with("Tokens:") {
                    let prompt_tokens = parse_aider_token_count(&line, "sent");
                    let completion_tokens = parse_aider_token_count(&line, "received");
                    let total_tokens = prompt_tokens + completion_tokens;
                    if total_tokens > 0 {
                        bp_task.fetch_add(prompt_tokens, std::sync::atomic::Ordering::Relaxed);
                        bc_task.fetch_add(completion_tokens, std::sync::atomic::Ordering::Relaxed);
                        let _ = app_out.emit(
                            "aider:tokens",
                            TokenUsageEvent {
                                conversation_id: conv_id_out.clone(),
                                subagent: subagent_label.clone(),
                                prompt_tokens,
                                completion_tokens,
                                total_tokens,
                                role: role_str.clone(),
                                provider: provider_str.clone(),
                                model: model_str.clone(),
                                context_limit: ctx_limit,
                            },
                        );
                    }
                }
                let _ = app_out.emit("aider:stream", (&conv_id_out, &line));
            }
        }
    });

    let c_err = Arc::clone(&collected);
    let app_err = app.clone();
    let conv_id_err = conversation_id.to_string();
    let retry_err = Arc::clone(&retry_hits);
    let notify_err = Arc::clone(&abort_notify);
    let err_task = tokio::spawn(async move {
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    if line.contains("Retrying in ") {
                        let count = retry_err.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        if count >= 6 {
                            notify_err.notify_one();
                        }
                    }
                    let mut guard = c_err.lock().await;
                    guard.push_str(&line);
                    guard.push('\n');
                    drop(guard);
                    let _ = app_err.emit("aider:stream", (&conv_id_err, &line));
                }
            }
        }
    });

    let wait_res = tokio::select! {
        biased;
        _ = wait_cancelled(cancel_rx) => {
            let _ = child.kill().await;
            AiderWaitOutcome::Cancelled
        }
        timed = tokio::time::timeout(std::time::Duration::from_secs(180), child.wait()) => {
            match timed {
                Ok(status_res) => AiderWaitOutcome::Finished(status_res?),
                Err(_) => {
                    let _ = child.kill().await;
                    AiderWaitOutcome::TimedOut
                }
            }
        }
        _ = abort_notify.notified() => {
            let _ = child.kill().await;
            AiderWaitOutcome::RetryAborted
        }
    };

    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), out_task).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), err_task).await;

    let text = collected.lock().await.trim().to_string();
    match wait_res {
        AiderWaitOutcome::Cancelled => {
            anyhow::bail!("CANCELLED_BY_USER");
        }
        AiderWaitOutcome::RetryAborted => {
            let excerpt = tail_chars(&text, 900);
            anyhow::bail!(
                "Appels répétés en échec (quota/rate-limit ou modèle indisponible sur {provider}:{model}) :\n{excerpt}"
            );
        }
        AiderWaitOutcome::TimedOut => {
            if text.is_empty() {
                anyhow::bail!("Délai dépassé (180s) sans réponse de {provider}:{model}.");
            } else {
                let excerpt = tail_chars(&text, 900);
                anyhow::bail!(
                    "Délai dépassé (180s) sur {provider}:{model}. Dernière sortie Aider :\n{excerpt}"
                );
            }
        }
        AiderWaitOutcome::Finished(status) => {
            if !status.success() && text.is_empty() {
                anyhow::bail!("Aider s'est terminé avec le code {:?}", status.code());
            }
        }
    }
    Ok((
        text,
        billed_prompt.load(std::sync::atomic::Ordering::Relaxed),
        billed_completion.load(std::sync::atomic::Ordering::Relaxed),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn execute_ollama_streaming(
    app: &AppHandle,
    conversation_id: &str,
    settings: &AppSettings,
    model: &str,
    role: &str,
    prompt: &str,
    image_base64_list: Vec<String>,
    cancel_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<String> {
    if is_cancelled(cancel_rx) {
        anyhow::bail!("CANCELLED_BY_USER");
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()?;
    let url = format!("{}/api/chat", settings.ollama_url.trim_end_matches('/'));

    let images_opt = if image_base64_list.is_empty() {
        None
    } else {
        Some(image_base64_list)
    };

    let payload = OllamaChatRequest {
        model: model.trim(),
        stream: true,
        messages: vec![
            OllamaChatMsg {
                role: "system",
                content: "Tu es AiderStudio, un expert en pair-programming et génération de code. RÈGLE STRICTE ET ABSOLUE : Tu réponds TOUJOURS ET EXCLUSIVEMENT EN FRANÇAIS. Ne formule jamais tes explications en anglais. Tu as accès aux outils pour créer des fichiers directement sur le disque dur en écrivant des blocs Markdown précédés de leur chemin (ex: ### src/App.vue puis ```vue ... ```) ou via <tool_call>{\"name\":\"write_file\",\"arguments\":{\"path\":\"src/...\",\"content\":\"...\"}}</tool_call>. Tu peux aussi exécuter des commandes terminal via <tool_call>{\"name\":\"run_command\",\"arguments\":{\"command\":\"...\"}}</tool_call>. Si tu réfléchis avant de répondre, place impérativement ta pensée entre les balises <think> et </think>. Tout ton texte explicatif DOIT être en français.",
                images: None,
            },
            OllamaChatMsg {
                role: "user",
                content: prompt,
                images: images_opt,
            },
        ],
        options: Some(OllamaOptions {
            num_ctx: Some(effective_num_ctx(settings)),
        }),
    };

    let mut resp = tokio::select! {
        biased;
        _ = wait_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
        res = client.post(url).json(&payload).send() => res?,
    };
    if !resp.status().is_success() {
        anyhow::bail!("Ollama HTTP {}", resp.status());
    }

    let mut full_accumulated = String::new();
    let mut in_think = false;
    let mut live_thinking = String::new();
    let mut live_content = String::new();
    let mut buffer = String::new();
    let role_label = format!("{} (Ollama)", role_display_name(role));
    let ctx_limit = providers::resolve_model_context_limit("ollama", model, settings);

    loop {
        let chunk_opt = tokio::select! {
            biased;
            _ = wait_cancelled(cancel_rx) => anyhow::bail!("CANCELLED_BY_USER"),
            res = resp.chunk() => res?,
        };
        let Some(chunk_bytes) = chunk_opt else {
            break;
        };
        let text_chunk = String::from_utf8_lossy(&chunk_bytes);
        buffer.push_str(&text_chunk);

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            if let Ok(parsed) = serde_json::from_str::<OllamaStreamChunk>(&line) {
                if parsed.done == Some(true) {
                    let prompt_tokens = parsed.prompt_eval_count.unwrap_or(0);
                    let completion_tokens = parsed.eval_count.unwrap_or(0);
                    let total_tokens = prompt_tokens + completion_tokens;
                    if total_tokens > 0 {
                        let _ = app.emit(
                            "aider:tokens",
                            TokenUsageEvent {
                                conversation_id: conversation_id.to_string(),
                                subagent: role_label.clone(),
                                prompt_tokens,
                                completion_tokens,
                                total_tokens,
                                role: role.to_string(),
                                provider: "ollama".to_string(),
                                model: model.trim().to_string(),
                                context_limit: ctx_limit,
                            },
                        );
                    }
                }
                if let Some(msg) = parsed.message {
                    if let Some(delta) = msg.content {
                        full_accumulated.push_str(&delta);

                        if delta.contains("<think>") {
                            in_think = true;
                        }

                        if in_think {
                            live_thinking.push_str(&delta);
                            let clean_think = live_thinking
                                .replace("<think>", "")
                                .replace("</think>", "");
                            let _ = app.emit(
                                "aider:thinking",
                                (conversation_id, clean_think.trim()),
                            );
                            if delta.contains("</think>") {
                                in_think = false;
                            }
                        } else {
                            live_content.push_str(&delta);
                            let _ = app.emit(
                                "aider:live_chunk",
                                (conversation_id, &live_content),
                            );
                        }
                    }
                }
            }
        }
    }

    if !buffer.trim().is_empty() {
        if let Ok(parsed) = serde_json::from_str::<OllamaStreamChunk>(buffer.trim()) {
            if let Some(msg) = parsed.message {
                if let Some(delta) = msg.content {
                    full_accumulated.push_str(&delta);
                }
            }
        }
    }

    Ok(full_accumulated.trim().to_string())
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
    // Snapshot inputs and record user message immediately
    let (settings, project, conv_ctx, user_msg_time, is_first_message, initial_auto_title) = {
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
        let _ = storage::save_state(&guard).await;
        (settings, project, conv_ctx, now, is_first_message, initial_auto_title)
    };

    // Extract image base64 if present
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

    // Ensure project_root is initialized as a git repo so Aider never climbs to parent directories!
    let project_root_path = PathBuf::from(&project.root_path);
    if project_root_path.exists() {
        if !project_root_path.join(".git").exists() {
            let _ = tokio::process::Command::new("git")
                .args(["init"])
                .current_dir(&project_root_path)
                .output()
                .await;
        }
        // Protect 8K context automatically with .aiderignore
        let _ = ensure_aiderignore(&project_root_path).await;
    }

    let mut stack = check_stack_status(&settings).await;
    if !stack.ollama_running && settings.auto_start_ollama && stack.ollama_installed {
        if let Ok(updated) = start_ollama_service(&settings).await {
            stack = updated;
        }
    }
    let will_run_aider = image_base64_list.is_empty() && stack.aider_bin.is_some();

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
        let arch_sys = "Tu es l'Architecte logiciel d'AiderStudio. Analyse la demande utilisateur et décide du sous-agent d'exécution le plus adapté.\nRetourne UNIQUEMENT un objet JSON valide, sans texte ni markdown autour :\n{\"role\":\"designer|coder|refactoring|tester\",\"runTests\":false,\"spec\":\"3 à 6 consignes techniques précises et concises en français pour guider l'exécution\"}";
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
        match arch_call_res
        {
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

    let aider_prompt_opt = if will_run_aider {
        Some(build_aider_prompt(
            &project.global_context,
            &conv_ctx,
            &user_prompt,
            attachments.as_deref(),
            &project.shared_directories,
            architect_spec.as_deref(),
        ))
    } else {
        None
    };
    let direct_prompt_opt = if !will_run_aider {
        let mut p = build_augmented_prompt(
            &project.global_context,
            &conv_ctx,
            &user_prompt,
            attachments.as_deref(),
            &project.shared_directories,
        );
        if let Some(spec) = architect_spec.as_deref() {
            p.push_str(&format!("\n\n[Plan & Consignes Architecte]\n{spec}"));
        }
        Some(p)
    } else {
        None
    };

    if !architect_llm_ran {
        let init_prompt_ref = aider_prompt_opt
            .as_deref()
            .or(direct_prompt_opt.as_deref())
            .unwrap_or("");
        let init_prompt_tokens = (init_prompt_ref.len() as f64 / 3.8).ceil() as usize;
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

    // SUB-AGENT 2/5: Cartographe & Analyseur
    let (carto_prov, carto_model) = resolve_agent_slot("cartographer", &settings);
    emit_stage(
        app,
        &conversation_id,
        2,
        "2. Cartographe & Analyseur",
        &format!("Sélection des fichiers cibles ({} · {}).", carto_prov, carto_model),
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

    let carto = if will_run_aider {
        let plan = tokio::select! {
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
                &user_prompt,
                attachments.as_deref(),
                &settings,
                &carto_prov,
                &carto_model,
                stack.ollama_running,
                exec_role == "designer",
            ) => p,
        };
        if let Some((p_tok, c_tok)) = plan.cartographer_tokens {
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
        let target_detail = if plan.target_files.is_empty() {
            plan.strategy_note.clone()
        } else {
            format!("Ciblage : {} ({})", plan.target_files.join(", "), plan.strategy_note)
        };
        emit_stage(
            app,
            &conversation_id,
            2,
            "2. Cartographe & Analyseur",
            &target_detail,
            "running",
        );
        plan
    } else {
        CartographerPlan {
            target_files: Vec::new(),
            explicit_match: false,
            strategy_note: "Mode direct".to_string(),
            cartographer_tokens: None,
        }
    };

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

    // SUB-AGENT 3/5: Exécution (Designer / Codeur / Refactoring / Testeur)
    let (exec_prov, exec_model) = resolve_agent_slot(exec_role, &settings);
    let strategy = choose_aider_strategy(
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
    let stage3_detail = if will_run_aider {
        format!(
            "Format {} · map {} tok · {} fichier(s) sur {}:{}",
            strategy.edit_format,
            strategy.map_tokens,
            carto.target_files.len(),
            exec_prov,
            exec_model
        )
    } else {
        format!(
            "Génération et application des modifications ciblées sur {}:{}",
            exec_prov, exec_model
        )
    };
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

    let mut aider_ran = false;
    let mut exec_tokens_used = 0usize;
    let mut exec_status = "done".to_string();
    let exec_note: String;
    let (assistant_text, stage_note) = if will_run_aider {
        // Sanitize any existing conflict markers before Aider runs
        git_diff::sanitize_corrupted_diff_files(&project.root_path).await;

        let aider_bin = stack.aider_bin.as_deref().unwrap();
        let aider_prompt = aider_prompt_opt.unwrap_or_default();

        match execute_aider_cli(
            app,
            &conversation_id,
            aider_bin,
            &project.root_path,
            &exec_prov,
            &exec_model,
            exec_role,
            &settings,
            &aider_prompt,
            &carto.target_files,
            &strategy,
            &mut cancel_rx,
        )
        .await
        {
            Ok((output, bp, bc)) => {
                aider_ran = true;
                exec_tokens_used = bp + bc;
                exec_note = format!(
                    "Exécution Aider CLI (format `{}`, max {} l.)",
                    strategy.edit_format, strategy.max_file_lines
                );
                record_external_api_usage(app, state_lock, &exec_prov, bp, bc).await;
                let final_out = if output.is_empty() {
                    "Aider a terminé l'opération sans sortie textuelle supplémentaire.".to_string()
                } else {
                    output
                };
                (
                    final_out,
                    format!(
                        "{} · {} ({}:{})",
                        role_display_name(exec_role),
                        strategy.edit_format,
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
                let aider_err_clean = synthesize_error_log(&err.to_string());
                if exec_prov != "ollama" && providers::find_provider_key(&settings, &exec_prov).is_some() {
                    emit_stage(
                        app,
                        &conversation_id,
                        3,
                        &stage3_label,
                        &format!(
                            "Aider CLI en échec sur {exec_prov}:{exec_model}, tentative via API directe..."
                        ),
                        "running",
                    );

                    let mut fallback_prompt = build_augmented_prompt(
                        &project.global_context,
                        &conv_ctx,
                        &user_prompt,
                        attachments.as_deref(),
                        &project.shared_directories,
                    );
                    if let Some(spec) = architect_spec.as_deref() {
                        fallback_prompt.push_str(&format!("\n\n[Plan & Consignes Architecte]\n{spec}"));
                    }
                    if !carto.target_files.is_empty() {
                        let mut file_sections = Vec::new();
                        for rel in carto.target_files.iter().take(3) {
                            let full = project_root_path.join(rel);
                            if let Ok(content) = tokio::fs::read_to_string(&full).await {
                                let clipped = truncate_chars(&content, 12_000);
                                file_sections.push(format!("### {rel}\n```\n{clipped}\n```"));
                            }
                        }
                        if !file_sections.is_empty() {
                            fallback_prompt.push_str(&format!(
                                "\n\n[Contenu actuel des fichiers cibles]\n{}",
                                file_sections.join("\n\n")
                            ));
                        }
                    }

                    let direct_sys = "Tu es AiderStudio, un expert en architecture logicielle et génération de code. RÈGLE STRICTE : Réponds TOUJOURS EN FRANÇAIS. Pour modifier ou créer un fichier, écris son chemin exact sans backtick suivi du bloc de code complet (ex: ### src/App.vue puis ```vue ... ```) ou un <tool_call>{\"name\":\"write_file\",\"arguments\":{\"path\":\"...\",\"content\":\"...\"}}</tool_call>.";
                    let direct_fallback_res = tokio::select! {
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
                            &exec_prov,
                            &exec_model,
                            &settings,
                            direct_sys,
                            &fallback_prompt,
                            8192,
                            0.2,
                        ) => res,
                    };
                    match direct_fallback_res {
                        Ok((reply, p_tok, c_tok)) if !reply.trim().is_empty() => {
                            record_external_api_usage(app, state_lock, &exec_prov, p_tok, c_tok).await;
                            let total_tok = p_tok + c_tok;
                            exec_tokens_used = total_tok;
                            exec_status = "fallback".to_string();
                            exec_note = "Aider en échec → Repli API directe réussi".to_string();
                            let _ = app.emit(
                                "aider:tokens",
                                TokenUsageEvent {
                                    conversation_id: conversation_id.clone(),
                                    subagent: format!("{} (API directe)", role_display_name(exec_role)),
                                    prompt_tokens: p_tok,
                                    completion_tokens: c_tok,
                                    total_tokens: total_tok,
                                    role: exec_role.to_string(),
                                    provider: exec_prov.clone(),
                                    model: exec_model.clone(),
                                    context_limit: providers::resolve_model_context_limit(
                                        &exec_prov,
                                        &exec_model,
                                        &settings,
                                    ),
                                },
                            );
                            (
                                reply,
                                format!(
                                    "{} API directe ({}:{})",
                                    role_display_name(exec_role),
                                    exec_prov,
                                    exec_model
                                ),
                            )
                        }
                        Ok(_) => {
                            exec_status = "error".to_string();
                            exec_note = format!("Aider + API directe vides (`{aider_err_clean}`)");
                            (
                                format!(
                                    "⚠️ Aider a échoué (`{aider_err_clean}`) et l'API directe `{exec_prov}:{exec_model}` n'a retourné aucun contenu."
                                ),
                                format!("{exec_prov} erreur"),
                            )
                        }
                        Err(direct_err) => {
                            let direct_clean = synthesize_error_log(&direct_err.to_string());
                            exec_status = "error".to_string();
                            exec_note = direct_clean.clone();
                            let a_note = if direct_clean != aider_err_clean {
                                format!("\n\n**Détail Aider CLI :** `{aider_err_clean}`")
                            } else {
                                String::new()
                            };
                            (
                                format!(
                                    "⚠️ Erreur sur `{exec_prov}:{exec_model}` :\n`{direct_clean}`{a_note}\n\nVérifiez le modèle sélectionné, vos quotas API ou réessayez dans quelques instants."
                                ),
                                format!("{exec_prov} erreur"),
                            )
                        }
                    }
                } else {
                    exec_status = "error".to_string();
                    exec_note = aider_err_clean.clone();
                    (
                        format!(
                            "⚠️ Erreur lors de l'exécution d'Aider (`{exec_prov}:{exec_model}`) :\n`{aider_err_clean}`\n\nVérifiez qu'Ollama est démarré (`ollama serve`) et que le modèle `{exec_model}` est téléchargé."
                        ),
                        format!("Aider CLI ({exec_prov} erreur)"),
                    )
                }
            }
        }
    } else if exec_prov != "ollama" && providers::find_provider_key(&settings, &exec_prov).is_some() {
        let augmented_prompt = direct_prompt_opt.unwrap_or_default();
        let direct_sys = "Tu es AiderStudio, un expert en architecture logicielle et génération de code. RÈGLE STRICTE : Réponds TOUJOURS EN FRANÇAIS. Pour modifier ou créer un fichier, écris son chemin suivi d'un bloc de code complet (ex: ### src/App.vue puis ```vue ... ```).";
        let direct_call_res = tokio::select! {
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
                &exec_prov,
                &exec_model,
                &settings,
                direct_sys,
                &augmented_prompt,
                4096,
                0.3,
            ) => res,
        };
        match direct_call_res
        {
            Ok((reply, p_tok, c_tok)) => {
                record_external_api_usage(app, state_lock, &exec_prov, p_tok, c_tok).await;
                exec_tokens_used = p_tok + c_tok;
                exec_note = "Exécution API directe".to_string();
                let _ = app.emit(
                    "aider:tokens",
                    TokenUsageEvent {
                        conversation_id: conversation_id.clone(),
                        subagent: format!("{} ({})", role_display_name(exec_role), exec_prov),
                        prompt_tokens: p_tok,
                        completion_tokens: c_tok,
                        total_tokens: exec_tokens_used,
                        role: exec_role.to_string(),
                        provider: exec_prov.clone(),
                        model: exec_model.clone(),
                        context_limit: providers::resolve_model_context_limit(
                            &exec_prov,
                            &exec_model,
                            &settings,
                        ),
                    },
                );
                let out = if reply.is_empty() {
                    format!("Le modèle `{exec_model}` ({exec_prov}) n'a retourné aucun contenu.")
                } else {
                    reply
                };
                (
                    out,
                    format!(
                        "{} direct ({}:{})",
                        role_display_name(exec_role),
                        exec_prov,
                        exec_model
                    ),
                )
            }
            Err(err) => {
                let err_clean = synthesize_error_log(&err.to_string());
                exec_status = "error".to_string();
                exec_note = err_clean.clone();
                (
                    format!(
                        "⚠️ Erreur de communication avec {} (`{}`) : `{}`",
                        exec_prov, exec_model, err_clean
                    ),
                    format!("{exec_prov} erreur"),
                )
            }
        }
    } else if stack.ollama_running {
        let augmented_prompt = direct_prompt_opt.unwrap_or_default();
        let stream_model = if exec_prov == "ollama" && !exec_model.trim().is_empty() {
            exec_model.as_str()
        } else {
            settings.ollama_model.as_str()
        };
        match execute_ollama_streaming(
            app,
            &conversation_id,
            &settings,
            stream_model,
            exec_role,
            &augmented_prompt,
            image_base64_list,
            &mut cancel_rx,
        )
        .await
        {
            Ok(reply) if !reply.is_empty() => {
                exec_note = "Exécution Ollama stream direct".to_string();
                (reply, format!("Ollama direct (stream) · {stream_model}"))
            }
            Ok(_) => {
                exec_note = "Réponse vide Ollama".to_string();
                (
                    "Ollama n'a retourné aucun contenu.".to_string(),
                    "Ollama direct".to_string(),
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
                let err_clean = synthesize_error_log(&err.to_string());
                exec_status = "error".to_string();
                exec_note = err_clean.clone();
                (
                    format!(
                        "⚠️ Erreur de communication avec Ollama (`{}`) : `{}`",
                        settings.ollama_url, err_clean
                    ),
                    "Ollama erreur".to_string(),
                )
            }
        }
    } else {
        exec_status = "error".to_string();
        exec_note = "Stack locale arrêtée et aucune clé API sélectionnée".to_string();
        let diag = format!(
            "### Stack locale non démarrée\n\n- **Aider CLI** : {}\n- **Ollama** : {}\n- **Modèle cible** : `{}:{}`\n\nLancez Ollama (`ollama serve` puis `ollama pull {}`) et installez `aider-chat`, ou sélectionnez un fournisseur API externe validé dans les paramètres pour exécuter ce prompt sur `{}`.",
            if stack.aider_installed { "Détecté" } else { "Non détecté (`pipx install aider-chat`)" },
            if stack.ollama_installed { "Installé mais arrêté" } else { "Non détecté" },
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

    // Extract thinking and apply local tools directly to create/edit files in local
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
    let (extracted_thinking, cleaned_content) = extract_thinking_and_content(&assistant_text);
    let tool_results =
        apply_tools_and_file_blocks(&project_root_path, &cleaned_content, &user_prompt, aider_ran)
            .await;

    // SUB-AGENT 4/5: Testeur & Vérifieur
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

    if run_tests_after && exec_role != "tester" && will_run_aider && is_agent_mode {
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
                    summary: format!("Rédaction des tests pour `{}`...", changed_paths.join(", ")),
                    status: "running".to_string(),
                    tokens: 0,
                },
            );
            let test_prompt = format!(
                "À la suite de la demande : « {} »\nLes fichiers suivants viennent d'être modifiés : {}.\nÉcris ou complète uniquement les tests unitaires/d'intégration pertinents pour vérifier ces changements sans modifier la logique métier existante. Réponds brièvement en français.",
                truncate_chars(user_prompt.trim(), 300),
                changed_paths.join(", ")
            );
            let test_strategy = choose_aider_strategy(
                &project_root_path,
                &changed_paths,
                true,
                &test_prov,
                &settings,
            );
            if let Some(aider_bin) = stack.aider_bin.as_deref() {
                match execute_aider_cli(
                    app,
                    &conversation_id,
                    aider_bin,
                    &project.root_path,
                    &test_prov,
                    &test_model,
                    "tester",
                    &settings,
                    &test_prompt,
                    &changed_paths,
                    &test_strategy,
                    &mut cancel_rx,
                )
                .await
                {
                    Ok((_, bp, bc)) => {
                        record_external_api_usage(app, state_lock, &test_prov, bp, bc).await;
                        tester_ran = true;
                        tester_tokens_used = bp + bc;
                        tester_summary = format!("Tests générés/mis à jour pour `{}`", changed_paths.join(", "));
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
                        tester_summary = format!("Échec rédaction tests : `{}`", synthesize_error_log(&e.to_string()));
                    }
                }
            }
        }
    }

    let test_res = tokio::select! {
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
                        provider: "ollama".to_string(),
                        model: test_res.command.clone(),
                        summary: format!("{verif_short} · {}", truncate_chars(&test_res.output_summary, 90)),
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
                        provider: "ollama".to_string(),
                        model: test_res.command.clone(),
                        summary: format!("{verif_short} · {}", truncate_chars(&test_res.output_summary.replace('\n', " "), 90)),
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

    let (mut synthesized_text, cleaned_narrative, had_retries_or_errors, synth_tokens) = tokio::select! {
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
            &cleaned_content,
            aider_ran,
            stack.ollama_running,
            &synth_prov,
            &synth_model,
            &settings,
            &modified_files,
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

    let artifact_source = if aider_ran {
        cleaned_narrative.as_str()
    } else {
        cleaned_content.as_str()
    };
    let new_artifacts = extract_artifacts_from_markdown(artifact_source);

    let created_files_list: Vec<String> = tool_results
        .iter()
        .filter(|r| r.tool == "write_file" && r.success)
        .filter_map(|r| r.path.as_ref().map(|p| format!("- ✅ `{}`", p)))
        .collect();
    if !created_files_list.is_empty() {
        synthesized_text = format!(
            "{}\n\n### 📁 Fichiers créés sur votre disque dur\n{}",
            synthesized_text,
            created_files_list.join("\n")
        );
    }

    // Build structured collapsible AI Thinking drawer contents (extracted thinking + Verifier + Raw Aider log)
    let mut thinking_sections: Vec<String> = Vec::new();
    if let Some(t) = extracted_thinking {
        thinking_sections.push(t);
    }

    if !tool_results.is_empty() || !tester_note.is_empty() || !verifier_note.is_empty() {
        let mut notes = Vec::new();
        for r in &tool_results {
            notes.push(format!("• 🛠️ {}", r.message));
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
    if aider_ran && (had_retries_or_errors || cleaned_narrative != cleaned_content.trim()) {
        thinking_sections.push(format!(
            "### Journal brut Aider\n{}",
            tail_chars(cleaned_content.trim(), 3500)
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
            agent_trace,
        });

        for art in new_artifacts {
            guard.conversations[conv_idx].artifacts.push(art);
        }
        guard.conversations[conv_idx].modified_files = modified_files.clone();
        guard.conversations[conv_idx].lines_added = lines_added;
        guard.conversations[conv_idx].lines_removed = lines_removed;
        guard.conversations[conv_idx].updated_at = now.clone();

        // Update FIFO conversation rolling context (keep custom objective + latest 6 bullets)
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
    use crate::models::DiffLine;

    fn make_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aiderstudio_test_{}_{}", name, Uuid::new_v4()));
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
    fn test_choose_aider_strategy_adaptive() {
        let dir = make_temp_dir("strategy");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("vite.config.js"), "export default {}\n").unwrap();
        let big_file = "const x = 1;\n".repeat(280);
        std::fs::write(dir.join("src/Large.ts"), big_file).unwrap();
        let medium_file = "const y = 2;\n".repeat(150);
        std::fs::write(dir.join("src/Medium.ts"), medium_file).unwrap();

        let settings = AppSettings::default();

        // Single small explicit file -> whole + map 0
        let s1 = choose_aider_strategy(&dir, &["vite.config.js".to_string()], true, "ollama", &settings);
        assert_eq!(s1.edit_format, "whole");
        assert_eq!(s1.map_tokens, 0);
        assert_eq!(s1.max_chat_history_tokens, 1024);

        // Large single explicit file -> diff + map 0
        let s2 = choose_aider_strategy(&dir, &["src/Large.ts".to_string()], true, "ollama", &settings);
        assert_eq!(s2.edit_format, "diff");
        assert_eq!(s2.map_tokens, 0);

        // Non-explicit match -> diff + map 192
        let s2b = choose_aider_strategy(&dir, &["src/Large.ts".to_string()], false, "ollama", &settings);
        assert_eq!(s2b.edit_format, "diff");
        assert_eq!(s2b.map_tokens, 192);

        // Cloud provider on 150-line file (> 120 threshold) -> diff + 2048 chat history
        let s_cloud = choose_aider_strategy(&dir, &["src/Medium.ts".to_string()], true, "gemini", &settings);
        assert_eq!(s_cloud.edit_format, "diff");
        assert_eq!(s_cloud.max_chat_history_tokens, 2048);

        // 2 small files -> whole + map 192
        std::fs::write(dir.join("package.json"), "{}\n").unwrap();
        let s3 = choose_aider_strategy(
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
        let s4 = choose_aider_strategy(&dir, &["vite.config.js".to_string()], true, "ollama", &s_diff_override);
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

        // Direct model override supersedes agent routing
        settings.active_target = "ollama:qwen2.5-coder:7b".to_string();
        assert_eq!(
            resolve_agent_slot("designer", &settings),
            ("ollama".to_string(), "qwen2.5-coder:7b".to_string())
        );
    }

    #[test]
    fn test_clean_aider_output_strips_multi_round_diff_noise() {
        let raw = r#"Pour résoudre le problème de vite.config.js qui empêche le site de démarrer, nous devons modifier ce fichier. Voici les modifications nécessaires :

vite.config.js
```javascript
<<<<<<< SEARCH
import { defineConfig } from 'vite'
=======
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
>>>>>>> REPLACE
```

Veuillez ajouter `vite.config.js` au chat pour que je puisse effectuer ces modifications.

1 SEARCH/REPLACE block failed to match!
SearchReplaceNoExactMatch: This SEARCH block failed to exactly match lines in vite.config.js
<<<<<<< SEARCH
import { defineConfig } from 'vite'
=======
import vue from '@vitejs/plugin-vue'
>>>>>>> REPLACE

Did you mean to match some of these actual lines from vite.config.js?
```
// Configuration de base de Vite
export default defineConfig({
  plugins: []
})
```
The SEARCH section must exactly match an existing block of lines.

Je suis désolé pour la confusion. Voici les SEARCH/REPLACE blocks corrects :

vite.config.js
```javascript
<<<<<<< SEARCH
  plugins: []
=======
  plugins: [vue()]
>>>>>>> REPLACE
```

Veuillez appliquer ces modifications et me confirmer lorsque vous aurez fait ces changements."#;

        let modified = vec![ModifiedFileDiff {
            path: "vite.config.js".to_string(),
            added: 1,
            removed: 0,
            modified: 1,
            lines: vec![DiffLine {
                kind: "mod".to_string(),
                old_line: Some(3),
                new_line: Some(3),
                content: "  plugins: [vue()]".to_string(),
            }],
        }];

        let (cleaned, had_retries) = clean_aider_output_for_display(raw, true, &modified);
        assert!(had_retries);
        assert!(!cleaned.contains("<<<<<<< SEARCH"));
        assert!(!cleaned.contains(">>>>>>> REPLACE"));
        assert!(!cleaned.contains("SearchReplaceNoExactMatch"));
        assert!(!cleaned.contains("Did you mean to match"));
        assert!(!cleaned.contains("Veuillez ajouter"));
        assert!(!cleaned.contains("Veuillez appliquer"));
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
}
