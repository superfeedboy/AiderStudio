use crate::models::{McpServerConfig, McpToolInfo};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub async fn list_tools_for_server(
    server: &McpServerConfig,
    project_root: &Path,
) -> Result<Vec<McpToolInfo>, String> {
    let mut cmd = Command::new(&server.command);
    cmd.args(&server.args);
    cmd.current_dir(project_root);
    for (k, v) in &server.env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "Impossible de démarrer le serveur MCP '{}' (commande '{}') : {}",
            server.name, server.command, e
        )
    })?;

    let mut stdin = child.stdin.take().ok_or("stdin indisponible")?;
    let stdout = child.stdout.take().ok_or("stdout indisponible")?;
    let mut reader = BufReader::new(stdout).lines();

    // 1. Send initialize request
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "vAIx STUDIO",
                "version": "0.1.0"
            }
        }
    });
    let mut init_line = init_req.to_string();
    init_line.push('\n');
    stdin.write_all(init_line.as_bytes()).await.map_err(|e| e.to_string())?;

    // Read initialize response
    let _init_resp = tokio::time::timeout(std::time::Duration::from_secs(10), reader.next_line())
        .await
        .map_err(|_| "Timeout lors de l'initialisation MCP".to_string())?
        .map_err(|e| e.to_string())?
        .ok_or("Pas de réponse de l'initialisation MCP")?;

    // 2. Send initialized notification
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let mut notif_line = notif.to_string();
    notif_line.push('\n');
    stdin.write_all(notif_line.as_bytes()).await.map_err(|e| e.to_string())?;

    // 3. Send tools/list request
    let tools_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    let mut tools_line = tools_req.to_string();
    tools_line.push('\n');
    stdin.write_all(tools_line.as_bytes()).await.map_err(|e| e.to_string())?;

    let tools_resp_str = tokio::time::timeout(std::time::Duration::from_secs(10), reader.next_line())
        .await
        .map_err(|_| "Timeout lors de l'appel tools/list".to_string())?
        .map_err(|e| e.to_string())?
        .ok_or("Pas de réponse à tools/list")?;

    let _ = child.kill().await;

    let parsed: Value = serde_json::from_str(&tools_resp_str).map_err(|e| e.to_string())?;
    let raw_tools = parsed
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .ok_or_else(|| {
            let err_msg = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("Réponse tools/list invalide");
            err_msg.to_string()
        })?;

    let mut result = Vec::new();
    for t in raw_tools {
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let schema = t.get("inputSchema").cloned().unwrap_or(json!({}));
        if !name.is_empty() {
            result.push(McpToolInfo {
                server_id: server.id.clone(),
                server_name: server.name.clone(),
                name,
                description: desc,
                input_schema: schema,
            });
        }
    }

    Ok(result)
}

pub async fn call_tool_for_server(
    server: &McpServerConfig,
    project_root: &Path,
    tool_name: &str,
    arguments: &Value,
) -> Result<String, String> {
    let mut cmd = Command::new(&server.command);
    cmd.args(&server.args);
    cmd.current_dir(project_root);
    for (k, v) in &server.env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "Impossible d'exécuter l'outil sur le serveur MCP '{}' : {}",
            server.name, e
        )
    })?;

    let mut stdin = child.stdin.take().ok_or("stdin indisponible")?;
    let stdout = child.stdout.take().ok_or("stdout indisponible")?;
    let mut reader = BufReader::new(stdout).lines();

    // 1. Initialize
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "vAIx STUDIO",
                "version": "0.1.0"
            }
        }
    });
    let mut init_line = init_req.to_string();
    init_line.push('\n');
    stdin.write_all(init_line.as_bytes()).await.map_err(|e| e.to_string())?;

    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), reader.next_line())
        .await
        .map_err(|_| "Timeout handshake MCP".to_string())?
        .map_err(|e| e.to_string())?;

    // 2. Initialized notification
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let mut notif_line = notif.to_string();
    notif_line.push('\n');
    stdin.write_all(notif_line.as_bytes()).await.map_err(|e| e.to_string())?;

    // 3. Call tool
    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });
    let mut call_line = call_req.to_string();
    call_line.push('\n');
    stdin.write_all(call_line.as_bytes()).await.map_err(|e| e.to_string())?;

    let call_resp_str = tokio::time::timeout(std::time::Duration::from_secs(30), reader.next_line())
        .await
        .map_err(|_| format!("Timeout lors de l'exécution de l'outil MCP '{}'", tool_name))?
        .map_err(|e| e.to_string())?
        .ok_or("Pas de réponse de l'outil MCP")?;

    let _ = child.kill().await;

    let parsed: Value = serde_json::from_str(&call_resp_str).map_err(|e| e.to_string())?;
    if let Some(err) = parsed.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("Erreur MCP");
        return Err(msg.to_string());
    }

    let contents = parsed
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array());

    if let Some(items) = contents {
        let text_pieces: Vec<String> = items
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .collect();
        if !text_pieces.is_empty() {
            return Ok(text_pieces.join("\n"));
        }
    }

    Ok(parsed.get("result").map(|r| r.to_string()).unwrap_or_else(|| call_resp_str))
}
