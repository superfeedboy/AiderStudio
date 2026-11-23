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
    pub app_files_count: usize,
    pub is_uninitialized: bool,
    pub detected_ecosystems: Vec<String>,
    pub detected_package_managers: Vec<String>,
    pub environment_notes: Vec<String>,
}

/// Indique si un fichier relatif appartient uniquement aux métadonnées d'outillage IA ou Git
/// et ne constitue donc pas du code ou une configuration applicative du projet.
pub fn is_internal_meta_file(rel: &str) -> bool {
    let clean = rel.trim().replace('\\', "/");
    let clean = clean.trim_start_matches("./");
    let low = clean.to_lowercase();
    matches!(
        low.as_str(),
        ".aiderignore" | "agents.md" | "claude.md" | ".gitignore" | ".gitattributes"
    ) || low.starts_with(".agents/")
        || low.starts_with(".agent/")
        || low.starts_with(".vaix/")
        || low.starts_with(".aider/")
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

fn detect_js_package_manager_name(project_root: &Path) -> &'static str {
    if project_root.join("pnpm-lock.yaml").exists() {
        return "pnpm";
    }
    if project_root.join("yarn.lock").exists() {
        return "yarn";
    }
    if project_root.join("bun.lockb").exists() || project_root.join("bun.lock").exists() {
        return "bun";
    }
    if project_root.join("package-lock.json").exists() {
        return "npm";
    }
    if let Ok(pkg_raw) = std::fs::read_to_string(project_root.join("package.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&pkg_raw) {
            if let Some(pm) = json.get("packageManager").and_then(|v| v.as_str()) {
                let low = pm.to_lowercase();
                if low.starts_with("pnpm") {
                    return "pnpm";
                }
                if low.starts_with("yarn") {
                    return "yarn";
                }
                if low.starts_with("bun") {
                    return "bun";
                }
            }
        }
    }
    "npm"
}

fn detect_js_package_runner(project_root: &Path) -> &'static str {
    match detect_js_package_manager_name(project_root) {
        "pnpm" => "pnpm run",
        "yarn" => "yarn",
        "bun" => "bun run",
        _ => "npm run",
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
        "types",
        "typings",
        "files",
        "extends",
        "include",
        "exclude",
        "outdir",
        "rootdir",
        "output",
        "assets",
        "static",
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
                        if !project_root.join(&clean).exists() {
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

fn json_object_has_entries(val: Option<&serde_json::Value>) -> bool {
    val.and_then(|v| v.as_object())
        .map(|m| !m.is_empty())
        .unwrap_or(false)
}

fn push_unique_str(vec: &mut Vec<String>, item: &str) {
    if !item.is_empty() && !vec.iter().any(|existing| existing == item) {
        vec.push(item.to_string());
    }
}

struct DeclarativeEcosystem {
    name: &'static str,
    manifest_files: &'static [&'static str],
    file_extensions: &'static [&'static str],
    lockfile_managers: &'static [(&'static str, &'static str)],
    default_manager: &'static str,
    vendor_dir: Option<&'static str>,
    manifest_for_vendor: Option<&'static str>,
}

