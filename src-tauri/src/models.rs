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
    pub aider_path: String,
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
            aider_path: "aider".to_string(),
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
    pub agent_trace: Vec<AgentTraceStep>,
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
pub struct StackStatus {
    pub aider_installed: bool,
    pub aider_bin: Option<String>,
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
