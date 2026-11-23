use crate::models::{DiffLine, ModifiedFileDiff};
use regex::Regex;
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub fn git_cmd() -> Command {
    crate::tools::create_hidden_command("git")
}

pub const GIT_EXCLUDE_PATHSPECS: &[&str] = &[
    ":(exclude)node_modules",
    ":(exclude)target",
    ":(exclude)dist",
    ":(exclude)build",
    ":(exclude).next",
    ":(exclude).nuxt",
    ":(exclude)vendor",
    ":(exclude).venv",
    ":(exclude)venv",
    ":(exclude)__pycache__",
    ":(exclude).svelte-kit",
    ":(exclude).turbo",
    ":(exclude)coverage",
    ":(exclude)package-lock.json",
    ":(exclude)pnpm-lock.yaml",
    ":(exclude)yarn.lock",
    ":(exclude)bun.lock",
    ":(exclude)bun.lockb",
    ":(exclude)Cargo.lock",
    ":(exclude)composer.lock",
    ":(exclude)Gemfile.lock",
    ":(exclude)poetry.lock",
    ":(exclude)uv.lock",
    ":(exclude)go.sum",
    ":(exclude)*.lock",
];

/// Repairs files and directories mistakenly prefixed/suffixed with backticks (e.g. `` `src/App.vue ``)
/// that small local LLMs occasionally emit inside file headers.
pub async fn sanitize_backtick_paths(project_root: &str) {
    let root = PathBuf::from(project_root);
    if !root.exists() {
        return;
    }

    let _ = tokio::task::spawn_blocking(move || {
        fn collect_entries(
            dir: &Path,
            root: &Path,
            dirty_files: &mut Vec<(PathBuf, PathBuf)>,
            dirty_dirs: &mut Vec<PathBuf>,
        ) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str == ".git"
                    || name_str == "node_modules"
                    || name_str == "target"
                    || name_str == "dist"
                {
                    continue;
                }

                if path.is_dir() {
                    collect_entries(&path, root, dirty_files, dirty_dirs);
                    if name_str.starts_with('`') || name_str.ends_with('`') {
                        dirty_dirs.push(path);
                    }
                } else if let Ok(rel) = path.strip_prefix(root) {
                    let mut has_backtick = false;
                    let mut clean_rel = PathBuf::new();
                    for comp in rel.components() {
                        let s = comp.as_os_str().to_string_lossy();
                        let cleaned = s.trim_matches('`').trim();
                        if cleaned != s.as_ref() {
                            has_backtick = true;
                        }
                        if !cleaned.is_empty() {
                            clean_rel.push(cleaned);
                        }
                    }
                    if has_backtick && !clean_rel.as_os_str().is_empty() {
                        dirty_files.push((path, root.join(clean_rel)));
                    }
                }
            }
        }

        let mut dirty_files = Vec::new();
        let mut dirty_dirs = Vec::new();
        collect_entries(&root, &root, &mut dirty_files, &mut dirty_dirs);

        for (src, dst) in dirty_files {
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::rename(&src, &dst).is_err() {
                if std::fs::copy(&src, &dst).is_ok() {
                    let _ = std::fs::remove_file(&src);
                }
            }
        }

        // Remove empty backtick-prefixed directories bottom-up
        for dir in dirty_dirs {
            let _ = std::fs::remove_dir(&dir);
        }
    })
    .await;
}

