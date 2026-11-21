use regex::{Regex, RegexBuilder};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
struct IgnoreRule {
    regex: Regex,
    negated: bool,
    dir_only: bool,
}

#[derive(Debug, Clone)]
pub struct IgnoreRules {
    rules: Vec<IgnoreRule>,
    has_negations: bool,
}

fn glob_to_regex_str(pattern: &str, anchored: bool) -> String {
    let mut re = String::new();
    if anchored {
        re.push('^');
    } else {
        re.push_str("(?:^|/)");
    }

    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' {
            if i + 1 < chars.len() && chars[i + 1] == '*' {
                if i + 2 < chars.len() && chars[i + 2] == '/' {
                    re.push_str("(?:.+/)?");
                    i += 3;
                    continue;
                }
                re.push_str(".*");
                i += 2;
                continue;
            }
            re.push_str("[^/]*");
        } else if c == '?' {
            re.push_str("[^/]");
        } else if matches!(
            c,
            '.' | '+' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\'
        ) {
            re.push('\\');
            re.push(c);
        } else {
            re.push(c);
        }
        i += 1;
    }

    re.push_str("(?:$|/)");
    re
}

impl IgnoreRules {
    pub fn load(project_root: &Path) -> Self {
        let mut rules = Vec::new();
        let mut has_negations = false;

        for file_name in [".gitignore", ".aiderignore", ".studioignore"] {
            let p = project_root.join(file_name);
            let Ok(content) = std::fs::read_to_string(&p) else {
                continue;
            };
            for raw_line in content.lines() {
                let line = raw_line.trim_end_matches('\r').trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let (negated, pat_after_bang) = if let Some(rest) = line.strip_prefix('!') {
                    has_negations = true;
                    (true, rest.trim())
                } else {
                    (false, line)
                };
                if pat_after_bang.is_empty() {
                    continue;
                }

                let dir_only = pat_after_bang.ends_with('/');
                let pat_no_trailing = pat_after_bang.trim_end_matches('/');
                let anchored = pat_no_trailing.starts_with('/') || pat_no_trailing.contains('/');
                let clean_pat = pat_no_trailing.trim_start_matches('/');
                if clean_pat.is_empty() {
                    continue;
                }

                let regex_str = glob_to_regex_str(clean_pat, anchored);
                if let Ok(regex) = Regex::new(&regex_str) {
                    rules.push(IgnoreRule {
                        regex,
                        negated,
                        dir_only,
                    });
                }
            }
        }

        Self {
            rules,
            has_negations,
        }
    }

    pub fn is_hard_excluded_component(name: &str) -> bool {
        let low = name.to_lowercase();
        if matches!(
            low.as_str(),
            ".git" | ".svn" | ".hg" | "node_modules" | "target" | "dist" | "build" | "coverage"
        ) {
            return true;
        }
        if name.starts_with('.') && name != ".github" {
            return true;
        }
        false
    }

    pub fn is_hard_excluded_file(rel_path: &str) -> bool {
        let low = rel_path.to_lowercase();
        let base = low.rsplit('/').next().unwrap_or(&low);
        if matches!(
            base,
            "package-lock.json"
                | "pnpm-lock.yaml"
                | "yarn.lock"
                | "cargo.lock"
                | ".ds_store"
                | "thumbs.db"
        ) || base == ".env"
            || base.starts_with(".env.")
        {
            return true;
        }

        let excluded_suffixes = [
            ".min.js", ".min.css", ".map", ".lock", ".log", ".png", ".jpg", ".jpeg", ".gif",
            ".webp", ".ico", ".icns", ".woff", ".woff2", ".ttf", ".eot", ".pdf", ".mov", ".mp4",
            ".mp3", ".zip", ".tar", ".gz", ".wasm", ".sqlite", ".sqlite3", ".db", ".exe", ".dll",
            ".so", ".dylib", ".rlib", ".a", ".o",
        ];
        excluded_suffixes.iter().any(|ext| low.ends_with(ext))
    }

    pub fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
        let norm = rel_path.trim_start_matches("./").replace('\\', "/");
        if norm.is_empty() {
            return false;
        }
        for comp in norm.split('/') {
            if Self::is_hard_excluded_component(comp) {
                return true;
            }
        }
        if !is_dir && Self::is_hard_excluded_file(&norm) {
            return true;
        }