static DECLARATIVE_ECOSYSTEMS: &[DeclarativeEcosystem] = &[
    DeclarativeEcosystem {
        name: "Rust",
        manifest_files: &["Cargo.toml"],
        file_extensions: &["rs"],
        lockfile_managers: &[],
        default_manager: "cargo",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "Python",
        manifest_files: &["pyproject.toml", "requirements.txt", "Pipfile"],
        file_extensions: &["py"],
        lockfile_managers: &[
            ("uv.lock", "uv"),
            ("poetry.lock", "poetry"),
            ("Pipfile", "pipenv"),
        ],
        default_manager: "pip",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "Go",
        manifest_files: &["go.mod"],
        file_extensions: &["go"],
        lockfile_managers: &[],
        default_manager: "go",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "PHP",
        manifest_files: &["composer.json"],
        file_extensions: &["php"],
        lockfile_managers: &[],
        default_manager: "composer",
        vendor_dir: Some("vendor"),
        manifest_for_vendor: Some("composer.json"),
    },
    DeclarativeEcosystem {
        name: "Ruby",
        manifest_files: &["Gemfile"],
        file_extensions: &["rb"],
        lockfile_managers: &[("Gemfile.lock", "bundle")],
        default_manager: "bundler",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "Java / JVM",
        manifest_files: &["pom.xml"],
        file_extensions: &["java", "scala"],
        lockfile_managers: &[],
        default_manager: "maven",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "Java / Kotlin",
        manifest_files: &["build.gradle", "build.gradle.kts"],
        file_extensions: &["kt", "kts"],
        lockfile_managers: &[],
        default_manager: "gradle",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "Dart / Flutter",
        manifest_files: &["pubspec.yaml"],
        file_extensions: &["dart"],
        lockfile_managers: &[],
        default_manager: "flutter/dart",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: ".NET / C#",
        manifest_files: &[],
        file_extensions: &["csproj", "sln", "cs", "fs"],
        lockfile_managers: &[],
        default_manager: "dotnet",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "C / C++ (CMake)",
        manifest_files: &["CMakeLists.txt"],
        file_extensions: &["c", "cpp", "cc", "cxx", "h", "hpp"],
        lockfile_managers: &[],
        default_manager: "cmake",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "Elixir",
        manifest_files: &["mix.exs"],
        file_extensions: &["ex", "exs"],
        lockfile_managers: &[("mix.lock", "mix")],
        default_manager: "mix",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
    DeclarativeEcosystem {
        name: "Zig",
        manifest_files: &["build.zig"],
        file_extensions: &["zig"],
        lockfile_managers: &[],
        default_manager: "zig",
        vendor_dir: None,
        manifest_for_vendor: None,
    },
];

fn analyze_project_environment(
    project_root: &Path,
    all_files: &[String],
    manifests: &[RootManifest],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut ecosystems = Vec::new();
    let mut managers = Vec::new();
    let mut notes = Vec::new();

    let has_file = |name: &str| {
        all_files.iter().any(|f| f.eq_ignore_ascii_case(name))
            || project_root.join(name).is_file()
    };
    let has_ext = |ext: &str| {
        let suffix = format!(".{}", ext.to_lowercase());
        all_files.iter().any(|f| f.to_lowercase().ends_with(&suffix))
    };

    // 1. Écosystème JavaScript / TypeScript
    if has_file("package.json")
        || has_file("tsconfig.json")
        || all_files.iter().any(|f| f.to_lowercase().contains("vite.config."))
    {
        let pm = detect_js_package_manager_name(project_root);
        let is_ts = has_file("tsconfig.json") || has_ext("ts") || has_ext("tsx");
        push_unique_str(
            &mut ecosystems,
            if is_ts {
                "Node.js / TypeScript"
            } else {
                "Node.js / JavaScript"
            },
        );
        push_unique_str(&mut managers, pm);

        if let Some(pkg) = manifests.iter().find(|m| m.path.eq_ignore_ascii_case("package.json")) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&pkg.content) {
                let has_deps = json_object_has_entries(json.get("dependencies"))
                    || json_object_has_entries(json.get("devDependencies"));
                if has_deps && !project_root.join("node_modules").exists() {
                    notes.push(format!(
                        "`package.json` déclare des dépendances mais `node_modules/` est absent : exécuter `{pm} install` via `run_command`."
                    ));
                }
            }
        }
    }

    // 2. Écosystèmes déclaratifs unifiés
    for eco in DECLARATIVE_ECOSYSTEMS {
        let matched_manifest = eco.manifest_files.iter().any(|m| has_file(m));
        let matched_ext = eco.file_extensions.iter().any(|ext| has_ext(ext));

        if matched_manifest || matched_ext {
            push_unique_str(&mut ecosystems, eco.name);

            // Détection du gestionnaire de paquets spécifique par lockfile
            let detected_manager = eco
                .lockfile_managers
                .iter()
                .find(|(lock, _)| has_file(lock))
                .map(|(_, mgr)| *mgr)
                .unwrap_or(eco.default_manager);

            push_unique_str(&mut managers, detected_manager);

            // Vérification de répertoire de dépendances si applicable
            if let (Some(vdir), Some(mfile)) = (eco.vendor_dir, eco.manifest_for_vendor) {
                if has_file(mfile) && !project_root.join(vdir).exists() {
                    if let Some(manifest) = manifests.iter().find(|m| m.path.eq_ignore_ascii_case(mfile)) {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&manifest.content) {
                            let has_reqs = json_object_has_entries(json.get("require"))
                                || json_object_has_entries(json.get("require-dev"));
                            if has_reqs {
                                notes.push(format!(
                                    "`{mfile}` déclare des dépendances mais `{vdir}/` est absent : exécuter `{detected_manager} install` via `run_command`."
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Fallback site web statique pur
    if has_file("index.html") && ecosystems.is_empty() {
        push_unique_str(&mut ecosystems, "Web Statique (HTML/CSS/JS)");
    }

    (ecosystems, managers, notes)
}

pub fn build_workspace_snapshot(project_root: &Path, is_cloud: bool) -> WorkspaceSnapshot {
    let all_files = crate::repo_map::list_project_files(project_root, 1000);
    if !project_root.exists() {
        return WorkspaceSnapshot {
            all_files,
            is_uninitialized: true,
            ..Default::default()
        };
    }

    let mut root_candidates: Vec<String> = all_files
        .iter()
        .filter(|rel| is_root_manifest_or_config(rel) && !is_internal_meta_file(rel))
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

    let app_files_count = all_files.iter().filter(|f| !is_internal_meta_file(f)).count();
    let is_uninitialized = app_files_count == 0 && root_manifests.is_empty();
    let (detected_ecosystems, detected_package_managers, environment_notes) =
        analyze_project_environment(project_root, &all_files, &root_manifests);

    let declared_scripts = extract_declared_scripts(project_root, &root_manifests);
    let broken_declared_paths = verify_declared_paths(project_root, &root_manifests);

    WorkspaceSnapshot {
        all_files,
        root_manifests,
        declared_scripts,
        broken_declared_paths,
        app_files_count,
        is_uninitialized,
        detected_ecosystems,
        detected_package_managers,
        environment_notes,
    }
}

impl WorkspaceSnapshot {
    pub fn format_for_prompt(&self, is_cloud: bool) -> String {
        let mut sections = Vec::new();

        if self.is_uninitialized {
            sections.push(
                "[État & Diagnostic Déterministe du Projet]\n\
                STATUT : PROJET VIERGE / NON INITIALISÉ (0 fichier applicatif, 0 manifeste technique).\n\
                DIRECTIVE : Toute création de projet, site web, application ou API DOIT commencer par préparer/initialiser l'environnement et installer les dépendances requises via `run_command` (en mode strictement non-interactif selon l'outil demandé par l'utilisateur ou le contexte du projet) ET créer les manifestes et fichiers sources complets via `write_file`."
                    .to_string(),
            );
        } else {
            let mut diag_lines = Vec::new();
            if !self.detected_ecosystems.is_empty() {
                diag_lines.push(format!(
                    "- Écosystème(s) détecté(s) : {}",
                    self.detected_ecosystems.join(", ")
                ));
            }
            if !self.detected_package_managers.is_empty() {
                diag_lines.push(format!(
                    "- Gestionnaire(s) de paquets / build détecté(s) : {}",
                    self.detected_package_managers.join(", ")
                ));
            }
            for note in &self.environment_notes {
                diag_lines.push(format!("- ⚠️ {note}"));
            }
            if !diag_lines.is_empty() {
                sections.push(format!(
                    "[État & Diagnostic Déterministe du Projet]\n{}",
                    diag_lines.join("\n")
                ));
            }
        }

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

/// Déduit une commande de vérification déterministe et non-bloquante à partir des scripts déclarés
/// ou des manifestes standards du projet, quel que soit le langage.
pub fn detect_project_verification_command(
    project_root: &Path,
    snapshot: &WorkspaceSnapshot,
) -> Option<String> {
    if let Some(script) = select_declared_verification_script(snapshot) {
        return Some(script.runner_cmd);
    }
    if project_root.join("Cargo.toml").is_file() {
        return Some("cargo check".to_string());
    }
    if project_root.join("go.mod").is_file() {
        return Some("go build ./...".to_string());
    }
    if project_root.join("composer.json").is_file() {
        return Some("composer validate --no-check-publish".to_string());
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
            r#"<!DOCTYPE html><html><head><link rel="stylesheet" href="/dist/tailwind.css"></head><body><div id="app"></div><script type="module" src="/src/main.ts"></script></body></html>"#,
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
        assert!(snap
            .broken_declared_paths
            .iter()
            .any(|b| b.source_file == "index.html" && b.declared_path == "/dist/tailwind.css"));

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

    #[test]
    fn test_snapshot_detects_uninitialized_project_despite_meta_files() {
        let dir = make_temp_dir("uninit_meta");
        std::fs::write(dir.join(".aiderignore"), "node_modules/\n").unwrap();
        std::fs::write(dir.join("AGENTS.md"), "# Directives\n").unwrap();
        std::fs::create_dir_all(dir.join(".agents/rules")).unwrap();
        std::fs::write(dir.join(".agents/rules/validation.md"), "# Validation\n").unwrap();

        let snap = build_workspace_snapshot(&dir, true);
        assert!(snap.is_uninitialized);
        assert_eq!(snap.app_files_count, 0);
        let prompt_str = snap.format_for_prompt(true);
        assert!(prompt_str.contains("PROJET VIERGE / NON INITIALISÉ"));

        // Ensure package.json triggers ecosystem & missing node_modules note
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"web","packageManager":"pnpm@9.0.0","dependencies":{"vue":"^3.5.0"}}"#,
        )
        .unwrap();
        let snap2 = build_workspace_snapshot(&dir, true);
        assert!(!snap2.is_uninitialized);
        assert!(snap2.detected_package_managers.iter().any(|m| m == "pnpm"));
        assert!(snap2.environment_notes.iter().any(|n| n.contains("pnpm install")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
