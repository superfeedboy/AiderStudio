use crate::models::{AppSettings, AppStateData, Conversation, Project};
use chrono::Utc;
use std::path::PathBuf;
use uuid::Uuid;

pub fn state_file_path() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("AiderStudio").join("state.json")
}

pub fn default_workspace_path() -> String {
    if let Ok(cwd) = std::env::current_dir() {
        // When running `cargo tauri dev` inside src-tauri, move up to workspace root
        if cwd.ends_with("src-tauri") {
            if let Some(parent) = cwd.parent() {
                return parent.to_string_lossy().to_string();
            }
        }
        return cwd.to_string_lossy().to_string();
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .to_string_lossy()
        .to_string()
}

pub fn create_seed_state() -> AppStateData {
    let now = Utc::now().to_rfc3339();
    let project_id = Uuid::new_v4().to_string();
    let conv_id = Uuid::new_v4().to_string();

    let project = Project {
        id: project_id.clone(),
        name: "AiderStudio".to_string(),
        root_path: default_workspace_path(),
        global_context: "Stack : Tauri v2 + Rust + Vue 3 + TypeScript + Pinia.\nObjectif : IDE pair-programming pilotant Aider avec Ollama (qwen2.5-coder:3b).".to_string(),
        shared_directories: Vec::new(),
        updated_at: now.clone(),
    };

    let conv = Conversation {
        id: conv_id.clone(),
        project_id: project_id.clone(),
        title: "Nouvelle session".to_string(),
        conversation_context: String::new(),
        messages: Vec::new(),
        artifacts: Vec::new(),
        modified_files: Vec::new(),
        lines_added: 0,
        lines_removed: 0,
        created_at: now.clone(),
        updated_at: now,
    };

    AppStateData {
        settings: AppSettings::default(),
        projects: vec![project],
        conversations: vec![conv],
        active_project_id: Some(project_id),
        active_conversation_id: Some(conv_id),
        mcp_servers: Vec::new(),
    }
}

pub async fn load_state() -> AppStateData {
    let path = state_file_path();
    if let Ok(raw) = tokio::fs::read_to_string(&path).await {
        if let Ok(parsed) = serde_json::from_str::<AppStateData>(&raw) {
            if !parsed.projects.is_empty() {
                return parsed;
            }
        }
    }
    let seed = create_seed_state();
    let _ = save_state(&seed).await;
    seed
}

pub async fn save_state(state: &AppStateData) -> anyhow::Result<()> {
    let path = state_file_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let serialized = serde_json::to_string_pretty(state)?;
    tokio::fs::write(&path, serialized).await?;
    Ok(())
}
