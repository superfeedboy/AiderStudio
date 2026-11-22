use crate::models::ToolExecutionResult;
use crate::patcher;
use crate::repo_map;
use std::path::Path;

pub fn sanitize_tool_path(raw: &str) -> Result<String, String> {
    let clean = patcher::sanitize_rel_path(raw)
        .ok_or_else(|| "Chemin de fichier invalide ou non autorisé".to_string())?;
    let lower = clean.to_lowercase();
    if clean.is_empty()
        || clean.contains("..")
        || clean.starts_with('/')
        || lower == ".git"
        || lower.starts_with(".git/")
        || lower.contains("/.git/")
    {
        return Err("Chemin de fichier invalide ou non autorisé".to_string());
    }
    Ok(clean)
}

pub async fn write_file(
    project_root: &Path,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<ToolExecutionResult, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Paramètre 'path' manquant")?;
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let clean_path = sanitize_tool_path(path_str)?;
    let target_path = project_root.join(&clean_path);
    let existed = target_path.exists();

    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    let final_content =
        if content.contains("<<<<<<< SEARCH") && content.contains(">>>>>>> REPLACE") {
            if existed {
                let orig = tokio::fs::read_to_string(&target_path)
                    .await
                    .unwrap_or_default();
                let patched_res =
                    patcher::parse_edit_blocks(content, &[clean_path.clone()], &[], "")
                        .into_iter()
                        .fold(Ok(orig), |acc, blk| match (acc, blk) {
                            (
                                Ok(cur),
                                patcher::EditBlock::SearchReplace {
                                    search, replace, ..
                                },
                            ) => patcher::apply_search_replace_fuzzy(&cur, &search, &replace),
                            (res, _) => res,
                        });
                patched_res.unwrap_or_else(|_| patcher::extract_clean_replace_code(content))
            } else {
                patcher::extract_clean_replace_code(content)
            }
        } else {
            content.to_string()
        };

    tokio::fs::write(&target_path, &final_content)
        .await
        .map_err(|e| e.to_string())?;

    let action_label = if existed { "mis à jour" } else { "créé" };
    Ok(ToolExecutionResult {
        tool: tool_name.to_string(),
        success: true,
        message: format!(
            "Fichier '{}' {} avec succès ({} lignes, {} octets)",
            clean_path,
            action_label,
            final_content.lines().count(),
            final_content.len()
        ),
        path: Some(clean_path),
    })
}

fn extract_edit_pair(obj: &serde_json::Value) -> Option<(String, String)> {
    let search = obj
        .get("search")
        .or_else(|| obj.get("old_text"))
        .or_else(|| obj.get("old_string"))
        .and_then(|v| v.as_str())?;
    let replace = obj
        .get("replace")
        .or_else(|| obj.get("new_text"))
        .or_else(|| obj.get("new_string"))
        .and_then(|v| v.as_str())?;
    Some((search.to_string(), replace.to_string()))
}

pub async fn edit_file(
    project_root: &Path,
    args: &serde_json::Value,
) -> Result<ToolExecutionResult, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Paramètre 'path' manquant")?;
    let clean_path = sanitize_tool_path(path_str)?;
    let target_path = project_root.join(&clean_path);

    if !target_path.exists() {
        return Err(format!(
            "Fichier '{}' introuvable : utilise `write_file` pour créer un nouveau fichier.",
            clean_path
        ));
    }

    let mut edits: Vec<(String, String)> = Vec::new();
    if let Some(arr) = args.get("edits").and_then(|v| v.as_array()) {
        for (idx, item) in arr.iter().enumerate() {
            let pair = extract_edit_pair(item).ok_or_else(|| {
                format!(
                    "Élément #{} invalide dans `edits` : `search` et `replace` sont requis.",
                    idx + 1
                )
            })?;
            edits.push(pair);
        }
    } else if let Some(pair) = extract_edit_pair(args) {
        edits.push(pair);
    }

    if edits.is_empty() {
        return Err(
            "Paramètres d'édition manquants : fournis `search`+`replace` ou `edits: [{search, replace}]`."
                .to_string(),
        );
    }

    let original = tokio::fs::read_to_string(&target_path)
        .await
        .map_err(|e| e.to_string())?;
    let orig_line_count = original.lines().count();
    let mut working = original;

    for (idx, (search, replace)) in edits.iter().enumerate() {
        match patcher::apply_search_replace_fuzzy(&working, search, replace) {
            Ok(updated) => {
                working = updated;
            }
            Err(fail) => {
                let mut msg = format!(
                    "Échec de l'édition #{} sur '{}' : {} (aucune modification écrite sur disque)",
                    idx + 1,
                    clean_path,
                    fail.reason
                );
                if let Some(ln) = fail.closest_line {
                    msg.push_str(&format!("\nLigne la plus proche : {}", ln));
                }
                if let Some(closest) = fail.closest_match {
                    msg.push_str(&format!("\nExtrait réel du fichier :\n```\n{}\n```", closest));
                }
                if orig_line_count <= 120 {
                    msg.push_str(&format!(
                        "\nAstuce : ce fichier est court ({orig_line_count} lignes). Lis-le avec read_file ou utilise write_file pour le réécrire intégralement."
                    ));
                }
                return Err(msg);
            }
        }
    }

    tokio::fs::write(&target_path, &working)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ToolExecutionResult {
        tool: "edit_file".to_string(),
        success: true,
        message: format!(
            "Fichier '{}' modifié avec succès ({} bloc(s) appliqué(s), {} lignes, {} octets)",
            clean_path,
            edits.len(),
            working.lines().count(),
            working.len()
        ),
        path: Some(clean_path),
    })
}

