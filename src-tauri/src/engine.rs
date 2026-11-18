use crate::git_diff;
use crate::models::{
    AppSettings, AppStateData, ArtifactItem, AttachedFile, ChatMessage, ModifiedFileDiff,
    PromptStageEvent, SendPromptResult, StackStatus, TokenUsageEvent, ToolExecutionResult,
};
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
struct OllamaChatRequest<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<OllamaChatMsg<'a>>,
}

#[derive(Debug, Serialize)]
struct OllamaChatMsg<'a> {
    role: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    images: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaChatResponse {
    message: Option<OllamaChatMsgOut>,
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
    let mut clean_lines = Vec::new();
    for line in raw_err.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Error navigating to")
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
        if trimmed.contains("error TS") || trimmed.contains("error:") || trimmed.contains("FAILED") {
            clean_lines.push(trimmed.to_string());
        }
    }
    if clean_lines.is_empty() {
        truncate_chars(raw_err, 220)
    } else {
        clean_lines.into_iter().take(4).collect::<Vec<_>>().join("\n")
    }
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
        parts.push(format!("[Contexte de la conversation]\n{}", conv_ctx.trim()));
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

fn is_aider_noise_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    let prefixes = [
        "Tokens:",
        "Applied edit to",
        "Added ",
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
    ];
    if prefixes.iter().any(|p| t.starts_with(p)) {
        return true;
    }
    // Standalone backticked filename line like `` `src/App.vue ``
    if t.starts_with('`') && !t.ends_with('`') && !t.contains(' ') {
        return true;
    }
    false
}

fn build_clean_synthesis(raw_output: &str, modified_files: &[ModifiedFileDiff]) -> String {
    let trimmed = raw_output.trim();
    if trimmed.starts_with("⚠️") || trimmed.starts_with("### Stack locale") {
        return trimmed.to_string();
    }

    // Preserve fenced code blocks intact while removing Aider CLI noise outside fences
    // and cleaning stray backticks on headings (e.g. `### \`tsconfig.json` -> `### tsconfig.json`).
    let mut cleaned_lines: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut prev_blank = false;

    for line in trimmed.lines() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") {
            in_fence = !in_fence;
            cleaned_lines.push(line.trim_end().to_string());
            prev_blank = false;
            continue;
        }

        if in_fence {
            cleaned_lines.push(line.to_string());
            prev_blank = false;
            continue;
        }

        if is_aider_noise_line(line) {
            continue;
        }

        let mut normalized = line.trim_end().to_string();
        if let Some(rest) = normalized.strip_prefix("### ") {
            let clean_heading = rest.trim().trim_matches('`').trim();
            normalized = format!("### {clean_heading}");
        }

        if normalized.is_empty() {
            if !prev_blank && !cleaned_lines.is_empty() {
                cleaned_lines.push(String::new());
                prev_blank = true;
            }
        } else {
            cleaned_lines.push(normalized);
            prev_blank = false;
        }
    }

    let narrative = cleaned_lines.join("\n").trim().to_string();

    if modified_files.is_empty() {
        return if narrative.is_empty() {
            trimmed.to_string()
        } else {
            narrative
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
        sections.push(narrative);
    } else {
        sections.push("Modifications appliquées en arrière-plan sur le projet.".to_string());
    }
    if !created.is_empty() {
        sections.push(format!("### Fichiers créés\n{}", created.join("\n")));
    }
    if !updated.is_empty() {
        sections.push(format!("### Fichiers modifiés\n{}", updated.join("\n")));
    }
    if !deleted.is_empty() {
        sections.push(format!("### Fichiers supprimés\n{}", deleted.join("\n")));
    }

    sections.join("\n\n")
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let truncated: String = input.chars().take(max_chars).collect();
    format!("{truncated}…")
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

async fn execute_aider_cli(
    app: &AppHandle,
    conversation_id: &str,
    aider_bin: &str,
    project_root: &str,
    settings: &AppSettings,
    prompt: &str,
) -> anyhow::Result<String> {
    let model_arg = if settings.ollama_model.starts_with("ollama") {
        settings.ollama_model.clone()
    } else {
        format!("ollama_chat/{}", settings.ollama_model.trim())
    };

    let mut child = Command::new(aider_bin)
        .current_dir(project_root)
        .env("OLLAMA_API_BASE", settings.ollama_url.trim_end_matches('/'))
        .args([
            "--model",
            &model_arg,
            "--yes-always",
            "--no-pretty",
            "--no-auto-commits",
            "--no-show-model-warnings",
            "--no-check-update",
            "--edit-format",
            "diff",
            "--map-tokens",
            "512",
            "--chat-language",
            "fr",
            "--message",
            prompt,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let collected = Arc::new(Mutex::new(String::new()));
    let c_out = Arc::clone(&collected);
    let app_out = app.clone();
    let conv_id_out = conversation_id.to_string();

    let out_task = tokio::spawn(async move {
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut guard = c_out.lock().await;
                guard.push_str(&line);
                guard.push('\n');
                if line.trim().starts_with("Tokens:") {
                    let prompt_tokens = parse_aider_token_count(&line, "sent");
                    let completion_tokens = parse_aider_token_count(&line, "received");
                    let total_tokens = prompt_tokens + completion_tokens;
                    if total_tokens > 0 {
                        let _ = app_out.emit(
                            "aider:tokens",
                            TokenUsageEvent {
                                conversation_id: conv_id_out.clone(),
                                subagent: "3. Codeur Atomique (Aider)".to_string(),
                                prompt_tokens,
                                completion_tokens,
                                total_tokens,
                            },
                        );
                    }
                }
                let _ = app_out.emit("aider:stream", (&conv_id_out, &line));
            }
        }
    });

    let c_err = Arc::clone(&collected);
    let err_task = tokio::spawn(async move {
        if let Some(err) = stderr {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    let mut guard = c_err.lock().await;
                    guard.push_str(&line);
                    guard.push('\n');
                }
            }
        }
    });

    let status = tokio::time::timeout(std::time::Duration::from_secs(180), child.wait()).await??;
    let _ = out_task.await;
    let _ = err_task.await;

    let text = collected.lock().await.trim().to_string();
    if !status.success() && text.is_empty() {
        anyhow::bail!("Aider s'est terminé avec le code {:?}", status.code());
    }
    Ok(text)
}