        let mut ignored = false;
        for rule in &self.rules {
            if rule.dir_only && !is_dir {
                // Si c'est un fichier, la règle dossier matche si un préfixe dossier correspond
                if let Some((parent, _)) = norm.rsplit_once('/') {
                    if rule.regex.is_match(parent) {
                        ignored = !rule.negated;
                    }
                }
                continue;
            }
            if rule.regex.is_match(&norm) {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

/// Liste les fichiers sources du projet en respectant `.gitignore`, `.aiderignore` et `.studioignore`.
pub fn list_project_files(project_root: &Path, max_files: usize) -> Vec<String> {
    let mut results = Vec::new();
    if !project_root.exists() {
        return results;
    }

    let rules = IgnoreRules::load(project_root);
    let cap = if max_files == 0 { 1000 } else { max_files };

    fn walk(
        dir: &Path,
        root: &Path,
        rules: &IgnoreRules,
        cap: usize,
        list: &mut Vec<String>,
    ) {
        if list.len() >= cap {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if list.len() >= cap {
                break;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if IgnoreRules::is_hard_excluded_component(&name_str) {
                continue;
            }

            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");

            if path.is_dir() {
                if !rules.has_negations && rules.is_ignored(&rel_str, true) {
                    continue;
                }
                walk(&path, root, rules, cap, list);
            } else if path.is_file() && !rules.is_ignored(&rel_str, false) {
                list.push(rel_str);
            }
        }
    }

    walk(project_root, project_root, &rules, cap, &mut results);
    results.sort();
    results
}

/// Recherche rapide dans les fichiers sources du projet (outil `search_code`).
pub fn search_in_project(
    project_root: &Path,
    query: &str,
    path_filter: Option<&str>,
    max_results: usize,
) -> Result<String, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("Paramètre 'query' vide.".to_string());
    }

    let re = RegexBuilder::new(trimmed)
        .case_insensitive(true)
        .build()
        .or_else(|_| {
            RegexBuilder::new(&regex::escape(trimmed))
                .case_insensitive(true)
                .build()
        })
        .map_err(|e| format!("Motif de recherche invalide : {e}"))?;

    let cap = if max_results == 0 { 40 } else { max_results };
    let filter_low = path_filter
        .map(|s| s.trim().replace('\\', "/").to_lowercase())
        .filter(|s| !s.is_empty());

    let files = list_project_files(project_root, 2000);
    let mut matches = Vec::new();

    for rel in files {
        if matches.len() >= cap {
            break;
        }
        if let Some(ref pf) = filter_low {
            if !rel.to_lowercase().contains(pf) {
                continue;
            }
        }

        let full = project_root.join(&rel);
        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if meta.len() > 262_144 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&full) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let content = String::from_utf8_lossy(&bytes);
        for (idx, line) in content.lines().enumerate() {
            if re.is_match(line) {
                let trimmed_line = line.trim();
                let snippet: String = if trimmed_line.chars().count() > 160 {
                    let s: String = trimmed_line.chars().take(160).collect();
                    format!("{s}…")
                } else {
                    trimmed_line.to_string()
                };
                matches.push(format!("{}:{}: {}", rel, idx + 1, snippet));
                if matches.len() >= cap {
                    break;
                }
            }
        }
    }

    if matches.is_empty() {
        Ok(format!("Aucun résultat trouvé pour '{}'.", trimmed))
    } else {
        Ok(matches.join("\n"))
    }
}

fn truncate_sig(s: &str, max_chars: usize) -> String {
    let clean = s
        .trim()
        .trim_end_matches('{')
        .trim_end_matches(':')
        .trim();
    if clean.chars().count() <= max_chars {
        clean.to_string()
    } else {
        let cut: String = clean.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}

/// Extrait jusqu'à 16 signatures clés d'un fichier selon son extension.
pub fn extract_file_signatures(rel_path: &str, content: &str) -> Vec<String> {
    let ext = rel_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let base_low = rel_path
        .rsplit('/')
        .next()
        .unwrap_or(rel_path)
        .to_lowercase();

    let mut sigs = Vec::new();
    let max_sigs = if matches!(ext.as_str(), "css" | "scss") {
        8usize
    } else {
        16usize
    };

    for raw_line in content.lines() {
        if sigs.len() >= max_sigs {
            break;
        }
        let t = raw_line.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }

        let matched = match ext.as_str() {
            "rs" => {
                !t.starts_with('#')
                    && (t.starts_with("pub fn ")
                        || t.starts_with("pub async fn ")
                        || t.starts_with("fn ")
                        || t.starts_with("async fn ")
                        || t.starts_with("pub struct ")
                        || t.starts_with("struct ")
                        || t.starts_with("pub enum ")
                        || t.starts_with("enum ")
                        || t.starts_with("pub trait ")
                        || t.starts_with("trait ")
                        || t.starts_with("impl ")
                        || t.starts_with("impl<")
                        || t.starts_with("pub mod ")
                        || t.starts_with("pub type "))
            }
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "mts" => {
                t.starts_with("export ")
                    || t.starts_with("function ")
                    || t.starts_with("async function ")
                    || t.starts_with("class ")
                    || t.starts_with("interface ")
                    || t.starts_with("type ")
                    || t.starts_with("enum ")
                    || ((t.starts_with("const ") || t.starts_with("async const "))
                        && (t.contains("=>") || t.contains("defineStore")))
            }
            "vue" => {
                t.starts_with("import ")
                    || t.starts_with("export default")
                    || t.starts_with("<style")
                    || t.starts_with("defineProps")
                    || t.starts_with("defineEmits")
                    || t.starts_with("function ")
                    || t.starts_with("async function ")
                    || t.starts_with("interface ")
                    || t.starts_with("type ")
                    || (t.starts_with("const ")
                        && (t.contains("= ref")
                            || t.contains("= computed")
                            || t.contains("=>")))
            }
            "css" | "scss" => {
                t.starts_with("@import")
                    || t.starts_with("@tailwind")
                    || t.starts_with(":root")
                    || ((t.starts_with('.')
                        || t.starts_with('#')
                        || t.starts_with("html")
                        || t.starts_with("body"))
                        && t.contains('{'))
            }
            "html" => {
                t.starts_with("<title") || t.starts_with("<link ") || t.starts_with("<script ")
            }
            "json" if base_low == "package.json" => {
                t.starts_with("\"name\":")
                    || [
                        "\"vue\"",
                        "\"react\"",
                        "\"svelte\"",
                        "\"next\"",
                        "\"nuxt\"",
                        "\"vite\"",
                        "\"tailwindcss\"",
                        "\"@tailwindcss/vite\"",
                        "\"@tailwindcss/postcss\"",
                        "\"postcss\"",
                    ]
                    .iter()
                    .any(|dep| t.contains(dep))
            }
            "py" => {
                !t.starts_with('#')
                    && (t.starts_with("def ") || t.starts_with("async def ") || t.starts_with("class "))
            }
            "go" => {
                t.starts_with("func ")
                    || (t.starts_with("type ") && (t.contains(" struct") || t.contains(" interface")))
            }
            "java" | "kt" | "cs" | "swift" | "php" | "c" | "cpp" | "h" | "hpp" => {
                !t.starts_with('#')
                    && (t.starts_with("class ")
                        || t.starts_with("interface ")
                        || t.starts_with("struct ")
                        || t.starts_with("enum ")
                        || t.starts_with("func ")
                        || t.starts_with("public ")
                        || t.starts_with("private ")
                        || t.starts_with("protected "))
            }
            "md" => t.starts_with("# ") || t.starts_with("## ") || t.starts_with("### "),
            _ => false,
        };

        if matched {
            let formatted = truncate_sig(t, 92);
            if !formatted.is_empty() && !sigs.contains(&formatted) {
                sigs.push(formatted);
            }
        }
    }

    sigs
}

/// Construit une Repo-Map légère et budgétée en tokens pour orienter le modèle sans saturer la fenêtre.
pub fn build_repo_map(
    project_root: &Path,
    all_files: &[String],
    target_files: &[String],
    read_only_files: &[String],
    user_prompt: &str,
    map_tokens: usize,
) -> Option<String> {
    if map_tokens == 0 || all_files.is_empty() {
        return None;
    }

    let budget_chars = ((map_tokens as f64) * 3.8) as usize;
    if budget_chars < 64 {
        return None;
    }

    let excluded_set: HashSet<String> = target_files
        .iter()
        .chain(read_only_files.iter())
        .map(|s| s.to_lowercase())
        .collect();

    // Lire brièvement les target_files pour repérer les modules/fichiers importés
    let mut target_contents_lower = String::new();
    for rel in target_files.iter().take(6) {
        if let Ok(content) = std::fs::read_to_string(project_root.join(rel)) {
            let excerpt: String = content.chars().take(16_000).collect();
            target_contents_lower.push_str(&excerpt.to_lowercase());
            target_contents_lower.push('\n');
        }
    }

    let prompt_keywords: Vec<String> = user_prompt
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| s.len() >= 4)
        .collect();