pub async fn read_file(
    project_root: &Path,
    args: &serde_json::Value,
) -> Result<ToolExecutionResult, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Paramètre 'path' manquant")?;
    let clean_path = path_str
        .trim()
        .trim_matches('`')
        .trim_start_matches("./")
        .trim()
        .replace('\\', "/");
    if clean_path.is_empty() || clean_path.contains("..") || clean_path.starts_with('/') {
        return Err("Chemin relatif non autorisé".to_string());
    }
    let target_path = project_root.join(&clean_path);
    let content = tokio::fs::read_to_string(&target_path)
        .await
        .map_err(|e| e.to_string())?;

    let start_line_opt = args
        .get("start_line")
        .or_else(|| args.get("startLine"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let end_line_opt = args
        .get("end_line")
        .or_else(|| args.get("endLine"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let formatted = if start_line_opt.is_some() || end_line_opt.is_some() {
        if total_lines == 0 {
            "[Lignes 0-0 / 0]\n(fichier vide)".to_string()
        } else {
            let start = start_line_opt.unwrap_or(1).max(1);
            if start > total_lines {
                format!(
                    "[Fichier '{clean_path}' : {total_lines} lignes au total, start_line={start} hors plage]"
                )
            } else {
                let raw_end = end_line_opt.unwrap_or_else(|| start.saturating_add(199));
                let end = raw_end
                    .max(start)
                    .min(start.saturating_add(249))
                    .min(total_lines);
                let mut out_lines = vec![format!("[Lignes {start}-{end} / {total_lines}]")];
                for idx in (start - 1)..end {
                    out_lines.push(format!("{}: {}", idx + 1, lines[idx]));
                }
                out_lines.join("\n")
            }
        }
    } else if total_lines > 220 {
        let end = 160usize.min(total_lines);
        let mut out_lines = vec![format!("[Lignes 1-{end} / {total_lines}]")];
        for idx in 0..end {
            out_lines.push(format!("{}: {}", idx + 1, lines[idx]));
        }
        out_lines.push(format!(
            "... [Fichier de {total_lines} lignes — utilise read_file avec start_line/end_line ou search_code pour inspecter la suite]"
        ));
        out_lines.join("\n")
    } else {
        content
    };

    Ok(ToolExecutionResult {
        tool: "read_file".to_string(),
        success: true,
        message: formatted,
        path: Some(clean_path),
    })
}

pub async fn search_code(
    project_root: &Path,
    args: &serde_json::Value,
) -> Result<ToolExecutionResult, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("Paramètre 'query' manquant")?;
    let path_filter = args.get("path_filter").and_then(|v| v.as_str());
    let out = repo_map::search_in_project(project_root, query, path_filter, 40)?;
    Ok(ToolExecutionResult {
        tool: "search_code".to_string(),
        success: true,
        message: out,
        path: None,
    })
}

pub async fn list_files(project_root: &Path) -> Result<ToolExecutionResult, String> {
    let files = crate::engine::list_project_files_internal(project_root);
    Ok(ToolExecutionResult {
        tool: "list_files".to_string(),
        success: true,
        message: serde_json::to_string(&files).unwrap_or_default(),
        path: None,
    })
}

pub async fn delete_file(
    project_root: &Path,
    args: &serde_json::Value,
) -> Result<ToolExecutionResult, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Paramètre 'path' manquant")?;
    let clean_path = sanitize_tool_path(path_str)?;
    let target = project_root.join(&clean_path);
    if !target.exists() {
        return Err(format!("Fichier '{}' introuvable", clean_path));
    }
    if !target.is_file() {
        return Err(format!(
            "'{}' n'est pas un fichier supprimable",
            clean_path
        ));
    }
    tokio::fs::remove_file(&target)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ToolExecutionResult {
        tool: "delete_file".to_string(),
        success: true,
        message: format!("Fichier '{}' supprimé", clean_path),
        path: Some(clean_path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn make_temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vaix_tools_fs_{label}_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_write_file_creates_parents_and_rejects_traversal_and_git() {
        let dir = make_temp_dir("write");

        let ok = write_file(
            &dir,
            "write_file",
            &serde_json::json!({
                "path": "src/nested/hello.ts",
                "content": "export const x = 1;\nexport const y = 2;\n"
            }),
        )
        .await
        .unwrap();
        assert!(ok.success);
        assert_eq!(ok.path.as_deref(), Some("src/nested/hello.ts"));
        assert!(ok.message.contains("créé"));
        assert_eq!(
            std::fs::read_to_string(dir.join("src/nested/hello.ts")).unwrap(),
            "export const x = 1;\nexport const y = 2;\n"
        );

        let err_traversal = write_file(
            &dir,
            "write_file",
            &serde_json::json!({"path": "../outside.txt", "content": "nope"}),
        )
        .await;
        assert!(err_traversal.is_err());

        let err_git = write_file(
            &dir,
            "write_file",
            &serde_json::json!({"path": ".git/config", "content": "nope"}),
        )
        .await;
        assert!(err_git.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_edit_file_single_aliases_batch_and_atomic_rollback() {
        let dir = make_temp_dir("edit");
        let file_path = dir.join("app.ts");
        std::fs::write(
            &file_path,
            "const a = 1;\nconst b = 2;\nconst c = 3;\n",
        )
        .unwrap();

        // Single edit with old_text/new_text aliases
        let res1 = edit_file(
            &dir,
            &serde_json::json!({
                "path": "app.ts",
                "old_text": "const a = 1;",
                "new_text": "const a = 10;"
            }),
        )
        .await
        .unwrap();
        assert!(res1.success);
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "const a = 10;\nconst b = 2;\nconst c = 3;\n"
        );

        // Multi-block batch edit
        let res2 = edit_file(
            &dir,
            &serde_json::json!({
                "path": "app.ts",
                "edits": [
                    {"search": "const b = 2;", "replace": "const b = 20;"},
                    {"search": "const c = 3;", "replace": "const c = 30;"}
                ]
            }),
        )
        .await
        .unwrap();
        assert!(res2.success);
        assert!(res2.message.contains("2 bloc(s)"));
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "const a = 10;\nconst b = 20;\nconst c = 30;\n"
        );

        // Atomic rollback when 2nd block fails
        let before_fail = std::fs::read_to_string(&file_path).unwrap();
        let err_atomic = edit_file(
            &dir,
            &serde_json::json!({
                "path": "app.ts",
                "edits": [
                    {"search": "const a = 10;", "replace": "const a = 999;"},
                    {"search": "const nonexistent_var = 42;", "replace": "const z = 0;"}
                ]
            }),
        )
        .await
        .unwrap_err();
        assert!(err_atomic.contains("Échec de l'édition #2"));
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), before_fail);

        // Closest match hint when snippet fails to reach fuzzy threshold (< 0.88)
        let err_close = edit_file(
            &dir,
            &serde_json::json!({
                "path": "app.ts",
                "search": "const a = 10;\nconst b = 999999999;\nconst c = 888888888;",
                "replace": "const a = 0;"
            }),
        )
        .await
        .unwrap_err();
        assert!(err_close.contains("Extrait réel du fichier"));
        assert!(err_close.contains("Ligne la plus proche"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_delete_file_behavior_and_guards() {
        let dir = make_temp_dir("delete");
        std::fs::write(dir.join("obsolete.ts"), "export {};\n").unwrap();

        let ok = delete_file(&dir, &serde_json::json!({"path": "obsolete.ts"}))
            .await
            .unwrap();
        assert!(ok.success);
        assert_eq!(ok.path.as_deref(), Some("obsolete.ts"));
        assert!(!dir.join("obsolete.ts").exists());

        let err_missing = delete_file(&dir, &serde_json::json!({"path": "missing.ts"})).await;
        assert!(err_missing.is_err());

        let err_traversal = delete_file(&dir, &serde_json::json!({"path": "../foo.ts"})).await;
        assert!(err_traversal.is_err());

        let err_git = delete_file(&dir, &serde_json::json!({"path": ".git/HEAD"})).await;
        assert!(err_git.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
