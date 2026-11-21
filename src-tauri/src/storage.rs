use crate::models::{
    ApiUsageState, AppSettings, AppStateData, ArtifactItem, Conversation, Project,
};
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
        global_context: "Stack : Tauri v2 + Rust + Vue 3 + TypeScript + Pinia.\nObjectif : IDE pair-programming doté d'un moteur natif Rust orchestrant Ollama en local et les fournisseurs cloud.".to_string(),
        shared_directories: Vec::new(),
        updated_at: now.clone(),
    };

    let conv = Conversation {
        id: conv_id.clone(),
        project_id: project_id.clone(),
        title: "Nouvelle session".to_string(),
        conversation_context: String::new(),
        messages: Vec::new(),
        artifacts: vec![crate::models::create_default_evolution_artifact(&now)],
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
        api_usage: ApiUsageState::default(),
    }
}

pub async fn load_state() -> AppStateData {
    let path = state_file_path();
    if let Ok(raw) = tokio::fs::read_to_string(&path).await {
        if let Ok(mut parsed) = serde_json::from_str::<AppStateData>(&raw) {
            if !parsed.projects.is_empty() {
                sanitize_conversations(&mut parsed);
                return parsed;
            }
        }
    }
    let seed = create_seed_state();
    let _ = save_state(&seed).await;
    seed
}

fn is_code_artifact(art: &ArtifactItem) -> bool {
    let has_code_ext = |s: &str| {
        let low = s.to_ascii_lowercase();
        low.ends_with(".vue")
            || low.ends_with(".ts")
            || low.ends_with(".tsx")
            || low.ends_with(".js")
            || low.ends_with(".jsx")
            || low.ends_with(".rs")
            || low.ends_with(".css")
            || low.ends_with(".scss")
            || low.ends_with(".html")
            || low.ends_with(".json")
            || low.ends_with(".py")
            || low.ends_with(".toml")
            || low.ends_with(".yaml")
            || low.ends_with(".yml")
            || low.ends_with(".sh")
            || low.ends_with(".bash")
            || low.ends_with(".sql")
    };

    if let Some(path) = art.file_path.as_deref() {
        if has_code_ext(path) {
            return true;
        }
    }
    if has_code_ext(&art.title) {
        return true;
    }
    matches!(
        art.language.to_ascii_lowercase().as_str(),
        "rust" | "rs" | "typescript" | "ts" | "javascript" | "js" | "vue" | "python" | "py" | "css" | "scss" | "html" | "json" | "sh" | "bash"
    )
}

pub fn sanitize_conversations(state: &mut AppStateData) {
    for conv in &mut state.conversations {
        // Supprimer définitivement les fichiers de code source enregistrés par erreur comme artefacts
        conv.artifacts.retain(|art| !is_code_artifact(art));
        crate::models::backfill_conversation_evolution_artifact(conv);
    }
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