    struct ScoredFile {
        path: String,
        score: i32,
        sigs: Vec<String>,
    }

    let mut candidates: Vec<ScoredFile> = Vec::new();
    for rel in all_files.iter().take(350) {
        let rel_low = rel.to_lowercase();
        if excluded_set.contains(&rel_low) {
            continue;
        }

        let full = project_root.join(rel);
        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if !meta.is_file() || meta.len() > 131_072 {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&full) else {
            continue;
        };

        let sigs = extract_file_signatures(rel, &content);
        if sigs.is_empty() {
            continue;
        }

        let mut score: i32 = 0;

        // +5 si le fichier ou module est mentionné dans les fichiers cibles
        let base_no_ext = rel_low
            .rsplit('/')
            .next()
            .unwrap_or(&rel_low)
            .split('.')
            .next()
            .unwrap_or("");
        if base_no_ext.len() >= 3 && target_contents_lower.contains(base_no_ext) {
            score += 5;
        }

        // +3 par mot-clé du prompt présent dans le chemin ou les signatures
        let sigs_low = sigs.join(" ").to_lowercase();
        let mut seen_kw = HashSet::new();
        for kw in &prompt_keywords {
            if seen_kw.insert(kw) && (rel_low.contains(kw) || sigs_low.contains(kw)) {
                score += 3;
            }
        }

        // +2 pour les dossiers sources prioritaires ou manifestes racine
        if rel.starts_with("src/")
            || rel.starts_with("src-tauri/src/")
            || !rel.contains('/')
        {
            score += 2;
        }

        candidates.push(ScoredFile {
            path: rel.clone(),
            score,
            sigs,
        });
    }

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.sigs.len().cmp(&a.sigs.len()))
            .then_with(|| a.path.len().cmp(&b.path.len()))
    });

    let mut out = String::new();
    for item in candidates {
        let mut block = format!("{}:\n", item.path);
        for sig in item.sigs {
            block.push_str(&format!("  │ {}\n", sig));
        }
        if out.chars().count() + block.chars().count() > budget_chars {
            if out.is_empty() {
                // Au moins inclure le premier fichier tronqué au budget
                let clipped: String = block.chars().take(budget_chars).collect();
                out.push_str(&clipped);
            }
            break;
        }
        out.push_str(&block);
    }

    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn make_temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aiderstudio_repomap_test_{}_{}", name, Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_ignore_rules_and_list_project_files() {
        let dir = make_temp_dir("ignore");
        std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
        std::fs::create_dir_all(dir.join(".git/objects")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("logs")).unwrap();

        std::fs::write(dir.join(".gitignore"), "*.log\nlogs/\n!logs/keep.txt\n").unwrap();
        std::fs::write(dir.join(".github/workflows/build.yml"), "name: CI\n").unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=123\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("debug.log"), "noise\n").unwrap();
        std::fs::write(dir.join("logs/app.txt"), "noise\n").unwrap();
        std::fs::write(dir.join("logs/keep.txt"), "important\n").unwrap();

        let files = list_project_files(&dir, 500);
        assert!(files.contains(&".github/workflows/build.yml".to_string()));
        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(files.contains(&"logs/keep.txt".to_string()));
        assert!(!files.iter().any(|f| f.starts_with(".git/")));
        assert!(!files.iter().any(|f| f == ".env"));
        assert!(!files.iter().any(|f| f == "debug.log"));
        assert!(!files.iter().any(|f| f == "logs/app.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_signatures_multi_lang() {
        let rust_src = r#"
pub struct UserStore { id: u32 }
pub async fn load_users() -> Vec<UserStore> { vec![] }
impl UserStore {
    pub fn count(&self) -> usize { 0 }
}
"#;
        let rs_sigs = extract_file_signatures("src/store.rs", rust_src);
        assert!(rs_sigs.iter().any(|s| s.contains("pub struct UserStore")));
        assert!(rs_sigs.iter().any(|s| s.contains("pub async fn load_users")));
        assert!(rs_sigs.iter().any(|s| s.contains("impl UserStore")));

        let ts_src = r#"
export interface ApiResult { ok: boolean; }
export const fetchItems = async () => { return []; }
export function normalizePath(p: string): string { return p; }
"#;
        let ts_sigs = extract_file_signatures("src/api.ts", ts_src);
        assert_eq!(ts_sigs.len(), 3);

        let py_src = "class Client:\n    pass\nasync def fetch():\n    pass\n";
        let py_sigs = extract_file_signatures("app.py", py_src);
        assert_eq!(py_sigs.len(), 2);

        let vue_src = "<script setup lang=\"ts\">\nconst count = ref(0);\nfunction submitForm() {}\n</script>";
        let vue_sigs = extract_file_signatures("src/Form.vue", vue_src);
        assert_eq!(vue_sigs.len(), 2);
    }

    #[test]
    fn test_build_repo_map_respects_budget_and_scores_imports() {
        let dir = make_temp_dir("budget");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/main.rs"),
            "mod auth;\nfn main() { auth::login(); }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/auth.rs"),
            "pub fn login() -> bool { true }\npub fn logout() {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/extra.rs"),
            (0..20)
                .map(|i| format!("pub fn helper_func_{i}() {{}}\n"))
                .collect::<String>(),
        )
        .unwrap();

        let all = vec![
            "src/main.rs".to_string(),
            "src/auth.rs".to_string(),
            "src/extra.rs".to_string(),
        ];
        let map = build_repo_map(
            &dir,
            &all,
            &["src/main.rs".to_string()],
            &[],
            "Vérifie l'authentification login",
            60,
        )
        .unwrap();

        // src/main.rs est déjà dans target_files donc exclu de la map
        assert!(!map.contains("src/main.rs:"));
        // src/auth.rs doit être prioritaire
        assert!(map.contains("src/auth.rs:"));
        // Budget strict : 60 * 3.8 = 228 chars
        assert!(map.chars().count() <= 230);

        // map_tokens == 0 -> None
        assert!(build_repo_map(&dir, &all, &[], &[], "login", 0).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_search_in_project() {
        let dir = make_temp_dir("search");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/alpha.ts"), "export function computeTotal() { return 42; }\n")
            .unwrap();
        std::fs::write(dir.join("src/beta.ts"), "const other = 1;\n").unwrap();
        std::fs::write(dir.join(".env"), "computeTotal=secret\n").unwrap();

        let res = search_in_project(&dir, "computeTotal", None, 20).unwrap();
        assert!(res.contains("src/alpha.ts:1: export function computeTotal()"));
        assert!(!res.contains(".env"));

        let filtered = search_in_project(&dir, "computeTotal", Some("beta"), 20).unwrap();
        assert!(filtered.contains("Aucun résultat"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_signatures_frontend_files() {
        let vue_src = "<script setup>\nimport Sidebar from './components/Sidebar.vue';\nconst open = ref(true);\nexport default {}\n</script>\n<style scoped>\n.app { display: flex; }\n</style>";
        let vue_sigs = extract_file_signatures("src/App.vue", vue_src);
        assert!(vue_sigs.iter().any(|s| s.contains("import Sidebar")));
        assert!(vue_sigs.iter().any(|s| s.contains("const open = ref(true);")));
        assert!(vue_sigs.iter().any(|s| s.contains("export default")));
        assert!(vue_sigs.iter().any(|s| s.contains("<style scoped>")));

        let css_src = "@import \"tailwindcss\";\n:root {\n  --bg: #111;\n}\nbody {\n  margin: 0;\n}\n.sidebar {\n  width: 170px;\n}\n";
        let css_sigs = extract_file_signatures("src/index.css", css_src);
        assert!(css_sigs.iter().any(|s| s.contains("@import \"tailwindcss\"")));
        assert!(css_sigs.iter().any(|s| s.contains(":root")));
        assert!(css_sigs.iter().any(|s| s.contains("body")));
        assert!(css_sigs.iter().any(|s| s.contains(".sidebar")));

        let html_src = "<html><head>\n<title>Minou AI</title>\n<link rel=\"stylesheet\" href=\"/style.css\">\n</head><body>\n<script type=\"module\" src=\"/src/main.js\"></script>\n</body></html>";
        let html_sigs = extract_file_signatures("index.html", html_src);
        assert_eq!(html_sigs.len(), 3);

        let pkg_src = "{\n  \"name\": \"minou-ai\",\n  \"dependencies\": {\n    \"vue\": \"^3.5.0\",\n    \"lodash\": \"^4.17.21\"\n  },\n  \"devDependencies\": {\n    \"vite\": \"^6.0.0\",\n    \"tailwindcss\": \"^4.0.0\"\n  }\n}\n";
        let pkg_sigs = extract_file_signatures("package.json", pkg_src);
        assert!(pkg_sigs.iter().any(|s| s.contains("\"name\": \"minou-ai\"")));
        assert!(pkg_sigs.iter().any(|s| s.contains("\"vue\"")));
        assert!(pkg_sigs.iter().any(|s| s.contains("\"vite\"")));
        assert!(pkg_sigs.iter().any(|s| s.contains("\"tailwindcss\"")));
        assert!(!pkg_sigs.iter().any(|s| s.contains("\"lodash\"")));

        let other_json = "{ \"vue\": true }\n";
        assert!(extract_file_signatures("tsconfig.json", other_json).is_empty());
    }
}