async fn execute_ollama_streaming(
    app: &AppHandle,
    conversation_id: &str,
    settings: &AppSettings,
    prompt: &str,
    image_base64_list: Vec<String>,
) -> anyhow::Result<String> {
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
        model: settings.ollama_model.trim(),
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
    };

    let mut resp = client.post(url).json(&payload).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Ollama HTTP {}", resp.status());
    }

    let mut full_accumulated = String::new();
    let mut in_think = false;
    let mut live_thinking = String::new();
    let mut live_content = String::new();
    let mut buffer = String::new();

    while let Some(chunk_bytes) = resp.chunk().await? {
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
                                subagent: "3. Codeur Atomique (Ollama)".to_string(),
                                prompt_tokens,
                                completion_tokens,
                                total_tokens,
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

pub async fn run_prompt_pipeline(
    app: &AppHandle,
    state_lock: &Arc<Mutex<AppStateData>>,
    conversation_id: String,
    user_prompt: String,
    attachments: Option<Vec<AttachedFile>>,
) -> anyhow::Result<SendPromptResult> {
    // Snapshot inputs and record user message immediately
    let (settings, project, conv_ctx, user_msg_time) = {
        let mut guard = state_lock.lock().await;
        let conv_idx = guard
            .conversations
            .iter()
            .position(|c| c.id == conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Conversation introuvable"))?;

        let now = Utc::now().to_rfc3339();
        if guard.conversations[conv_idx].messages.is_empty() {
            guard.conversations[conv_idx].title = truncate_chars(user_prompt.trim(), 38);
        }
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
        (settings, project, conv_ctx, now)
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

    // SUB-AGENT 1/5: Architecte & Orchestrateur
    emit_stage(
        app,
        &conversation_id,
        1,
        "1. Architecte & Orchestrateur",
        "Analyse de la demande, structuration du plan d'action et protection du contexte 8K.",
        "running",
    );
    let augmented_prompt = build_augmented_prompt(
        &project.global_context,
        &conv_ctx,
        &user_prompt,
        attachments.as_deref(),
        &project.shared_directories,
    );

    // Initial prompt token footprint estimation
    let init_prompt_tokens = (augmented_prompt.len() as f64 / 3.8).ceil() as usize;
    let _ = app.emit(
        "aider:tokens",
        TokenUsageEvent {
            conversation_id: conversation_id.clone(),
            subagent: "1. Architecte & Contexte".to_string(),
            prompt_tokens: init_prompt_tokens,
            completion_tokens: 0,
            total_tokens: init_prompt_tokens,
        },
    );

    // SUB-AGENT 2/5: Cartographe & Analyseur
    emit_stage(
        app,
        &conversation_id,
        2,
        "2. Cartographe & Analyseur",
        "Sélection des fichiers cibles et isolement des interfaces utiles.",
        "running",
    );

    // SUB-AGENT 3/5: Codeur Atomique
    emit_stage(
        app,
        &conversation_id,
        3,
        "3. Codeur Atomique (Aider & Ollama)",
        &format!("Génération et application des modifications ciblées sur {}", settings.ollama_model),
        "running",
    );

    let mut stack = check_stack_status(&settings).await;
    if !stack.ollama_running && settings.auto_start_ollama && stack.ollama_installed {
        if let Ok(updated) = start_ollama_service(&settings).await {
            stack = updated;
        }
    }

    let mut aider_ran = false;
    let (assistant_text, stage_note) = if image_base64_list.is_empty()
        && stack.aider_bin.is_some()
    {
        let aider_bin = stack.aider_bin.as_deref().unwrap();
        match execute_aider_cli(
            app,
            &conversation_id,
            aider_bin,
            &project.root_path,
            &settings,
            &augmented_prompt,
        )
        .await
        {
            Ok(output) if !output.is_empty() => {
                aider_ran = true;
                (
                    output,
                    format!("Aider Diff · {}", settings.ollama_model),
                )
            }
            Ok(_) => {
                aider_ran = true;
                (
                    "Aider a terminé l'opération sans sortie textuelle supplémentaire.".to_string(),
                    format!("Aider Diff · {}", settings.ollama_model),
                )
            }
            Err(err) => (
                format!(
                    "⚠️ Erreur lors de l'exécution d'Aider :\n`{}`\n\nVérifiez qu'Ollama est démarré (`ollama serve`) et que le modèle `{}` est téléchargé.",
                    synthesize_error_log(&err.to_string()), settings.ollama_model
                ),
                "Aider CLI (erreur)".to_string(),
            ),
        }
    } else if stack.ollama_running {
        match execute_ollama_streaming(
            app,
            &conversation_id,
            &settings,
            &augmented_prompt,
            image_base64_list,
        )
        .await
        {
            Ok(reply) if !reply.is_empty() => (
                reply,
                format!("Ollama direct (stream) · {}", settings.ollama_model),
            ),
            Ok(_) => (
                "Ollama n'a retourné aucun contenu.".to_string(),
                "Ollama direct".to_string(),
            ),
            Err(err) => (
                format!(
                    "⚠️ Erreur de communication avec Ollama (`{}`) : `{}`",
                    settings.ollama_url, synthesize_error_log(&err.to_string())
                ),
                "Ollama erreur".to_string(),
            ),
        }
    } else {
        let diag = format!(
            "### Stack locale non démarrée\n\n- **Aider CLI** : {}\n- **Ollama** : {}\n- **Modèle cible** : `{}`\n\nLancez Ollama (`ollama serve` puis `ollama pull {}`) et installez `aider-chat` pour exécuter ce prompt directement sur le dépôt `{}`.",
            if stack.aider_installed { "Détecté" } else { "Non détecté (`pipx install aider-chat`)" },
            if stack.ollama_installed { "Installé mais arrêté" } else { "Non détecté" },
            settings.ollama_model,
            settings.ollama_model,
            project.root_path
        );
        (diag, "Diagnostic stack".to_string())
    };

    // Extract thinking and apply local tools directly to create/edit files in local
    let (extracted_thinking, cleaned_content) = extract_thinking_and_content(&assistant_text);
    let tool_results = apply_tools_and_file_blocks(&project_root_path, &cleaned_content, &user_prompt, aider_ran).await;

    // SUB-AGENT 4/5: Testeur & Vérifieur
    emit_stage(
        app,
        &conversation_id,
        4,
        "4. Testeur & Vérifieur",
        "Validation de la syntaxe, compilation et vérifications automatisées.",
        "running",
    );
    let test_res = run_test_verifier(&project_root_path).await;

    let mut thinking_with_tools = extracted_thinking;
    let mut verifier_note = String::new();
    if test_res.ran {
        if test_res.success {
            verifier_note = format!("• ✅ **Vérifieur ({})** : {}", test_res.command, test_res.output_summary);
        } else {
            verifier_note = format!("• ⚠️ **Vérifieur ({})** : Compilation en erreur :\n```\n{}\n```", test_res.command, test_res.output_summary);
        }
    }

    if !tool_results.is_empty() || !verifier_note.is_empty() {
        let mut notes = Vec::new();
        for r in &tool_results {
            notes.push(format!("• 🛠️ {}", r.message));
        }
        if !verifier_note.is_empty() {
            notes.push(verifier_note);
        }
        let block_str = format!("### Contrôles & Outils exécutés en local\n{}", notes.join("\n"));
        thinking_with_tools = match thinking_with_tools {
            Some(t) => Some(format!("{}\n\n{}", t, block_str)),
            None => Some(block_str),
        };
    }

    // SUB-AGENT 5/5: Synthétiseur & Mémoire Globale
    emit_stage(
        app,
        &conversation_id,
        5,
        "5. Synthétiseur & Mémoire Globale",
        "Calcul du diff Git, extraction des artefacts et enregistrement contextuel.",
        "running",
    );

    git_diff::sanitize_backtick_paths(&project.root_path).await;
    let (modified_files, lines_added, lines_removed) =
        git_diff::collect_project_diffs(&project.root_path).await;
    let new_artifacts = extract_artifacts_from_markdown(&cleaned_content);
    let mut synthesized_text = build_clean_synthesis(&cleaned_content, &modified_files);

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
        });

        for art in new_artifacts {
            guard.conversations[conv_idx].artifacts.push(art);
        }
        guard.conversations[conv_idx].modified_files = modified_files.clone();
        guard.conversations[conv_idx].lines_added = lines_added;
        guard.conversations[conv_idx].lines_removed = lines_removed;
        guard.conversations[conv_idx].updated_at = now.clone();

        // Update conversation rolling context (prompt and brief summary)
        let summary_snippet = truncate_chars(synthesized_text.replace('\n', " ").trim(), 220);
        let conv_entry = format!(
            "• [{}] Demande: {} → Résumé: {}",
            user_msg_time.get(..16).unwrap_or(&user_msg_time),
            truncate_chars(user_prompt.trim(), 100),
            summary_snippet
        );
        let prev_conv_ctx = guard.conversations[conv_idx].conversation_context.clone();
        guard.conversations[conv_idx].conversation_context = if prev_conv_ctx.trim().is_empty() {
            conv_entry
        } else {
            truncate_chars(&format!("{prev_conv_ctx}\n{conv_entry}"), 2000)
        };

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
        4,
        "Prêt",
        "Réponse intégrée, outils appliqués et contexte global synchronisé.",
        "done",
    );

    Ok(result)
}
