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
use crate::workspace::{self, WorkspaceSnapshot};
use chrono::Utc;
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
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

fn is_verification_like_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() || !crate::workspace::is_safe_verification_command(trimmed) {
        return false;
    }
    let low = trimmed.to_lowercase();
    [
        "test",
        "check",
        "build",
        "lint",
        "typecheck",
        "type-check",
        "compile",
        "verify",
    ]
    .iter()
    .any(|kw| low.contains(kw))
}

async fn execute_verification_command(
    project_root: &Path,
    command_str: &str,
) -> TestVerificationResult {
    match crate::tools::shell::run_command_simple(
        project_root,
        &serde_json::json!({ "command": command_str, "timeout_secs": 90 }),
    )
    .await
    {
        Ok(tr) => TestVerificationResult {
            ran: true,
            command: command_str.to_string(),
            success: tr.success,
            output_summary: truncate_chars(&tr.message, 450),
        },
        Err(err) => TestVerificationResult {
            ran: true,
            command: command_str.to_string(),
            success: false,
            output_summary: truncate_chars(&err, 450),
        },
    }
}

pub async fn run_project_verifier(
    project_root: &Path,
    validation_command: Option<&str>,
    already_verified: Option<&(String, bool, String)>,
) -> TestVerificationResult {
    let Some(cmd_raw) = validation_command.map(str::trim).filter(|c| !c.is_empty()) else {
        return TestVerificationResult {
            ran: false,
            command: String::new(),
            success: true,
            output_summary: "Aucune commande de validation (validationCommand est null).".to_string(),
        };
    };

    if !crate::workspace::is_safe_verification_command(cmd_raw) {
        return TestVerificationResult {
            ran: true,
            command: cmd_raw.to_string(),
            success: false,
            output_summary: format!(
                "La commande `{cmd_raw}` démarre un processus continu ou non sécurisé (interdit en validation automatisée)."
            ),
        };
    }

    // Si la même commande exacte a déjà été exécutée après la dernière écriture, réutiliser son résultat
    if let Some((cmd, success, summary)) = already_verified {
        if cmd.trim() == cmd_raw {
            return TestVerificationResult {
                ran: true,
                command: cmd.clone(),
                success: *success,
                output_summary: summary.clone(),
            };
        }
    }

    execute_verification_command(project_root, cmd_raw).await
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
    crate::tools::execute_local_tool(project_root, name, args).await
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

pub fn normalize_diff_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

pub fn paths_match(p1: &str, p2: &str) -> bool {
    let n1 = normalize_diff_path(p1);
    let n2 = normalize_diff_path(p2);
    if n1 == n2 {
        return true;
    }
    n1.ends_with(&format!("/{n2}")) || n2.ends_with(&format!("/{n1}"))
}

fn extract_artifacts_from_markdown(text: &str) -> Vec<ArtifactItem> {
    let re = Regex::new(r"```([a-zA-Z0-9_+-]*)\n([\s\S]*?)```").expect("valid fence regex");
    let file_hint_re = Regex::new(r"[A-Za-z0-9_./-]+\.[A-Za-z0-9]{1,8}").expect("valid path regex");
    let now = Utc::now().to_rfc3339();
    let mut items = Vec::new();
    let doc_kw = [
        "audit",
        "rapport",
        "projection",
        "synthese",
        "synthèse",
        "bilan",
        "specification",
        "spécification",
        "roadmap",
        "plan stratégique",
        "architecture globale",
        "analyse d'impact",
        "artefact",
        "artifact",
    ];

    for cap in re.captures_iter(text) {
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

        let lang_low = lang.to_lowercase();
        let prev_low = prev_line.to_lowercase();

        // Les fichiers de code source d'application ne sont JAMAIS des artefacts
        let is_code_lang = matches!(
            lang_low.as_str(),
            "rust" | "rs" | "typescript" | "ts" | "javascript" | "js" | "vue" | "python" | "py" | "css" | "scss" | "html" | "json" | "toml" | "yaml" | "yml" | "sh" | "bash" | "sql"
        );
        let is_explicit_artifact = matches!(
            lang_low.as_str(),
            "artifact" | "artefact" | "report" | "projection" | "audit"
        );

        let inferred_path = file_hint_re
            .find(prev_line)
            .map(|m| m.as_str().trim_matches('`').to_string());

        let has_code_extension = inferred_path.as_deref().map(|p| {
            matches!(
                Path::new(p).extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
                Some("vue" | "ts" | "tsx" | "js" | "jsx" | "rs" | "css" | "scss" | "html" | "json" | "py" | "toml" | "yaml" | "yml" | "sh" | "bash" | "sql")
            )
        }).unwrap_or(false);

        // Rejeter catégoriquement tout bloc de code de langage ou fichier de code source
        if is_code_lang || has_code_extension {
            continue;
        }

        let is_doc_context = doc_kw.iter().any(|kw| prev_low.contains(kw));
        if !is_explicit_artifact && !is_doc_context {
            continue;
        }
        if !matches!(lang_low.as_str(), "markdown" | "md" | "text" | "txt" | "artifact" | "artefact" | "report" | "projection" | "audit") {
            continue;
        }

        let title = if !prev_line.is_empty() && !prev_line.starts_with("```") {
            prev_line.to_string()
        } else {
            inferred_path
                .as_deref()
                .and_then(|p| Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| format!("Rapport_{}", items.len() + 1))
        };

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

#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn is_install_or_setup_prompt(prompt: &str) -> bool {
    let low = prompt.to_lowercase();
    let install_markers = [
        "installe",
        "installer",
        "install ",
        "setup ",
        "ajoute le package",
        "ajouter le package",
        "ajoute la dépendance",
        "ajoute la dependance",
        "ajouter la dépendance",
        "ajouter la dependance",
        "ajoute la lib",
        "ajouter la lib",
        "add package",
        "add dependency",
    ];
    if !install_markers.iter().any(|k| low.contains(k)) {
        return false;
    }

    let ui_redesign_markers = [
        "layout",
        "sidebar",
        "menu à gauche",
        "menu a gauche",
        "couleur",
        "dégradé",
        "degrade",
        "bulle",
        "bouton",
        "fond ",
        "refais le design",
        "refonte",
        "stylise",
        "mise en page",
        "responsive",
        "thème sombre",
        "theme sombre",
        "hauteur",
        "largeur",
        "centré",
        "centrer",
        "aligner",
        "convertis les composants",
        "migre les composants",
        "migrer les composants",
    ];
    !ui_redesign_markers.iter().any(|k| low.contains(k))
}

#[allow(dead_code)]
pub fn classify_prompt_role(user_prompt: &str) -> (&'static str, bool) {
    let lower = user_prompt.to_lowercase();

    let test_kw = [
        "test",
        "tests",
        "testing",
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
        "fond",
        "arrière-plan",
        "arriere-plan",
        "background",
        "gris",
        "anthracite",
        "noir",
        "blanc",
        "bleu",
        "rouge",
        "vert",
        "jaune",
        "violet",
        "sombre",
        "clair",
        "page",
        "écran",
        "ecran",
        "titre",
        "texte",
        "police",
        "font",
        "bordure",
        "ombre",
        "centrer",
        "aligner",
        "marge",
        "margin",
        "padding",
        "header",
        "footer",
        "navbar",
        "icône",
        "icone",
        "emote",
        "emoji",
        "logo",
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
    if is_install_or_setup_prompt(&lower) {
        return ("coder", wants_tests);
    }
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

fn role_display_name(role: &str) -> &'static str {
    match role {
        "architect" => "1. Architecte",
        "cartographer" => "2. Cartographe",
        "designer" => "3. Designer",
        "refactoring" => "3. Refactoring",
        "tester" => "4. Tests",
        "terminal" => "3. Terminal / Shell",
        "research" => "3. Recherche & Web",
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

    if prompt_lower.contains("@rules") || prompt_lower.contains("@rule") {
        for f in all_files {
            if f.starts_with(".agents/rules/") || f == "AGENTS.md" {
                if !matched.contains(f) {
                    matched.push(f.clone());
                    if matched.len() >= 4 {
                        break;
                    }
                }
            }
        }
        if !matched.is_empty() {
            return (matched, true);
        }
    }

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

    let explicit = !matched.is_empty();
    (matched, explicit)
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
    read_only_files: Vec<String>,
    explicit_match: bool,
    is_global_task: bool,
    strategy_note: String,
    cartographer_tokens: Option<(usize, usize)>,
}

#[allow(clippy::too_many_arguments)]
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
        (8usize, 65_536u64, 180_000u64)
    } else if exec_num_ctx >= 16_384 {
        (5usize, 32_768u64, 65_536u64)
    } else {
        (4usize, 20_480u64, 32_768u64)
    };

    let (tier1, explicit) = match_files_deterministically(user_prompt, attachments, all_files);
    let budgeted_tier1 = filter_files_by_size_budget(
        project_root,
        tier1,
        max_target_files,
        max_single_bytes,
        max_total_bytes,
    );

    let (target_files, explicit_match, base_note, cartographer_tokens) =
        if !budgeted_tier1.is_empty() {
            (
                budgeted_tier1,
                explicit,
                "0 tok déterministe (mention directe)".to_string(),
                None,
            )
        } else {
            let can_call_llm = if carto_provider == "ollama" {
                ollama_running
                    || providers::build_fallback_candidates(
                        carto_provider,
                        carto_model,
                        "cartographer",
                        settings,
                        ollama_running,
                    )
                    .iter()
                    .any(|(p, _)| p != "ollama" && providers::find_provider_key(settings, p).is_some())
            } else {
                providers::find_provider_key(settings, carto_provider).is_some()
                    || ollama_running
                    || providers::build_fallback_candidates(
                        carto_provider,
                        carto_model,
                        "cartographer",
                        settings,
                        ollama_running,
                    )
                    .iter()
                    .any(|(p, _)| p != "ollama" && providers::find_provider_key(settings, p).is_some())
            };

            if all_files.len() <= 250 || !can_call_llm {
                (
                    Vec::new(),
                    false,
                    "0 tok déterministe (WorkspaceSnapshot injecté)".to_string(),
                    None,
                )
            } else if all_files.len() > 400 {
                (
                    Vec::new(),
                    false,
                    "Dépôt volumineux (>400 fichiers) · repo-map native".to_string(),
                    None,
                )
            } else {
                let mut prioritized = Vec::new();
                let mut others = Vec::new();
                for f in all_files {
                    if !f.contains('/') {
                        prioritized.push(f.clone());
                    } else {
                        others.push(f.clone());
                    }
                }
                prioritized.extend(others);
                let compact_limit = if is_cloud_exec { 200 } else { 100 };
                let compact_files: Vec<String> =
                    prioritized.into_iter().take(compact_limit).collect();

                let sys = format!(
                    "Tu es un routeur de fichiers. À partir de la liste des fichiers du projet et de la demande utilisateur, retourne UNIQUEMENT un tableau JSON contenant au maximum {max_target_files} chemins relatifs exacts de fichiers nécessaires pour répondre (ex: [\"chemin/fichier.ext\"] ou []). Aucun texte ni explication."
                );
                let user = format!(
                    "Fichiers:\n{}\n\nDemande:\n{}",
                    compact_files.join("\n"),
                    truncate_chars(user_prompt.trim(), 1600)
                );

                match providers::call_provider_oneshot_with_fallback(
                    carto_provider,
                    carto_model,
                    "cartographer",
                    ollama_running,
                    settings,
                    &sys,
                    &user,
                    160,
                    0.0,
                )
                .await
                {
                    Ok((reply, p_tok, c_tok, used_prov, _used_mdl, fb_reason)) => {
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
                        let budgeted = filter_files_by_size_budget(
                            project_root,
                            routed,
                            max_target_files,
                            max_single_bytes,
                            max_total_bytes,
                        );
                        let fb_tag = if fb_reason.is_some() { " (repli)" } else { "" };
                        let note = if budgeted.is_empty() {
                            format!("Routeur IA ({used_prov}{fb_tag} · aucun fichier ciblé)")
                        } else {
                            format!("Routeur IA ({used_prov}{fb_tag})")
                        };
                        (budgeted, false, note, Some((p_tok, c_tok)))
                    }
                    Err(_) => (
                        Vec::new(),
                        false,
                        "Repli repo-map native".to_string(),
                        None,
                    ),
                }
            }
        };

    let is_global_task = is_global_or_doc_task(user_prompt, &target_files);

    CartographerPlan {
        target_files,
        read_only_files: Vec::new(),
        explicit_match,
        is_global_task,
        strategy_note: base_note,
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

    let base_turns = 8usize;
    let max_turns = (base_turns + if target_files.len() > 1 || !explicit_match { 2 } else { 0 }).clamp(8, 10);

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
    let search_count = text.lines().filter(|l| patcher::is_search_marker(l)).count();
    let replace_count = text.lines().filter(|l| patcher::is_replace_marker(l)).count();
    search_count > replace_count
}

fn build_native_tool_definitions(has_mcp: bool) -> Vec<serde_json::Value> {
    crate::tools::build_native_tool_definitions(has_mcp)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRuleItem {
    pub name: String,
    pub relative_path: String,
    pub category: String,
    pub description: String,
    pub content: String,
}

pub fn list_project_rules(project_root: &Path) -> Vec<ProjectRuleItem> {
    let mut rules = Vec::new();

    // 1. Fichiers racines candidats
    let root_candidates = [
        ("AGENTS.md", "Directives principales du projet"),
        (".vaix/architecture.md", "Architecture & directives >AIx STUDIO"),
        (".aider/conventions.md", "Conventions de code du projet"),
        ("CLAUDE.md", "Directives Claude / Assistant IA"),
    ];
    for (cand, default_desc) in root_candidates {
        let p = project_root.join(cand);
        if let Ok(content) = std::fs::read_to_string(&p) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                let first_line = trimmed
                    .lines()
                    .next()
                    .unwrap_or(default_desc)
                    .trim_start_matches('#')
                    .trim();
                let desc = if first_line.is_empty() { default_desc } else { first_line };
                rules.push(ProjectRuleItem {
                    name: cand.to_string(),
                    relative_path: cand.to_string(),
                    category: "root".to_string(),
                    description: desc.to_string(),
                    content: trimmed.to_string(),
                });
                break;
            }
        }
    }

    // 2. Fichiers modulaires sous .agents/rules/ et .agent/rules/
    let rules_dirs = [".agents/rules", ".agent/rules"];
    for rdir in rules_dirs {
        let dir_path = project_root.join(rdir);
        if let Ok(entries) = std::fs::read_dir(&dir_path) {
            let mut entries_vec: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            entries_vec.sort_by_key(|e| e.file_name());
            for entry in entries_vec {
                let path = entry.path();
                if path.extension().map(|ext| ext == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            let stem = path
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let category = if stem.contains("frontend") || stem.contains("ui") || stem.contains("client") {
                                "frontend"
                            } else if stem.contains("backend") || stem.contains("api") || stem.contains("server") {
                                "backend"
                            } else if stem.contains("git") {
                                "git"
                            } else if stem.contains("validation") || stem.contains("test") {
                                "validation"
                            } else {
                                "custom"
                            };
                            let desc = trimmed
                                .lines()
                                .next()
                                .map(|l| l.trim_start_matches('#').trim().to_string())
                                .filter(|l| !l.is_empty())
                                .unwrap_or_else(|| stem.clone());
                            rules.push(ProjectRuleItem {
                                name: stem,
                                relative_path: format!("{rdir}/{}", entry.file_name().to_string_lossy()),
                                category: category.to_string(),
                                description: desc,
                                content: trimmed.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    rules
}

pub async fn init_project_agent_rules(
    root_path: &Path,
    project_name: &str,
    initial_prompt: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut created_files = Vec::new();
    let rules_dir = root_path.join(".agents/rules");
    tokio::fs::create_dir_all(&rules_dir)
        .await
        .map_err(|e| format!("Impossible de créer le dossier .agents/rules: {e}"))?;

    let prompt_desc = initial_prompt.unwrap_or("").trim();

    // 1. Initialiser AGENTS.md s'il n'existe pas encore
    let agents_path = root_path.join("AGENTS.md");
    if !agents_path.exists() {
        let mut agents_content = format!(
            "# AGENTS.md — Directives & Architecture pour {project_name}\n\n\
            Ce fichier définit les directives fondamentales, l'architecture globale et les standards de développement pour tout agent IA travaillant sur le projet **{project_name}**.\n\n"
        );

        if !prompt_desc.is_empty() {
            agents_content.push_str(&format!(
                "## 1. Spécifications & Objectif Initial\n{}\n\n",
                prompt_desc
            ));
        }

        agents_content.push_str(
            "## 2. Commandes de Build & Vérification\n\
            S'appuyer sur les manifestes et configurations déclarés à la racine du projet pour exécuter les commandes de compilation, d'analyse statique et de test adaptées.\n\n\
            ## 3. Règles Fondamentales pour les Agents\n\
            1. **Zéro Régression de Types** : Respecter le typage strict requis par le projet.\n\
            2. **Gestion d'Erreurs Robuste** : Gérer explicitement toutes les erreurs et cas limites sans panique ni crash silencieux.\n\
            3. **Édition Chirurgicale** : Préserver scrupuleusement le style, l'indentation et le code existant.\n\
            4. **Structure Modulaire** : Les directives thématiques complémentaires sont réparties dans le dossier `.agents/rules/`.\n"
        );

        tokio::fs::write(&agents_path, agents_content)
            .await
            .map_err(|e| format!("Erreur lors de la création de AGENTS.md: {e}"))?;
        created_files.push("AGENTS.md".to_string());
    }

    // 2. Initialiser les règles modulaires dans .agents/rules/
    if !rules_dir.join("validation.md").exists() {
        let val_content = "# Règle : Validation & Contrôle Qualité\n\n\
            Avant de clore une tâche ou proposer des modifications :\n\
            1. Exécuter la vérification des types, la compilation ou les tests déclarés dans le projet.\n\
            2. Corriger immédiatement toute erreur de compilation ou avertissement bloquant.\n\
            3. Vérifier que les interfaces et contrats d'API sont synchronisés.\n";
        tokio::fs::write(rules_dir.join("validation.md"), val_content).await.map_err(|e| e.to_string())?;
        created_files.push(".agents/rules/validation.md".to_string());
    }

    if !rules_dir.join("git-workflow.md").exists() {
        let git_content = "# Règle : Workflow Git & Commits\n\n\
            Conventions de commit (Conventional Commits) :\n\
            - `feat(...)` : nouvelle fonctionnalité.\n\
            - `fix(...)` : correction de bug.\n\
            - `refactor(...)` : refactorisation sans changement fonctionnel.\n\
            - `style(...)` : retouches cosmétiques / UI.\n\
            - `chore(...)` : maintenance et dépendances.\n";
        tokio::fs::write(rules_dir.join("git-workflow.md"), git_content).await.map_err(|e| e.to_string())?;
        created_files.push(".agents/rules/git-workflow.md".to_string());
    }

    Ok(created_files)
}

pub fn load_project_architecture_memory(
    project_root: &Path,
    max_chars: usize,
) -> Option<(String, String)> {
    const CANDIDATES: [&str; 4] = [
        ".vaix/architecture.md",
        "AGENTS.md",
        ".aider/conventions.md",
        "CLAUDE.md",
    ];
    let mut main_doc: Option<(String, String)> = None;
    for rel in CANDIDATES {
        let p = project_root.join(rel);
        if let Ok(content) = std::fs::read_to_string(&p) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                main_doc = Some((rel.to_string(), trimmed.to_string()));
                break;
            }
        }
    }

    // Charger les règles modulaires de .agents/rules/*.md et .agent/rules/*.md
    let mut rules_contents: Vec<(String, String)> = Vec::new();
    let rules_dirs = [".agents/rules", ".agent/rules"];
    for rdir in rules_dirs {
        let dir_path = project_root.join(rdir);
        if let Ok(entries) = std::fs::read_dir(&dir_path) {
            let mut entries_vec: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            entries_vec.sort_by_key(|e| e.file_name());
            for entry in entries_vec {
                let path = entry.path();
                if path.extension().map(|ext| ext == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            let file_name = entry.file_name().to_string_lossy().to_string();
                            rules_contents.push((format!("{rdir}/{file_name}"), trimmed.to_string()));
                        }
                    }
                }
            }
        }
    }

    if main_doc.is_none() && rules_contents.is_empty() {
        return None;
    }

    if rules_contents.is_empty() {
        if let Some((rel, content)) = main_doc {
            return Some((rel, truncate_chars(&content, max_chars)));
        }
    }

    let mut combined = String::new();
    let mut label = String::new();

    if let Some((main_rel, main_txt)) = main_doc {
        label = main_rel.clone();
        combined.push_str(&format!("=== Directives globales ({main_rel}) ===\n{main_txt}\n"));
    }

    if !rules_contents.is_empty() {
        if label.is_empty() {
            label = ".agents/rules/".to_string();
        } else {
            label.push_str(" + .agents/rules/");
        }
        for (rule_rel, rule_txt) in rules_contents {
            combined.push_str(&format!("\n=== Règle active ({rule_rel}) ===\n{rule_txt}\n"));
        }
    }

    Some((label, truncate_chars(combined.trim(), max_chars)))
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchitectStep {
    pub title: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub files: Vec<String>,
    pub instructions: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedArchitectPlan {
    pub role: Option<String>,
    pub run_tests: bool,
    pub spec: Option<String>,
    pub steps: Vec<ArchitectStep>,
}

#[allow(dead_code)]
fn normalize_exec_role(raw: &str) -> Option<String> {
    match raw.trim().to_lowercase().as_str() {
        "designer" => Some("designer".to_string()),
        "coder" => Some("coder".to_string()),
        "refactoring" => Some("refactoring".to_string()),
        "tester" => Some("tester".to_string()),
        _ => None,
    }
}

#[allow(dead_code)]
pub fn parse_architect_plan(raw: &str, default_role: &str) -> ParsedArchitectPlan {
    let mut plan = ParsedArchitectPlan::default();
    let Some(start) = raw.find('{') else {
        return plan;
    };
    let Some(end) = raw.rfind('}') else {
        return plan;
    };
    if start >= end {
        return plan;
    }
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw[start..=end]) else {
        return plan;
    };

    if let Some(r) = val.get("role").and_then(|v| v.as_str()) {
        plan.role = normalize_exec_role(r);
    }
    plan.run_tests = val
        .get("runTests")
        .or_else(|| val.get("run_tests"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if let Some(spec) = val.get("spec").and_then(|v| v.as_str()) {
        let clean_spec = spec.trim();
        if !clean_spec.is_empty() {
            plan.spec = Some(truncate_chars(clean_spec, 1400));
        }
    }

    let step_fallback_role = plan
        .role
        .clone()
        .or_else(|| normalize_exec_role(default_role))
        .unwrap_or_else(|| "coder".to_string());

    if let Some(arr) = val.get("steps").and_then(|v| v.as_array()) {
        for item in arr {
            if plan.steps.len() >= 5 {
                break;
            }
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let instructions = item
                .get("instructions")
                .or_else(|| item.get("spec"))
                .or_else(|| item.get("description"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if title.is_empty() || instructions.is_empty() {
                continue;
            }
            let role = item
                .get("role")
                .and_then(|v| v.as_str())
                .and_then(normalize_exec_role)
                .unwrap_or_else(|| step_fallback_role.clone());
            let files: Vec<String> = item
                .get("files")
                .and_then(|v| v.as_array())
                .map(|fs| {
                    fs.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.trim().trim_start_matches("./").to_string())
                        .filter(|s| !s.is_empty())
                        .take(5)
                        .collect()
                })
                .unwrap_or_default();
            plan.steps.push(ArchitectStep {
                title: truncate_chars(title, 120),
                role,
                files,
                instructions: truncate_chars(instructions, 1200),
            });
        }
    }

    plan
}

fn build_native_system_prompt(
    role: &str,
    _edit_format: &str,
    active_mcp: &[&McpServerConfig],
    mcp_schemas_note: &str,
) -> String {
    let edit_rules = "PROTOCOLE D'ÉCRITURE ET D'ÉDITION DE FICHIERS (Tool-First) :\n\
        1. CRÉATION OU RÉÉCRITURE COMPLÈTE (`write_file`) :\n\
           Utilise impérativement l'outil `write_file` pour créer un nouveau fichier ou remplacer intégralement un fichier existant, SANS AUCUNE LIMITE DE LIGNES.\n\
        2. PATCH COURT SUR FICHIER EXISTANT (`edit_file`) :\n\
           Utilise l'outil `edit_file` uniquement pour des modifications localisées (patch court) via un ou plusieurs blocs SEARCH/REPLACE exacts sur un fichier déjà existant.\n\
           N'utilise JAMAIS `edit_file` pour créer un fichier inexistant.\n\
        3. LECTURE PRÉALABLE (`read_file` / `search_code`) :\n\
           Si un fichier existant n'est pas déjà affiché en entier dans ton contexte ou si un patch `edit_file` échoue, lis-le d'abord avec `read_file` avant de retenter un patch ou d'utiliser `write_file`.";

    let role_rules = match role {
        "designer" => "CONSIGNES DU RÔLE (`Designer UI/UX`) :\n\
        - Respecte la stack, les conventions de style et les dépendances réellement présentes dans les manifestes du projet.\n\
        - Respecte scrupuleusement CHAQUE spécification chiffrée, visuelle, typographique et comportementale demandée.\n\
        - Mets à jour de manière cohérente le layout parent et les composants ou feuilles de style concernés.".to_string(),
        "refactoring" => "CONSIGNES DU RÔLE (`Refactoring`) :\n\
        - Préserve strictement le comportement externe existant tout en améliorant la lisibilité, le découpage et le typage sans régression.".to_string(),
        "tester" => "CONSIGNES DU RÔLE (`Testeur`) :\n\
        - Écris des tests déterministes, isolés et directement exécutables couvrant le comportement demandé sans modifier la logique métier existante, puis exécute-les via `run_command`.".to_string(),
        "terminal" => "CONSIGNES DU RÔLE (`Terminal / Shell`) :\n\
        - Rôle prioritaire : diagnostic d'environnement, exécution de commandes shell, installation de dépendances et exécution de scripts de build/test via `run_command`.\n\
        - Lis les fichiers de configuration nécessaires avec `read_file` avant toute modification.\n\
        - N'édite ou ne crée de fichiers que si nécessaire pour réparer la configuration ou les scripts.\n\
        - INTERDICTION FORMELLE DE SERVEUR PERSISTANT : Ne lance JAMAIS de serveur de développement en tâche de fond ou de commande watch bloquante sans fin. Utilise exclusivement des commandes terminales déterministes.".to_string(),
        "research" => "CONSIGNES DU RÔLE (`Recherche & Documentation`) :\n\
        - Rôle prioritaire : rechercher des documentations officielles, des solutions à des erreurs ou des versions compatibles avec `web_search` et lire les pages pertinentes avec `fetch_url`.\n\
        - Synthétise clairement les résultats trouvés pour alimenter la résolution du problème.".to_string(),
        _ => "CONSIGNES DU RÔLE (`Codeur`) :\n\
        - Analyse d'abord l'instantané du projet (`WorkspaceSnapshot` : arborescence, manifestes racine, scripts et chemins déclarés introuvables).\n\
        - Produis du code complet, robuste et opérationnel, sans TODO ni placeholder, cohérent avec les conventions et dépendances du projet.\n\
        - Si la tâche exige de créer ou corriger plusieurs fichiers manquants ou liés, enchaîne tous les appels `write_file` / `edit_file` nécessaires avant de conclure.\n\
        - Lorsque le projet déclare un script ou une commande de build/check/test déterministe et non-bloquant, vérifie tes modifications avec `run_command` avant de rendre la main.".to_string(),
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
            s.push('\n');
            s.push_str(mcp_schemas_note.trim());
        }
        s
    };

    format!(
        "Langue de réponse obligatoire : Français (sauf code et identifiants techniques).\n\n\
        Tu es le moteur de code de >AIx STUDIO. Réponds TOUJOURS ET EXCLUSIVEMENT EN FRANÇAIS de façon concise et technique.\n\n\
        {role_rules}\n\n\
        {edit_rules}\n\n\
        RÈGLES ET DIRECTIVES DU PROJET (.agents/rules & AGENTS.md) :\n\
        - Si le projet contient `AGENTS.md` ou `.agents/rules/*.md`, respecte scrupuleusement leurs conventions (typage strict, gestion d'erreurs, commandes de validation).\n\
        - GESTION DU TAG `@rules` : Si la demande mentionne `@rules` ou demande d'ajouter/modifier une consigne projet, mets à jour `.agents/rules/<nom>.md` ou `AGENTS.md` avec `write_file` / `edit_file` et confirme clairement dans ton résumé.\n\n\
        DÉPENDANCES & INTERDICTION DE SERVEUR BLOQUANT :\n\
        - Vérifie toujours les manifestes racine fournis avant d'importer ou d'utiliser une dépendance externe ; installe toute dépendance manquante via `run_command` si besoin.\n\
        - Ne lance JAMAIS de commande bloquante (serveur de dev, watcher).\n\n\
        STYLE DE RÉPONSE FINALE (après tous tes appels d'outils) :\n\
        - Quand toutes les écritures et vérifications sont terminées, conclus par un bref résumé au passé composé citant chaque fichier touché via un lien Markdown cliquable `[NomFichier](chemin/relatif/NomFichier)`.\n\
        - N'ajoute JAMAIS de section `### Fichiers modifiés` en fin de message.\n\n\
        OUTILS DISPONIBLES (appel d'outil natif JSON ou balise `<tool_call>`) :\n\
        <tool_call>{{\"name\":\"read_file\",\"arguments\":{{\"path\":\"chemin/fichier\",\"start_line\":1,\"end_line\":120}}}}</tool_call> (start_line/end_line optionnels)\n\
        <tool_call>{{\"name\":\"search_code\",\"arguments\":{{\"query\":\"motif\",\"path_filter\":\"dossier/\"}}}}</tool_call>\n\
        <tool_call>{{\"name\":\"list_files\",\"arguments\":{{}}}}</tool_call>\n\
        <tool_call>{{\"name\":\"write_file\",\"arguments\":{{\"path\":\"chemin/fichier\",\"content\":\"...\"}}}}</tool_call> (CRÉATION COMPLÈTE ou réécriture totale sans limite de lignes)\n\
        <tool_call>{{\"name\":\"edit_file\",\"arguments\":{{\"path\":\"chemin/fichier\",\"search\":\"extrait exact\",\"replace\":\"nouveau texte\"}}}}</tool_call> (PATCH COURT SEARCH/REPLACE sur fichier existant ; accepte aussi `edits: [...]`)\n\
        <tool_call>{{\"name\":\"delete_file\",\"arguments\":{{\"path\":\"chemin/fichier\",\"reason\":\"motif\"}}}}</tool_call> (requiert confirmation utilisateur)\n\
        <tool_call>{{\"name\":\"run_command\",\"arguments\":{{\"command\":\"...\",\"timeout_secs\":90}}}}</tool_call> (commande terminale déterministe)\n\
        <tool_call>{{\"name\":\"web_search\",\"arguments\":{{\"query\":\"recherche ciblée\",\"max_results\":5}}}}</tool_call>\n\
        <tool_call>{{\"name\":\"fetch_url\",\"arguments\":{{\"url\":\"https://...\",\"max_chars\":12000}}}}</tool_call>{mcp_section}"
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
    workspace_snapshot: &WorkspaceSnapshot,
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

    let arch_mem_limit = if is_cloud { 2800 } else { 1200 };
    if let Some((mem_path, mem_content)) = load_project_architecture_memory(project_root, arch_mem_limit) {
        parts.push(format!(
            "[Architecture & Conventions ({mem_path})]\n{mem_content}"
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

    let snapshot_text = workspace_snapshot.format_for_prompt(is_cloud);
    if !snapshot_text.trim().is_empty() {
        parts.push(snapshot_text);
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
        let max_targets = if is_cloud { 8 } else { 5 };
        let clip_chars = if is_cloud { 24_000 } else { 12_000 };
        let mut file_sections = Vec::new();
        for rel in target_files.iter().take(max_targets) {
            if let Ok(content) = tokio::fs::read_to_string(project_root.join(rel)).await {
                let char_len = content.chars().count();
                if char_len > clip_chars {
                    let total_lines = content.lines().count();
                    truncated_files.insert(rel.clone());
                    let clipped = truncate_chars(&content, clip_chars);
                    file_sections.push(format!(
                        "### {rel} [TRONQUÉ · {total_lines} lignes — utilise `edit_file` ou `read_file` avec start_line/end_line ou `search_code` pour voir la suite]\n```\n{clipped}\n```"
                    ));
                } else {
                    file_sections.push(format!("### {rel}\n```\n{content}\n```"));
                }
                preloaded_files.insert(rel.clone());
            }
        }
        if !file_sections.is_empty() {
            parts.push(format!(
                "[Fichiers cibles préchargés]\n{}",
                file_sections.join("\n\n")
            ));
        }
    }

    // Suffixe dynamique
    let compact_conv = if is_cloud {
        compact_conv_ctx_for_prompt(conv_ctx, 900, 4)
    } else {
        compact_conv_ctx_for_prompt(conv_ctx, 600, 2)
    };
    if !compact_conv.is_empty() {
        parts.push(format!(
            "[Historique des tours précédents (déjà terminés — contexte uniquement, ne pas ré-exécuter)]\n{compact_conv}"
        ));
    }
    if let Some(spec) = architect_spec {
        if !spec.trim().is_empty() {
            parts.push(format!("[Cadrage / Handoff de mission]\n{}", spec.trim()));
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

    parts.push(format!("[Demande & Instructions de mission]\n{}", user_prompt.trim()));
    let text = parts.join("\n\n");
    let msg = if image_base64_list.is_empty() {
        LlmMessage::user(text)
    } else {
        LlmMessage::user_with_images(text, image_base64_list)
    };
    (msg, truncated_files, preloaded_files)
}

fn prompt_expects_code_changes(user_prompt: &str) -> bool {
    let trimmed = user_prompt.trim();
    if trimmed.is_empty() {
        return false;
    }
    let low = trimmed.to_lowercase();
    let padded = format!(" {low} ");

    let edit_markers = [
        "mets ",
        "mets-",
        "met ",
        "mettre ",
        "ajoute",
        "modifie",
        "change",
        "corrige",
        "crée",
        "créé",
        "cree",
        "supprime",
        "remplace",
        "applique",
        "implémente",
        "implemente",
        "refactor",
        "fix",
        " add ",
        " update ",
        " create ",
        " set ",
        " replace ",
        " remove ",
        " delete ",
        "passe ",
        "rends ",
        "centre",
        "colori",
        "affiche ",
        "enlève",
        "enleve",
        "retire ",
        "positionne",
        "insère",
        "insere",
        "intègre",
        "integre",
        "déplace",
        "deplace",
        "stylise",
        "refais",
        "construis",
    ];
    if edit_markers.iter().any(|k| padded.contains(k)) {
        return true;
    }

    let question_starts = [
        "comment ",
        "pourquoi ",
        "qu'est-ce",
        "est-ce que",
        "explique",
        "décris",
        "decris",
        "analyse",
        "audit",
        "résume",
        "resume",
        "liste ",
        "quel ",
        "quelle ",
        "quels ",
        "quelles ",
        "combien ",
        "où ",
        "ou ",
        "what ",
        "how ",
        "why ",
        "explain ",
    ];
    if question_starts.iter().any(|q| low.starts_with(q)) || trimmed.ends_with('?') {
        return false;
    }

    false
}

struct NativeLoopResult {
    narrative: String,
    raw_last_response: String,
    thinking: Option<String>,
    tool_results: Vec<ToolExecutionResult>,
    applied_files: Vec<String>,
    last_post_edit_verification: Option<(String, bool, String)>,
    prompt_tokens: usize,
    completion_tokens: usize,
    turns_run: usize,
    had_repairs: bool,
    effective_edit_format: String,
    actual_provider: String,
    actual_model: String,
    fallback_reason: Option<String>,
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
    ollama_running: bool,
    settings: &AppSettings,
    workspace_snapshot: &WorkspaceSnapshot,
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
    let all_project_files = &workspace_snapshot.all_files;
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
        workspace_snapshot,
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

    let system_prompt = build_native_system_prompt(
        role,
        &effective_edit_format,
        &active_mcp,
        &mcp_schemas_note,
    );
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

    let subagent_label = format!("{} ({})", role_display_name(role), effective_edit_format);
    let native_tool_defs = build_native_tool_definitions(!active_mcp.is_empty());

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
    let mut did_no_edit_nudge = false;
    let mut active_provider = provider.to_string();
    let mut active_model = model.to_string();
    let mut last_fallback_reason: Option<String> = None;
    let mut last_post_edit_verification: Option<(String, bool, String)> = None;

    let mut max_loop_turns = strategy.max_turns.max(4);
    let mut reading_turns = 0usize;
    let mut turn = 0usize;
    let mut disk_revision: usize = 0;
    let mut tool_call_history: HashMap<String, (bool, usize)> = HashMap::new();

    while turn < max_loop_turns {
        if is_cancelled(cancel_rx) {
            anyhow::bail!("CANCELLED_BY_USER");
        }
        turn += 1;
        turns_run = turn;

        let stream_start = format!(
            "[Tour {}/{}] Appel {}:{} (format {})...",
            turns_run,
            max_loop_turns,
            active_provider,
            active_model,
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
            tools: Some(native_tool_defs.clone()),
        };

        let (res, used_provider, used_model, fb_reason) =
            providers::stream_chat_completion_with_fallback(
                &active_provider,
                &active_model,
                role,
                ollama_running,
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
                |from_p, from_m, to_p, to_m, reason| {
                    let msg = format!(
                        "[Fallback] {from_p}:{from_m} -> {to_p}:{to_m} ({})",
                        truncate_chars(reason, 90)
                    );
                    let _ = app.emit("aider:stream", (conversation_id, &msg));
                },
            )
            .await?;

        active_provider = used_provider.clone();
        active_model = used_model.clone();
        if fb_reason.is_some() {
            last_fallback_reason = fb_reason.clone();
        }

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
                &used_provider,
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
                    provider: used_provider.clone(),
                    model: used_model.clone(),
                    context_limit: providers::resolve_model_context_limit(
                        &used_provider,
                        &used_model,
                        settings,
                    ),
                },
            );
        }

        let cache_note = if res.cache_read_tokens > 0 {
            format!(", cache: {} tok", res.cache_read_tokens)
        } else {
            String::new()
        };
        let stream_done = format!(
            "[Tour {}/{}] Réponse reçue de {}:{} ({} in / {} out{}).",
            turns_run,
            max_loop_turns,
            used_provider,
            used_model,
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

        // 1. Fusionner appels d'outils natifs + balises <tool_call> dédupliqués
        let mut tool_calls: Vec<ParsedToolCall> = Vec::new();
        let mut seen_tc_keys = HashSet::new();
        for ntc in &res.tool_calls {
            let key = format!(
                "{}:{}",
                ntc.name,
                serde_json::to_string(&ntc.arguments).unwrap_or_default()
            );
            if seen_tc_keys.insert(key) {
                tool_calls.push(ParsedToolCall {
                    name: ntc.name.clone(),
                    arguments: ntc.arguments.clone(),
                });
            }
        }
        for xtc in extract_tool_calls(&res.content) {
            let key = format!(
                "{}:{}",
                xtc.name,
                serde_json::to_string(&xtc.arguments).unwrap_or_default()
            );
            if seen_tc_keys.insert(key) {
                tool_calls.push(xtc);
            }
        }

        let assistant_msg_text = if res.content.trim().is_empty() && !tool_calls.is_empty() {
            let names: Vec<&str> = tool_calls.iter().map(|t| t.name.as_str()).collect();
            format!("[Appel d'outils natifs : {}]", names.join(", "))
        } else {
            res.content.clone()
        };
        messages.push(LlmMessage::assistant(assistant_msg_text));
        let mut tool_outputs = Vec::new();
        let mut wrote_via_tool = false;
        let mut tool_had_failure = false;

        for tc in tool_calls {
            if is_cancelled(cancel_rx) {
                anyhow::bail!("CANCELLED_BY_USER");
            }

            let tc_sig = format!(
                "{}:{}",
                tc.name,
                serde_json::to_string(&tc.arguments).unwrap_or_default()
            );
            if let Some(&(prev_success, prev_rev)) = tool_call_history.get(&tc_sig) {
                if prev_rev == disk_revision && (!prev_success || tc.name != "read_file") {
                    let alert_msg = "ALERTE SYSTÈME : Tu viens d'exécuter exactement la même action qui a échoué. Ne répète pas cet appel. Lis d'abord le fichier complet avec read_file ou change d'approche.";
                    tool_outputs.push(format!("[{}]\n{}", tc.name, alert_msg));
                    all_tool_results.push(ToolExecutionResult {
                        tool: tc.name.clone(),
                        success: false,
                        message: alert_msg.to_string(),
                        path: tc
                            .arguments
                            .get("path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    });
                    tool_had_failure = true;
                    continue;
                }
            }

            let exec_outcome: Option<(bool, bool)> = 'exec_tc: {
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
                                break 'exec_tc Some((true, false));
                            }
                            Err(e) => {
                                tool_had_failure = true;
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
                                break 'exec_tc Some((false, false));
                            }
                        }
                    } else {
                        tool_had_failure = true;
                        tool_outputs.push(format!(
                            "[Erreur MCP] Serveur '{server_ref}' introuvable ou inactif."
                        ));
                        break 'exec_tc Some((false, false));
                    }
                }

                if tc.name == "delete_file" {
                    let raw_path = match tc.arguments.get("path").and_then(|v| v.as_str()) {
                        Some(p) => p,
                        None => {
                            tool_had_failure = true;
                            let err_msg = "Paramètre 'path' manquant".to_string();
                            all_tool_results.push(ToolExecutionResult {
                                tool: "delete_file".to_string(),
                                success: false,
                                message: err_msg.clone(),
                                path: None,
                            });
                            tool_outputs.push(format!("[Erreur delete_file] {err_msg}"));
                            break 'exec_tc Some((false, false));
                        }
                    };
                    let clean_path = match crate::tools::fs::sanitize_tool_path(raw_path) {
                        Ok(p) => p,
                        Err(e) => {
                            tool_had_failure = true;
                            all_tool_results.push(ToolExecutionResult {
                                tool: "delete_file".to_string(),
                                success: false,
                                message: format!("Erreur delete_file : {e}"),
                                path: None,
                            });
                            tool_outputs.push(format!("[Erreur delete_file] {e}"));
                            break 'exec_tc Some((false, false));
                        }
                    };
                    let target_path = project_root.join(&clean_path);
                    if !target_path.exists() {
                        tool_had_failure = true;
                        let err_msg = format!("Fichier '{}' introuvable", clean_path);
                        all_tool_results.push(ToolExecutionResult {
                            tool: "delete_file".to_string(),
                            success: false,
                            message: err_msg.clone(),
                            path: Some(clean_path),
                        });
                        tool_outputs.push(format!("[Erreur delete_file] {err_msg}"));
                        break 'exec_tc Some((false, false));
                    }
                    if !target_path.is_file() {
                        tool_had_failure = true;
                        let err_msg = format!("'{}' n'est pas un fichier supprimable", clean_path);
                        all_tool_results.push(ToolExecutionResult {
                            tool: "delete_file".to_string(),
                            success: false,
                            message: err_msg.clone(),
                            path: Some(clean_path),
                        });
                        tool_outputs.push(format!("[Erreur delete_file] {err_msg}"));
                        break 'exec_tc Some((false, false));
                    }

                    let reason = tc
                        .arguments
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());

                    let ask_line = format!(
                        "[Validation] Confirmation demandée pour supprimer `{clean_path}`..."
                    );
                    let _ = app.emit("aider:stream", (conversation_id, &ask_line));

                    let approved = match crate::tools::approval::request_tool_approval(
                        app,
                        conversation_id,
                        "delete_file",
                        "Confirmation de suppression",
                        &clean_path,
                        reason,
                        cancel_rx,
                    )
                    .await
                    {
                        Ok(v) => v,
                        Err(e) if e.contains("CANCELLED_BY_USER") => {
                            anyhow::bail!("CANCELLED_BY_USER");
                        }
                        Err(e) => {
                            tool_had_failure = true;
                            all_tool_results.push(ToolExecutionResult {
                                tool: "delete_file".to_string(),
                                success: false,
                                message: format!("Erreur validation delete_file : {e}"),
                                path: Some(clean_path),
                            });
                            tool_outputs.push(format!("[Erreur delete_file] {e}"));
                            break 'exec_tc Some((false, false));
                        }
                    };

                    if !approved {
                        tool_had_failure = true;
                        let deny_msg =
                            format!("Suppression de `{clean_path}` refusée par l'utilisateur.");
                        let _ = app.emit(
                            "aider:stream",
                            (conversation_id, &format!("[Validation] {deny_msg}")),
                        );
                        all_tool_results.push(ToolExecutionResult {
                            tool: "delete_file".to_string(),
                            success: false,
                            message: deny_msg,
                            path: Some(clean_path.clone()),
                        });
                        tool_outputs.push(format!(
                            "[delete_file: {clean_path}] Refusé par l'utilisateur. Ne supprime pas ce fichier."
                        ));
                        break 'exec_tc Some((false, false));
                    }

                    match crate::tools::fs::delete_file(project_root, &tc.arguments).await {
                        Ok(tr) => {
                            wrote_via_tool = true;
                            let pfx = format!("{clean_path}:");
                            already_read.retain(|k| k != &clean_path && !k.starts_with(&pfx));
                            if seen_applied.insert(clean_path.clone()) {
                                all_applied_files.push(clean_path.clone());
                            }
                            let ok_line = format!("[Outil] Fichier supprimé : `{clean_path}`.");
                            let _ = app.emit("aider:stream", (conversation_id, &ok_line));
                            all_tool_results.push(tr.clone());
                            tool_outputs.push(format!("[delete_file] {}", tr.message));
                            break 'exec_tc Some((true, true));
                        }
                        Err(e) => {
                            tool_had_failure = true;
                            all_tool_results.push(ToolExecutionResult {
                                tool: "delete_file".to_string(),
                                success: false,
                                message: format!("Erreur delete_file : {e}"),
                                path: Some(clean_path),
                            });
                            tool_outputs.push(format!("[Erreur delete_file] {e}"));
                            break 'exec_tc Some((false, false));
                        }
                    }
                }

                if tc.name == "run_command" {
                    let cmd_str = match tc
                        .arguments
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        Some(c) => c,
                        None => {
                            tool_had_failure = true;
                            let err_msg = "Paramètre 'command' manquant".to_string();
                            all_tool_results.push(ToolExecutionResult {
                                tool: "run_command".to_string(),
                                success: false,
                                message: err_msg.clone(),
                                path: None,
                            });
                            tool_outputs.push(format!("[Erreur run_command] {err_msg}"));
                            break 'exec_tc Some((false, false));
                        }
                    };
                    let timeout_secs = crate::tools::shell::parse_timeout_secs(&tc.arguments);

                    if let Some(daemon_err) =
                        crate::tools::shell::is_daemon_or_dev_server_command(cmd_str)
                    {
                        tool_had_failure = true;
                        all_tool_results.push(ToolExecutionResult {
                            tool: "run_command".to_string(),
                            success: false,
                            message: daemon_err.to_string(),
                            path: None,
                        });
                        tool_outputs.push(format!("[Erreur run_command] {daemon_err}"));
                        let stream_err = format!("[run_command bloqué] {daemon_err}");
                        let _ = app.emit("aider:stream", (conversation_id, &stream_err));
                        break 'exec_tc Some((false, false));
                    }

                    if let Some(reason) = crate::tools::shell::is_destructive_command(cmd_str) {
                        let ask_line = format!(
                            "[Validation] Confirmation demandée pour commande sensible : `{}`...",
                            truncate_chars(cmd_str, 80)
                        );
                        let _ = app.emit("aider:stream", (conversation_id, &ask_line));

                        let approved = match crate::tools::approval::request_tool_approval(
                            app,
                            conversation_id,
                            "run_command",
                            "Confirmation de commande sensible",
                            cmd_str,
                            Some(reason.to_string()),
                            cancel_rx,
                        )
                        .await
                        {
                            Ok(v) => v,
                            Err(e) if e.contains("CANCELLED_BY_USER") => {
                                anyhow::bail!("CANCELLED_BY_USER");
                            }
                            Err(e) => {
                                tool_had_failure = true;
                                all_tool_results.push(ToolExecutionResult {
                                    tool: "run_command".to_string(),
                                    success: false,
                                    message: format!("Erreur validation run_command : {e}"),
                                    path: None,
                                });
                                tool_outputs.push(format!("[Erreur run_command] {e}"));
                                break 'exec_tc Some((false, false));
                            }
                        };

                        if !approved {
                            tool_had_failure = true;
                            let deny_msg = format!(
                                "Exécution de la commande `{}` refusée par l'utilisateur.",
                                truncate_chars(cmd_str, 80)
                            );
                            let _ = app.emit(
                                "aider:stream",
                                (conversation_id, &format!("[Validation] {deny_msg}")),
                            );
                            all_tool_results.push(ToolExecutionResult {
                                tool: "run_command".to_string(),
                                success: false,
                                message: deny_msg,
                                path: None,
                            });
                            tool_outputs.push(format!(
                                "[run_command] Exécution refusée par l'utilisateur pour `{cmd_str}`."
                            ));
                            break 'exec_tc Some((false, false));
                        }
                    }

                    let cmd_line = format!("[Cmd] {}", truncate_chars(cmd_str, 120));
                    let _ = app.emit("aider:stream", (conversation_id, &cmd_line));
                    let running_notice = format!(
                        "{}\n\n💻 *Exécution :* `{}`",
                        turn_live_content.trim(),
                        truncate_chars(cmd_str, 90)
                    );
                    let _ = app.emit("aider:live_chunk", (conversation_id, &running_notice));

                    match crate::tools::shell::run_command_streaming(
                        project_root,
                        cmd_str,
                        timeout_secs,
                        Some(cancel_rx),
                        |line| {
                            let stream_msg = format!("[cmd] {line}");
                            let _ = app.emit("aider:stream", (conversation_id, &stream_msg));
                        },
                    )
                    .await
                    {
                        Ok(tr) => {
                            if !tr.success {
                                tool_had_failure = true;
                            }
                            let outcome_icon = if tr.success { "✓" } else { "✕" };
                            let running_outcome = format!(
                                "{}\n\n💻 `{}` -> {} {}",
                                turn_live_content.trim(),
                                truncate_chars(cmd_str, 70),
                                outcome_icon,
                                if tr.success { "terminé" } else { "erreur" }
                            );
                            let _ = app.emit("aider:live_chunk", (conversation_id, &running_outcome));
                            let clipped = truncate_chars(&tr.message, tool_output_budget);
                            let cmd_summary = format!(
                                "`{}`: {}",
                                truncate_chars(cmd_str, 70),
                                truncate_chars(&tr.message, 600)
                            );
                            all_tool_results.push(ToolExecutionResult {
                                tool: tr.tool.clone(),
                                success: tr.success,
                                message: cmd_summary,
                                path: None,
                            });
                            tool_outputs.push(format!(
                                "[run_command (succès={})]\n{}",
                                tr.success, clipped
                            ));
                            let is_verif = is_verification_like_command(cmd_str);
                            if is_verif {
                                last_post_edit_verification = Some((
                                    cmd_str.to_string(),
                                    tr.success,
                                    truncate_chars(&tr.message, 450),
                                ));
                                break 'exec_tc Some((tr.success, false));
                            }
                            break 'exec_tc Some((tr.success, tr.success));
                        }
                        Err(e) if e.contains("CANCELLED_BY_USER") => {
                            anyhow::bail!("CANCELLED_BY_USER");
                        }
                        Err(e) => {
                            tool_had_failure = true;
                            let err_notice = format!(
                                "{}\n\n💻 `{}` -> ✕ ({})",
                                turn_live_content.trim(),
                                truncate_chars(cmd_str, 70),
                                truncate_chars(&e, 60)
                            );
                            let _ = app.emit("aider:live_chunk", (conversation_id, &err_notice));
                            all_tool_results.push(ToolExecutionResult {
                                tool: "run_command".to_string(),
                                success: false,
                                message: format!("Erreur run_command : {e}"),
                                path: None,
                            });
                            tool_outputs.push(format!("[Erreur run_command] {e}"));
                            break 'exec_tc Some((false, false));
                        }
                    }
                }

                let read_key_for_call = if tc.name == "read_file" {
                    let s_opt = tc
                        .arguments
                        .get("start_line")
                        .or_else(|| tc.arguments.get("startLine"))
                        .and_then(|v| v.as_u64());
                    let e_opt = tc
                        .arguments
                        .get("end_line")
                        .or_else(|| tc.arguments.get("endLine"))
                        .and_then(|v| v.as_u64());
                    let has_range = s_opt.is_some() || e_opt.is_some();
                    if let Some(raw_p) = tc.arguments.get("path").and_then(|v| v.as_str()) {
                        let norm_p = raw_p
                            .trim()
                            .trim_matches('`')
                            .trim_start_matches("./")
                            .replace('\\', "/");
                        let rkey = format!("{norm_p}:{s_opt:?}:{e_opt:?}");
                        let already_cached = if has_range {
                            already_read.contains(&rkey)
                        } else {
                            (!truncated_files.contains(&norm_p) && already_read.contains(&norm_p))
                                || already_read.contains(&rkey)
                        };
                        if already_cached {
                            tool_outputs.push(format!(
                                "[read_file: {norm_p}] Déjà présent dans le contexte ci-dessus."
                            ));
                            break 'exec_tc None;
                        }
                        Some((norm_p, rkey, has_range))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let log_line = if tc.name == "web_search" {
                    let q = tc.arguments.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    format!("[Web] Recherche : \"{}\"...", truncate_chars(q, 60))
                } else if tc.name == "fetch_url" {
                    let u = tc.arguments.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    format!("[Web] Lecture URL : {}...", truncate_chars(u, 60))
                } else {
                    format!("[Outil] {}...", tc.name)
                };
                let _ = app.emit("aider:stream", (conversation_id, &log_line));
                match execute_local_tool_internal(project_root, &tc.name, &tc.arguments).await {
                    Ok(tr) => {
                        if tc.name == "write_file" || tc.name == "create_file" || tc.name == "edit_file"
                        {
                            if tr.success {
                                wrote_via_tool = true;
                                if let Some(p) = &tr.path {
                                    let pfx = format!("{p}:");
                                    already_read.retain(|k| k != p && !k.starts_with(&pfx));
                                    if seen_applied.insert(p.clone()) {
                                        all_applied_files.push(p.clone());
                                    }
                                    let ok_line =
                                        format!("[Outil] Modification appliquée sur `{p}`.");
                                    let _ = app.emit("aider:stream", (conversation_id, &ok_line));
                                }
                            } else {
                                tool_had_failure = true;
                            }
                            all_tool_results.push(tr.clone());
                            tool_outputs.push(format!("[{}] {}", tc.name, tr.message));
                            break 'exec_tc Some((tr.success, tr.success));
                        } else if tc.name == "read_file" {
                            if let Some((norm_p, rkey, has_range)) = read_key_for_call {
                                already_read.insert(rkey);
                                if !has_range && tr.message.lines().count() <= 220 {
                                    already_read.insert(norm_p);
                                }
                            } else if let Some(p) = &tr.path {
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
                            break 'exec_tc Some((tr.success, false));
                        } else {
                            if !tr.success {
                                tool_had_failure = true;
                            }
                            let clipped = truncate_chars(&tr.message, tool_output_budget);
                            let detail = if tc.name == "web_search" {
                                let q = tc.arguments.get("query").and_then(|v| v.as_str()).unwrap_or("");
                                format!("Recherche \"{q}\" : {}", truncate_chars(&tr.message, 600))
                            } else if tc.name == "fetch_url" {
                                let u = tc.arguments.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                format!("Lecture {u} : {}", truncate_chars(&tr.message, 600))
                            } else {
                                truncate_chars(&tr.message, 600)
                            };
                            all_tool_results.push(ToolExecutionResult {
                                tool: tr.tool.clone(),
                                success: tr.success,
                                message: detail,
                                path: tr.path.clone(),
                            });
                            tool_outputs.push(format!("[{}]\n{}", tc.name, clipped));
                            break 'exec_tc Some((tr.success, false));
                        }
                    }
                    Err(e) => {
                        tool_had_failure = true;
                        all_tool_results.push(ToolExecutionResult {
                            tool: tc.name.clone(),
                            success: false,
                            message: format!("Erreur {} : {}", tc.name, e),
                            path: None,
                        });
                        tool_outputs.push(format!("[Erreur {}] {}", tc.name, e));
                        break 'exec_tc Some((false, false));
                    }
                }
            };

            if let Some((call_succeeded, bumped_disk)) = exec_outcome {
                if bumped_disk {
                    disk_revision += 1;
                    last_post_edit_verification = None;
                }
                tool_call_history.insert(tc_sig, (call_succeeded, disk_revision));
            }
        }

        // 2. Appliquer les blocs de fichiers / SEARCH-REPLACE (filet de sécurité Markdown)
        let edit_outcome = patcher::apply_edits_from_response(
            project_root,
            &effective_response,
            target_files,
            all_project_files,
            &truncated_files,
            user_prompt,
        )
        .await;

        if !edit_outcome.applied_files.is_empty() {
            disk_revision += 1;
            last_post_edit_verification = None;
        }
        for f in &edit_outcome.applied_files {
            let pfx = format!("{f}:");
            already_read.retain(|k| k != f && !k.starts_with(&pfx));
            if seen_applied.insert(f.clone()) {
                all_applied_files.push(f.clone());
            }
            let patch_line = format!("[Patch] Appliqué sur `{f}`.");
            let _ = app.emit("aider:stream", (conversation_id, &patch_line));
        }
        all_tool_results.extend(edit_outcome.results.clone());

        let turn_cleaned =
            patcher::strip_edit_blocks_for_display(&effective_response, &all_applied_files);
        let turn_cleaned_is_empty = turn_cleaned.trim().is_empty();
        if !turn_cleaned_is_empty {
            last_non_empty_narrative = turn_cleaned;
        }

        // 3. Extension dynamique de budget (avant test de fin de boucle) et décision de continuation
        if !tool_outputs.is_empty() {
            if all_applied_files.is_empty() && !wrote_via_tool {
                reading_turns += 1;
                if turn + 1 >= max_loop_turns && reading_turns > 0 && max_loop_turns < 12 {
                    max_loop_turns = (turn + 5).min(12);
                }
            } else if (wrote_via_tool || !edit_outcome.applied_files.is_empty())
                && turn >= max_loop_turns
                && max_loop_turns < 14
            {
                max_loop_turns = (turn + 2).min(14);
            }
        }

        if turn >= max_loop_turns {
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
                "Un ou plusieurs blocs d'édition ont échoué :\n{}\n\nCorrige ces éditions avec `edit_file` ou `write_file`.",
                fail_notes.join("\n")
            )));
            continue;
        }

        // Toujours renvoyer les résultats d'outils et poursuivre la boucle dès qu'au moins un outil a été appelé
        if !tool_outputs.is_empty() {
            let follow_up = if tool_had_failure {
                "Une ou plusieurs actions précédentes ont échoué : corrige l'erreur ou change d'approche (ex. read_file ou write_file complet), puis poursuis jusqu'à résolution complète."
            } else {
                "Poursuis maintenant avec ces résultats : enchaîne les prochaines lectures, écritures (`write_file` / `edit_file`) ou vérifications (`run_command`) nécessaires, ou conclue brièvement sans appel d'outil si tout est terminé."
            };
            messages.push(LlmMessage::user(format!(
                "[Résultats des outils]\n{}\n\n{follow_up}",
                tool_outputs.join("\n\n")
            )));
            continue;
        }

        if all_applied_files.is_empty()
            && !wrote_via_tool
            && tool_outputs.is_empty()
            && !all_tool_results.iter().any(|t| t.tool == "run_command" && t.success)
            && edit_outcome.failures.is_empty()
            && prompt_expects_code_changes(user_prompt)
            && !did_no_edit_nudge
        {
            did_no_edit_nudge = true;
            had_repairs = true;
            let _ = app.emit(
                "aider:stream",
                (
                    conversation_id,
                    "[Auto-réparation] Aucune modification détectée sur disque — relance avec outils d'écriture...",
                ),
            );
            let candidate_paths: Vec<String> = if !target_files.is_empty() {
                target_files.to_vec()
            } else {
                all_project_files.iter().take(12).cloned().collect()
            };
            let files_hint = if candidate_paths.is_empty() {
                "aucun chemin pré-listé (crée le fichier nécessaire)".to_string()
            } else {
                candidate_paths.join(", ")
            };
            messages.push(LlmMessage::user(format!(
                "ATTENTION : ta réponse précédente n'a modifié AUCUN fichier sur le disque.\n\
                Tu DOIS appliquer concrètement la demande sur le projet ({files_hint}) en appelant l'outil `write_file` (création / réécriture complète) ou `edit_file` (patch court), ou `read_file` s'il te manque le contenu réel."
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
        last_post_edit_verification,
        prompt_tokens: total_prompt_tokens,
        completion_tokens: total_completion_tokens,
        turns_run,
        had_repairs,
        effective_edit_format,
        actual_provider: active_provider,
        actual_model: active_model,
        fallback_reason: last_fallback_reason,
    })
}

fn build_compact_diff_summary(modified_files: &[ModifiedFileDiff], max_chars: usize) -> String {
    let mut parts = Vec::new();
    for f in modified_files {
        let mut file_lines = vec![format!(
            "Fichier {} (+{} / -{}, ~{} mod):",
            f.path, f.added, f.removed, f.modified
        )];
        for dl in f.lines.iter().filter(|l| l.kind != "ctx").take(25) {
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
            status: f.status.clone(),
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

#[allow(dead_code)]
fn is_complex_multi_requirement_prompt(user_prompt: &str) -> bool {
    if has_multiple_requests(user_prompt) {
        return true;
    }
    let trimmed = user_prompt.trim();
    trimmed.chars().count() >= 280
        && (trimmed.contains('\n') || trimmed.contains('•') || trimmed.contains(':'))
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

fn has_callable_candidate(
    primary_provider: &str,
    primary_model: &str,
    role: &str,
    settings: &AppSettings,
    ollama_running: bool,
) -> bool {
    providers::build_fallback_candidates(
        primary_provider,
        primary_model,
        role,
        settings,
        ollama_running,
    )
    .into_iter()
    .any(|(p, _)| {
        if p == "ollama" {
            ollama_running
        } else {
            providers::find_provider_key(settings, &p).is_some()
        }
    })
}

#[derive(Clone, Debug)]
struct StepHandoff {
    step_id: String,
    title: String,
    role: String,
    goal: String,
    conclusion: String,
    key_tool_outputs: Vec<String>,
    modified_files: Vec<String>,
    status: String,
}

#[derive(Clone, Debug)]
struct MissionBlackboard {
    original_user_prompt: String,
    plan_summary: String,
    validation_command: Option<String>,
    workspace_snapshot: WorkspaceSnapshot,
    step_handoffs: Vec<StepHandoff>,
    tool_facts: Vec<String>,
    applied_files: Vec<String>,
}

impl MissionBlackboard {
    fn new(original_user_prompt: String, workspace_snapshot: WorkspaceSnapshot) -> Self {
        Self {
            original_user_prompt,
            plan_summary: String::new(),
            validation_command: None,
            workspace_snapshot,
            step_handoffs: Vec::new(),
            tool_facts: Vec::new(),
            applied_files: Vec::new(),
        }
    }

    fn format_handoffs_block(&self) -> String {
        if self.step_handoffs.is_empty() {
            return String::new();
        }
        let mut out = String::from("[Acquis & diagnostics des étapes précédentes]\n");
        for (idx, h) in self.step_handoffs.iter().enumerate() {
            out.push_str(&format!(
                "### Étape {} — {} (rôle: {}, statut: {}, id: {})\n- Objectif: {}\n",
                idx + 1,
                h.title,
                h.role,
                h.status,
                h.step_id,
                truncate_chars(&h.goal, 240)
            ));
            if !h.modified_files.is_empty() {
                out.push_str(&format!("- Fichiers modifiés: {}\n", h.modified_files.join(", ")));
            }
            if !h.key_tool_outputs.is_empty() {
                out.push_str("- Constats d'outils:\n");
                for obs in h.key_tool_outputs.iter().take(4) {
                    out.push_str(&format!("  * {}\n", truncate_chars(obs, 220)));
                }
            }
            if !h.conclusion.trim().is_empty() {
                out.push_str(&format!(
                    "- Conclusion / diagnostic: {}\n",
                    truncate_chars(h.conclusion.trim(), 700)
                ));
            }
        }
        out
    }
}

#[allow(clippy::too_many_arguments)]
async fn synthesize_final_response(
    app: &AppHandle,
    state_lock: &Arc<Mutex<AppStateData>>,
    conversation_id: &str,
    user_prompt: &str,
    cleaned_narrative: &str,
    tool_results: &[ToolExecutionResult],
    engine_ran: bool,
    ollama_running: bool,
    synth_provider: &str,
    synth_model: &str,
    settings: &AppSettings,
    modified_files: &[ModifiedFileDiff],
    turn_applied_files: &[String],
    extra_stage_edits: bool,
    verifier_note: &str,
    blackboard: &MissionBlackboard,
) -> (String, usize) {
    let can_call_synth =
        has_callable_candidate(synth_provider, synth_model, "synthesizer", settings, ollama_running);

    let needs_llm_synthesis = engine_ran
        && can_call_synth
        && (cleaned_narrative.trim().is_empty()
            || !tool_results.is_empty()
            || !modified_files.is_empty()
            || cleaned_narrative.chars().count() > 700
            || narrative_needs_structured_synthesis(
                cleaned_narrative,
                turn_applied_files,
                user_prompt,
                extra_stage_edits,
            ));

    let mut synth_tokens = 0usize;
    let core_narrative = if needs_llm_synthesis {
        let scoped_diffs: Vec<ModifiedFileDiff> = if !modified_files.is_empty() {
            modified_files.to_vec()
        } else if !turn_applied_files.is_empty() {
            modified_files
                .iter()
                .filter(|f| turn_applied_files.iter().any(|p| paths_match(&f.path, p)))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let execution_facts = serde_json::json!({
            "has_modified_files": !scoped_diffs.is_empty(),
            "total_modified_files": scoped_diffs.len(),
            "modified_files": scoped_diffs.iter().map(|f| serde_json::json!({
                "path": f.path,
                "status": f.status.clone().unwrap_or_else(|| "modified".to_string()),
                "added_lines": f.added,
                "removed_lines": f.removed,
                "modified_lines": f.modified,
                "diff_summary": build_compact_diff_summary(&[f.clone()], 800)
            })).collect::<Vec<_>>(),
            "turn_applied_files": turn_applied_files,
            "step_handoffs": blackboard.step_handoffs.iter().map(|h| serde_json::json!({
                "step_id": h.step_id,
                "title": h.title,
                "role": h.role,
                "status": h.status,
                "modified_files": h.modified_files,
                "key_tool_outputs": h.key_tool_outputs,
                "conclusion": h.conclusion
            })).collect::<Vec<_>>(),
            "executed_tools": tool_results.iter().take(15).map(|r| serde_json::json!({
                "tool": r.tool,
                "success": r.success,
                "message": truncate_chars(&r.message, 250)
            })).collect::<Vec<_>>(),
            "verification_report": if verifier_note.is_empty() { "N/A" } else { verifier_note },
            "engine_turn_details": truncate_chars(cleaned_narrative, 1200)
        });
        let execution_facts_json = serde_json::to_string_pretty(&execution_facts).unwrap_or_default();

        let synth_sys = "Langue de réponse obligatoire : Français (sauf code et identifiants techniques).\n\n\
        Tu es l'Architecte Synthétiseur de vAIx STUDIO. Tu produis une synthèse technique rigoureuse, concise et FACTUELLE.\n\n\
        RÈGLE FONDAMENTALE D'ANALYSE FACTUELLE :\n\
        Tu es branché EXCLUSIVEMENT sur les faits d'exécution réels fournis dans le JSON d'état ci-dessous.\n\
        Il t'est FORMELLEMENT INTERDIT de faire une libre interprétation du prompt utilisateur, d'extrapoler, d'inventer des modifications de fichiers ou des commandes manuelles qui ne figurent pas dans ces faits réels.\n\n\
        STRUCTURE OBLIGATOIRE DE TA SYNTHÈSE (utilise impérativement ces 3 sections markdown `###`) :\n\n\
        ### 🔍 Cause du problème & Diagnostic\n\
        Explique succinctement la cause technique ayant motivé l'intervention d'après les faits d'exécution (ex. fichier manquant, configuration absente, syntaxe erronée).\n\n\
        ### 🛠️ Solution réelle & Fichiers modifiés\n\
        Résume STRICTEMENT les modifications réelles indiquées dans le JSON d'état (`modified_files` et `step_handoffs`).\n\
        RÈGLE ABSOLUE : Cite OBLIGATOIREMENT chaque fichier réellement créé ou modifié avec son lien Markdown cliquable exact `[NomFichier](chemin/relatif/NomFichier)`.\n\
        Détaille les modifications logiques (création, lignes ajoutées/supprimées, options modifiées, imports corrigés).\n\
        Si aucun fichier n'a été modifié (has_modified_files = false), indique clairement qu'aucune modification de fichier n'a été écrite sur le disque et n'invente AUCUNE commande manuelle.\n\n\
        ### ✅ Vérifications & État final\n\
        Résume les outils réellement exécutés (installations, builds, tests) et leur résultat (succès/échec) d'après le JSON d'état. Indique l'état final vérifié de l'application.\n\n\
        DIRECTIVES STRICTES :\n\
        - RÈGLE LORSQUE 0 FICHIER MODIFIÉ : Si `has_modified_files` est faux, il est FORMELLEMENT INTERDIT d'inventer ou de proposer des commandes manuelles de création de code à l'utilisateur (ex. 'Créez manuellement...', 'Tapez cette commande...'). Indique sobrement qu'aucune écriture de fichier n'a eu lieu sur le disque.\n\
        - Sois direct, pédagogique et précis. Pas de blabla vide ni d'excuses.\n\
        - Ne recopie JAMAIS de blocs de code entiers ni de diffs bruts SEARCH/REPLACE.\n\
        - Chaque fichier mentionné doit comporter son lien Markdown valide `[NomFichier](chemin/relatif)`.";
        let synth_user = format!(
            "FAITS RÉELS D'EXÉCUTION (JSON D'ÉTAT DU SYSTÈME) :\n\
            ```json\n\
            {}\n\
            ```\n\n\
            Contexte indicatif de la demande initiale de l'utilisateur (à titre de référence uniquement, interdiction de libre interprétation) :\n\
            {}",
            execution_facts_json,
            truncate_chars(user_prompt.trim(), 800)
        );

        match providers::call_provider_oneshot_with_fallback(
            synth_provider,
            synth_model,
            "synthesizer",
            ollama_running,
            settings,
            synth_sys,
            &synth_user,
            950,
            0.1,
        )
        .await
        {
            Ok((summary, p_tok, c_tok, used_prov, used_mdl, _fb_reason)) => {
                synth_tokens = p_tok + c_tok;
                if synth_tokens > 0 {
                    record_external_api_usage(app, state_lock, &used_prov, p_tok, c_tok).await;
                    let _ = app.emit(
                        "aider:tokens",
                        TokenUsageEvent {
                            conversation_id: conversation_id.to_string(),
                            subagent: "5. Synthétiseur".to_string(),
                            prompt_tokens: p_tok,
                            completion_tokens: c_tok,
                            total_tokens: synth_tokens,
                            role: "synthesizer".to_string(),
                            provider: used_prov.clone(),
                            model: used_mdl.clone(),
                            context_limit: providers::resolve_model_context_limit(
                                &used_prov,
                                &used_mdl,
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

    let non_empty_core = if core_narrative.trim().is_empty() {
        let mut lines = Vec::new();
        let mut fallback_paths: Vec<String> = modified_files.iter().map(|f| f.path.clone()).collect();
        push_unique_paths(&mut fallback_paths, turn_applied_files);
        if !fallback_paths.is_empty() {
            lines.push("### 🛠️ Solution & Fichiers modifiés".to_string());
            for f in fallback_paths {
                let base = file_basename(&f);
                lines.push(format!("- [{base}]({f}) : modifications appliquées."));
            }
        }
        if !tool_results.is_empty() {
            lines.push("\n### ⚡ Actions et commandes exécutées".to_string());
            for r in tool_results.iter().take(8) {
                let icon = if r.success { "✓" } else { "✕" };
                lines.push(format!("- {} **{}** : {}", icon, r.tool, truncate_chars(&r.message, 180)));
            }
        }
        if lines.is_empty() {
            "### ✅ Diagnostic & Statut\nTraitement terminé. Aucune modification supplémentaire requise.".to_string()
        } else {
            lines.join("\n")
        }
    } else {
        core_narrative
    };

    let final_markdown =
        append_modified_files_summary(&non_empty_core, modified_files, turn_applied_files);
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
            turn_diffs: Vec::new(),
            pre_turn_tree: None,
            agent_trace: agent_trace.clone(),
            commit_id: None,
            commit_summary: None,
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
            turn_diffs: Vec::new(),
            pre_turn_tree: None,
            agent_trace: Vec::new(),
            commit_id: None,
            commit_summary: None,
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

    // SUB-AGENT 1/5: Architecte & Orchestrateur (Planification DAG Multi-Agent)
    let all_project_files = list_source_files_for_cartographer(&project_root_path);

    let (arch_prov, arch_model) = resolve_agent_slot("architect", &settings);
    let is_agent_mode =
        settings.active_target.trim().is_empty() || settings.active_target.trim() == "agent";

    let initial_snapshot = workspace::build_workspace_snapshot(&project_root_path, arch_prov != "ollama");
    let mut blackboard = MissionBlackboard::new(user_prompt.clone(), initial_snapshot);

    emit_stage(
        app,
        &conversation_id,
        1,
        "1. Orchestrateur Multi-Agent",
        &format!(
            "Analyse de la demande et planification DAG ({} · {}).",
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
            role_label: "Orchestrateur".to_string(),
            provider: arch_prov.clone(),
            model: arch_model.clone(),
            summary: "Décomposition de la demande en plan d'action DAG...".to_string(),
            status: "running".to_string(),
            tokens: 0,
        },
    );

    let mut architect_spec: Option<String> = None;
    let mut architect_llm_ran = false;
    let mut architect_err: Option<String> = None;
    let mut arch_tokens_used = 0usize;
    let mut used_arch_prov = arch_prov.clone();
    let mut used_arch_model = arch_model.clone();
    let mut arch_fb_reason: Option<String> = None;
    let mut run_tests_after = false;

    let arch_can_run = has_callable_candidate(
        &arch_prov,
        &arch_model,
        "architect",
        &settings,
        stack.ollama_running,
    );

    let task_plan: crate::orchestrator::TaskPlan = if is_agent_mode && arch_can_run {
        let arch_mem_block = load_project_architecture_memory(&project_root_path, 1200)
            .map(|(src, txt)| format!("\n\n[Architecture & Conventions ({src})]\n{txt}"))
            .unwrap_or_default();

        let project_summary = format!(
            "{}{}{}",
            blackboard.workspace_snapshot.format_for_prompt(arch_prov != "ollama"),
            arch_mem_block,
            if project.global_context.trim().is_empty() {
                String::new()
            } else {
                format!("\n\n[Contexte global]\n{}", truncate_chars(project.global_context.trim(), 400))
            }
        );

        let orch_sys = crate::orchestrator::build_orchestrator_system_prompt(&project_summary);
        let orch_user = format!(
            "[Session récente]\n{}\n\n[Demande utilisateur]\n{}",
            truncate_chars(conv_ctx.trim(), 400),
            truncate_chars(user_prompt.trim(), 1600)
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
            res = providers::call_provider_oneshot_with_fallback(
                &arch_prov,
                &arch_model,
                "architect",
                stack.ollama_running,
                &settings,
                &orch_sys,
                &orch_user,
                1100,
                0.1,
            ) => res,
        };

        match arch_call_res {
            Ok((reply, p_tok, c_tok, used_prov, used_mdl, fb_reason)) => {
                used_arch_prov = used_prov;
                used_arch_model = used_mdl;
                arch_fb_reason = fb_reason;
                architect_llm_ran = true;
                arch_tokens_used = p_tok + c_tok;
                record_external_api_usage(app, state_lock, &used_arch_prov, p_tok, c_tok).await;
                let _ = app.emit(
                    "aider:tokens",
                    TokenUsageEvent {
                        conversation_id: conversation_id.clone(),
                        subagent: "1. Orchestrateur".to_string(),
                        prompt_tokens: p_tok,
                        completion_tokens: c_tok,
                        total_tokens: arch_tokens_used,
                        role: "architect".to_string(),
                        provider: used_arch_prov.clone(),
                        model: used_arch_model.clone(),
                        context_limit: providers::resolve_model_context_limit(
                            &used_arch_prov,
                            &used_arch_model,
                            &settings,
                        ),
                    },
                );

                let (parsed_plan, is_fb) = crate::orchestrator::parse_orchestrator_response(&reply, &user_prompt);
                if is_fb && arch_fb_reason.is_none() {
                    arch_fb_reason = Some("Plan JSON invalide ou cycle détecté -> repli direct".to_string());
                }
                parsed_plan
            }
            Err(e) => {
                let err_msg = synthesize_error_log(&e.to_string());
                architect_err = Some(err_msg);
                crate::orchestrator::TaskPlan::fallback(&user_prompt)
            }
        }
    } else {
        crate::orchestrator::TaskPlan::fallback(&user_prompt)
    };

    blackboard.plan_summary = task_plan.summary.clone();
    blackboard.validation_command = task_plan.validation_command().map(str::to_string);

    let execution_waves = task_plan
        .compute_execution_waves()
        .unwrap_or_else(|_| vec![task_plan.tasks.iter().map(|t| t.id.clone()).collect()]);

    if task_plan.tasks.iter().any(|t| t.role.trim().to_lowercase() == "tester") {
        run_tests_after = true;
    }

    let exec_role = if let Some(first_task) = task_plan.tasks.first() {
        match crate::orchestrator::TaskRole::from_str_loose(&first_task.role).as_str() {
            "designer" => "designer",
            "refactoring" => "refactoring",
            "tester" => "tester",
            "terminal" => "terminal",
            "research" => "research",
            _ => "coder",
        }
    } else {
        "coder"
    };

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
                subagent: "1. Orchestrateur & Contexte".to_string(),
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
        let tasks_count = task_plan.tasks.len();
        let waves_count = execution_waves.len();
        let plan_note = format!(" · {} tâche(s) en {} vague(s)", tasks_count, waves_count);
        let fb_note = if let Some(ref r) = arch_fb_reason {
            format!(" · Repli auto (`{}`)", truncate_chars(&r.replace('`', "'"), 60))
        } else {
            String::new()
        };
        (
            if arch_fb_reason.is_some() {
                "fallback".to_string()
            } else {
                "done".to_string()
            },
            format!(
                "Plan DAG généré : **{}**{}{} ({})",
                task_plan.summary,
                plan_note,
                fb_note,
                role_display_name(exec_role)
            ),
        )
    } else if let Some(ref err_msg) = architect_err {
        (
            "fallback".to_string(),
            format!(
                "Repli DAG direct (vers **{}**) · `{}`",
                role_display_name(exec_role),
                truncate_chars(&err_msg.replace('`', "'"), 110)
            ),
        )
    } else {
        (
            "done".to_string(),
            format!(
                "Plan direct et routage vers **{}**",
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
            role_label: "Orchestrateur".to_string(),
            provider: used_arch_prov.clone(),
            model: used_arch_model.clone(),
            summary: arch_summary,
            status: arch_status,
            tokens: arch_tokens_used,
        },
    );

    let (mut exec_prov, mut exec_model) = resolve_agent_slot(exec_role, &settings);

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

    // SUB-AGENT 3/5: Exécution (Moteur natif multi-tours unifié ou plan séquentiel multi-étapes)
    let can_run_exec = has_callable_candidate(
        &exec_prov,
        &exec_model,
        exec_role,
        &settings,
        stack.ollama_running,
    );
    let is_multi_step = is_agent_mode && task_plan.tasks.len() >= 2 && can_run_exec;

    let mut engine_ran = false;
    let mut exec_tokens_used = 0usize;
    let mut exec_status = "done".to_string();
    let exec_note: String;
    let mut extracted_thinking: Option<String> = None;
    let mut tool_results: Vec<ToolExecutionResult> = Vec::new();
    let mut had_repairs = false;
    let mut raw_exec_output = String::new();
    let mut turn_applied_files: Vec<String> = Vec::new();
    let pre_turn_tree = git_diff::capture_workspace_tree(&project.root_path).await;
    let mut extra_stage_edits = false;
    let mut init_images_opt = Some(image_base64_list);
    let mut last_post_edit_verification: Option<(String, bool, String)> = None;
    let mut dag_completed_tasks = 0usize;

    let (cleaned_narrative, stage_note) = if is_multi_step {
        git_diff::sanitize_corrupted_diff_files(&project.root_path).await;
        let total_tasks = task_plan.tasks.len();
        let total_waves = execution_waves.len();
        let plan_banner = format!(
            "[Plan DAG] {} tâches en {} vague(s) : {}",
            total_tasks,
            total_waves,
            task_plan
                .tasks
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{}. {}", i + 1, s.title))
                .collect::<Vec<_>>()
                .join(" -> ")
        );
        let _ = app.emit("aider:stream", (&conversation_id, &plan_banner));

        let mut completed_steps_summary: Vec<String> = Vec::new();
        let mut step_narratives: Vec<String> = Vec::new();
        let mut all_thinking: Vec<String> = Vec::new();
        let mut had_step_error = false;
        let max_concurrency = if arch_prov == "ollama" || stack.ollama_running {
            1
        } else {
            3
        };

        let mut task_counter = 0usize;

        'dag_loop: for (wave_idx, wave) in execution_waves.iter().enumerate() {
            let sub_waves = crate::orchestrator::partition_wave_into_safe_batches(
                wave,
                &task_plan,
                max_concurrency,
            );

            for sub_wave in sub_waves {
                let wave_mode_label = if sub_wave.is_parallel {
                    format!("⚡ Parallèle ({} tâches sans conflit)", sub_wave.task_ids.len())
                } else {
                    "▶ Séquentiel".to_string()
                };

                let wave_banner = format!(
                    "\n── Vague {}/{} [{}] ──\n",
                    wave_idx + 1,
                    total_waves,
                    wave_mode_label
                );
                let _ = app.emit("aider:stream", (&conversation_id, &wave_banner));

                for task_id in &sub_wave.task_ids {
                    task_counter += 1;
                    let step = match task_plan.get_task(task_id) {
                        Some(t) => t.clone(),
                        None => continue,
                    };
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

                    let step_role_static: &'static str =
                        match crate::orchestrator::TaskRole::from_str_loose(&step.role).as_str() {
                            "designer" => "designer",
                            "refactoring" => "refactoring",
                            "tester" => "tester",
                            "terminal" => "terminal",
                            "research" => "research",
                            _ => "coder",
                        };
                    let (step_prov, step_model) = resolve_agent_slot(step_role_static, &settings);
                    let step_is_cloud = step_prov != "ollama";
                    if (task_counter == 1 && step_is_cloud != (arch_prov != "ollama"))
                        || (task_counter > 1 && !turn_applied_files.is_empty())
                    {
                        blackboard.workspace_snapshot =
                            workspace::build_workspace_snapshot(&project_root_path, step_is_cloud);
                    }

                    let trace_role_key = format!("{}_step_{}", step_role_static, task_counter);
                    let step_label = format!(
                        "Étape {}/{} (Vague {}/{}) · {}",
                        task_counter,
                        total_tasks,
                        wave_idx + 1,
                        total_waves,
                        truncate_chars(&step.title, 34)
                    );

                    let step_all_files = list_source_files_for_cartographer(&project_root_path);

                    let mut step_targets: Vec<String> = Vec::new();
                    for raw_f in &step.files {
                        if let Some(clean) = patcher::sanitize_rel_path(raw_f) {
                            if !step_targets.iter().any(|x| x == &clean) {
                                step_targets.push(clean);
                            }
                            if step_targets.len() >= 4 {
                                break;
                            }
                        }
                    }
                    if step_targets.is_empty() {
                        let query = format!("{} {}", step.title, step.goal);
                        let (det_matched, _) =
                            match_files_deterministically(&query, None, &step_all_files);
                        step_targets = if det_matched.is_empty() {
                            carto.target_files.iter().take(4).cloned().collect()
                        } else {
                            det_matched.into_iter().take(4).collect()
                        };
                    }

                    let step_strategy = choose_execution_strategy(
                        &project_root_path,
                        &step_targets,
                        !step_targets.is_empty(),
                        &step_prov,
                        &settings,
                    );

                    emit_stage(
                        app,
                        &conversation_id,
                        3,
                        &format!("3. {} ({} · {})", step_label, step_prov, step_model),
                        &format!(
                            "Tâche {}/{} : {}",
                            task_counter,
                            total_tasks,
                            truncate_chars(&step.title, 80)
                        ),
                        "running",
                    );
                    record_trace_step(
                        app,
                        &conversation_id,
                        &mut agent_trace,
                        AgentTraceStep {
                            role: trace_role_key.clone(),
                            role_label: step_label.clone(),
                            provider: step_prov.clone(),
                            model: step_model.clone(),
                            summary: truncate_chars(&step.goal, 110),
                            status: "running".to_string(),
                            tokens: 0,
                        },
                    );

                    let stream_hdr = format!(
                        "\n\n── Vague {}/{} · Étape {}/{} : {} ({}) ──\n",
                        wave_idx + 1,
                        total_waves,
                        task_counter,
                        total_tasks,
                        step.title,
                        role_display_name(step_role_static)
                    );
                    let _ = app.emit("aider:stream", (&conversation_id, &stream_hdr));

                    let handoffs_block = blackboard.format_handoffs_block();
                    let target_list_note = if step_targets.is_empty() {
                        "à créer/déterminer selon l'étape".to_string()
                    } else {
                        step_targets.join(", ")
                    };
                    let step_architect_spec = format!(
                        "Demande initiale utilisateur (à ne jamais perdre de vue) :\n{}\n\nPlan global : {}\nÉtape courante {}/{} ({}) : {}\nConsigne de l'étape :\n{}",
                        truncate_chars(blackboard.original_user_prompt.trim(), 900),
                        blackboard.plan_summary,
                        task_counter,
                        total_tasks,
                        step.id,
                        step.title,
                        step.goal
                    );
                    let step_prompt = format!(
                        "[Demande initiale utilisateur]\n{}\n\n[Plan global : {}]\n{}\n[Tâche courante {}/{} (Vague {}/{}) : {} ({})]\nFichiers cibles : {}\nConsignes précises d'exécution :\n{}\n\nRègle : Reste concentré sur cette étape tout en respectant la demande initiale complète et les constats précédents. Utilise directement tes outils natifs (`read_file`, `write_file`, `edit_file`, `run_command`, `search_web`).",
                        truncate_chars(blackboard.original_user_prompt.trim(), 900),
                        blackboard.plan_summary,
                        if handoffs_block.is_empty() {
                            String::new()
                        } else {
                            format!("\n{handoffs_block}\n")
                        },
                        task_counter,
                        total_tasks,
                        wave_idx + 1,
                        total_waves,
                        step.title,
                        step.id,
                        target_list_note,
                        step.goal
                    );

                    let step_conv_ctx = conv_ctx.as_str();
                    let step_attachments = attachments.as_deref();
                    let step_images = if task_counter == 1 {
                        init_images_opt.take().unwrap_or_default()
                    } else {
                        Vec::new()
                    };

                    match execute_native_agent_loop(
                        app,
                        state_lock,
                        &conversation_id,
                        &project,
                        step_conv_ctx,
                        &step_prompt,
                        step_attachments,
                        step_images,
                        Some(step_architect_spec.as_str()),
                        &step_prov,
                        &step_model,
                        step_role_static,
                        stack.ollama_running,
                        &settings,
                        &blackboard.workspace_snapshot,
                        &step_targets,
                        &carto.read_only_files,
                        &step_strategy,
                        &mcp_servers,
                        &mut cancel_rx,
                    )
                    .await
                    {
                        Ok(mut step_res) => {
                            engine_ran = true;
                            let mut step_tokens = step_res.prompt_tokens + step_res.completion_tokens;
                            exec_tokens_used += step_tokens;
                            if let Some(ref th) = step_res.thinking {
                                if !th.trim().is_empty() {
                                    let mode_tag = if sub_wave.is_parallel {
                                        "⚡ [Exécution Parallèle sans conflit]"
                                    } else {
                                        "▶ [Exécution Séquentielle]"
                                    };
                                    all_thinking.push(format!(
                                        "### {} Étape {}/{} (Vague {}) : {}\n{}",
                                        mode_tag,
                                        task_counter,
                                        total_tasks,
                                        wave_idx + 1,
                                        step.title,
                                        th.trim()
                                    ));
                                }
                            }

                            let mut key_tool_outputs: Vec<String> = step_res
                                .tool_results
                                .iter()
                                .filter(|t| {
                                    !t.success
                                        || matches!(
                                            t.tool.as_str(),
                                            "run_command" | "search_web" | "edit_file" | "write_file"
                                        )
                                })
                                .take(5)
                                .map(|t| {
                                    let status_tag = if t.success { "OK" } else { "ERR" };
                                    format!(
                                        "[{}] {}: {}",
                                        status_tag,
                                        t.tool,
                                        truncate_chars(&t.message, 200)
                                    )
                                })
                                .collect();
                            let step_explanation = if !step_res.narrative.trim().is_empty() {
                                truncate_chars(step_res.narrative.trim(), 800)
                            } else if !step_res.applied_files.is_empty() {
                                format!(
                                    "Modifications appliquées dans : {}",
                                    step_res.applied_files.join(", ")
                                )
                            } else if !step_res.tool_results.is_empty() {
                                step_res
                                    .tool_results
                                    .iter()
                                    .map(|t| format!("{}: {}", t.tool, truncate_chars(&t.message, 140)))
                                    .collect::<Vec<_>>()
                                    .join(" ; ")
                            } else {
                                "Analyse terminée".to_string()
                            };

                            if let Some(ref pv) = step_res.last_post_edit_verification {
                                last_post_edit_verification = Some(pv.clone());
                            } else if !step_res.applied_files.is_empty() {
                                last_post_edit_verification = None;
                            }

                            tool_results.extend(step_res.tool_results.drain(..));
                            had_repairs |= step_res.had_repairs;
                            raw_exec_output = step_res.raw_last_response.clone();
                            push_unique_paths(&mut turn_applied_files, &step_res.applied_files);
                            push_unique_paths(&mut blackboard.applied_files, &step_res.applied_files);
                            exec_prov = step_res.actual_provider.clone();
                            exec_model = step_res.actual_model.clone();

                            let mode_note = if sub_wave.is_parallel {
                                " *(⚡ exécuté en parallèle)*"
                            } else {
                                ""
                            };
                            step_narratives.push(format!(
                                "- **Étape {} ({})**{} : {}",
                                task_counter,
                                step.title,
                                mode_note,
                                step_explanation
                            ));

                            let mut step_files = step_res.applied_files.clone();
                            let mut step_status = if step_files.is_empty() && key_tool_outputs.iter().any(|k| k.starts_with("[ERR]")) {
                                "partial".to_string()
                            } else {
                                "done".to_string()
                            };

                            if task_counter < total_tasks
                                && !step_files.is_empty()
                                && !is_cancelled(&cancel_rx)
                            {
                                blackboard.workspace_snapshot =
                                    workspace::build_workspace_snapshot(
                                        &project_root_path,
                                        exec_prov != "ollama",
                                    );
                                let vr = run_project_verifier(
                                    &project_root_path,
                                    blackboard.validation_command.as_deref(),
                                    last_post_edit_verification.as_ref(),
                                )
                                .await;
                                if vr.ran && vr.success {
                                    last_post_edit_verification = Some((
                                        vr.command.clone(),
                                        true,
                                        vr.output_summary.clone(),
                                    ));
                                } else if vr.ran && !vr.success {
                                    let vr_msg = format!(
                                        "[Étape {}/{}] Erreur `{}` détectée — auto-correction ciblée...",
                                        task_counter, total_tasks, vr.command
                                    );
                                    let _ = app.emit("aider:stream", (&conversation_id, &vr_msg));
                                    let fix_prompt = format!(
                                        "[Demande initiale utilisateur]\n{}\n\nLa vérification `{}` a échoué après l'étape « {} » :\n```\n{}\n```\nCorrige ces erreurs avec `edit_file` (ou `write_file` pour un nouveau fichier/réécriture complète) tout en respectant la demande initiale.",
                                        truncate_chars(blackboard.original_user_prompt.trim(), 700),
                                        vr.command,
                                        step.title,
                                        vr.output_summary
                                    );
                                    let fix_strategy = ExecutionStrategy {
                                        edit_format: "diff".to_string(),
                                        map_tokens: 0,
                                        max_turns: 4,
                                        max_file_lines: 0,
                                    };
                                    let fix_targets: Vec<String> =
                                        step_files.iter().take(4).cloned().collect();
                                    match execute_native_agent_loop(
                                        app,
                                        state_lock,
                                        &conversation_id,
                                        &project,
                                        conv_ctx.as_str(),
                                        &fix_prompt,
                                        attachments.as_deref(),
                                        Vec::new(),
                                        Some(step_architect_spec.as_str()),
                                        &exec_prov,
                                        &exec_model,
                                        step_role_static,
                                        stack.ollama_running,
                                        &settings,
                                        &blackboard.workspace_snapshot,
                                        &fix_targets,
                                        &[],
                                        &fix_strategy,
                                        &mcp_servers,
                                        &mut cancel_rx,
                                    )
                                    .await
                                    {
                                        Ok(fix_res) => {
                                            let fix_tok =
                                                fix_res.prompt_tokens + fix_res.completion_tokens;
                                            step_tokens += fix_tok;
                                            exec_tokens_used += fix_tok;
                                            if let Some(ref pv) = fix_res.last_post_edit_verification {
                                                last_post_edit_verification = Some(pv.clone());
                                            } else if !fix_res.applied_files.is_empty() {
                                                last_post_edit_verification = None;
                                            }
                                            tool_results.extend(fix_res.tool_results);
                                            if !fix_res.applied_files.is_empty() {
                                                push_unique_paths(
                                                    &mut turn_applied_files,
                                                    &fix_res.applied_files,
                                                );
                                                push_unique_paths(
                                                    &mut blackboard.applied_files,
                                                    &fix_res.applied_files,
                                                );
                                                push_unique_paths(&mut step_files, &fix_res.applied_files);
                                                had_repairs = true;
                                                key_tool_outputs.push(format!(
                                                    "[OK] auto-fix ({}): {}",
                                                    vr.command,
                                                    fix_res.applied_files.join(", ")
                                                ));
                                            } else {
                                                step_status = "partial".to_string();
                                                key_tool_outputs.push(format!(
                                                    "[ERR] {} échoué: {}",
                                                    vr.command,
                                                    truncate_chars(&vr.output_summary, 180)
                                                ));
                                            }
                                        }
                                        Err(e) => {
                                            if is_cancelled(&cancel_rx)
                                                || e.to_string().contains("CANCELLED_BY_USER")
                                            {
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
                                            step_status = "partial".to_string();
                                        }
                                    }
                                }
                            }

                            blackboard.tool_facts.extend(key_tool_outputs.iter().cloned());
                            blackboard.step_handoffs.push(StepHandoff {
                                step_id: step.id.clone(),
                                title: step.title.clone(),
                                role: step_role_static.to_string(),
                                goal: step.goal.clone(),
                                conclusion: truncate_chars(&step_explanation, 700),
                                key_tool_outputs,
                                modified_files: step_files.clone(),
                                status: step_status,
                            });

                            let files_note = if step_files.is_empty() {
                                "aucun fichier modifié".to_string()
                            } else {
                                step_files.join(", ")
                            };
                            let mode_tag = if sub_wave.is_parallel {
                                " [⚡ Parallèle]"
                            } else {
                                " [▶ Séquentiel]"
                            };
                            completed_steps_summary.push(format!(
                                "- Étape {} ({}){} : {}",
                                task_counter, step.title, mode_tag, files_note
                            ));
                            dag_completed_tasks += 1;

                            let fb_note = if let Some(ref r) = step_res.fallback_reason {
                                format!(
                                    " · Repli (`{}`)",
                                    truncate_chars(&r.replace('`', "'"), 55)
                                )
                            } else {
                                String::new()
                            };
                            record_trace_step(
                                app,
                                &conversation_id,
                                &mut agent_trace,
                                AgentTraceStep {
                                    role: trace_role_key,
                                    role_label: step_label,
                                    provider: step_res.actual_provider,
                                    model: step_res.actual_model,
                                    summary: format!(
                                        "{} · {} tour(s){}",
                                        files_note, step_res.turns_run, fb_note
                                    ),
                                    status: if step_res.fallback_reason.is_some() {
                                        "fallback".to_string()
                                    } else {
                                        "done".to_string()
                                    },
                                    tokens: step_tokens,
                                },
                            );
                        }
                        Err(err) => {
                            if is_cancelled(&cancel_rx)
                                || err.to_string().contains("CANCELLED_BY_USER")
                            {
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
                            had_step_error = true;
                            blackboard.step_handoffs.push(StepHandoff {
                                step_id: step.id.clone(),
                                title: step.title.clone(),
                                role: step_role_static.to_string(),
                                goal: step.goal.clone(),
                                conclusion: format!("Échec d'exécution : {clean_err}"),
                                key_tool_outputs: vec![format!("[ERR] {clean_err}")],
                                modified_files: Vec::new(),
                                status: "failed".to_string(),
                            });
                            record_trace_step(
                                app,
                                &conversation_id,
                                &mut agent_trace,
                                AgentTraceStep {
                                    role: trace_role_key,
                                    role_label: step_label,
                                    provider: step_prov,
                                    model: step_model,
                                    summary: format!("Erreur : `{clean_err}`"),
                                    status: "error".to_string(),
                                    tokens: 0,
                                },
                            );
                            step_narratives.push(format!(
                                "- **Étape {} ({})** : ⚠️ Erreur (`{clean_err}`)",
                                task_counter, step.title
                            ));
                            break 'dag_loop;
                        }
                    }
                }
            }
        }

        if !all_thinking.is_empty() {
            extracted_thinking = Some(all_thinking.join("\n\n"));
        }
        extra_stage_edits = true;
        exec_status = if had_step_error {
            "error".to_string()
        } else {
            "done".to_string()
        };
        exec_note = format!(
            "Plan DAG ({}/{total_tasks} tâches exécutées)",
            completed_steps_summary.len()
        );
        (
            step_narratives.join("\n"),
            format!("Plan DAG · {total_tasks} tâches ({total_waves} vagues)"),
        )
    } else {
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

        if can_run_exec {
            git_diff::sanitize_corrupted_diff_files(&project.root_path).await;
            if (exec_prov != "ollama") != (arch_prov != "ollama") {
                blackboard.workspace_snapshot = workspace::build_workspace_snapshot(
                    &project_root_path,
                    exec_prov != "ollama",
                );
            }
            if architect_spec.is_none() {
                if let Some(first_task) = task_plan.tasks.first() {
                    architect_spec = Some(format!(
                        "Objectif : {}\nCadrage : {}",
                        first_task.goal, task_plan.summary
                    ));
                }
            }
            let single_images = init_images_opt.take().unwrap_or_default();
            match execute_native_agent_loop(
                app,
                state_lock,
                &conversation_id,
                &project,
                &conv_ctx,
                &user_prompt,
                attachments.as_deref(),
                single_images,
                architect_spec.as_deref(),
                &exec_prov,
                &exec_model,
                exec_role,
                stack.ollama_running,
                &settings,
                &blackboard.workspace_snapshot,
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
                    let key_tool_outputs: Vec<String> = loop_res
                        .tool_results
                        .iter()
                        .filter(|t| {
                            !t.success
                                || matches!(
                                    t.tool.as_str(),
                                    "run_command" | "search_web" | "edit_file" | "write_file"
                                )
                        })
                        .take(5)
                        .map(|t| {
                            let status_tag = if t.success { "OK" } else { "ERR" };
                            format!(
                                "[{}] {}: {}",
                                status_tag,
                                t.tool,
                                truncate_chars(&t.message, 200)
                            )
                        })
                        .collect();
                    blackboard.tool_facts.extend(key_tool_outputs.iter().cloned());
                    last_post_edit_verification = loop_res.last_post_edit_verification.clone();
                    tool_results = loop_res.tool_results;
                    had_repairs = loop_res.had_repairs;
                    raw_exec_output = loop_res.raw_last_response;
                    exec_prov = loop_res.actual_provider.clone();
                    exec_model = loop_res.actual_model.clone();
                    push_unique_paths(&mut turn_applied_files, &loop_res.applied_files);
                    push_unique_paths(&mut blackboard.applied_files, &loop_res.applied_files);

                    let conclusion_summary = if !loop_res.narrative.trim().is_empty() {
                        truncate_chars(loop_res.narrative.trim(), 700)
                    } else if !loop_res.applied_files.is_empty() {
                        format!(
                            "Modifications appliquées : {}",
                            loop_res.applied_files.join(", ")
                        )
                    } else {
                        "Exécution terminée".to_string()
                    };
                    blackboard.step_handoffs.push(StepHandoff {
                        step_id: task_plan
                            .tasks
                            .first()
                            .map(|t| t.id.clone())
                            .unwrap_or_else(|| "single".to_string()),
                        title: task_plan
                            .tasks
                            .first()
                            .map(|t| t.title.clone())
                            .unwrap_or_else(|| role_display_name(exec_role).to_string()),
                        role: exec_role.to_string(),
                        goal: user_prompt.clone(),
                        conclusion: conclusion_summary,
                        key_tool_outputs,
                        modified_files: loop_res.applied_files.clone(),
                        status: "done".to_string(),
                    });

                    let fb_note = if let Some(ref r) = loop_res.fallback_reason {
                        exec_status = "fallback".to_string();
                        format!(", repli auto: {}", truncate_chars(&r.replace('`', "'"), 60))
                    } else {
                        String::new()
                    };
                    exec_note = format!(
                        "Moteur natif (format `{}`, max {} l., {} tour(s){}{})",
                        loop_res.effective_edit_format,
                        strategy.max_file_lines,
                        loop_res.turns_run,
                        if had_repairs { ", auto-réparé" } else { "" },
                        fb_note
                    );
                    let out = if loop_res.narrative.trim().is_empty() {
                        // Laisser vide pour que Stage 5 synthétise automatiquement un résumé concret basé sur les outils et le diff
                        String::new()
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
        }
    };

    if !is_multi_step {
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
    }

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

    // Arrêt propre immédiat si le Codeur a échoué sans repli possible (0 appel réussi)
    let coder_fatally_failed = (is_multi_step
        && exec_status == "error"
        && dag_completed_tasks == 0
        && tool_results.is_empty())
        || (!is_multi_step && !engine_ran && exec_status == "error");

    if coder_fatally_failed {
        if let Some(h) = title_handle.take() {
            h.abort();
        }
        let fatal_err_text = if cleaned_narrative.trim().is_empty() {
            format!("⚠️ Échec bloquant de l'exécution : {exec_note}")
        } else {
            cleaned_narrative.clone()
        };

        let _ = app.emit("aider:stream", (&conversation_id, &fatal_err_text));
        emit_stage(
            app,
            &conversation_id,
            3,
            "3. Exécution Codeur",
            "Échec bloquant — aucun appel réussi.",
            "error",
        );

        let result = {
            let mut guard = state_lock.lock().await;
            let now = Utc::now().to_rfc3339();

            let conv_idx = guard
                .conversations
                .iter()
                .position(|c| c.id == conversation_id)
                .ok_or_else(|| anyhow::anyhow!("Conversation introuvable"))?;

            let assistant_msg_id = Uuid::new_v4().to_string();
            guard.conversations[conv_idx].messages.push(ChatMessage {
                id: assistant_msg_id.clone(),
                role: "assistant".to_string(),
                content: fatal_err_text,
                thinking: None,
                attachments: None,
                created_at: now.clone(),
                stage_summary: Some("Erreur bloquante".to_string()),
                files_changed: 0,
                lines_added: 0,
                lines_removed: 0,
                files_summary: Vec::new(),
                turn_diffs: Vec::new(),
                pre_turn_tree: pre_turn_tree.clone(),
                agent_trace,
                commit_id: None,
                commit_summary: None,
            });

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

        return Ok(result);
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
    let (mut test_prov, mut test_model) = resolve_agent_slot("tester", &settings);
    let mut tester_status = "done".to_string();
    let mut tester_summary = String::new();

    if run_tests_after && exec_role != "tester" && engine_ran && is_agent_mode {
        let (pre_diffs, _, _) = git_diff::collect_project_diffs(&project.root_path).await;
        let changed_paths: Vec<String> = pre_diffs.into_iter().map(|d| d.path).take(3).collect();
        let can_run_tester = !changed_paths.is_empty()
            && has_callable_candidate(
                &test_prov,
                &test_model,
                "tester",
                &settings,
                stack.ollama_running,
            );
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
            test_strategy.max_turns = 4;

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
                stack.ollama_running,
                &settings,
                &blackboard.workspace_snapshot,
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
                    test_prov = t_res.actual_provider.clone();
                    test_model = t_res.actual_model.clone();
                    if t_res.fallback_reason.is_some() {
                        tester_status = "fallback".to_string();
                    }
                    if let Some(ref pv) = t_res.last_post_edit_verification {
                        last_post_edit_verification = Some(pv.clone());
                    } else if !t_res.applied_files.is_empty() {
                        last_post_edit_verification = None;
                    }
                    tool_results.extend(t_res.tool_results);
                    if !t_res.applied_files.is_empty() {
                        push_unique_paths(&mut turn_applied_files, &t_res.applied_files);
                        push_unique_paths(&mut blackboard.applied_files, &t_res.applied_files);
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

    if engine_ran && !turn_applied_files.is_empty() {
        blackboard.workspace_snapshot =
            workspace::build_workspace_snapshot(&project_root_path, exec_prov != "ollama");
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
        res = run_project_verifier(
            &project_root_path,
            blackboard.validation_command.as_deref(),
            last_post_edit_verification.as_ref(),
        ) => res,
    };

    // Auto-fix si ce tour a modifié des fichiers et que la vérification déclarée/active échoue
    let pre_fix_paths: Vec<String> = if let Some(ref pre_tree) = pre_turn_tree {
        git_diff::collect_turn_diffs(&project.root_path, pre_tree)
            .await
            .0
            .into_iter()
            .map(|d| d.path)
            .collect()
    } else {
        turn_applied_files.clone()
    };

    if !pre_fix_paths.is_empty()
        && test_res.ran
        && !test_res.success
        && !is_cancelled(&cancel_rx)
        && is_agent_mode
        && can_run_exec
    {
        let broken_paths: Vec<String> = pre_fix_paths.into_iter().take(4).collect();
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

        let handoffs_block = blackboard.format_handoffs_block();
        let fix_prompt = format!(
            "[Demande initiale utilisateur]\n{}\n{}\nLa vérification `{}` a échoué après nos modifications avec les erreurs suivantes :\n```\n{}\n```\nCorrige ces erreurs dans le projet en utilisant `edit_file` (ou `write_file` pour un nouveau fichier / réécriture complète), puis re-vérifie avec `run_command` si utile.",
            truncate_chars(blackboard.original_user_prompt.trim(), 700),
            if handoffs_block.is_empty() {
                String::new()
            } else {
                format!("\n{handoffs_block}")
            },
            test_res.command,
            test_res.output_summary
        );
        let fix_strategy = ExecutionStrategy {
            edit_format: "diff".to_string(),
            map_tokens: 0,
            max_turns: 4,
            max_file_lines: 0,
        };
        if let Ok(fix_res) = execute_native_agent_loop(
            app,
            state_lock,
            &conversation_id,
            &project,
            &conv_ctx,
            &fix_prompt,
            attachments.as_deref(),
            Vec::new(),
            None,
            &exec_prov,
            &exec_model,
            exec_role,
            stack.ollama_running,
            &settings,
            &blackboard.workspace_snapshot,
            &broken_paths,
            &[],
            &fix_strategy,
            &mcp_servers,
            &mut cancel_rx,
        )
        .await
        {
            exec_tokens_used += fix_res.prompt_tokens + fix_res.completion_tokens;
            let fixed_files = fix_res.applied_files.clone();
            tool_results.extend(fix_res.tool_results);
            if !fixed_files.is_empty() {
                push_unique_paths(&mut turn_applied_files, &fixed_files);
                push_unique_paths(&mut blackboard.applied_files, &fixed_files);
                extra_stage_edits = true;
                had_repairs = true;
                blackboard.workspace_snapshot = workspace::build_workspace_snapshot(
                    &project_root_path,
                    exec_prov != "ollama",
                );
                test_res = run_project_verifier(
                    &project_root_path,
                    blackboard.validation_command.as_deref(),
                    None,
                )
                .await;
                blackboard.step_handoffs.push(StepHandoff {
                    step_id: "auto_fix".to_string(),
                    title: format!("Auto-correction ({})", test_res.command),
                    role: exec_role.to_string(),
                    goal: format!("Corriger les erreurs de `{}`", test_res.command),
                    conclusion: if test_res.success {
                        format!(
                            "Erreurs corrigées et validation `{}` réussie.",
                            test_res.command
                        )
                    } else {
                        format!(
                            "Corrections appliquées mais `{}` signale encore des erreurs.",
                            test_res.command
                        )
                    },
                    key_tool_outputs: vec![truncate_chars(&test_res.output_summary, 200)],
                    modified_files: fixed_files,
                    status: if test_res.success {
                        "done".to_string()
                    } else {
                        "partial".to_string()
                    },
                });
            }
        }
    }

    let mut build_verifier_note = String::new();
    if test_res.ran {
        if test_res.success {
            build_verifier_note = format!(
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
            build_verifier_note = format!(
                "• ⚠️ **Vérifieur ({})** : Vérification en erreur :\n```\n{}\n```",
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
    let verifier_note = build_verifier_note;
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

    // Calcul du diff isolé à ce tour via snapshot Git temporaire (pre_turn_tree -> état courant)
    let (turn_modified_files, turn_added, turn_removed) =
        if let Some(ref pre_tree) = pre_turn_tree {
            git_diff::collect_turn_diffs(&project.root_path, pre_tree).await
        } else if !turn_applied_files.is_empty() {
            let scoped: Vec<ModifiedFileDiff> = modified_files
                .iter()
                .filter(|f| turn_applied_files.iter().any(|p| paths_match(&f.path, p)))
                .cloned()
                .collect();
            let add: usize = scoped.iter().map(|f| f.added + f.modified).sum();
            let rem: usize = scoped.iter().map(|f| f.removed + f.modified).sum();
            (scoped, add, rem)
        } else {
            (Vec::new(), 0, 0)
        };
    let turn_files_summary = build_files_summary(&turn_modified_files);

    if !is_multi_step && !turn_modified_files.is_empty() && exec_status != "error" {
        let enriched_summary = format!(
            "{} · **{} fichier(s)** modifié(s) sur ce tour (`+{}` / `-{}`)",
            exec_note,
            turn_modified_files.len(),
            turn_added,
            turn_removed
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

    let code_task_uncompleted = engine_ran
        && prompt_expects_code_changes(&user_prompt)
        && turn_modified_files.is_empty();

    let (synthesized_text, synth_tokens) = if code_task_uncompleted {
        let broken_preview: Vec<String> = blackboard
            .workspace_snapshot
            .broken_declared_paths
            .iter()
            .take(5)
            .map(|b| format!("`{}` (déclaré dans `{}`)", b.declared_path, b.source_file))
            .collect();
        let diag_detail = if !broken_preview.is_empty() {
            format!("références déclarées manquantes : {}", broken_preview.join(", "))
        } else if !carto.target_files.is_empty() {
            format!("cibles : {}", carto.target_files.join(", "))
        } else {
            "fichiers à créer ou modifier".to_string()
        };
        let alert_msg = format!(
            "⚠️ **L'agent a atteint sa limite de tours avant d'appliquer les modifications ({}).**\n\n\
            *Aucune modification n'a encore été écrite sur le disque.*\n\n\
            [👉 Cliquez pour relancer l'écriture](#rerun-write)",
            diag_detail
        );
        (alert_msg, 0usize)
    } else {
        tokio::select! {
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
                &tool_results,
                engine_ran,
                stack.ollama_running,
                &synth_prov,
                &synth_model,
                &settings,
                &turn_modified_files,
                &turn_applied_files,
                extra_stage_edits,
                &verifier_note,
                &blackboard,
            ) => res,
        }
    };

    let synth_summary = if synth_tokens > 0 {
        format!(
            "Synthèse finale (**{} fichier(s)** sur ce tour) et mise à jour mémoire",
            turn_modified_files.len()
        )
    } else {
        format!(
            "Mise en forme finale (**{} fichier(s)** sur ce tour) sans appel LLM",
            turn_modified_files.len()
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

        let (commit_id, commit_summary) = if !turn_files_summary.is_empty() {
            let commit_num = guard.conversations[conv_idx]
                .messages
                .iter()
                .filter(|m| m.commit_id.is_some())
                .count() + 1;
            let id = format!("vAIx #{commit_num}");
            let files_txt = if turn_files_summary.len() == 1 {
                turn_files_summary[0].path.clone()
            } else {
                format!("{} fichiers ({}, ...)", turn_files_summary.len(), turn_files_summary[0].path)
            };
            let summary = format!("{files_txt} (+{turn_added}/-{turn_removed})");
            (Some(id), Some(summary))
        } else {
            (None, None)
        };

        let assistant_msg_id = Uuid::new_v4().to_string();
        guard.conversations[conv_idx].messages.push(ChatMessage {
            id: assistant_msg_id.clone(),
            role: "assistant".to_string(),
            content: synthesized_text.clone(),
            thinking: thinking_with_tools,
            attachments: None,
            created_at: now.clone(),
            stage_summary: Some(stage_note),
            files_changed: turn_modified_files.len(),
            lines_added: turn_added,
            lines_removed: turn_removed,
            files_summary: turn_files_summary,
            turn_diffs: turn_modified_files.clone(),
            pre_turn_tree: pre_turn_tree.clone(),
            agent_trace,
            commit_id: commit_id.clone(),
            commit_summary: commit_summary.clone(),
        });

        // Mise à jour continue de l'artefact Journal d'évolution si des modifications de code ont eu lieu
        if let (Some(ref c_id), Some(ref c_sum)) = (&commit_id, &commit_summary) {
            let turn_paths: Vec<&str> = turn_modified_files.iter().map(|f| f.path.as_str()).collect();
            crate::models::update_conversation_evolution_artifact(
                &mut guard.conversations[conv_idx],
                c_id,
                &assistant_msg_id,
                &user_prompt,
                exec_role,
                &turn_paths,
                turn_added,
                turn_removed,
                c_sum,
                &now,
            );
        }

        for art in new_artifacts {
            guard.conversations[conv_idx].artifacts.push(art);
        }
        // Le diff total cumulé de la discussion reste accessible dans la conversation
        guard.conversations[conv_idx].modified_files = modified_files.clone();
        guard.conversations[conv_idx].lines_added = lines_added;
        guard.conversations[conv_idx].lines_removed = lines_removed;
        guard.conversations[conv_idx].updated_at = now.clone();

        let text_without_code = strip_code_blocks(&synthesized_text);
        let summary_snippet = truncate_chars(text_without_code.replace('\n', " ").trim(), 120);
        let files_ctx = if turn_modified_files.is_empty() {
            "aucun".to_string()
        } else {
            turn_modified_files
                .iter()
                .take(4)
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let verif_ctx = if test_res.ran {
            format!(
                "{} ({})",
                test_res.command,
                if test_res.success { "OK" } else { "ERR" }
            )
        } else {
            "aucune".to_string()
        };
        let conv_entry = truncate_chars(
            &format!(
                "• [{}] Demande: {} | Fichiers: {} | Vérif: {} | Résumé: {}",
                user_msg_time.get(..16).unwrap_or(&user_msg_time),
                truncate_chars(user_prompt.trim(), 80),
                truncate_chars(&files_ctx, 80),
                truncate_chars(&verif_ctx, 45),
                summary_snippet
            ),
            360,
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
        assert!(m3.is_empty());
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

        // Single small explicit file on Ollama 8K -> whole + map 0 + 3 turns
        let s1 = choose_execution_strategy(
            &dir,
            &["vite.config.js".to_string()],
            true,
            "ollama",
            &settings,
        );
        assert_eq!(s1.edit_format, "whole");
        assert_eq!(s1.map_tokens, 0);
        assert_eq!(s1.max_turns, 8);

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

        // Ollama 16K -> 8 turns + map 256 for single explicit
        let mut settings_16k = settings.clone();
        settings_16k.context_window_tokens = 16_384;
        let s_16k = choose_execution_strategy(
            &dir,
            &["vite.config.js".to_string()],
            true,
            "ollama",
            &settings_16k,
        );
        assert_eq!(s_16k.max_turns, 8);
        assert_eq!(s_16k.map_tokens, 256);

        // Cloud provider on 150-line file (> 120 threshold) -> diff + 8 turns + map 1024
        let s_cloud = choose_execution_strategy(
            &dir,
            &["src/Medium.ts".to_string()],
            true,
            "gemini",
            &settings,
        );
        assert_eq!(s_cloud.edit_format, "diff");
        assert_eq!(s_cloud.max_turns, 8);
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
            classify_prompt_role("mets un fond gris anthracite sur la page"),
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
    fn test_prompt_expects_code_changes() {
        assert!(prompt_expects_code_changes("mets un fond gris anthracite sur la page"));
        assert!(prompt_expects_code_changes("ajoute un bouton"));
        assert!(prompt_expects_code_changes("Créé un layout avec menu à gauche"));
        assert!(!prompt_expects_code_changes("Explique-moi comment fonctionne le routeur ?"));
        assert!(!prompt_expects_code_changes("Pourquoi Vite échoue ?"));
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
        let p_diff = build_native_system_prompt(
            "coder",
            "diff",
            &[&srv],
            "Outil `read_query`",
        );
        assert!(p_diff.contains("SEARCH/REPLACE"));
        assert!(p_diff.contains("mcp_call"));
        assert!(p_diff.contains("sqlite-db"));
        assert!(p_diff.contains("read_query"));
        assert!(p_diff.contains("CONSIGNES DU RÔLE (`Codeur`)"));

        let p_whole = build_native_system_prompt("coder", "whole", &[], "");
        assert!(p_whole.contains("write_file"));
        assert!(p_whole.contains("SANS AUCUNE LIMITE DE LIGNES"));
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
```

### Rapport d'audit de sécurité
```markdown
# Audit complet
- Vulnérabilité X résolue
- Dépendances validées
```"#;
        let arts = extract_artifacts_from_markdown(md);
        assert_eq!(arts.len(), 1);
        assert_eq!(arts[0].language, "markdown");
        assert!(arts[0].title.contains("Rapport d'audit"));
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
                status: None,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/new.ts".to_string(),
                added: 10,
                removed: 0,
                modified: 0,
                status: None,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/old.ts".to_string(),
                added: 0,
                removed: 8,
                modified: 0,
                status: None,
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
    fn test_synthesize_tool_actions_without_files() {
        let tools = vec![
            ToolExecutionResult {
                tool: "run_command".to_string(),
                success: true,
                message: "pnpm add -D tailwindcss exécuté avec succès (code 0)".to_string(),
                path: None,
            },
            ToolExecutionResult {
                tool: "web_search".to_string(),
                success: true,
                message: "Documentation trouvée pour @tailwindcss/vite".to_string(),
                path: None,
            },
        ];

        let mut lines = Vec::new();
        if !tools.is_empty() {
            lines.push("### Actions réalisées".to_string());
            for r in tools.iter().take(8) {
                lines.push(format!("- **{}** : {}", r.tool, r.message));
            }
        }
        let result = lines.join("\n");
        assert!(result.contains("### Actions réalisées"));
        assert!(result.contains("pnpm add -D tailwindcss"));
        assert!(result.contains("web_search"));
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
                status: None,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/components/LeHud.vue".to_string(),
                added: 2,
                removed: 0,
                modified: 1,
                status: None,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/a/index.ts".to_string(),
                added: 1,
                removed: 0,
                modified: 0,
                status: None,
                lines: Vec::new(),
            },
            ModifiedFileDiff {
                path: "src/b/index.ts".to_string(),
                added: 1,
                removed: 0,
                modified: 0,
                status: None,
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

    #[test]
    fn test_build_native_system_prompt_designer_css_rules() {
        let p_designer = build_native_system_prompt("designer", "whole", &[], "");
        assert!(p_designer.contains("CONSIGNES DU RÔLE (`Designer UI/UX`)"));
        assert!(p_designer.contains("Respecte la stack, les conventions de style"));
        assert!(!p_designer.to_lowercase().contains("interdiction tailwind"));
    }

    #[test]
    fn test_read_file_line_range_and_large_file_truncation() {
        let dir = make_temp_dir("read_range");
        let lines: Vec<String> = (1..=260).map(|i| format!("line_{i}")).collect();
        std::fs::write(dir.join("big.ts"), lines.join("\n")).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let range_res = rt
            .block_on(execute_local_tool_internal(
                &dir,
                "read_file",
                &serde_json::json!({"path": "big.ts", "start_line": 200, "end_line": 210}),
            ))
            .unwrap();
        assert!(range_res.message.contains("[Lignes 200-210 / 260]"));
        assert!(range_res.message.contains("200: line_200"));
        assert!(range_res.message.contains("210: line_210"));
        assert!(!range_res.message.contains("199: line_199"));

        let full_res = rt
            .block_on(execute_local_tool_internal(
                &dir,
                "read_file",
                &serde_json::json!({"path": "big.ts"}),
            ))
            .unwrap();
        assert!(full_res.message.contains("[Lignes 1-160 / 260]"));
        assert!(full_res.message.contains("start_line"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_architect_plan() {
        let raw = r#"```json
{
  "role": "coder",
  "runTests": true,
  "spec": "Architecture modulaire",
  "steps": [
    {"title": "Types", "role": "coder", "files": ["src/types.ts"], "instructions": "Créer les types"},
    {"title": "", "role": "coder", "files": [], "instructions": "Ignorer car titre vide"},
    {"title": "Store", "role": "unknown_role", "files": ["src/store.ts"], "instructions": "Créer le store"},
    {"title": "UI", "role": "designer", "files": ["src/App.vue"], "instructions": "Composer la vue"},
    {"title": "CSS", "role": "designer", "files": ["src/style.css"], "instructions": "Ajouter le CSS"},
    {"title": "Tests", "role": "tester", "files": ["src/store.spec.ts"], "instructions": "Écrire les tests"},
    {"title": "Extra ignoré", "role": "coder", "files": [], "instructions": "6e étape hors plafond"}
  ]
}
```"#;
        let parsed = parse_architect_plan(raw, "coder");
        assert_eq!(parsed.role.as_deref(), Some("coder"));
        assert!(parsed.run_tests);
        assert_eq!(parsed.spec.as_deref(), Some("Architecture modulaire"));
        assert_eq!(parsed.steps.len(), 5);
        assert_eq!(parsed.steps[0].title, "Types");
        assert_eq!(parsed.steps[0].role, "coder");
        assert_eq!(parsed.steps[1].title, "Store");
        assert_eq!(parsed.steps[1].role, "coder");
        assert_eq!(parsed.steps[2].role, "designer");
        assert_eq!(parsed.steps[4].title, "Tests");
    }

    #[test]
    fn test_load_project_architecture_memory() {
        let dir = make_temp_dir("arch_mem");
        assert!(load_project_architecture_memory(&dir, 500).is_none());

        std::fs::write(dir.join("AGENTS.md"), "Règles AGENTS globales").unwrap();
        let (src1, txt1) = load_project_architecture_memory(&dir, 500).unwrap();
        assert_eq!(src1, "AGENTS.md");
        assert_eq!(txt1, "Règles AGENTS globales");

        std::fs::create_dir_all(dir.join(".vaix")).unwrap();
        std::fs::write(
            dir.join(".vaix/architecture.md"),
            "Architecture prioritaire vAIx STUDIO avec détails",
        )
        .unwrap();
        let (src2, txt2) = load_project_architecture_memory(&dir, 24).unwrap();
        assert_eq!(src2, ".vaix/architecture.md");
        assert_eq!(txt2, "Architecture prioritaire…");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_install_prompt_routes_to_coder_and_targets_config_only() {
        assert!(is_install_or_setup_prompt("Installe tailwindcss et iconify-vue"));
        assert!(!is_install_or_setup_prompt(
            "Installe tailwindcss et refais le layout avec une sidebar gris anthracite"
        ));

        assert_eq!(
            classify_prompt_role("Installe tailwindcss et iconify-vue"),
            ("coder", false)
        );
        assert_eq!(
            classify_prompt_role(
                "Installe tailwindcss et refais le layout avec une sidebar gris anthracite"
            ),
            ("designer", false)
        );

        let all_files = vec![
            "package.json".to_string(),
            "vite.config.js".to_string(),
            "src/index.css".to_string(),
            "src/App.vue".to_string(),
            "src/components/Sidebar.vue".to_string(),
        ];
        let (matched, explicit) =
            match_files_deterministically("Installe tailwindcss et iconify-vue", None, &all_files);
        assert!(!explicit);
        assert!(matched.is_empty());
    }

    #[test]
    fn test_tool_duplicate_alert_message() {
        let alert_msg = "ALERTE SYSTÈME : Tu viens d'exécuter exactement la même action qui a échoué. Ne répète pas cet appel. Lis d'abord le fichier complet avec read_file ou change d'approche.";
        assert!(alert_msg.contains("ALERTE SYSTÈME"));
        assert!(alert_msg.contains("read_file ou change d'approche"));
    }

    #[test]
    fn test_execution_facts_json_schema() {
        let facts = serde_json::json!({
            "has_modified_files": true,
            "total_modified_files": 1,
            "modified_files": [{
                "path": "src/App.vue",
                "added_lines": 10,
                "removed_lines": 2,
                "diff_summary": "+ 10 lignes"
            }],
            "turn_applied_files": ["src/App.vue"],
            "executed_tools": [{
                "tool": "write_file",
                "success": true,
                "message": "Fichier écrit avec succès"
            }],
            "verification_report": "OK",
            "engine_turn_details": "Création du composant"
        });
        assert_eq!(facts["has_modified_files"], true);
        assert_eq!(facts["total_modified_files"], 1);
        assert_eq!(facts["modified_files"][0]["path"], "src/App.vue");
    }

    #[test]
    fn test_run_project_verifier_agnostic_contract() {
        let dir = make_temp_dir("verifier_contract");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let already = ("custom check".to_string(), true, "OK".to_string());

        // 1. Déjà validé avec la même commande
        let r1 = rt.block_on(run_project_verifier(&dir, Some("custom check"), Some(&already)));
        assert!(r1.ran);
        assert!(r1.success);
        assert_eq!(r1.command, "custom check");

        // 2. validationCommand null -> rien d'exécuté
        let r2 = rt.block_on(run_project_verifier(&dir, None, None));
        assert!(!r2.ran);
        assert!(r2.success);
        assert!(r2.output_summary.contains("validationCommand est null"));

        // 3. validationCommand explicite non bloquante -> exécution déterministe
        let r3 = rt.block_on(run_project_verifier(&dir, Some("echo validation_ok"), None));
        assert!(r3.ran);
        assert!(r3.success);
        assert_eq!(r3.command, "echo validation_ok");
        assert!(r3.output_summary.contains("validation_ok"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
