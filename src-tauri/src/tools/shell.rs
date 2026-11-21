use crate::models::ToolExecutionResult;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};

const MAX_COMMAND_OUTPUT_BYTES: usize = 64_000;
const STREAM_LINE_MAX_CHARS: usize = 160;

fn parse_nvm_version_tuple(name: &str) -> (u64, u64, u64) {
    let trimmed = name.trim().trim_start_matches(['v', 'V']);
    let mut parts = trimmed.split('.');
    let major = parts.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    (major, minor, patch)
}

pub fn build_enriched_path() -> String {
    let mut dirs_list: Vec<PathBuf> = Vec::new();
    let mut push_unique = |p: PathBuf, check_exists: bool| {
        if check_exists && !p.is_dir() {
            return;
        }
        if !dirs_list.iter().any(|existing| existing == &p) {
            dirs_list.push(p);
        }
    };

    if let Ok(path_env) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_env) {
            if !dir.as_os_str().is_empty() {
                push_unique(dir, false);
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        for sys_dir in [
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
        ] {
            push_unique(PathBuf::from(sys_dir), true);
        }

        if let Some(home) = dirs::home_dir() {
            for rel in [
                ".cargo/bin",
                ".local/bin",
                ".bun/bin",
                ".volta/bin",
                "Library/pnpm",
                ".local/share/pnpm",
            ] {
                push_unique(home.join(rel), true);
            }

            let nvm_node_dir = home.join(".nvm/versions/node");
            if nvm_node_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&nvm_node_dir) {
                    let mut versions: Vec<((u64, u64, u64), PathBuf)> = entries
                        .flatten()
                        .filter_map(|entry| {
                            let path = entry.path();
                            let bin_dir = path.join("bin");
                            if bin_dir.is_dir() {
                                let name = entry.file_name().to_string_lossy().to_string();
                                Some((parse_nvm_version_tuple(&name), bin_dir))
                            } else {
                                None
                            }
                        })
                        .collect();
                    versions.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
                    if let Some((_, latest_bin)) = versions.pop() {
                        push_unique(latest_bin, true);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(home) = dirs::home_dir() {
            push_unique(home.join(".cargo").join("bin"), true);
            push_unique(home.join(".bun").join("bin"), true);
        }
        if let Some(local) = dirs::data_local_dir() {
            push_unique(local.join("pnpm"), true);
        }
    }

    match std::env::join_paths(&dirs_list) {
        Ok(joined) => joined.to_string_lossy().to_string(),
        Err(_) => {
            let sep = if cfg!(target_os = "windows") { ";" } else { ":" };
            dirs_list
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(sep)
        }
    }
}

pub fn is_destructive_command(cmd: &str) -> Option<&'static str> {
    static RM_RECURSIVE_RE: OnceLock<Regex> = OnceLock::new();
    static GIT_CLEAN_RE: OnceLock<Regex> = OnceLock::new();
    static GIT_PUSH_FORCE_RE: OnceLock<Regex> = OnceLock::new();
    static SUDO_RE: OnceLock<Regex> = OnceLock::new();
    static MKFS_RE: OnceLock<Regex> = OnceLock::new();
    static DD_RE: OnceLock<Regex> = OnceLock::new();
    static CHMOD_777_RE: OnceLock<Regex> = OnceLock::new();
    static DEV_WRITE_RE: OnceLock<Regex> = OnceLock::new();

    let norm = cmd
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if norm.is_empty() {
        return None;
    }

    let rm_re = RM_RECURSIVE_RE.get_or_init(|| {
        Regex::new(r"(?:^|[;&|]\s*)rm\s+(?:[^;&|]*\s+)?(?:-[a-z]*r[a-z]*\b|--recursive\b)")
            .expect("valid rm regex")
    });
    if rm_re.is_match(&norm) || norm.contains("rmdir /s") || norm.contains("del /f /s") {
        return Some("Suppression récursive ou forcée détectée (`rm -r` / `rm -rf`).");
    }

    let git_clean_re = GIT_CLEAN_RE.get_or_init(|| {
        Regex::new(r"\bgit\s+clean\b[^;&|]*\s-(?:[a-z]*f[a-z]*d|[a-z]*d[a-z]*f)[a-z]*\b")
            .expect("valid git clean regex")
    });
    if norm.contains("git reset --hard") || git_clean_re.is_match(&norm) {
        return Some("Réinitialisation destructive du dépôt Git (`git reset --hard` / `git clean`).");
    }

    let git_push_force_re = GIT_PUSH_FORCE_RE.get_or_init(|| {
        Regex::new(r"\bgit\s+push\b[^;&|]*(?:\s--force\b|\s-f\b)").expect("valid git push regex")
    });
    if git_push_force_re.is_match(&norm) {
        return Some("Push Git forcé (`--force` / `-f`) risquant d'écraser l'historique distant.");
    }

    if norm.contains("drop table")
        || norm.contains("drop database")
        || norm.contains("truncate table")
    {
        return Some("Instruction SQL destructive (`DROP` / `TRUNCATE`).");
    }

    let sudo_re =
        SUDO_RE.get_or_init(|| Regex::new(r"(?:^|[;&|]\s*)sudo\s").expect("valid sudo regex"));
    if sudo_re.is_match(&norm) {
        return Some("Exécution privilégiée via `sudo`.");
    }

    let mkfs_re = MKFS_RE
        .get_or_init(|| Regex::new(r"\bmkfs(?:\.[a-z0-9]+)?\b").expect("valid mkfs regex"));
    let dd_re = DD_RE.get_or_init(|| Regex::new(r"\bdd\s+if=").expect("valid dd regex"));
    let chmod_re = CHMOD_777_RE.get_or_init(|| {
        Regex::new(r"\bchmod\s+(?:-r|--recursive)\s+777\b").expect("valid chmod regex")
    });
    let dev_re = DEV_WRITE_RE.get_or_init(|| {
        Regex::new(r">\s*/dev/(?:sd|nvme|disk|hd)").expect("valid dev write regex")
    });

    if mkfs_re.is_match(&norm)
        || dd_re.is_match(&norm)
        || chmod_re.is_match(&norm)
        || dev_re.is_match(&norm)
    {
        return Some("Commande système critique potentiellement destructive.");
    }

    None
}

pub fn parse_timeout_secs(args: &serde_json::Value) -> u64 {
    args.get("timeout_secs")
        .or_else(|| args.get("timeoutSecs"))
        .and_then(|v| v.as_u64())
        .unwrap_or(90)
        .clamp(5, 300)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let truncated: String = input.chars().take(max_chars).collect();
    format!("{truncated}…")
}

async fn wait_opt_cancelled(cancel_rx: &mut Option<&mut watch::Receiver<bool>>) {
    match cancel_rx.as_deref_mut() {
        Some(rx) => {
            if *rx.borrow() {
                return;
            }
            while rx.changed().await.is_ok() {
                if *rx.borrow() {
                    return;
                }
            }
            std::future::pending::<()>().await;
        }
        None => std::future::pending::<()>().await,
    }
}

pub async fn run_command_streaming<F>(
    project_root: &Path,
    cmd_str: &str,
    timeout_secs: u64,
    mut cancel_rx: Option<&mut watch::Receiver<bool>>,
    mut on_line: F,
) -> Result<ToolExecutionResult, String>
where
    F: FnMut(&str),
{
    if let Some(rx) = cancel_rx.as_ref() {
        if *rx.borrow() {
            return Err("CANCELLED_BY_USER".to_string());
        }
    }

    let trimmed_cmd = cmd_str.trim();
    if trimmed_cmd.is_empty() {
        return Err("Paramètre 'command' vide".to_string());
    }

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", trimmed_cmd]);
        c.creation_flags(0x08000000);
        c
    };

    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", trimmed_cmd]);
        c
    };

    cmd.current_dir(project_root)
        .env("PATH", build_enriched_path())
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (line_tx, mut line_rx) = mpsc::channel::<String>(128);
    if let Some(out) = stdout {
        let tx = line_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(out).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).await.is_err() {
                    break;
                }
            }
        });
    }
    if let Some(err) = stderr {
        let tx = line_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(err).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(line_tx);

    let mut output_buf = String::new();
    let mut was_truncated = false;
    let sleep = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            biased;
            _ = wait_opt_cancelled(&mut cancel_rx) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err("CANCELLED_BY_USER".to_string());
            }
            _ = &mut sleep => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let tail = if output_buf.trim().is_empty() {
                    String::new()
                } else {
                    format!("\nDernières lignes :\n{}", output_buf.trim())
                };
                return Err(format!(
                    "Timeout lors de l'exécution de la commande ({timeout_secs}s).{tail}"
                ));
            }
            maybe_line = line_rx.recv() => {
                match maybe_line {
                    Some(line) => {
                        let clean = line.trim();
                        if !clean.is_empty() {
                            on_line(&truncate_chars(clean, STREAM_LINE_MAX_CHARS));
                        }
                        if output_buf.len() < MAX_COMMAND_OUTPUT_BYTES {
                            if !output_buf.is_empty() {
                                output_buf.push('\n');
                            }
                            let remaining = MAX_COMMAND_OUTPUT_BYTES.saturating_sub(output_buf.len());
                            if line.len() <= remaining {
                                output_buf.push_str(&line);
                            } else {
                                let clipped: String = line.chars().take(remaining).collect();
                                output_buf.push_str(&clipped);
                                was_truncated = true;
                            }
                        } else {
                            was_truncated = true;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    let status = child.wait().await.map_err(|e| e.to_string())?;
    let mut message = if output_buf.trim().is_empty() {
        "(aucune sortie)".to_string()
    } else {
        output_buf
    };
    if was_truncated {
        message.push_str("\n... [sortie tronquée à 64 Ko]");
    }
    if !status.success() {
        let code_str = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        message.push_str(&format!("\n[code de sortie: {code_str}]"));
    }

    Ok(ToolExecutionResult {
        tool: "run_command".to_string(),
        success: status.success(),
        message,
        path: None,
    })
}

pub async fn run_command_simple(
    project_root: &Path,
    args: &serde_json::Value,
) -> Result<ToolExecutionResult, String> {
    let cmd_str = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or("Paramètre 'command' manquant")?;
    let timeout_secs = parse_timeout_secs(args);
    run_command_streaming(project_root, cmd_str, timeout_secs, None, |_| {}).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_is_destructive_command_detection() {
        assert!(is_destructive_command("rm -rf ./dist").is_some());
        assert!(is_destructive_command("ls && rm -fr /tmp/test").is_some());
        assert!(is_destructive_command("git   reset  --hard HEAD~1").is_some());
        assert!(is_destructive_command("git clean -xfd").is_some());
        assert!(is_destructive_command("git push --force origin main").is_some());
        assert!(is_destructive_command("psql -c 'DROP TABLE users;'").is_some());
        assert!(is_destructive_command("sudo apt-get update").is_some());

        // Safe / standard development commands must pass through
        assert!(is_destructive_command("pnpm install").is_none());
        assert!(is_destructive_command("npm run build").is_none());
        assert!(is_destructive_command("cargo test --lib").is_none());
        assert!(is_destructive_command("git status").is_none());
        assert!(is_destructive_command("echo hello > /dev/null").is_none());
    }

    #[tokio::test]
    async fn test_run_command_streaming_stdout_callback_and_exit_code() {
        let dir = std::env::temp_dir();
        let mut streamed: Vec<String> = Vec::new();

        let ok = run_command_streaming(
            &dir,
            "echo line_one && echo line_two",
            15,
            None,
            |line| streamed.push(line.to_string()),
        )
        .await
        .unwrap();
        assert!(ok.success);
        assert!(ok.message.contains("line_one"));
        assert!(ok.message.contains("line_two"));
        assert!(streamed.iter().any(|l| l.contains("line_one")));
        assert!(streamed.iter().any(|l| l.contains("line_two")));

        let fail = run_command_streaming(
            &dir,
            "echo bad_thing >&2 && exit 2",
            15,
            None,
            |_| {},
        )
        .await
        .unwrap();
        assert!(!fail.success);
        assert!(fail.message.contains("bad_thing"));
        assert!(fail.message.contains("[code de sortie: 2]"));
    }

    #[tokio::test]
    async fn test_run_command_streaming_cancellation() {
        let dir = std::env::temp_dir();
        let (cancel_tx, mut cancel_rx) = watch::channel(false);

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            let _ = cancel_tx.send(true);
        });

        let started = Instant::now();
        let res = run_command_streaming(
            &dir,
            "sleep 10",
            30,
            Some(&mut cancel_rx),
            |_| {},
        )
        .await;
        assert_eq!(res.unwrap_err(), "CANCELLED_BY_USER");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
