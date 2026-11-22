use regex::Regex;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootManifest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredScript {
    pub source: String,
    pub name: String,
    pub command: String,
    pub runner_cmd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenDeclaredPath {
    pub source_file: String,
    pub declared_path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceSnapshot {
    pub all_files: Vec<String>,
    pub root_manifests: Vec<RootManifest>,
    pub declared_scripts: Vec<DeclaredScript>,
    pub broken_declared_paths: Vec<BrokenDeclaredPath>,
}

fn truncate_chars_safe(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(15);
    let prefix: String = s.chars().take(keep).collect();
    format!("{prefix}\n... [tronqué]")
}

fn is_root_manifest_or_config(rel: &str) -> bool {
    if rel.contains('/') {
        return false;
    }
    let low = rel.to_lowercase();
    if matches!(
        low.as_str(),
        "makefile"
            | "justfile"
            | "dockerfile"
            | "procfile"
            | "requirements.txt"
            | "gemfile"
            | "rakefile"
            | "cmakelists.txt"
            | ".htaccess"
            | ".editorconfig"
            | ".prettierrc"
            | ".eslintrc"
    ) {
        return true;
    }
    if low.contains(".config.") {
        return true;
    }
    matches!(
        low.rsplit('.').next(),
        Some("json" | "toml" | "yaml" | "yml" | "html" | "htm" | "mod" | "xml" | "ini" | "cfg" | "conf")
    )
}

fn manifest_priority(rel: &str) -> (u8, usize) {
    let low = rel.to_lowercase();
    let tier = if matches!(
        low.as_str(),
        "package.json" | "cargo.toml" | "composer.json" | "pyproject.toml" | "go.mod"
    ) {
        0
    } else if low.ends_with(".html") || low.ends_with(".htm") || low == "makefile" || low == "justfile" {
        1
    } else if low.contains(".config.") || low.ends_with(".json") || low.ends_with(".toml") {
        2
    } else {
        3
    };
    (tier, rel.len())
}

fn detect_js_package_runner(project_root: &Path) -> &'static str {
    if project_root.join("pnpm-lock.yaml").exists() {
        "pnpm run"
    } else if project_root.join("yarn.lock").exists() {
        "yarn"
    } else if project_root.join("bun.lockb").exists() || project_root.join("bun.lock").exists() {
        "bun run"
    } else {
        "npm run"
    }
}

fn extract_declared_scripts(
    project_root: &Path,
    manifests: &[RootManifest],
) -> Vec<DeclaredScript> {
    let mut scripts = Vec::new();
    let makefile_target_re =
        Regex::new(r"(?m)^([A-Za-z0-9][A-Za-z0-9_-]*)\s*:").expect("valid target regex");

    for manifest in manifests {
        let low = manifest.path.to_lowercase();
        if low.ends_with(".json") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&manifest.content) {
                if let Some(obj) = json.get("scripts").and_then(|v| v.as_object()) {
                    for (name, val) in obj {
                        let Some(cmd_raw) = val.as_str() else {
                            continue;
                        };
                        let cmd = cmd_raw.trim();
                        if cmd.is_empty() {
                            continue;
                        }
                        let runner_cmd = if low == "package.json" {
                            format!("{} {}", detect_js_package_runner(project_root), name)
                        } else if low == "composer.json" {
                            format!("composer run-script {name}")
                        } else {
                            cmd.to_string()
                        };
                        scripts.push(DeclaredScript {
                            source: manifest.path.clone(),
                            name: name.clone(),
                            command: cmd.to_string(),
                            runner_cmd,
                        });
                    }
                }
            }
        } else if low == "makefile" || low == "justfile" {
            let runner = if low == "makefile" { "make" } else { "just" };
            let mut seen_targets = HashSet::new();
            for cap in makefile_target_re.captures_iter(&manifest.content) {
                let Some(m) = cap.get(1) else {
                    continue;
                };
                let target = m.as_str();
                if target.eq_ignore_ascii_case("phony") || !seen_targets.insert(target.to_string()) {
                    continue;
                }
                scripts.push(DeclaredScript {
                    source: manifest.path.clone(),
                    name: target.to_string(),
                    command: format!("{runner} {target}"),
                    runner_cmd: format!("{runner} {target}"),
                });
            }
        }
    }

    scripts
}

fn is_ignored_output_dir(clean_path: &str) -> bool {
    let low = clean_path.trim_start_matches("./").trim_start_matches('/').to_lowercase();
    let first = low.split('/').next().unwrap_or("");
    matches!(first, "dist" | "build" | "target" | "out" | "coverage" | ".next" | ".nuxt" | "node_modules" | "vendor")
}

