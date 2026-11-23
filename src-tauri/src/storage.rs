use crate::models::{
    ApiUsageState, AppSettings, AppStateData, ArtifactItem, Conversation, McpServerConfig, Project,
    ProviderConfig,
};
use chrono::Utc;
use std::path::{Path, PathBuf};

pub const PRIMARY_APP_DATA_DIR: &str = "vAIx STUDIO";
const LEGACY_AIDERSTUDIO_DIR: &str = "AiderStudio";
#[cfg(not(target_os = "windows"))]
const LEGACY_VAIX_BRACKET_DIR: &str = ">AIx Studio";

pub fn state_file_path_in(base: &Path) -> PathBuf {
    base.join(PRIMARY_APP_DATA_DIR).join("state.json")
}

pub fn state_file_path() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    state_file_path_in(&base)
}

fn legacy_state_file_paths_in(base: &Path) -> Vec<PathBuf> {
    let mut paths = vec![base.join(LEGACY_AIDERSTUDIO_DIR).join("state.json")];
    #[cfg(not(target_os = "windows"))]
    {
        paths.push(base.join(LEGACY_VAIX_BRACKET_DIR).join("state.json"));
    }
    paths
}

pub fn create_empty_state() -> AppStateData {
    let mut settings = AppSettings::default();
    settings.sanitize();
    AppStateData {
        settings,
        projects: Vec::new(),
        conversations: Vec::new(),
        active_project_id: None,
        active_conversation_id: None,
        mcp_servers: Vec::new(),
        api_usage: ApiUsageState::default(),
    }
}

pub fn sanitize_state(state: &mut AppStateData) {
    state.settings.sanitize();
    sanitize_conversations(state);

    let active_proj_valid = state
        .active_project_id
        .as_deref()
        .is_some_and(|pid| state.projects.iter().any(|p| p.id == pid));

    if !active_proj_valid {
        state.active_project_id = state.projects.first().map(|p| p.id.clone());
    }

    if let Some(ref active_pid) = state.active_project_id {
        let active_conv_valid = state.active_conversation_id.as_deref().is_some_and(|cid| {
            state
                .conversations
                .iter()
                .any(|c| c.id == cid && c.project_id == *active_pid)
        });
        if !active_conv_valid {
            state.active_conversation_id = state
                .conversations
                .iter()
                .find(|c| c.project_id == *active_pid)
                .map(|c| c.id.clone());
        }
    } else {
        state.active_conversation_id = None;
    }
}

fn parse_state_resilient(raw: &str) -> Option<AppStateData> {
    if let Ok(parsed) = serde_json::from_str::<AppStateData>(raw) {
        return Some(parsed);
    }

    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = value.as_object()?;
    let has_known_key = ["projects", "settings", "conversations", "mcpServers"]
        .iter()
        .any(|k| obj.contains_key(*k));
    if !has_known_key {
        return None;
    }

    let mut recovered = create_empty_state();

    if let Some(settings_val) = obj.get("settings") {
        if let Ok(parsed_settings) = serde_json::from_value::<AppSettings>(settings_val.clone()) {
            recovered.settings = parsed_settings;
        } else if let Some(so) = settings_val.as_object() {
            if let Some(theme) = so.get("theme").and_then(|v| v.as_str()) {
                recovered.settings.theme = theme.to_string();
            }
            if let Some(url) = so.get("ollamaUrl").and_then(|v| v.as_str()) {
                recovered.settings.ollama_url = url.to_string();
            }
            if let Some(model) = so.get("ollamaModel").and_then(|v| v.as_str()) {
                recovered.settings.ollama_model = model.to_string();
            }
            if let Some(providers_arr) = so.get("providers").and_then(|v| v.as_array()) {
                let valid_providers: Vec<ProviderConfig> = providers_arr
                    .iter()
                    .filter_map(|item| serde_json::from_value::<ProviderConfig>(item.clone()).ok())
                    .collect();
                if !valid_providers.is_empty() {
                    recovered.settings.providers = valid_providers;
                }
            }
        }
    }

    if let Some(arr) = obj.get("projects").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(proj) = serde_json::from_value::<Project>(item.clone()) {
                recovered.projects.push(proj);
            }
        }
    }

    if let Some(arr) = obj.get("conversations").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(conv) = serde_json::from_value::<Conversation>(item.clone()) {
                recovered.conversations.push(conv);
            }
        }
    }

    if let Some(arr) = obj.get("mcpServers").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(srv) = serde_json::from_value::<McpServerConfig>(item.clone()) {
                recovered.mcp_servers.push(srv);
            }
        }
    }

    if let Some(pid) = obj.get("activeProjectId").and_then(|v| v.as_str()) {
        recovered.active_project_id = Some(pid.to_string());
    }

    if let Some(cid) = obj.get("activeConversationId").and_then(|v| v.as_str()) {
        recovered.active_conversation_id = Some(cid.to_string());
    }

    if let Some(usage_val) = obj.get("apiUsage") {
        if let Ok(usage) = serde_json::from_value::<ApiUsageState>(usage_val.clone()) {
            recovered.api_usage = usage;
        }
    }

    Some(recovered)
}

