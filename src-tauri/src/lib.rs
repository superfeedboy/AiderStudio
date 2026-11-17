mod engine;
mod git_diff;
mod mcp;
mod models;
mod storage;
mod telemetry;

use chrono::Utc;
use models::{
    AppSettings, AppStateData, Conversation, HardwareTelemetry, Project, SendPromptResult,
    StackStatus,
};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct ManagedState(pub Arc<Mutex<AppStateData>>);

#[tauri::command]
async fn get_initial_state(state: State<'_, ManagedState>) -> Result<AppStateData, String> {
    let guard = state.0.lock().await;
    Ok(guard.clone())
}

#[tauri::command]
async fn save_settings(
    state: State<'_, ManagedState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let mut guard = state.0.lock().await;
    guard.settings = settings.clone();
    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
async fn create_project(
    state: State<'_, ManagedState>,
    name: String,
    root_path: String,
    global_context: String,
) -> Result<AppStateData, String> {
    let mut guard = state.0.lock().await;
    let now = Utc::now().to_rfc3339();
    let project_id = Uuid::new_v4().to_string();
    let conv_id = Uuid::new_v4().to_string();

    let project = Project {
        id: project_id.clone(),
        name: name.trim().to_string(),
        root_path: root_path.trim().to_string(),
        global_context,
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

    guard.projects.insert(0, project);
    guard.conversations.insert(0, conv);
    guard.active_project_id = Some(project_id);
    guard.active_conversation_id = Some(conv_id);

    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(guard.clone())
}

#[tauri::command]
async fn update_project_context(
    state: State<'_, ManagedState>,
    project_id: String,
    global_context: String,
) -> Result<Project, String> {
    let mut guard = state.0.lock().await;
    let idx = guard
        .projects
        .iter()
        .position(|p| p.id == project_id)
        .ok_or_else(|| "Projet introuvable".to_string())?;
    guard.projects[idx].global_context = global_context;
    guard.projects[idx].updated_at = Utc::now().to_rfc3339();
    let updated = guard.projects[idx].clone();
    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

#[tauri::command]
async fn update_project(
    state: State<'_, ManagedState>,
    project_id: String,
    name: String,
    global_context: String,
    shared_directories: Vec<String>,
) -> Result<AppStateData, String> {
    let mut guard = state.0.lock().await;
    let idx = guard
        .projects
        .iter()
        .position(|p| p.id == project_id)
        .ok_or_else(|| "Projet introuvable".to_string())?;

    let trimmed = name.trim();
    if !trimmed.is_empty() {
        guard.projects[idx].name = trimmed.to_string();
    }
    guard.projects[idx].global_context = global_context;
    guard.projects[idx].shared_directories = shared_directories;
    guard.projects[idx].updated_at = Utc::now().to_rfc3339();

    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(guard.clone())
}

#[tauri::command]
async fn create_conversation(
    state: State<'_, ManagedState>,
    project_id: String,
    title: Option<String>,
) -> Result<AppStateData, String> {
    let mut guard = state.0.lock().await;
    let now = Utc::now().to_rfc3339();
    let conv_id = Uuid::new_v4().to_string();

    let conv = Conversation {
        id: conv_id.clone(),
        project_id: project_id.clone(),
        title: title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Nouvelle session".to_string()),
        conversation_context: String::new(),
        messages: Vec::new(),
        artifacts: Vec::new(),
        modified_files: Vec::new(),
        lines_added: 0,
        lines_removed: 0,
        created_at: now.clone(),
        updated_at: now,
    };

    guard.conversations.insert(0, conv);
    guard.active_project_id = Some(project_id);
    guard.active_conversation_id = Some(conv_id);

    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(guard.clone())
}

#[tauri::command]
async fn select_conversation(
    state: State<'_, ManagedState>,
    project_id: String,
    conversation_id: String,
) -> Result<(), String> {
    let mut guard = state.0.lock().await;
    guard.active_project_id = Some(project_id);
    guard.active_conversation_id = Some(conversation_id);
    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn delete_project(
    state: State<'_, ManagedState>,
    project_id: String,
) -> Result<AppStateData, String> {
    let mut guard = state.0.lock().await;
    guard.projects.retain(|p| p.id != project_id);
    guard.conversations.retain(|c| c.project_id != project_id);
    if guard.active_project_id.as_deref() == Some(&project_id) {
        guard.active_project_id = guard.projects.first().map(|p| p.id.clone());
        guard.active_conversation_id = guard.conversations.first().map(|c| c.id.clone());
    }
    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(guard.clone())
}

#[tauri::command]
async fn delete_conversation(
    state: State<'_, ManagedState>,
    conversation_id: String,
) -> Result<AppStateData, String> {
    let mut guard = state.0.lock().await;
    guard.conversations.retain(|c| c.id != conversation_id);
    if guard.active_conversation_id.as_deref() == Some(&conversation_id) {
        guard.active_conversation_id = guard.conversations.first().map(|c| c.id.clone());
    }
    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(guard.clone())
}

#[tauri::command]
async fn clear_conversation_context(
    state: State<'_, ManagedState>,
    conversation_id: String,
) -> Result<(), String> {
    let mut guard = state.0.lock().await;
    if let Some(conv) = guard.conversations.iter_mut().find(|c| c.id == conversation_id) {
        conv.conversation_context.clear();
        storage::save_state(&guard)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn send_prompt(
    app: AppHandle,
    state: State<'_, ManagedState>,
    conversation_id: String,
    prompt: String,
    attachments: Option<Vec<models::AttachedFile>>,
) -> Result<SendPromptResult, String> {
    engine::run_prompt_pipeline(&app, &state.0, conversation_id, prompt, attachments)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_project_files(
    state: State<'_, ManagedState>,
    project_id: Option<String>,
) -> Result<Vec<String>, String> {
    let root_path = {
        let guard = state.0.lock().await;
        let pid = project_id
            .or_else(|| guard.active_project_id.clone())
            .ok_or_else(|| "Aucun projet actif".to_string())?;
        let proj = guard
            .projects
            .iter()
            .find(|p| p.id == pid)
            .ok_or_else(|| "Projet introuvable".to_string())?;
        proj.root_path.clone()
    };
    Ok(engine::list_project_files_internal(std::path::Path::new(&root_path)))
}

#[tauri::command]
async fn execute_local_tool(
    state: State<'_, ManagedState>,
    project_id: Option<String>,
    tool_name: String,
    arguments: serde_json::Value,
) -> Result<models::ToolExecutionResult, String> {
    let root_path = {
        let guard = state.0.lock().await;
        let pid = project_id
            .or_else(|| guard.active_project_id.clone())
            .ok_or_else(|| "Aucun projet actif".to_string())?;
        let proj = guard
            .projects
            .iter()
            .find(|p| p.id == pid)
            .ok_or_else(|| "Projet introuvable".to_string())?;
        proj.root_path.clone()
    };
    engine::execute_local_tool_internal(
        std::path::Path::new(&root_path),
        &tool_name,
        &arguments,
    )
    .await
}

#[tauri::command]
async fn refresh_project_diff(
    state: State<'_, ManagedState>,
    conversation_id: String,
) -> Result<Conversation, String> {
    let project_root = {
        let guard = state.0.lock().await;
        let conv = guard
            .conversations
            .iter()
            .find(|c| c.id == conversation_id)
            .ok_or_else(|| "Conversation introuvable".to_string())?;
        let proj = guard
            .projects
            .iter()
            .find(|p| p.id == conv.project_id)
            .ok_or_else(|| "Projet introuvable".to_string())?;
        proj.root_path.clone()
    };

    let (modified_files, lines_added, lines_removed) =
        git_diff::collect_project_diffs(&project_root).await;

    let mut guard = state.0.lock().await;
    let idx = guard
        .conversations
        .iter()
        .position(|c| c.id == conversation_id)
        .ok_or_else(|| "Conversation introuvable".to_string())?;
    guard.conversations[idx].modified_files = modified_files;
    guard.conversations[idx].lines_added = lines_added;
    guard.conversations[idx].lines_removed = lines_removed;
    let updated = guard.conversations[idx].clone();
    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

#[tauri::command]
async fn get_hardware_telemetry() -> HardwareTelemetry {
    telemetry::collect_hardware_telemetry().await
}

#[tauri::command]
async fn check_stack_status(state: State<'_, ManagedState>) -> Result<StackStatus, String> {
    let settings = {
        let guard = state.0.lock().await;
        guard.settings.clone()
    };
    Ok(engine::check_stack_status(&settings).await)
}

#[tauri::command]
async fn start_ollama_service(state: State<'_, ManagedState>) -> Result<StackStatus, String> {
    let settings = {
        let guard = state.0.lock().await;
        guard.settings.clone()
    };
    engine::start_ollama_service(&settings)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn pick_project_directory() -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let output = tokio::process::Command::new("osascript")
            .args([
                "-e",
                "POSIX path of (choose folder with prompt \"Sélectionnez le répertoire du projet\")",
            ])
            .output()
            .await
            .map_err(|e| e.to_string())?;

        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let cleaned = if raw.len() > 1 {
                raw.trim_end_matches('/').to_string()
            } else {
                raw
            };
            if cleaned.is_empty() {
                return Ok(None);
            }
            return Ok(Some(cleaned));
        }
        return Ok(None);
    }

    #[cfg(target_os = "windows")]
    {
        let script = "Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.FolderBrowserDialog; $f.Description = 'Sélectionnez le répertoire du projet'; $f.UseDescriptionForTitle = $true; if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $f.SelectedPath }";
        let output = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-STA", "-Command", script])
            .output()
            .await
            .map_err(|e| e.to_string())?;

        if output.status.success() {
            let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !selected.is_empty() {
                return Ok(Some(selected));
            }
        }
        return Ok(None);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(None)
    }
}

#[tauri::command]
async fn get_mcp_servers(
    state: State<'_, ManagedState>,
) -> Result<Vec<models::McpServerConfig>, String> {
    let guard = state.0.lock().await;
    Ok(guard.mcp_servers.clone())
}

#[tauri::command]
async fn save_mcp_server(
    state: State<'_, ManagedState>,
    server: models::McpServerConfig,
) -> Result<Vec<models::McpServerConfig>, String> {
    let mut guard = state.0.lock().await;
    if let Some(pos) = guard.mcp_servers.iter().position(|s| s.id == server.id) {
        guard.mcp_servers[pos] = server;
    } else {
        guard.mcp_servers.push(server);
    }
    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(guard.mcp_servers.clone())
}

#[tauri::command]
async fn delete_mcp_server(
    state: State<'_, ManagedState>,
    server_id: String,
) -> Result<Vec<models::McpServerConfig>, String> {
    let mut guard = state.0.lock().await;
    guard.mcp_servers.retain(|s| s.id != server_id);
    storage::save_state(&guard)
        .await
        .map_err(|e| e.to_string())?;
    Ok(guard.mcp_servers.clone())
}

#[tauri::command]
async fn test_mcp_server(
    state: State<'_, ManagedState>,
    server: models::McpServerConfig,
    project_id: Option<String>,
) -> Result<Vec<models::McpToolInfo>, String> {
    let root_path = {
        let guard = state.0.lock().await;
        let pid = project_id
            .or_else(|| guard.active_project_id.clone())
            .ok_or_else(|| "Aucun projet actif".to_string())?;
        let proj = guard
            .projects
            .iter()
            .find(|p| p.id == pid)
            .ok_or_else(|| "Projet introuvable".to_string())?;
        proj.root_path.clone()
    };
    mcp::list_tools_for_server(&server, std::path::Path::new(&root_path)).await
}

#[tauri::command]
async fn execute_mcp_tool(
    state: State<'_, ManagedState>,
    server_id: String,
    tool_name: String,
    arguments: serde_json::Value,
    project_id: Option<String>,
) -> Result<String, String> {
    let (server, root_path) = {
        let guard = state.0.lock().await;
        let server = guard
            .mcp_servers
            .iter()
            .find(|s| s.id == server_id)
            .cloned()
            .ok_or_else(|| "Serveur MCP introuvable".to_string())?;
        let pid = project_id
            .or_else(|| guard.active_project_id.clone())
            .ok_or_else(|| "Aucun projet actif".to_string())?;
        let proj = guard
            .projects
            .iter()
            .find(|p| p.id == pid)
            .ok_or_else(|| "Projet introuvable".to_string())?;
        (server, proj.root_path.clone())
    };
    mcp::call_tool_for_server(
        &server,
        std::path::Path::new(&root_path),
        &tool_name,
        &arguments,
    )
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_log::Builder::default().build())
        .setup(|app| {
            let loaded = tauri::async_runtime::block_on(storage::load_state());
            app.manage(ManagedState(Arc::new(Mutex::new(loaded))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_initial_state,
            save_settings,
            create_project,
            update_project,
            delete_project,
            update_project_context,
            create_conversation,
            select_conversation,
            delete_conversation,
            clear_conversation_context,
            send_prompt,
            refresh_project_diff,
            get_hardware_telemetry,
            check_stack_status,
            start_ollama_service,
            pick_project_directory,
            list_project_files,
            execute_local_tool,
            get_mcp_servers,
            save_mcp_server,
            delete_mcp_server,
            test_mcp_server,
            execute_mcp_tool
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