fn clean_declared_local_path(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .trim()
        .replace('\\', "/");
    if trimmed.is_empty()
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("//")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("mailto:")
        || trimmed.starts_with('#')
        || trimmed.starts_with('$')
        || trimmed.contains('*')
        || trimmed.contains("..")
    {
        return None;
    }
    let clean = trimmed
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string();
    if clean.is_empty() {
        return None;
    }
    Some(clean)
}

fn check_path_exists_with_optional_ext(
    project_root: &Path,
    clean_path: &str,
    allow_json_ext: bool,
) -> bool {
    let candidate = project_root.join(clean_path.trim_end_matches('/'));
    if candidate.exists() {
        return true;
    }
    if allow_json_ext && !clean_path.ends_with(".json") {
        let with_json = project_root.join(format!("{clean_path}.json"));
        if with_json.exists() {
            return true;
        }
    }
    false
}

fn collect_json_declared_paths(
    project_root: &Path,
    source_file: &str,
    key: &str,
    val: &serde_json::Value,
    seen: &mut HashSet<String>,
    out: &mut Vec<BrokenDeclaredPath>,
) {
    match val {
        serde_json::Value::String(s) => {
            let raw = s.trim();
            // Ignorer les identifiants nus (ex. "lodash", "node:fs") ou presets npm scoped sans début ./
            let looks_local = raw.starts_with("./")
                || raw.starts_with('/')
                || raw.ends_with('/')
                || (!raw.starts_with('@') && (raw.contains('/') || raw.contains('.')));
            if !looks_local {
                return;
            }
            if let Some(clean) = clean_declared_local_path(raw) {
                if is_ignored_output_dir(&clean) {
                    return;
                }
                let allow_json_ext = key == "extends";
                if !check_path_exists_with_optional_ext(project_root, &clean, allow_json_ext) {
                    let dedup_key = format!("{source_file}:{raw}");
                    if seen.insert(dedup_key) {
                        out.push(BrokenDeclaredPath {
                            source_file: source_file.to_string(),
                            declared_path: raw.to_string(),
                            reason: format!(
                                "clé `{key}` déclare `{raw}` (introuvable sur disque)"
                            ),
                        });
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_json_declared_paths(project_root, source_file, key, item, seen, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map {
                collect_json_declared_paths(project_root, source_file, key, v, seen, out);
            }
        }
        _ => {}
    }
}

fn scan_json_value_for_path_keys(
    project_root: &Path,
    source_file: &str,
    val: &serde_json::Value,
    seen: &mut HashSet<String>,
    out: &mut Vec<BrokenDeclaredPath>,
) {
    let Some(obj) = val.as_object() else {
        return;
    };
    const PATH_KEYS: &[&str] = &[
        "main",
        "module",
        "bin",
        "entry",
        "source",
        "files",
        "extends",
        "psr-4",
        "psr-0",
    ];
    for (k, v) in obj {
        let low_k = k.to_lowercase();
        if PATH_KEYS.iter().any(|pk| *pk == low_k) {
            collect_json_declared_paths(project_root, source_file, k, v, seen, out);
        } else if v.is_object() {
            // Descente limitée pour autoload -> psr-4 / composer.json, compilerOptions, etc.
            scan_json_value_for_path_keys(project_root, source_file, v, seen, out);
        }
    }
}

fn verify_declared_paths(
    project_root: &Path,
    manifests: &[RootManifest],
) -> Vec<BrokenDeclaredPath> {
    let mut broken = Vec::new();
    let mut seen = HashSet::new();

    let toml_path_re =
        Regex::new(r#"(?m)^\s*(path|readme)\s*=\s*["']([^"']+)["']"#).expect("valid toml path regex");
    let html_script_re =
        Regex::new(r#"(?i)<script\b[^>]*\bsrc\s*=\s*["']([^"']+)["']"#).expect("valid script regex");
    let html_link_re =
        Regex::new(r#"(?i)<link\b[^>]*\bhref\s*=\s*["']([^"']+)["']"#).expect("valid link regex");

    for manifest in manifests {
        let low = manifest.path.to_lowercase();
        if low.ends_with(".json") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&manifest.content) {
                scan_json_value_for_path_keys(
                    project_root,
                    &manifest.path,
                    &json,
                    &mut seen,
                    &mut broken,
                );
            }
        } else if low.ends_with(".toml") {
            for cap in toml_path_re.captures_iter(&manifest.content) {
                let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) else {
                    continue;
                };
                let raw = v.as_str().trim();
                if let Some(clean) = clean_declared_local_path(raw) {
                    if !is_ignored_output_dir(&clean) && !project_root.join(&clean).exists() {
                        let dedup_key = format!("{}:{raw}", manifest.path);
                        if seen.insert(dedup_key) {
                            broken.push(BrokenDeclaredPath {
                                source_file: manifest.path.clone(),
                                declared_path: raw.to_string(),
                                reason: format!(
                                    "clé `{}` déclare `{raw}` (introuvable sur disque)",
                                    k.as_str()
                                ),
                            });
                        }
                    }
                }
            }
        } else if low.ends_with(".html") || low.ends_with(".htm") {
            for (tag, re) in [("<script src>", &html_script_re), ("<link href>", &html_link_re)] {
                for cap in re.captures_iter(&manifest.content) {
                    let Some(m) = cap.get(1) else {
                        continue;
                    };
                    let raw = m.as_str().trim();
                    if raw.to_lowercase().contains("favicon.") {
                        continue;
                    }
                    if let Some(clean) = clean_declared_local_path(raw) {
                        if !is_ignored_output_dir(&clean) && !project_root.join(&clean).exists() {
                            let dedup_key = format!("{}:{raw}", manifest.path);
                            if seen.insert(dedup_key) {
                                broken.push(BrokenDeclaredPath {
                                    source_file: manifest.path.clone(),
                                    declared_path: raw.to_string(),
                                    reason: format!(
                                        "{tag} référence `{raw}` (introuvable sur disque)"
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    broken
}

pub fn build_workspace_snapshot(project_root: &Path, is_cloud: bool) -> WorkspaceSnapshot {
    let all_files = crate::repo_map::list_project_files(project_root, 1000);
    if !project_root.exists() {
        return WorkspaceSnapshot {
            all_files,
            ..Default::default()
        };
    }

    let mut root_candidates: Vec<String> = all_files
        .iter()
        .filter(|rel| is_root_manifest_or_config(rel))
        .cloned()
        .collect();
    root_candidates.sort_by_key(|rel| manifest_priority(rel));

    let max_manifests = 8usize;
    let max_single_chars = 6_000usize;
    let max_total_chars = if is_cloud { 20_000usize } else { 10_000usize };

    let mut root_manifests = Vec::new();
    let mut total_chars = 0usize;

    for rel in root_candidates {
        if root_manifests.len() >= max_manifests || total_chars >= max_total_chars {
            break;
        }
        let full = project_root.join(&rel);
        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if !meta.is_file() || meta.len() > 8_192 {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&full) else {
            continue;
        };
        let clipped = truncate_chars_safe(&raw, max_single_chars);
        let len = clipped.chars().count();
        if total_chars > 0 && total_chars + len > max_total_chars {
            continue;
        }
        total_chars += len;
        root_manifests.push(RootManifest {
            path: rel,
            content: clipped,
        });
    }

    let declared_scripts = extract_declared_scripts(project_root, &root_manifests);
    let broken_declared_paths = verify_declared_paths(project_root, &root_manifests);

    WorkspaceSnapshot {
        all_files,
        root_manifests,
        declared_scripts,
        broken_declared_paths,
    }
}

impl WorkspaceSnapshot {
    pub fn format_for_prompt(&self, is_cloud: bool) -> String {
        let mut sections = Vec::new();

        if self.all_files.is_empty() {
            sections.push("[Arborescence complète du projet (0 fichier indexé)]".to_string());
        } else {
            let max_tree = if is_cloud { 300usize } else { 150usize };
            let shown: Vec<&str> = self
                .all_files
                .iter()
                .take(max_tree)
                .map(String::as_str)
                .collect();
            let mut tree = shown.join("\n  ");
            if self.all_files.len() > max_tree {
                tree.push_str(&format!(
                    "\n  ... (+{} autres fichiers)",
                    self.all_files.len() - max_tree
                ));
            }
            sections.push(format!(
                "[Arborescence complète du projet ({} fichiers)]\n  {}",
                self.all_files.len(),
                tree
            ));
        }

        if !self.root_manifests.is_empty() {
            let mut m_blocks = Vec::new();
            for m in &self.root_manifests {
                m_blocks.push(format!("### {}\n```\n{}\n```", m.path, m.content.trim_end()));
            }
            sections.push(format!(
                "[Manifestes & Configurations racine]\n{}",
                m_blocks.join("\n\n")
            ));
        }

        if !self.declared_scripts.is_empty() {
            let script_lines: Vec<String> = self
                .declared_scripts
                .iter()
                .take(20)
                .map(|s| format!("- {}: `{}` -> `{}` ({})", s.source, s.name, s.runner_cmd, s.command))
                .collect();
            sections.push(format!(
                "[Scripts déclarés dans le projet]\n{}",
                script_lines.join("\n")
            ));
        }

        if !self.broken_declared_paths.is_empty() {
            let broken_lines: Vec<String> = self
                .broken_declared_paths
                .iter()
                .map(|b| format!("- `{}` : {}", b.source_file, b.reason))
                .collect();
            sections.push(format!(
                "[⚠️ Chemins déclarés introuvables sur disque]\n{}",
                broken_lines.join("\n")
            ));
        }

        sections.join("\n\n")
    }
}

pub fn is_dummy_or_unspecified_test(command: &str) -> bool {
    let low = command.to_lowercase();
    low.contains("no test specified")
}

pub fn is_safe_verification_command(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    if crate::tools::shell::is_daemon_or_dev_server_command(trimmed).is_some() {
        return false;
    }
    let low = trimmed.to_lowercase();
    if low.contains("--watch")
        || low.contains(" watch")
        || low == "watch"
        || low.ends_with(" serve")
        || low.contains(" serve ")
    {
        return false;
    }
    true
}

pub fn select_declared_verification_script(snapshot: &WorkspaceSnapshot) -> Option<DeclaredScript> {
    let priority_names = [
        "typecheck",
        "type-check",
        "check",
        "build",
        "test",
        "lint",
    ];
    for target_name in priority_names {
        for script in &snapshot.declared_scripts {
            if script.name.eq_ignore_ascii_case(target_name)
                && is_safe_verification_command(&script.command)
                && is_safe_verification_command(&script.runner_cmd)
            {
                if is_dummy_or_unspecified_test(&script.command) {
                    continue;
                }
                return Some(script.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn make_temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vaix_ws_{}_{}", label, Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_snapshot_js_html_missing_entry_and_declared_build_script() {
        let dir = make_temp_dir("js_html");
        std::fs::write(
            dir.join("index.html"),
            r#"<!DOCTYPE html><html><body><div id="app"></div><script type="module" src="/src/main.ts"></script></body></html>"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"demo","scripts":{"dev":"vite","build":"vite build"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();

        let snap = build_workspace_snapshot(&dir, true);
        assert!(snap
            .broken_declared_paths
            .iter()
            .any(|b| b.source_file == "index.html" && b.declared_path == "/src/main.ts"));

        let selected = select_declared_verification_script(&snap).unwrap();
        assert_eq!(selected.name, "build");
        assert_eq!(selected.runner_cmd, "pnpm run build");

        let rendered = snap.format_for_prompt(true);
        assert!(rendered.contains("[Manifestes & Configurations racine]"));
        assert!(rendered.contains("[⚠️ Chemins déclarés introuvables sur disque]"));
        assert!(rendered.contains("pnpm run build"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_snapshot_php_composer_missing_autoload_and_test_script() {
        let dir = make_temp_dir("php_composer");
        std::fs::write(
            dir.join("composer.json"),
            r#"{"name":"acme/app","autoload":{"psr-4":{"App\\":"src/"}},"scripts":{"test":"phpunit"}}"#,
        )
        .unwrap();

        let snap = build_workspace_snapshot(&dir, true);
        assert!(snap
            .broken_declared_paths
            .iter()
            .any(|b| b.source_file == "composer.json" && b.declared_path == "src/"));

        let selected = select_declared_verification_script(&snap).unwrap();
        assert_eq!(selected.name, "test");
        assert_eq!(selected.runner_cmd, "composer run-script test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_snapshot_rust_cargo_missing_bin_path() {
        let dir = make_temp_dir("rust_cargo");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"cli\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"cli\"\npath = \"src/bin/cli.rs\"\n",
        )
        .unwrap();

        let snap = build_workspace_snapshot(&dir, true);
        assert!(snap
            .broken_declared_paths
            .iter()
            .any(|b| b.source_file == "Cargo.toml" && b.declared_path == "src/bin/cli.rs"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_snapshot_makefile_selects_test_and_ignores_dev() {
        let dir = make_temp_dir("go_makefile");
        std::fs::write(dir.join("go.mod"), "module example.com/app\ngo 1.22\n").unwrap();
        std::fs::write(
            dir.join("Makefile"),
            ".PHONY: dev test\ndev:\n\tgo run ./cmd/server\n\ntest:\n\tgo test ./...\n",
        )
        .unwrap();

        let snap = build_workspace_snapshot(&dir, true);
        assert!(snap.root_manifests.iter().any(|m| m.path == "go.mod"));
        assert!(snap.root_manifests.iter().any(|m| m.path == "Makefile"));

        let selected = select_declared_verification_script(&snap).unwrap();
        assert_eq!(selected.name, "test");
        assert_eq!(selected.runner_cmd, "make test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_snapshot_skips_dummy_test_and_prefers_build() {
        let dir = make_temp_dir("dummy_test_package");
        std::fs::write(
            dir.join("package.json"),
            r#"{
  "name": "my-app",
  "scripts": {
    "test": "echo \"Error: no test specified\" && exit 1",
    "build": "vite build"
  }
}"#,
        )
        .unwrap();

        let snap = build_workspace_snapshot(&dir, true);
        let selected = select_declared_verification_script(&snap).unwrap();
        assert_eq!(selected.name, "build");
        assert!(selected.command.contains("vite build"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