async fn backup_corrupted_state_file(path: &Path) {
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3f");
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state.json");
    let backup_path = path.with_file_name(format!("{file_name}.corrupt-{stamp}.bak"));
    let _ = tokio::fs::copy(path, &backup_path).await;
}

pub async fn load_state_in(base: &Path) -> AppStateData {
    let primary = state_file_path_in(base);

    match tokio::fs::read_to_string(&primary).await {
        Ok(raw) => {
            if let Some(mut parsed) = parse_state_resilient(&raw) {
                sanitize_state(&mut parsed);
                return parsed;
            }
            backup_corrupted_state_file(&primary).await;
        }
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            // Do not overwrite an existing state file that failed to read due to an I/O error
            return create_empty_state();
        }
        Err(_) => {}
    }

    for legacy_path in legacy_state_file_paths_in(base) {
        if let Ok(raw) = tokio::fs::read_to_string(&legacy_path).await {
            if let Some(mut parsed) = parse_state_resilient(&raw) {
                sanitize_state(&mut parsed);
                let _ = save_state_to_path(&primary, &parsed).await;
                return parsed;
            }
        }
    }

    let mut empty = create_empty_state();
    sanitize_state(&mut empty);
    let _ = save_state_to_path(&primary, &empty).await;
    empty
}

pub async fn load_state() -> AppStateData {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    load_state_in(&base).await
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

pub async fn save_state_to_path(path: &Path, state: &AppStateData) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let serialized = serde_json::to_string_pretty(state)?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state.json");
    let tmp_path = path.with_file_name(format!("{file_name}.tmp"));

    if tokio::fs::write(&tmp_path, serialized.as_bytes()).await.is_ok() {
        if tokio::fs::rename(&tmp_path, path).await.is_ok() {
            return Ok(());
        }
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }

    tokio::fs::write(path, serialized.as_bytes()).await?;
    Ok(())
}

