use serde::{Deserialize, Serialize};

fn default_context_window() -> usize {
    8192
}

fn default_edit_format() -> String {
    "auto".to_string()
}

fn default_active_target() -> String {
    "agent".to_string()
}

fn default_ollama_provider() -> String {
    "ollama".to_string()
}

fn default_daily_token_limit() -> usize {
    500_000
}

pub fn today_date_string() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub validated: bool,
    #[serde(default)]
    pub available_models: Vec<String>,
    #[serde(default = "default_daily_token_limit")]
    pub daily_token_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelSlot {
    #[serde(default = "default_ollama_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

impl Default for AgentModelSlot {
    fn default() -> Self {
        Self {
            provider: default_ollama_provider(),
            model: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentRoutingConfig {
    #[serde(default)]
    pub architect: AgentModelSlot,
    #[serde(default)]
    pub cartographer: AgentModelSlot,
    #[serde(default)]
    pub designer: AgentModelSlot,
    #[serde(default)]
    pub coder: AgentModelSlot,
    #[serde(default)]
    pub refactoring: AgentModelSlot,
    #[serde(default)]
    pub tester: AgentModelSlot,
    #[serde(default)]
    pub synthesizer: AgentModelSlot,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageEntry {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    pub calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiUsageState {
    #[serde(default = "today_date_string")]
    pub date: String,
    #[serde(default)]
    pub by_provider: std::collections::HashMap<String, ProviderUsageEntry>,
}

impl Default for ApiUsageState {
    fn default() -> Self {
        Self {
            date: today_date_string(),
            by_provider: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: String, // "system" | "dark" | "light"
    pub ollama_url: String,
    pub ollama_model: String,
    pub auto_start_ollama: bool,
    #[serde(default = "default_context_window")]
    pub context_window_tokens: usize,
    #[serde(default = "default_edit_format")]
    pub aider_edit_format: String, // "auto" | "whole" | "diff"
    #[serde(default = "default_active_target")]
    pub active_target: String, // "agent" | "<provider>:<model>"
    #[serde(default = "crate::providers::default_providers")]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub agent_routing: AgentRoutingConfig,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            ollama_url: "http://127.0.0.1:11434".to_string(),
            ollama_model: "qwen2.5-coder:3b".to_string(),
            auto_start_ollama: true,
            context_window_tokens: 8192,
            aider_edit_format: "auto".to_string(),
            active_target: "agent".to_string(),
            providers: crate::providers::default_providers(),
            agent_routing: AgentRoutingConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub global_context: String,
    #[serde(default)]
    pub shared_directories: Vec<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactItem {
    pub id: String,
    pub title: String,
    pub language: String,
    pub file_path: Option<String>,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: String, // "mod" | "add" | "del" | "ctx"
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifiedFileDiff {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeSummary {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedFile {
    pub id: String,
    pub name: String,
    pub size: usize,
    pub mime_type: String,
    pub is_image: bool,
    #[serde(default)]
    pub data_url: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionResult {
    pub tool: String,
    pub success: bool,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceStep {
    pub role: String,
    pub role_label: String,
    pub provider: String,
    pub model: String,
    pub summary: String,
    pub status: String, // "running" | "done" | "fallback" | "error"
    #[serde(default)]
    pub tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceEvent {
    pub conversation_id: String,
    pub step: AgentTraceStep,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: String, // "user" | "assistant" | "system"
    pub content: String,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub attachments: Option<Vec<AttachedFile>>,
    pub created_at: String,
    pub stage_summary: Option<String>,
    #[serde(default)]
    pub files_changed: usize,
    #[serde(default)]
    pub lines_added: usize,
    #[serde(default)]
    pub lines_removed: usize,
    #[serde(default)]
    pub files_summary: Vec<FileChangeSummary>,
    #[serde(default)]
    pub agent_trace: Vec<AgentTraceStep>,
    #[serde(default)]
    pub commit_id: Option<String>,
    #[serde(default)]
    pub commit_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub conversation_context: String,
    pub messages: Vec<ChatMessage>,
    pub artifacts: Vec<ArtifactItem>,
    pub modified_files: Vec<ModifiedFileDiff>,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub server_type: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    pub enabled: bool,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub server_id: String,
    pub server_name: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateData {
    pub settings: AppSettings,
    pub projects: Vec<Project>,
    pub conversations: Vec<Conversation>,
    pub active_project_id: Option<String>,
    pub active_conversation_id: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub api_usage: ApiUsageState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareTelemetry {
    pub cpu_usage: f32,
    pub gpu_usage: Option<f32>,
    pub mem_used_gb: f32,
    pub mem_total_gb: f32,
    pub mem_percent: f32,
    pub temp_celsius: Option<f32>,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageEvent {
    pub conversation_id: String,
    pub subagent: String,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub context_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStageEvent {
    pub conversation_id: String,
    pub stage_index: u8,
    pub stage_total: u8,
    pub stage_label: String,
    pub detail: String,
    pub status: String, // "running" | "done" | "error"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolApprovalRequestEvent {
    pub request_id: String,
    pub conversation_id: String,
    pub tool: String,
    pub title: String,
    pub target: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackStatus {
    pub native_engine_ready: bool,
    pub ollama_installed: bool,
    pub ollama_running: bool,
    pub model_available: bool,
    pub available_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendPromptResult {
    pub conversation: Conversation,
    pub project: Project,
}

pub fn create_default_evolution_artifact(now: &str) -> ArtifactItem {
    ArtifactItem {
        id: "evolution-journal".to_string(),
        title: "Journal d'évolution".to_string(),
        language: "markdown".to_string(),
        file_path: Some("CHANGELOG.md".to_string()),
        content: "# 🚀 Journal d'évolution du projet\n\n*Recensement chronologique des fonctionnalités et modifications de code de la conversation.*\n".to_string(),
        created_at: now.to_string(),
    }
}

pub fn classify_modification_emote(role: &str, user_prompt: &str, file_paths: &[&str]) -> &'static str {
    let p_low = user_prompt.to_lowercase();
    let has_style_ext = file_paths.iter().any(|p| {
        let low = p.to_lowercase();
        low.ends_with(".css") || low.ends_with(".scss") || low.ends_with(".sass") || low.ends_with(".less")
    });
    let is_style_prompt = p_low.contains("style")
        || p_low.contains("css")
        || p_low.contains("couleur")
        || p_low.contains("padding")
        || p_low.contains("margin")
        || p_low.contains("thème")
        || p_low.contains("theme")
        || p_low.contains("design")
        || p_low.contains("ui")
        || p_low.contains("halo")
        || p_low.contains("logo")
        || p_low.contains("palette")
        || p_low.contains("police")
        || p_low.contains("font")
        || p_low.contains("layout")
        || p_low.contains("visuel")
        || p_low.contains("bouton")
        || p_low.contains("bulle")
        || p_low.contains("animation");

    if role == "designer" || has_style_ext || is_style_prompt {
        "🎨"
    } else if role == "refactor"
        || role == "debugger"
        || file_paths.iter().any(|p| {
            let low = p.to_lowercase();
            low.ends_with(".json")
                || low.ends_with(".toml")
                || low.ends_with(".yaml")
                || low.ends_with(".yml")
                || low.ends_with(".sh")
                || low.ends_with("cargo.toml")
                || low.contains("config")
        })
        || p_low.contains("fix")
        || p_low.contains("bug")
        || p_low.contains("corrige")
        || p_low.contains("erreur")
        || p_low.contains("refactor")
        || p_low.contains("optim")
        || p_low.contains("perf")
        || p_low.contains("mcp")
        || p_low.contains("provider")
        || p_low.contains("api")
        || p_low.contains("config")
        || p_low.contains("setup")
        || p_low.contains("install")
        || p_low.contains("tool")
        || p_low.contains("test")
        || p_low.contains("outil")
        || p_low.contains("script")
        || p_low.contains("package")
    {
        "🛠️"
    } else {
        "📦"
    }
}

pub fn update_conversation_evolution_artifact(
    conversation: &mut Conversation,
    commit_id: &str,
    assistant_msg_id: &str,
    user_prompt: &str,
    exec_role: &str,
    file_paths: &[&str],
    turn_added: usize,
    turn_removed: usize,
    commit_summary: &str,
    now: &str,
) {
    let emote = classify_modification_emote(exec_role, user_prompt, file_paths);
    let raw_title = user_prompt.lines().next().unwrap_or("Nouvelle fonctionnalité").trim();
    let section_title = if raw_title.chars().count() > 75 {
        let s: String = raw_title.chars().take(72).collect();
        format!("{s}…")
    } else {
        raw_title.to_string()
    };

    let files_summary_str = if file_paths.is_empty() {
        String::new()
    } else if file_paths.len() == 1 {
        let base = file_paths[0].split(['/', '\\']).last().unwrap_or(file_paths[0]);
        format!("`{base}`")
    } else {
        let names: Vec<String> = file_paths
            .iter()
            .map(|p| {
                let base = p.split(['/', '\\']).last().unwrap_or(*p);
                format!("`{base}`")
            })
            .collect();
        names.join(", ")
    };

    let commit_line = format!(
        "- {} [{}](#msg-{}) {} : {} (+{}/-{})",
        emote, commit_id, assistant_msg_id, files_summary_str, commit_summary, turn_added, turn_removed
    );

    if let Some(art) = conversation
        .artifacts
        .iter_mut()
        .find(|a| a.id == "evolution-journal" || a.title == "Journal d'évolution")
    {
        // Éviter les doublons
        let link_marker = format!("[{commit_id}](#msg-{assistant_msg_id})");
        if art.content.contains(&link_marker) {
            return;
        }

        let section_header = format!("## {section_title}");
        if art.content.contains(&section_header) {
            if let Some(pos) = art.content.find(&section_header) {
                let rest_start = pos + section_header.len();
                if let Some(newline_offset) = art.content[rest_start..].find('\n') {
                    let insert_pos = rest_start + newline_offset + 1;
                    art.content.insert_str(insert_pos, &format!("{}\n", commit_line));
                } else {
                    art.content.push_str(&format!("\n{}\n", commit_line));
                }
            } else {
                art.content.push_str(&format!("\n{}\n", commit_line));
            }
        } else {
            art.content.push_str(&format!("\n\n## {}\n{}\n", section_title, commit_line));
        }
        art.created_at = now.to_string();
    } else {
        let initial_content = format!(
            "# 🚀 Journal d'évolution du projet\n\n*Recensement chronologique des fonctionnalités et modifications de code de la conversation.*\n\n## {}\n{}\n",
            section_title, commit_line
        );
        conversation.artifacts.push(ArtifactItem {
            id: "evolution-journal".to_string(),
            title: "Journal d'évolution".to_string(),
            language: "markdown".to_string(),
            file_path: Some("CHANGELOG.md".to_string()),
            content: initial_content,
            created_at: now.to_string(),
        });
    }
}

pub fn backfill_conversation_evolution_artifact(conversation: &mut Conversation) {
    let now = chrono::Utc::now().to_rfc3339();
    if !conversation
        .artifacts
        .iter()
        .any(|a| a.id == "evolution-journal" || a.title == "Journal d'évolution")
    {
        conversation.artifacts.push(create_default_evolution_artifact(&now));
    }

    let msgs = conversation.messages.clone();
    for (i, msg) in msgs.iter().enumerate() {
        if msg.role != "assistant" {
            continue;
        }
        if let Some(commit_id) = &msg.commit_id {
            let mut user_prompt = String::new();
            for prev in msgs[..i].iter().rev() {
                if prev.role == "user" {
                    user_prompt = prev.content.clone();
                    break;
                }
            }
            if user_prompt.is_empty() {
                user_prompt = "Modification de code".to_string();
            }

            let file_paths: Vec<&str> = msg.files_summary.iter().map(|f| f.path.as_str()).collect();
            let summary = msg.commit_summary.clone().unwrap_or_else(|| {
                msg.stage_summary.clone().unwrap_or_else(|| "Modifications appliquées".to_string())
            });

            update_conversation_evolution_artifact(
                conversation,
                commit_id,
                &msg.id,
                &user_prompt,
                "coder",
                &file_paths,
                msg.lines_added,
                msg.lines_removed,
                &summary,
                &msg.created_at,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_modification_emote() {
        assert_eq!(classify_modification_emote("designer", "change button background", &[]), "🎨");
        assert_eq!(classify_modification_emote("coder", "ajoute du padding et refais le style", &[]), "🎨");
        assert_eq!(classify_modification_emote("coder", "modifie styles", &["src/index.css"]), "🎨");
        assert_eq!(classify_modification_emote("coder", "corrige le bug du parser", &[]), "🛠️");
        assert_eq!(classify_modification_emote("coder", "met à jour la config cargo", &["Cargo.toml"]), "🛠️");
        assert_eq!(classify_modification_emote("coder", "ajoute une nouvelle page d'accueil", &["src/Home.vue"]), "📦");
    }

    #[test]
    fn test_evolution_artifact_creation_and_backfill() {
        let mut conv = Conversation {
            id: "conv-1".to_string(),
            project_id: "proj-1".to_string(),
            title: "Test".to_string(),
            conversation_context: String::new(),
            messages: vec![
                ChatMessage {
                    id: "u1".to_string(),
                    role: "user".to_string(),
                    content: "Ajoute le logo de l'app".to_string(),
                    thinking: None,
                    attachments: None,
                    created_at: "2026-11-20T20:00:00Z".to_string(),
                    stage_summary: None,
                    files_changed: 1,
                    lines_added: 12,
                    lines_removed: 0,
                    files_summary: vec![FileChangeSummary {
                        path: "src/Logo.vue".to_string(),
                        added: 12,
                        removed: 0,
                        modified: 0,
                    }],
                    agent_trace: vec![],
                    commit_id: None,
                    commit_summary: None,
                },
                ChatMessage {
                    id: "a1".to_string(),
                    role: "assistant".to_string(),
                    content: "Logo ajouté avec succès.".to_string(),
                    thinking: None,
                    attachments: None,
                    created_at: "2026-11-20T20:00:10Z".to_string(),
                    stage_summary: Some("Ajout du composant Logo.vue".to_string()),
                    files_changed: 1,
                    lines_added: 12,
                    lines_removed: 0,
                    files_summary: vec![FileChangeSummary {
                        path: "src/Logo.vue".to_string(),
                        added: 12,
                        removed: 0,
                        modified: 0,
                    }],
                    agent_trace: vec![],
                    commit_id: Some("vAIx #1".to_string()),
                    commit_summary: Some("Ajout composant Logo".to_string()),
                },
            ],
            artifacts: vec![],
            modified_files: vec![],
            lines_added: 12,
            lines_removed: 0,
            created_at: "2026-11-20T20:00:00Z".to_string(),
            updated_at: "2026-11-20T20:00:10Z".to_string(),
        };

        backfill_conversation_evolution_artifact(&mut conv);

        assert_eq!(conv.artifacts.len(), 1);
        let art = &conv.artifacts[0];
        assert_eq!(art.id, "evolution-journal");
        assert_eq!(art.language, "markdown");
        assert!(art.content.contains("## Ajoute le logo de l'app"));
        assert!(art.content.contains("- 🎨 [vAIx #1](#msg-a1) `Logo.vue` : Ajout composant Logo (+12/-0)"));
    }
}