/// Scans modified files in the working directory and automatically cleans up any raw
/// `<<<<<<< SEARCH ... ======= ... >>>>>>> REPLACE` markers that may have corrupted the files,
/// restoring clean code.
pub async fn sanitize_corrupted_diff_files(project_root: &str) {
    let root = Path::new(project_root);
    if !root.exists() {
        return;
    }

    if let Ok(out) = git_cmd()
        .args(["-C", project_root, "status", "--porcelain"])
        .output()
        .await
    {
        if out.status.success() {
            let list = String::from_utf8_lossy(&out.stdout);
            for line in list.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let rel_path = parts[parts.len() - 1].trim_matches('"');
                    let full_path = root.join(rel_path);
                    if full_path.is_file() {
                        if let Ok(content) = tokio::fs::read_to_string(&full_path).await {
                            if content.contains("<<<<<<< SEARCH") && content.contains(">>>>>>> REPLACE") {
                                let clean = crate::patcher::extract_clean_replace_code(&content);
                                let _ = tokio::fs::write(&full_path, clean).await;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Collects modified, added, and deleted lines for a Git repository.
/// Adjacent deleted+added lines inside a hunk are paired into `"mod"` lines
/// so the UI renders modified lines on a blue background, pure additions on green,
/// and pure deletions on red.
pub async fn collect_project_diffs(project_root: &str) -> (Vec<ModifiedFileDiff>, usize, usize) {
    let root = Path::new(project_root);
    if !root.exists() {
        return (Vec::new(), 0, 0);
    }

    sanitize_backtick_paths(project_root).await;
    sanitize_corrupted_diff_files(project_root).await;

    // Ensure git repo is initialized with a valid HEAD commit
    if !root.join(".git").exists() {
        let _ = git_cmd()
            .args(["-C", project_root, "init"])
            .output()
            .await;
    }

    let mut has_head = git_cmd()
        .args(["-C", project_root, "rev-parse", "--verify", "HEAD"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !has_head {
        let _ = git_cmd()
            .args([
                "-C",
                project_root,
                "commit",
                "--allow-empty",
                "-m",
                ">AIx Studio repo initialization",
            ])
            .output()
            .await;
        has_head = git_cmd()
            .args(["-C", project_root, "rev-parse", "--verify", "HEAD"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
    }

    let mut diff_text = if has_head {
        let mut args = vec!["-C", project_root, "diff", "HEAD", "--unified=3", "--no-color", "--", "."];
        args.extend(GIT_EXCLUDE_PATHSPECS);
        git_cmd()
            .args(&args)
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    if diff_text.is_empty() {
        // Fallback: combine unstaged diff + cached staged diff
        let mut unstaged_args = vec!["-C", project_root, "diff", "--unified=3", "--no-color", "--", "."];
        unstaged_args.extend(GIT_EXCLUDE_PATHSPECS);
        let unstaged = git_cmd()
            .args(&unstaged_args)
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let mut staged_args = vec!["-C", project_root, "diff", "--cached", "--unified=3", "--no-color", "--", "."];
        staged_args.extend(GIT_EXCLUDE_PATHSPECS);
        let staged = git_cmd()
            .args(&staged_args)
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        diff_text = format!("{}\n{}", unstaged, staged).trim().to_string();
    }

    let mut files = parse_unified_diff(&diff_text);

    // Include untracked files as pure additions (up to 150 lines per file, strictly inside project_root)
    let mut ls_args = vec!["-C", project_root, "ls-files", "--others", "--exclude-standard", "--", "."];
    ls_args.extend(GIT_EXCLUDE_PATHSPECS);
    if let Ok(status_out) = git_cmd()
        .args(&ls_args)
        .output()
        .await
    {
        if status_out.status.success() {
            let list = String::from_utf8_lossy(&status_out.stdout);
            for rel_path in list.lines().take(20) {
                let rel_trimmed = rel_path.trim();
                if rel_trimmed.is_empty() || files.iter().any(|f| f.path == rel_trimmed) {
                    continue;
                }
                let full = root.join(rel_trimmed);
                if let Ok(content) = tokio::fs::read_to_string(&full).await {
                    let lines: Vec<DiffLine> = content
                        .lines()
                        .take(150)
                        .enumerate()
                        .map(|(idx, line)| DiffLine {
                            kind: "add".to_string(),
                            old_line: None,
                            new_line: Some(idx + 1),
                            content: line.to_string(),
                        })
                        .collect();
                    let added = lines.len();
                    if added > 0 {
                        files.push(ModifiedFileDiff {
                            path: rel_trimmed.to_string(),
                            added,
                            removed: 0,
                            modified: 0,
                            status: Some("created".to_string()),
                            lines,
                        });
                    }
                }
            }
        }
    }

    let total_added: usize = files.iter().map(|f| f.added + f.modified).sum();
    let total_removed: usize = files.iter().map(|f| f.removed + f.modified).sum();
    (files, total_added, total_removed)
}

/// Capture un arbre Git instantané (tree SHA) du workspace courant dans un index temporaire isolé,
/// sans jamais modifier `.git/index` ni `HEAD`.
pub async fn capture_workspace_tree(project_root: &str) -> Option<String> {
    let root = Path::new(project_root);
    if !root.exists() {
        return None;
    }

    if !root.join(".git").exists() {
        let _ = git_cmd()
            .args(["-C", project_root, "init"])
            .output()
            .await;
    }

    let tmp_idx = std::env::temp_dir().join(format!("vaix-idx-{}.idx", uuid::Uuid::new_v4()));
    let mut add_args = vec!["-C", project_root, "add", "-A", "--", "."];
    add_args.extend(GIT_EXCLUDE_PATHSPECS);
    let add_res = git_cmd()
        .args(&add_args)
        .env("GIT_INDEX_FILE", &tmp_idx)
        .output()
        .await;

    let tree_sha = match add_res {
        Ok(out) if out.status.success() => {
            let write_res = git_cmd()
                .args(["-C", project_root, "write-tree"])
                .env("GIT_INDEX_FILE", &tmp_idx)
                .output()
                .await;
            match write_res {
                Ok(w) if w.status.success() => {
                    let sha = String::from_utf8_lossy(&w.stdout).trim().to_string();
                    if sha.is_empty() {
                        None
                    } else {
                        Some(sha)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    };

    let _ = tokio::fs::remove_file(&tmp_idx).await;
    tree_sha
}

/// Calcule le diff exact d'un tour entre l'arbre capturé avant exécution (`pre_tree`) et l'état actuel sur disque.
pub async fn collect_turn_diffs(
    project_root: &str,
    pre_tree: &str,
) -> (Vec<ModifiedFileDiff>, usize, usize) {
    sanitize_backtick_paths(project_root).await;
    sanitize_corrupted_diff_files(project_root).await;

    let Some(post_tree) = capture_workspace_tree(project_root).await else {
        return (Vec::new(), 0, 0);
    };
    if post_tree == pre_tree {
        return (Vec::new(), 0, 0);
    }

    let diff_out = git_cmd()
        .args([
            "-C",
            project_root,
            "diff",
            pre_tree,
            &post_tree,
            "--unified=3",
            "--no-color",
            "--",
            ".",
        ])
        .output()
        .await;

    let Ok(out) = diff_out else {
        return (Vec::new(), 0, 0);
    };
    if !out.status.success() {
        return (Vec::new(), 0, 0);
    }

    let raw = String::from_utf8_lossy(&out.stdout);
    let mut files = parse_unified_diff(&raw);
    let total_added: usize = files.iter().map(|f| f.added + f.modified).sum();
    let total_removed: usize = files.iter().map(|f| f.removed + f.modified).sum();
    for f in &mut files {
        f.lines.truncate(500);
    }
    (files, total_added, total_removed)
}

pub fn parse_unified_diff(raw: &str) -> Vec<ModifiedFileDiff> {
    let hunk_re = Regex::new(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@").expect("valid hunk regex");
    let mut files: Vec<ModifiedFileDiff> = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_status: Option<String> = None;
    let mut current_lines: Vec<DiffLine> = Vec::new();
    let mut pending_del: Vec<(Option<usize>, String)> = Vec::new();
    let mut pending_add: Vec<(Option<usize>, String)> = Vec::new();
    let mut old_ln: usize = 0;
    let mut new_ln: usize = 0;

    let flush_pending = |del: &mut Vec<(Option<usize>, String)>,
                         add: &mut Vec<(Option<usize>, String)>,
                         out: &mut Vec<DiffLine>| {
        let mod_count = del.len().min(add.len());
        for i in 0..mod_count {
            out.push(DiffLine {
                kind: "mod".to_string(),
                old_line: del[i].0,
                new_line: add[i].0,
                content: add[i].1.clone(),
            });
        }
        for item in del.drain(mod_count..) {
            out.push(DiffLine {
                kind: "del".to_string(),
                old_line: item.0,
                new_line: None,
                content: item.1,
            });
        }
        for item in add.drain(mod_count..) {
            out.push(DiffLine {
                kind: "add".to_string(),
                old_line: None,
                new_line: item.0,
                content: item.1,
            });
        }
        del.clear();
        add.clear();
    };

    let finalize_file = |path: &mut Option<String>,
                         status: &mut Option<String>,
                         lines: &mut Vec<DiffLine>,
                         del: &mut Vec<(Option<usize>, String)>,
                         add: &mut Vec<(Option<usize>, String)>,
                         out: &mut Vec<ModifiedFileDiff>| {
        flush_pending(del, add, lines);
        let st = status.take();
        if let Some(p) = path.take() {
            let added = lines.iter().filter(|l| l.kind == "add").count();
            let removed = lines.iter().filter(|l| l.kind == "del").count();
            let modified = lines.iter().filter(|l| l.kind == "mod").count();
            let resolved_status = st.or_else(|| Some("modified".to_string()));
            if !lines.is_empty()
                || matches!(resolved_status.as_deref(), Some("created" | "deleted"))
            {
                out.push(ModifiedFileDiff {
                    path: p,
                    added,
                    removed,
                    modified,
                    status: resolved_status,
                    lines: std::mem::take(lines),
                });
            } else {
                lines.clear();
            }
        } else {
            lines.clear();
        }
    };

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            finalize_file(
                &mut current_path,
                &mut current_status,
                &mut current_lines,
                &mut pending_del,
                &mut pending_add,
                &mut files,
            );
            let parsed_path = rest
                .split(" b/")
                .nth(1)
                .unwrap_or(rest)
                .trim()
                .trim_matches('"')
                .to_string();
            current_path = Some(parsed_path);
            current_status = Some("modified".to_string());
            continue;
        }
        if line.starts_with("new file mode ") {
            current_status = Some("created".to_string());
            continue;
        }
        if line.starts_with("deleted file mode ") {
            current_status = Some("deleted".to_string());
            continue;
        }
        if line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("index ")
            || line.starts_with("old mode ")
            || line.starts_with("new mode ")
            || line.starts_with("similarity index ")
            || line.starts_with("rename from ")
            || line.starts_with("rename to ")
            || line.starts_with("Binary files ")
        {
            continue;
        }
        if let Some(cap) = hunk_re.captures(line) {
            flush_pending(&mut pending_del, &mut pending_add, &mut current_lines);
            old_ln = cap.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
            new_ln = cap.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
            continue;
        }
        if current_path.is_none() {
            continue;
        }
        if let Some(stripped) = line.strip_prefix('-') {
            pending_del.push((Some(old_ln), stripped.to_string()));
            old_ln += 1;
        } else if let Some(stripped) = line.strip_prefix('+') {
            pending_add.push((Some(new_ln), stripped.to_string()));
            new_ln += 1;
        } else if let Some(stripped) = line.strip_prefix(' ') {
            flush_pending(&mut pending_del, &mut pending_add, &mut current_lines);
            current_lines.push(DiffLine {
                kind: "ctx".to_string(),
                old_line: Some(old_ln),
                new_line: Some(new_ln),
                content: stripped.to_string(),
            });
            old_ln += 1;
            new_ln += 1;
        }
    }

    finalize_file(
        &mut current_path,
        &mut current_status,
        &mut current_lines,
        &mut pending_del,
        &mut pending_add,
        &mut files,
    );

    files
}

/// Restaure un fichier exactement à son état dans `tree_sha` (ou le supprime s'il n'existait pas dans cet arbre).
pub async fn undo_file_to_tree(
    project_root: &str,
    tree_sha: &str,
    file_path: &str,
) -> Result<(), String> {
    let clean_path = crate::tools::fs::sanitize_tool_path(file_path)?;
    let root = Path::new(project_root);
    let target = root.join(&clean_path);
    let spec = format!("{tree_sha}:{clean_path}");

    let exists_in_tree = git_cmd()
        .args(["-C", project_root, "cat-file", "-e", &spec])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if exists_in_tree {
        let out = git_cmd()
            .args(["-C", project_root, "show", &spec])
            .output()
            .await
            .map_err(|e| format!("Impossible de lire `{spec}` dans Git : {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "Échec restauration `{clean_path}` depuis `{tree_sha}`"
            ));
        }
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::write(&target, &out.stdout)
            .await
            .map_err(|e| e.to_string())?;
    } else if target.exists() {
        if target.is_file() {
            tokio::fs::remove_file(&target)
                .await
                .map_err(|e| e.to_string())?;
        } else if target.is_dir() {
            tokio::fs::remove_dir_all(&target)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// Restaure plusieurs fichiers exactement à leur état dans `tree_sha`.
pub async fn undo_files_to_tree(
    project_root: &str,
    tree_sha: &str,
    file_paths: &[String],
) -> Result<(), String> {
    for p in file_paths {
        undo_file_to_tree(project_root, tree_sha, p).await?;
    }
    Ok(())
}

/// Undoes modifications to a single file:
/// - Reverts tracked changes from HEAD (works for modified or deleted files)
/// - Deletes untracked newly-created files
pub async fn undo_file_modification(project_root: &str, file_path: &str) -> Result<(), String> {
    let root = Path::new(project_root);
    let target = root.join(file_path);

    // 1. Try checkout from HEAD for tracked files
    let checkout_out = git_cmd()
        .args(["-C", project_root, "checkout", "HEAD", "--", file_path])
        .output()
        .await;

    if let Ok(output) = checkout_out {
        if output.status.success() {
            // Also unstage just in case it was staged
            let _ = git_cmd()
                .args(["-C", project_root, "restore", "--staged", "--", file_path])
                .output()
                .await;
            return Ok(());
        }
    }

    // 2. Also unstage if staged as a new file
    let _ = git_cmd()
        .args(["-C", project_root, "rm", "-f", "--cached", "--", file_path])
        .output()
        .await;

    // 3. If file exists on disk and wasn't in HEAD, remove it
    if target.exists() {
        if target.is_file() {
            let _ = tokio::fs::remove_file(&target).await;
        } else if target.is_dir() {
            let _ = tokio::fs::remove_dir_all(&target).await;
        }
    }

    Ok(())
}

/// Undoes all uncommitted modifications across the entire project repository:
/// - Restores tracked files to HEAD
/// - Cleans untracked files and directories (respects .gitignore and .aiderignore)
pub async fn undo_all_modifications(project_root: &str) -> Result<(), String> {
    // 1. Unstage any staged files
    let _ = git_cmd()
        .args(["-C", project_root, "restore", "--staged", "--worktree", "."])
        .output()
        .await;

    // 2. Revert tracked files to HEAD
    let _ = git_cmd()
        .args(["-C", project_root, "checkout", "HEAD", "--", "."])
        .output()
        .await;

    // 3. Remove untracked files & directories (clean -fd respects .gitignore)
    let _ = git_cmd()
        .args(["-C", project_root, "clean", "-fd"])
        .output()
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_turn_diff_and_undo_on_previously_created_untracked_file() {
        let dir = std::env::temp_dir().join(format!("vaix_gitdiff_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let root_str = dir.to_string_lossy().to_string();

        let _ = git_cmd()
            .args(["-C", &root_str, "init"])
            .output()
            .await
            .unwrap();
        let _ = git_cmd()
            .args([
                "-C",
                &root_str,
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ])
            .output()
            .await
            .unwrap();

        // Tour 1 : création d'un fichier untracked de 10 lignes
        let initial_lines: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
        let initial_content = format!("{}\n", initial_lines.join("\n"));
        std::fs::write(dir.join("foo.ts"), &initial_content).unwrap();

        // Snapshot avant Tour 2
        let tree1 = capture_workspace_tree(&root_str)
            .await
            .expect("tree1 captured");

        // Tour 2 : ajout de 2 lignes seulement dans foo.ts (toujours untracked par rapport à HEAD)
        let mut updated_content = initial_content.clone();
        updated_content.push_str("line 11\nline 12\n");
        std::fs::write(dir.join("foo.ts"), &updated_content).unwrap();

        let (turn_files, turn_added, turn_removed) = collect_turn_diffs(&root_str, &tree1).await;
        assert_eq!(turn_files.len(), 1);
        assert_eq!(turn_files[0].path, "foo.ts");
        assert_eq!(turn_files[0].status.as_deref(), Some("modified"));
        assert_eq!(turn_files[0].added, 2);
        assert_eq!(turn_files[0].removed, 0);
        assert_eq!(turn_files[0].modified, 0);
        assert_eq!(turn_added, 2);
        assert_eq!(turn_removed, 0);
        assert_eq!(
            turn_files[0].lines.iter().filter(|l| l.kind == "add").count(),
            2
        );

        // Undo au niveau de l'étape -> restaure foo.ts à ses 10 lignes sans le supprimer
        undo_file_to_tree(&root_str, &tree1, "foo.ts")
            .await
            .unwrap();
        assert!(dir.join("foo.ts").exists());
        let restored = std::fs::read_to_string(dir.join("foo.ts")).unwrap();
        assert_eq!(restored, initial_content);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