pub async fn save_state(state: &AppStateData) -> anyhow::Result<()> {
    let path = state_file_path();
    save_state_to_path(&path, state).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "vaix_storage_test_{}_{}",
                label,
                Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[tokio::test]
    async fn first_install_starts_with_zero_projects_and_creates_empty_state() {
        let dir = TempDirGuard::new("first_install");
        let loaded = load_state_in(&dir.path).await;

        assert!(loaded.projects.is_empty());
        assert!(loaded.conversations.is_empty());
        assert_eq!(loaded.active_project_id, None);
        assert_eq!(loaded.active_conversation_id, None);

        let primary = state_file_path_in(&dir.path);
        assert!(primary.exists());
        let raw = std::fs::read_to_string(&primary).unwrap();
        let persisted: AppStateData = serde_json::from_str(&raw).unwrap();
        assert!(persisted.projects.is_empty());
        assert!(persisted.conversations.is_empty());
    }

    #[tokio::test]
    async fn reload_with_zero_projects_preserves_settings_and_mcp_without_seeding() {
        let dir = TempDirGuard::new("zero_projects_settings");
        let primary = state_file_path_in(&dir.path);

        let mut initial = create_empty_state();
        initial.settings.theme = "dark".to_string();
        initial.settings.ollama_model = "qwen2.5-coder:7b".to_string();
        if let Some(gem) = initial.settings.providers.iter_mut().find(|p| p.id == "gemini") {
            gem.api_key = "test-api-key".to_string();
            gem.validated = true;
        }
        initial.mcp_servers.push(McpServerConfig {
            id: "mcp-1".to_string(),
            name: "SQLite".to_string(),
            server_type: Some("sqlite".to_string()),
            command: "npx".to_string(),
            args: vec!["--db".to_string()],
            env: std::collections::HashMap::new(),
            enabled: true,
            project_id: None,
        });
        save_state_to_path(&primary, &initial).await.unwrap();

        let reloaded = load_state_in(&dir.path).await;
        assert!(reloaded.projects.is_empty());
        assert!(reloaded.conversations.is_empty());
        assert_eq!(reloaded.active_project_id, None);
        assert_eq!(reloaded.active_conversation_id, None);
        assert_eq!(reloaded.settings.theme, "dark");
        assert_eq!(reloaded.settings.ollama_model, "qwen2.5-coder:7b");
        assert_eq!(
            reloaded
                .settings
                .providers
                .iter()
                .find(|p| p.id == "gemini")
                .map(|p| p.api_key.as_str()),
            Some("test-api-key")
        );
        assert_eq!(reloaded.mcp_servers.len(), 1);
        assert_eq!(reloaded.mcp_servers[0].name, "SQLite");
    }

    #[tokio::test]
    async fn update_migrates_legacy_aiderstudio_state_including_v02_v03_fields() {
        let dir = TempDirGuard::new("migrate_legacy");
        let legacy_path = dir.path.join(LEGACY_AIDERSTUDIO_DIR).join("state.json");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();

        let legacy_json = r#"{
            "settings": {
                "theme": "light",
                "ollamaUrl": "http://127.0.0.1:11434",
                "ollamaModel": "qwen2.5-coder:7b",
                "aiderPath": "/usr/local/bin/aider",
                "autoStartOllama": false,
                "contextWindowTokens": 16384,
                "aiderEditFormat": "diff",
                "activeTarget": "gemini:gemini-2.5-pro",
                "providers": [
                    {
                        "id": "gemini",
                        "name": "Gemini",
                        "apiKey": "legacy-gem-key",
                        "validated": true,
                        "availableModels": ["gemini-2.5-pro"]
                    }
                ],
                "agentRouting": {
                    "architect": { "provider": "gemini", "model": "gemini-2.5-pro" },
                    "designer": { "provider": "openai", "model": "gpt-4.1" },
                    "tester": { "provider": "anthropic", "model": "claude-sonnet-4" },
                    "cartographer": { "provider": "ollama", "model": "qwen2.5-coder:3b" }
                }
            },
            "projects": [
                {
                    "id": "proj-1",
                    "name": "MonAppClient",
                    "rootPath": "/tmp/mon-app",
                    "globalContext": "Contexte legacy",
                    "updatedAt": "2026-11-18T10:00:00Z"
                }
            ],
            "conversations": [
                {
                    "id": "conv-1",
                    "projectId": "proj-1",
                    "title": "Session existante",
                    "conversationContext": "",
                    "messages": [],
                    "artifacts": [],
                    "modifiedFiles": [],
                    "linesAdded": 0,
                    "linesRemoved": 0,
                    "createdAt": "2026-11-18T10:00:00Z",
                    "updatedAt": "2026-11-18T10:00:00Z"
                }
            ],
            "activeProjectId": "proj-1",
            "activeConversationId": "conv-1",
            "mcpServers": []
        }"#;
        std::fs::write(&legacy_path, legacy_json).unwrap();

        let migrated = load_state_in(&dir.path).await;
        assert_eq!(migrated.projects.len(), 1);
        assert_eq!(migrated.projects[0].id, "proj-1");
        assert_eq!(migrated.projects[0].name, "MonAppClient");
        assert_eq!(migrated.conversations.len(), 1);
        assert_eq!(migrated.conversations[0].id, "conv-1");
        assert_eq!(migrated.active_project_id.as_deref(), Some("proj-1"));
        assert_eq!(migrated.active_conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(migrated.settings.theme, "light");
        assert_eq!(migrated.settings.agent_routing.coder.provider, "openai");
        assert_eq!(migrated.settings.agent_routing.coder.model, "gpt-4.1");
        assert_eq!(migrated.settings.agent_routing.reviewer.provider, "anthropic");
        assert_eq!(
            migrated
                .settings
                .providers
                .iter()
                .find(|p| p.id == "gemini")
                .map(|p| p.api_key.as_str()),
            Some("legacy-gem-key")
        );

        let primary = state_file_path_in(&dir.path);
        assert!(primary.exists());
        assert!(legacy_path.exists());

        // Subsequent loads read directly from primary
        std::fs::write(&legacy_path, "{}").unwrap();
        let second_load = load_state_in(&dir.path).await;
        assert_eq!(second_load.projects.len(), 1);
        assert_eq!(second_load.projects[0].name, "MonAppClient");
    }

    #[tokio::test]
    async fn deleting_all_projects_after_migration_does_not_reimport_legacy() {
        let dir = TempDirGuard::new("no_reimport_after_delete");
        let legacy_path = dir.path.join(LEGACY_AIDERSTUDIO_DIR).join("state.json");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();

        let legacy_json = r#"{
            "projects": [
                { "id": "p-old", "name": "OldProj", "rootPath": "/tmp/old" }
            ],
            "conversations": [
                { "id": "c-old", "projectId": "p-old", "title": "OldConv" }
            ],
            "activeProjectId": "p-old",
            "activeConversationId": "c-old"
        }"#;
        std::fs::write(&legacy_path, legacy_json).unwrap();

        let first = load_state_in(&dir.path).await;
        assert_eq!(first.projects.len(), 1);

        // User deletes all projects in the updated app
        let mut emptied = first.clone();
        emptied.projects.clear();
        emptied.conversations.clear();
        emptied.active_project_id = None;
        emptied.active_conversation_id = None;
        save_state_to_path(&state_file_path_in(&dir.path), &emptied)
            .await
            .unwrap();

        let after_restart = load_state_in(&dir.path).await;
        assert!(after_restart.projects.is_empty());
        assert!(after_restart.conversations.is_empty());
        assert_eq!(after_restart.active_project_id, None);
        assert_eq!(after_restart.active_conversation_id, None);
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn legacy_aiderstudio_takes_priority_over_bracket_dir() {
        let dir = TempDirGuard::new("legacy_priority");
        let aider_path = dir.path.join(LEGACY_AIDERSTUDIO_DIR).join("state.json");
        let bracket_path = dir.path.join(LEGACY_VAIX_BRACKET_DIR).join("state.json");
        std::fs::create_dir_all(aider_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(bracket_path.parent().unwrap()).unwrap();

        std::fs::write(
            &aider_path,
            r#"{"projects":[{"id":"p-aider","name":"FromAiderStudio","rootPath":"/tmp/a"}]}"#,
        )
        .unwrap();
        std::fs::write(
            &bracket_path,
            r#"{"projects":[{"id":"p-bracket","name":"FromBracket","rootPath":"/tmp/b"}]}"#,
        )
        .unwrap();

        let loaded = load_state_in(&dir.path).await;
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].name, "FromAiderStudio");
    }

    #[tokio::test]
    async fn corrupted_primary_creates_backup_and_recovers_or_uses_legacy() {
        let dir = TempDirGuard::new("corrupt_backup");
        let primary = state_file_path_in(&dir.path);
        std::fs::create_dir_all(primary.parent().unwrap()).unwrap();
        std::fs::write(&primary, "{not valid json!!!").unwrap();

        let legacy_path = dir.path.join(LEGACY_AIDERSTUDIO_DIR).join("state.json");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy_path,
            r#"{"projects":[{"id":"p-rec","name":"RecoveredFromLegacy","rootPath":"/tmp/r"}]}"#,
        )
        .unwrap();

        let loaded = load_state_in(&dir.path).await;
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].name, "RecoveredFromLegacy");

        let has_backup = std::fs::read_dir(primary.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with("state.json.corrupt-") && name.ends_with(".bak")
            });
        assert!(has_backup);

        // Partial best-effort recovery when one project item is malformed
        let partial_json = r#"{
            "projects": [
                { "broken": true },
                { "id": "p-ok", "name": "ValidProj", "rootPath": "/tmp/ok" }
            ],
            "activeProjectId": "p-ok"
        }"#;
        std::fs::write(&primary, partial_json).unwrap();
        let partial_loaded = load_state_in(&dir.path).await;
        assert_eq!(partial_loaded.projects.len(), 1);
        assert_eq!(partial_loaded.projects[0].name, "ValidProj");
    }

    #[test]
    fn primary_app_data_dir_is_valid_across_windows_and_posix() {
        let forbidden = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
        assert!(!PRIMARY_APP_DATA_DIR.is_empty());
        assert!(!PRIMARY_APP_DATA_DIR.chars().any(|c| forbidden.contains(&c)));
    }
}
