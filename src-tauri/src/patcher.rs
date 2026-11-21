use crate::models::ToolExecutionResult;
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditBlock {
    SearchReplace {
        path: String,
        search: String,
        replace: String,
    },
    WholeFile {
        path: String,
        content: String,
        implicit_target: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchFailure {
    pub path: String,
    pub search_snippet: String,
    pub closest_match: Option<String>,
    pub closest_line: Option<usize>,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyEditsOutcome {
    pub applied_files: Vec<String>,
    pub failures: Vec<PatchFailure>,
    pub results: Vec<ToolExecutionResult>,
}

/// Nettoie et valide un chemin relatif au projet.
/// Rejette tout chemin absolu, traversal `..`, URL, `.git/` ou `.env*`.
pub fn sanitize_rel_path(raw: &str) -> Option<String> {
    let mut s = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '`' | '\'' | '"' | '*' | '#'))
        .trim()
        .replace('\\', "/");

    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    let s = s.trim().to_string();

    if s.is_empty() {
        return None;
    }
    if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") {
        return None;
    }
    // Rejette les chemins Windows avec lettre de lecteur (ex: C:/foo)
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return None;
    }
    if s.contains("..") {
        return None;
    }
    if s.chars().any(|c| {
        matches!(c, '[' | ']' | '(' | ')' | '<' | '>' | '{' | '}' | '|' | '?' | '*')
            || c.is_whitespace()
    }) {
        return None;
    }
    let lower = s.to_lowercase();
    if lower == ".git" || lower.starts_with(".git/") || lower.contains("/.git/") {
        return None;
    }
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    if base == ".env" || base.starts_with(".env.") {
        return None;
    }
    // Exiger un point d'extension ou un nom manifeste connu (Makefile, Dockerfile, etc.)
    let known_no_ext = matches!(base, "makefile" | "dockerfile" | "procfile" | "rakefile");
    if !known_no_ext && (!base.contains('.') || base.ends_with('.')) {
        return None;
    }
    Some(s)
}

fn is_shell_lang(lang: &str) -> bool {
    matches!(
        lang.to_lowercase().as_str(),
        "bash" | "sh" | "zsh" | "shell" | "console" | "cmd" | "powershell" | "ps1"
    )
}

fn is_shell_script_path(path: &str) -> bool {
    let low = path.to_lowercase();
    low.ends_with(".sh")
        || low.ends_with(".zsh")
        || low.ends_with(".bash")
        || low.ends_with(".ps1")
        || low.ends_with(".bat")
        || low.ends_with(".cmd")
}

pub fn is_search_marker(line: &str) -> bool {
    let t = line.trim();
    let chev = t.bytes().take_while(|&b| b == b'<').count();
    if chev < 5 {
        return false;
    }
    let rest = t[chev..].trim().to_ascii_lowercase();
    rest == "search"
        || rest.starts_with("search ")
        || rest == "original"
        || rest.starts_with("original ")
}

fn is_divider_marker(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 5 && t.bytes().all(|b| b == b'=')
}

pub fn is_replace_marker(line: &str) -> bool {
    let t = line.trim();
    let chev = t.bytes().take_while(|&b| b == b'>').count();
    if chev < 5 {
        return false;
    }
    let rest = t[chev..].trim().to_ascii_lowercase();
    rest == "replace"
        || rest.starts_with("replace ")
        || rest == "updated"
        || rest.starts_with("updated ")
}

fn body_has_search_replace_pair(body: &str) -> bool {
    let has_s = body.lines().any(is_search_marker);
    let has_r = body.lines().any(is_replace_marker);
    has_s && has_r
}

fn body_has_any_search_or_replace_marker(body: &str) -> bool {
    body.lines()
        .any(|l| is_search_marker(l) || is_replace_marker(l))
}

fn resolve_project_rel_path(
    raw_path: &str,
    allowed_targets: &[String],
    all_project_files: &[String],
) -> String {
    let clean = raw_path.trim().replace('\\', "/");
    let clean = clean.strip_prefix("./").unwrap_or(&clean);

    // 1. Correspondance exacte insensible à la casse dans allowed_targets puis all_project_files
    if let Some(exact) = allowed_targets
        .iter()
        .find(|p| p.eq_ignore_ascii_case(clean))
    {
        return exact.clone();
    }
    if let Some(exact) = all_project_files
        .iter()
        .find(|p| p.eq_ignore_ascii_case(clean))
    {
        return exact.clone();
    }

    // 2. Correspondance unique par suffixe "/<clean>" ou basename
    let suffix = format!("/{}", clean.to_lowercase());
    let is_basename_only = !clean.contains('/');
    let find_unique = |list: &[String]| -> Option<String> {
        let mut matches: Vec<&String> = list
            .iter()
            .filter(|p| {
                let low = p.to_lowercase();
                low.ends_with(&suffix)
                    || (is_basename_only
                        && low.rsplit('/').next().unwrap_or("") == clean.to_lowercase())
            })
            .collect();
        matches.dedup();
        if matches.len() == 1 {
            Some(matches[0].clone())
        } else {
            None
        }
    };

    if let Some(found) = find_unique(allowed_targets) {
        return found;
    }
    if let Some(found) = find_unique(all_project_files) {
        return found;
    }

    clean.to_string()
}

fn lang_matches_path(lang: &str, path: &str) -> bool {
    let l = lang.trim().to_ascii_lowercase();
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if l.is_empty() || ext.is_empty() {
        return false;
    }
    if l == ext {
        return true;
    }
    matches!(
        (l.as_str(), ext.as_str()),
        ("css" | "scss" | "sass" | "less", "css" | "scss" | "sass" | "less")
            | ("html" | "htm", "html" | "htm")
            | ("ts" | "typescript" | "tsx", "ts" | "tsx")
            | ("js" | "javascript" | "jsx", "js" | "jsx")
            | ("rs" | "rust", "rs")
            | ("py" | "python", "py")
            | ("json" | "jsonc", "json")
            | ("yaml" | "yml", "yaml" | "yml")
            | ("md" | "markdown", "md")
    )
}

fn extract_path_from_text_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let low = trimmed.to_lowercase();
    if low.starts_with("$ ")
        || low.starts_with("npm ")
        || low.starts_with("npx ")
        || low.starts_with("cd ")
        || low.starts_with("cargo ")
        || low.starts_with("git ")
    {
        return None;
    }

    // Support d'un en-tête/mention au format lien Markdown [NomFichier](chemin/relatif.ext#L10)
    if let Ok(md_link_re) = Regex::new(r"\[[^\]]+\]\(([^)\s]+)\)") {
        for cap in md_link_re.captures_iter(trimmed) {
            if let Some(href_m) = cap.get(1) {
                let raw_href = href_m.as_str().split('#').next().unwrap_or("");
                if let Some(p) = sanitize_rel_path(raw_href) {
                    return Some(p);
                }
            }
        }
    }

    let path_re =
        Regex::new(r#"(?:^|[\s"'\*#`:(])([a-zA-Z0-9_\-./\\]+\.[a-zA-Z0-9]{1,10})(?:[\s"'\*`:).,;]|$)"#)
            .ok()?;

    // Vérifier d'abord si la ligne nettoyée est directement un chemin (ex: "### `src/App.vue`")
    let direct_clean = trimmed
        .trim_start_matches('#')
        .trim()
        .trim_matches(|c: char| matches!(c, '`' | '\'' | '"' | '*' | ':'))
        .trim();
    if !direct_clean.contains(' ') {
        if let Some(p) = sanitize_rel_path(direct_clean) {
            return Some(p);
        }
    }

    if let Some(cap) = path_re.captures(trimmed) {
        if let Some(m) = cap.get(1) {
            if let Some(p) = sanitize_rel_path(m.as_str()) {
                return Some(p);
            }
        }
    }
    None
}

fn detect_path_from_preceding_lines(lines: &[&str], fence_line_idx: usize) -> Option<String> {
    let mut checked = 0usize;
    let mut idx = fence_line_idx;
    while idx > 0 && checked < 3 {
        idx -= 1;
        let line = lines[idx].trim();
        if line.is_empty() {
            continue;
        }
        checked += 1;
        if let Some(p) = extract_path_from_text_line(line) {
            return Some(p);
        }
    }
    None
}

fn detect_path_from_first_code_line(code: &str) -> Option<String> {
    let first = code.lines().next()?.trim();
    if first.starts_with("//")
        || first.starts_with("/*")
        || first.starts_with('#')
        || first.starts_with("<!--")
    {
        return extract_path_from_text_line(first);
    }
    None
}

fn detect_path_from_info(info: &str) -> (String, Option<String>) {
    let trimmed = info.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }

    // Format "lang:path/to/file.ext" ou "lang path/to/file.ext"
    if let Some((left, right)) = trimmed.split_once(':') {
        let lang = left.trim().to_lowercase();
        let cand = right.trim();
        if let Some(p) = sanitize_rel_path(cand) {
            return (lang, Some(p));
        }
    }

    let first_tok = trimmed.split_whitespace().next().unwrap_or("");
    if let Some(p) = sanitize_rel_path(trimmed) {
        let ext = p.rsplit('.').next().unwrap_or("").to_lowercase();
        return (ext, Some(p));
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() >= 2 {
        if let Some(p) = sanitize_rel_path(tokens[1]) {
            return (tokens[0].to_lowercase(), Some(p));
        }
    }

    (first_tok.to_lowercase(), None)
}

fn heuristic_fallback_path(
    code: &str,
    user_prompt: &str,
    already_seen: &HashSet<String>,
    all_project_files: &[String],
) -> Option<String> {
    let lower_prompt = user_prompt.to_lowercase();
    let is_html = code.contains("<!DOCTYPE html>") || code.contains("<html");
    let is_css = code.contains('{')
        && code.contains('}')
        && (code.contains("margin:")
            || code.contains("padding:")
            || code.contains("color:")
            || code.contains("background")
            || code.contains("font-family:"));
    let is_js = code.contains("function ")
        || code.contains("const ")
        || code.contains("let ")
        || code.contains("import ");

    if is_html
        && (lower_prompt.contains("index.html")
            || lower_prompt.contains("site web")
            || lower_prompt.contains("page html")
            || lower_prompt.contains("vitrine")
            || lower_prompt.contains("site")
            || lower_prompt.contains("page")
            || lower_prompt.contains("fond"))
        && !already_seen.contains("index.html")
    {
        return Some(
            all_project_files
                .iter()
                .find(|f| f.eq_ignore_ascii_case("index.html"))
                .cloned()
                .unwrap_or_else(|| "index.html".to_string()),
        );
    }

    let wants_css_or_visual = lower_prompt.contains("css")
        || lower_prompt.contains("style")
        || lower_prompt.contains("fond")
        || lower_prompt.contains("arrière-plan")
        || lower_prompt.contains("arriere-plan")
        || lower_prompt.contains("background")
        || lower_prompt.contains("couleur")
        || lower_prompt.contains("gris")
        || lower_prompt.contains("page");

    if is_css && wants_css_or_visual {
        let css_candidates = [
            "src/style.css",
            "src/styles.css",
            "src/styles/main.css",
            "src/assets/main.css",
            "src/index.css",
            "src/main.css",
            "src/app.css",
            "style.css",
            "styles.css",
        ];
        for cand in css_candidates {
            if let Some(found) = all_project_files.iter().find(|f| f.eq_ignore_ascii_case(cand)) {
                if !already_seen.contains(found) {
                    return Some(found.clone());
                }
            }
        }
        if let Some(any_css) = all_project_files
            .iter()
            .filter(|f| {
                let l = f.to_lowercase();
                (l.ends_with(".css") || l.ends_with(".scss")) && !already_seen.contains(*f)
            })
            .min_by_key(|f| (f.matches('/').count(), f.len()))
        {
            return Some(any_css.clone());
        }
        if (lower_prompt.contains("css") || lower_prompt.contains("style"))
            && !already_seen.contains("style.css")
            && !already_seen.contains("styles.css")
        {
            return Some("styles.css".to_string());
        }
    }
    if is_js
        && (lower_prompt.contains("script.js") || lower_prompt.contains("app.js"))
        && !already_seen.contains("app.js")
        && !already_seen.contains("script.js")
    {
        return Some("app.js".to_string());
    }
    None
}

/// Extrait tous les couples `(SEARCH, REPLACE)` contenus dans un texte de patch.
fn parse_search_replace_hunks(body: &str) -> Vec<(String, String)> {
    let norm = body.replace("\r\n", "\n");
    let lines: Vec<&str> = norm.split('\n').collect();
    let mut hunks = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        if is_search_marker(lines[i]) {
            let mut search_lines = Vec::new();
            let mut replace_lines = Vec::new();
            let mut in_replace = false;
            let mut closed = false;
            i += 1;
            while i < lines.len() {
                if !in_replace && is_divider_marker(lines[i]) {
                    in_replace = true;
                } else if in_replace && is_replace_marker(lines[i]) {
                    closed = true;
                    break;
                } else if in_replace {
                    replace_lines.push(lines[i]);
                } else {
                    search_lines.push(lines[i]);
                }
                i += 1;
            }
            if closed {
                hunks.push((search_lines.join("\n"), replace_lines.join("\n")));
            }
        }
        i += 1;
    }
    hunks
}

/// Extrait les blocs d'édition (`SearchReplace` ou `WholeFile`) d'une réponse LLM.
pub fn parse_edit_blocks(
    text: &str,
    allowed_targets: &[String],
    all_project_files: &[String],
    user_prompt: &str,
) -> Vec<EditBlock> {
    let norm = text.replace("\r\n", "\n");
    let lines: Vec<&str> = norm.split('\n').collect();
    let mut blocks = Vec::new();
    let mut seen_whole_paths = HashSet::new();

    let mut i = 0usize;
    while i < lines.len() {
        let trimmed_start = lines[i].trim_start();

        // 1. Détection d'une fence ouvrante (``` ou ````...)
        let tick_count = trimmed_start.bytes().take_while(|&b| b == b'`').count();
        if tick_count >= 3 {
            let fence_start_idx = i;
            let info = trimmed_start[tick_count..].trim();
            let mut body_lines: Vec<&str> = Vec::new();
            i += 1;
            let mut closed = false;
            while i < lines.len() {
                let close_trim = lines[i].trim();
                let close_ticks = close_trim.bytes().take_while(|&b| b == b'`').count();
                if close_ticks >= tick_count && close_trim[close_ticks..].trim().is_empty() {
                    closed = true;
                    break;
                }
                body_lines.push(lines[i]);
                i += 1;
            }
            if !closed {
                // Bloc non fermé (ex. réponse tronquée) : ne pas écrire sur disque
                break;
            }
            let body = body_lines.join("\n");
            i += 1;

            if body.trim().is_empty() {
                continue;
            }

            let info_low = info.to_lowercase();
            if info_low == "tool_call" || info_low == "json:tool_call" {
                continue;
            }
            let body_trim = body.trim();
            if body_trim.starts_with("{\"name\"") && body_trim.contains("\"arguments\"") {
                continue;
            }

            let (lang, info_path) = detect_path_from_info(info);
            let has_sr_markers = body_has_search_replace_pair(&body);

            let explicit_or_context_path = info_path
                .or_else(|| detect_path_from_preceding_lines(&lines, fence_start_idx))
                .or_else(|| detect_path_from_first_code_line(&body));

            let mut implicit_target = false;
            let mut detected_path = explicit_or_context_path;

            if detected_path.is_none() && has_sr_markers && allowed_targets.len() == 1 {
                detected_path = sanitize_rel_path(&allowed_targets[0]);
            }
            if detected_path.is_none() && !has_sr_markers {
                if allowed_targets.len() == 1 && lang_matches_path(&lang, &allowed_targets[0]) {
                    detected_path = sanitize_rel_path(&allowed_targets[0]);
                    implicit_target = detected_path.is_some();
                }
            }
            if detected_path.is_none() && !has_sr_markers {
                if let Some(fb) = heuristic_fallback_path(
                    &body,
                    user_prompt,
                    &seen_whole_paths,
                    all_project_files,
                ) {
                    detected_path = Some(fb);
                    implicit_target = true;
                }
            }

            let Some(raw_path) = detected_path else {
                continue;
            };
            let path = resolve_project_rel_path(&raw_path, allowed_targets, all_project_files);

            if is_shell_lang(&lang) && !is_shell_script_path(&path) && !has_sr_markers {
                continue;
            }

            if has_sr_markers {
                for (search, replace) in parse_search_replace_hunks(&body) {
                    blocks.push(EditBlock::SearchReplace {
                        path: path.clone(),
                        search,
                        replace,
                    });
                }
            } else if !body_has_any_search_or_replace_marker(&body) {
                seen_whole_paths.insert(path.clone());
                blocks.push(EditBlock::WholeFile {
                    path,
                    content: body,
                    implicit_target,
                });
            }
            continue;
        }

        // 2. Bloc SEARCH/REPLACE nu (hors fence Markdown)
        if is_search_marker(lines[i]) {
            let sr_start_idx = i;
            let mut search_lines = Vec::new();
            let mut replace_lines = Vec::new();
            let mut in_replace = false;
            let mut closed = false;
            i += 1;
            while i < lines.len() {
                if !in_replace && is_divider_marker(lines[i]) {
                    in_replace = true;
                } else if in_replace && is_replace_marker(lines[i]) {
                    closed = true;
                    break;
                } else if in_replace {
                    replace_lines.push(lines[i]);
                } else {
                    search_lines.push(lines[i]);
                }
                i += 1;
            }
            if closed {
                let detected = detect_path_from_preceding_lines(&lines, sr_start_idx).or_else(
                    || {
                        if allowed_targets.len() == 1 {
                            sanitize_rel_path(&allowed_targets[0])
                        } else {
                            None
                        }
                    },
                );
                if let Some(raw_path) = detected {
                    let path =
                        resolve_project_rel_path(&raw_path, allowed_targets, all_project_files);
                    blocks.push(EditBlock::SearchReplace {
                        path,
                        search: search_lines.join("\n"),
                        replace: replace_lines.join("\n"),
                    });
                }
            }
            i += 1;
            continue;
        }

        i += 1;
    }

    blocks
}

fn restore_eol(text: String, use_crlf: bool) -> String {
    if use_crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

fn leading_ws_str(line: &str) -> &str {
    let end = line
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(line.len());
    &line[..end]
}

fn common_indent_prefix<'a>(lines: &[&'a str]) -> &'a str {
    let non_empty: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if non_empty.is_empty() {
        return "";
    }
    let first_ws = leading_ws_str(non_empty[0]);
    let mut prefix_len = first_ws.len();
    for line in &non_empty[1..] {
        let ws = leading_ws_str(line);
        let common = first_ws
            .char_indices()
            .zip(ws.chars())
            .take_while(|((_, a), b)| *a == *b)
            .map(|((idx, ch), _)| idx + ch.len_utf8())
            .last()
            .unwrap_or(0);
        prefix_len = prefix_len.min(common);
    }
    &first_ws[..prefix_len]
}

fn dedent_lines(lines: &[&str]) -> (Vec<String>, String) {
    let prefix = common_indent_prefix(lines).to_string();
    let plen = prefix.len();
    let dedented = lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else if l.starts_with(&prefix) {
                l[plen..].trim_end().to_string()
            } else {
                l.trim_end().to_string()
            }
        })
        .collect();
    (dedented, prefix)
}

fn reindent_replace(replace_str: &str, search_indent: &str, target_indent: &str) -> Vec<String> {
    let rep_lines: Vec<&str> = replace_str.split('\n').collect();
    if search_indent == target_indent {
        return rep_lines.into_iter().map(|s| s.to_string()).collect();
    }
    let slen = search_indent.len();
    rep_lines
        .into_iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                let stripped = if !search_indent.is_empty() && l.starts_with(search_indent) {
                    &l[slen..]
                } else if !search_indent.is_empty() {
                    // Retirer au maximum la longueur d'indentation commune
                    let l_ws = leading_ws_str(l);
                    let drop = l_ws.len().min(slen);
                    &l[drop..]
                } else {
                    l
                };
                format!("{target_indent}{stripped}")
            }
        })
        .collect()
}

fn normalized_levenshtein_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let n = a_chars.len();
    let m = b_chars.len();
    let max_len = n.max(m);
    if max_len == 0 {
        return 1.0;
    }

    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];

    for i in 1..=n {
        curr[0] = i;
        let ai = a_chars[i - 1];
        for j in 1..=m {
            let cost = usize::from(ai != b_chars[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    let dist = prev[m];
    1.0 - (dist as f64 / max_len as f64)
}

fn format_closest_snippet(lines: &[&str], start_idx: usize, win_len: usize) -> (String, usize) {
    let ctx_start = start_idx.saturating_sub(3);
    let ctx_end = (start_idx + win_len + 3).min(lines.len());
    let formatted = (ctx_start..ctx_end)
        .map(|idx| format!("{:>4} | {}", idx + 1, lines[idx]))
        .collect::<Vec<_>>()
        .join("\n");
    (formatted, start_idx + 1)
}

fn find_matching_windows_flex(
    orig_lines: &[&str],
    sub_lines: &[&str],
) -> Vec<(usize, String)> {
    let m = sub_lines.len();
    if m == 0 || m > orig_lines.len() {
        return Vec::new();
    }
    let (ded_sub, _) = dedent_lines(sub_lines);
    let mut matches = Vec::new();
    for i in 0..=orig_lines.len() - m {
        let win = &orig_lines[i..i + m];
        let (ded_win, win_indent) = dedent_lines(win);
        if ded_win == ded_sub {
            matches.push((i, win_indent));
        }
    }
    matches
}

fn replace_line_range(
    orig_lines: &[&str],
    start: usize,
    end: usize,
    replacement_lines: Vec<String>,
) -> String {
    let mut out: Vec<String> = Vec::with_capacity(orig_lines.len() + replacement_lines.len());
    for l in &orig_lines[..start] {
        out.push((*l).to_string());
    }
    out.extend(replacement_lines);
    for l in &orig_lines[end..] {
        out.push((*l).to_string());
    }
    out.join("\n")
}

/// Applique un bloc `SEARCH/REPLACE` sur `original` avec 4 niveaux de tolérance.
pub fn apply_search_replace_fuzzy(
    original: &str,
    search: &str,
    replace: &str,
) -> Result<String, PatchFailure> {
    let had_crlf = original.contains("\r\n");
    let orig_norm = original.replace("\r\n", "\n");
    let search_norm = search.replace("\r\n", "\n");
    let rep_norm = replace.replace("\r\n", "\n");

    let make_err = |reason: &str, closest: Option<String>, closest_line: Option<usize>| {
        PatchFailure {
            path: String::new(),
            search_snippet: search_norm.lines().take(8).collect::<Vec<_>>().join("\n"),
            closest_match: closest,
            closest_line,
            reason: reason.to_string(),
        }
    };

    if search_norm.is_empty() {
        if orig_norm.is_empty() {
            return Ok(restore_eol(rep_norm, had_crlf));
        }
        return Err(make_err(
            "Le bloc SEARCH est vide pour un fichier existant non vide.",
            None,
            None,
        ));
    }

    // Niveau 1 : Correspondance exacte
    if orig_norm.contains(&search_norm) {
        let updated = orig_norm.replacen(&search_norm, &rep_norm, 1);
        return Ok(restore_eol(updated, had_crlf));
    }

    let orig_lines: Vec<&str> = orig_norm.split('\n').collect();
    let search_lines: Vec<&str> = search_norm.split('\n').collect();
    let m = search_lines.len();

    if m > orig_lines.len() {
        let (snippet, ln) = format_closest_snippet(&orig_lines, 0, orig_lines.len().min(8));
        return Err(make_err(
            "Le bloc SEARCH est plus long que le fichier cible.",
            Some(snippet),
            Some(ln),
        ));
    }

    // Niveau 2 : Espaces en fin de ligne ignorés (.trim_end())
    let mut level2_matches = Vec::new();
    for i in 0..=orig_lines.len() - m {
        let matched = (0..m).all(|k| orig_lines[i + k].trim_end() == search_lines[k].trim_end());
        if matched {
            level2_matches.push(i);
        }
    }
    if level2_matches.len() == 1 {
        let idx = level2_matches[0];
        let rep_lines = rep_norm.split('\n').map(|s| s.to_string()).collect();
        let updated = replace_line_range(&orig_lines, idx, idx + m, rep_lines);
        return Ok(restore_eol(updated, had_crlf));
    }
    if level2_matches.len() > 1 {
        let (snippet, ln) = format_closest_snippet(&orig_lines, level2_matches[0], m);
        return Err(make_err(
            "Le bloc SEARCH correspond à plusieurs endroits du fichier (ambiguïté).",
            Some(snippet),
            Some(ln),
        ));
    }

    // Niveau 3 : Indentation flexible (dédentation commune + réindentation à l'origine)
    let (ded_search, search_indent) = dedent_lines(&search_lines);
    let mut level3_matches: Vec<(usize, String)> = Vec::new();
    for i in 0..=orig_lines.len() - m {
        let win = &orig_lines[i..i + m];
        let (ded_win, win_indent) = dedent_lines(win);
        if ded_win == ded_search {
            level3_matches.push((i, win_indent));
        }
    }
    if level3_matches.len() == 1 {
        let (idx, target_indent) = &level3_matches[0];
        let rep_lines = reindent_replace(&rep_norm, &search_indent, target_indent);
        let updated = replace_line_range(&orig_lines, *idx, *idx + m, rep_lines);
        return Ok(restore_eol(updated, had_crlf));
    }
    if level3_matches.len() > 1 {
        let (snippet, ln) = format_closest_snippet(&orig_lines, level3_matches[0].0, m);
        return Err(make_err(
            "Le bloc SEARCH (indentation ignorée) correspond à plusieurs endroits du fichier.",
            Some(snippet),
            Some(ln),
        ));
    }

    // Niveau 4a : Gestion des ellipses "..." isolées sur une ligne
    if let Some(ellipsis_pos) = search_lines.iter().position(|l| l.trim() == "...") {
        if rep_norm.split('\n').any(|l| l.trim() == "...") {
            return Err(make_err(
                "Le bloc REPLACE ne doit pas contenir de ligne '...' (fournissez le code complet).",
                None,
                None,
            ));
        }
        let prefix = &search_lines[..ellipsis_pos];
        let suffix = &search_lines[ellipsis_pos + 1..];
        if !prefix.is_empty() && !suffix.is_empty() {
            let p_matches = find_matching_windows_flex(&orig_lines, prefix);
            let s_matches = find_matching_windows_flex(&orig_lines, suffix);
            let mut pairs = Vec::new();
            for (pi, p_indent) in &p_matches {
                for (si, _) in &s_matches {
                    if *si >= pi + prefix.len() {
                        pairs.push((*pi, *si + suffix.len(), p_indent.clone()));
                    }
                }
            }
            if pairs.len() == 1 {
                let (start, end, target_indent) = &pairs[0];
                let (_, p_indent) = dedent_lines(prefix);
                let rep_lines = reindent_replace(&rep_norm, &p_indent, target_indent);
                let updated = replace_line_range(&orig_lines, *start, *end, rep_lines);
                return Ok(restore_eol(updated, had_crlf));
            }
        }
    }

    // Niveau 4b : Recherche floue par similarité de Levenshtein normalisée (>= 0.88, écart >= 0.05)
    let search_ded_joined = ded_search.join("\n");
    let search_len = search_ded_joined.chars().count();

    let mut win_sizes = vec![m];
    if m >= 4 {
        win_sizes.push(m - 1);
        if m + 1 <= orig_lines.len() {
            win_sizes.push(m + 1);
        }
    }

    #[derive(Clone)]
    struct Candidate {
        start: usize,
        end: usize,
        score: f64,
        indent: String,
    }

    let mut exact_scored: Vec<Candidate> = Vec::new();
    let mut approx_best: Option<(usize, usize, f64)> = None;

    let search_tokens: HashSet<&str> = search_ded_joined
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2)
        .collect();

    for &ws in &win_sizes {
        if ws == 0 || ws > orig_lines.len() {
            continue;
        }
        for i in 0..=orig_lines.len() - ws {
            let win = &orig_lines[i..i + ws];
            let (ded_win, win_indent) = dedent_lines(win);
            let win_joined = ded_win.join("\n");
            let win_len = win_joined.chars().count();
            let max_l = search_len.max(win_len);
            let min_l = search_len.min(win_len);

            // Borne mathématique Levenshtein : si min/max < 0.88, le score exact est forcément < 0.88
            let length_ratio = if max_l == 0 {
                1.0
            } else {
                min_l as f64 / max_l as f64
            };

            if length_ratio >= 0.88 {
                let score = normalized_levenshtein_similarity(&search_ded_joined, &win_joined);
                exact_scored.push(Candidate {
                    start: i,
                    end: i + ws,
                    score,
                    indent: win_indent,
                });
            } else if ws == m && !search_tokens.is_empty() {
                // Heuristique rapide pour proposer l'extrait le plus proche en cas d'échec
                let win_tokens: HashSet<&str> = win_joined
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .filter(|t| t.len() >= 2)
                    .collect();
                let inter = search_tokens.intersection(&win_tokens).count() as f64;
                let union = search_tokens.union(&win_tokens).count().max(1) as f64;
                let jaccard = inter / union;
                if approx_best.map(|b| jaccard > b.2).unwrap_or(true) {
                    approx_best = Some((i, i + ws, jaccard));
                }
            }
        }
    }

    exact_scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if let Some(best) = exact_scored.first() {
        let second_non_overlapping = exact_scored
            .iter()
            .skip(1)
            .find(|c| c.end <= best.start || best.end <= c.start)
            .map(|c| c.score)
            .unwrap_or(0.0);

        if best.score >= 0.88 && (best.score - second_non_overlapping) >= 0.05 {
            let rep_lines = reindent_replace(&rep_norm, &search_indent, &best.indent);
            let updated = replace_line_range(&orig_lines, best.start, best.end, rep_lines);
            return Ok(restore_eol(updated, had_crlf));
        }

        let (snippet, ln) = format_closest_snippet(&orig_lines, best.start, best.end - best.start);
        let reason = if best.score >= 0.88 {
            "Plusieurs passages similaires trouvés dans le fichier (ambiguïté)."
        } else {
            "Bloc SEARCH introuvable à l'identique dans le fichier."
        };
        return Err(make_err(reason, Some(snippet), Some(ln)));
    }

    let (closest_snip, closest_ln) = if let Some((s, e, _)) = approx_best {
        let (sn, ln) = format_closest_snippet(&orig_lines, s, e - s);
        (Some(sn), Some(ln))
    } else if !orig_lines.is_empty() {
        let (sn, ln) = format_closest_snippet(&orig_lines, 0, orig_lines.len().min(8));
        (Some(sn), Some(ln))
    } else {
        (None, None)
    };

    Err(make_err(
        "Bloc SEARCH introuvable dans le fichier cible.",
        closest_snip,
        closest_ln,
    ))
}

/// Extrait uniquement le code de remplacement propre d'un texte contenant des marqueurs SEARCH/REPLACE.
pub fn extract_clean_replace_code(patch: &str) -> String {
    let hunks = parse_search_replace_hunks(patch);
    if !hunks.is_empty() {
        return hunks
            .into_iter()
            .map(|(_, rep)| rep)
            .collect::<Vec<_>>()
            .join("\n");
    }

    patch
        .lines()
        .filter(|l| {
            !is_search_marker(l) && !is_divider_marker(l) && !is_replace_marker(l)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Applique les blocs d'édition d'une réponse LLM sur le disque avec garde-fou anti-troncature.
pub async fn apply_edits_from_response(
    project_root: &Path,
    assistant_text: &str,
    target_files: &[String],
    all_project_files: &[String],
    truncated_files: &HashSet<String>,
    user_prompt: &str,
) -> ApplyEditsOutcome {
    let blocks = parse_edit_blocks(assistant_text, target_files, all_project_files, user_prompt);
    let mut outcome = ApplyEditsOutcome::default();
    let mut seen_applied = HashSet::new();

    for block in blocks {
        match block {
            EditBlock::WholeFile {
                path,
                content,
                implicit_target,
            } => {
                if truncated_files.contains(&path) {
                    outcome.failures.push(PatchFailure {
                        path: path.clone(),
                        search_snippet: String::new(),
                        closest_match: None,
                        closest_line: None,
                        reason: format!(
                            "Le fichier '{}' a été tronqué dans votre contexte. Utilisez exclusivement des blocs SEARCH/REPLACE ciblés pour ne pas effacer le reste du fichier.",
                            path
                        ),
                    });
                    continue;
                }

                let target_path = project_root.join(&path);

                if implicit_target && target_path.is_file() {
                    if let Ok(existing) = tokio::fs::read_to_string(&target_path).await {
                        let existing_non_empty =
                            existing.lines().filter(|l| !l.trim().is_empty()).count();
                        let new_non_empty =
                            content.lines().filter(|l| !l.trim().is_empty()).count();
                        if existing_non_empty >= 15 && new_non_empty * 10 < existing_non_empty * 4 {
                            outcome.failures.push(PatchFailure {
                                path: path.clone(),
                                search_snippet: String::new(),
                                closest_match: Some(
                                    existing.lines().take(12).collect::<Vec<_>>().join("\n"),
                                ),
                                closest_line: Some(1),
                                reason: format!(
                                    "Bloc partiel sans nom de fichier explicite ({} lignes contre {} existantes dans '{}') : utilisez un bloc `SEARCH/REPLACE` ciblé ou fournissez le fichier complet précédé de `### {}`.",
                                    new_non_empty, existing_non_empty, path, path
                                ),
                            });
                            continue;
                        }
                    }
                }

                if let Some(parent) = target_path.parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        outcome.failures.push(PatchFailure {
                            path,
                            search_snippet: String::new(),
                            closest_match: None,
                            closest_line: None,
                            reason: format!("Impossible de créer le dossier parent : {e}"),
                        });
                        continue;
                    }
                }

                let final_content = if body_has_search_replace_pair(&content) {
                    extract_clean_replace_code(&content)
                } else {
                    content
                };

                if let Err(e) = tokio::fs::write(&target_path, &final_content).await {
                    outcome.failures.push(PatchFailure {
                        path,
                        search_snippet: String::new(),
                        closest_match: None,
                        closest_line: None,
                        reason: format!("Erreur d'écriture disque : {e}"),
                    });
                    continue;
                }

                if seen_applied.insert(path.clone()) {
                    outcome.applied_files.push(path.clone());
                }
                outcome.results.push(ToolExecutionResult {
                    tool: "write_file".to_string(),
                    success: true,
                    message: format!(
                        "Fichier '{}' écrit avec succès ({} octets)",
                        path,
                        final_content.len()
                    ),
                    path: Some(path),
                });
            }
            EditBlock::SearchReplace {
                path,
                search,
                replace,
            } => {
                let target_path = project_root.join(&path);
                let orig = if target_path.exists() {
                    tokio::fs::read_to_string(&target_path)
                        .await
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                match apply_search_replace_fuzzy(&orig, &search, &replace) {
                    Ok(patched) => {
                        if let Some(parent) = target_path.parent() {
                            let _ = tokio::fs::create_dir_all(parent).await;
                        }
                        if let Err(e) = tokio::fs::write(&target_path, &patched).await {
                            outcome.failures.push(PatchFailure {
                                path,
                                search_snippet: search,
                                closest_match: None,
                                closest_line: None,
                                reason: format!("Erreur d'écriture disque : {e}"),
                            });
                            continue;
                        }
                        if seen_applied.insert(path.clone()) {
                            outcome.applied_files.push(path.clone());
                        }
                        outcome.results.push(ToolExecutionResult {
                            tool: "patch_file".to_string(),
                            success: true,
                            message: format!("Patch appliqué avec succès à '{}'", path),
                            path: Some(path),
                        });
                    }
                    Err(mut fail) => {
                        fail.path = path;
                        outcome.failures.push(fail);
                    }
                }
            }
        }
    }

    if !outcome.applied_files.is_empty() && project_root.exists() {
        let _ = tokio::process::Command::new("git")
            .args(["add", "-A", "--", "."])
            .current_dir(project_root)
            .output()
            .await;
    }

    outcome
}

fn is_orphan_file_header_line(
    line: &str,
    applied_set: &HashSet<String>,
    allow_any: bool,
) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Ne jamais supprimer un élément de liste descriptif complet ; accepter uniquement un titre/étiquette de fichier isolé.
    let mut s = trimmed.trim_start_matches(|c: char| matches!(c, '#' | '-' | '*' | '>' | ' ' | '\t'));
    s = s.trim_matches(|c: char| matches!(c, '*' | '_' | '`' | ':' | ' ' | '\t'));

    let low = s.to_lowercase();
    for prefix in [
        "fichier :",
        "fichier:",
        "fichier ",
        "file :",
        "file:",
        "file ",
        "chemin :",
        "chemin:",
        "path :",
        "path:",
        "cible :",
        "cible:",
    ] {
        if low.starts_with(prefix) && s.len() >= prefix.len() {
            s = s[prefix.len()..].trim();
            break;
        }
    }

    // Support d'un en-tête/étiquette au format lien Markdown (ex. "### [App.vue](src/App.vue)")
    if s.starts_with('[') && s.ends_with(')') {
        if let Some(p) = extract_path_from_text_line(trimmed) {
            return allow_any || applied_set.contains(&p);
        }
    }

    let candidate = s.trim_matches(|c: char| matches!(c, '`' | '\'' | '"' | '*' | '_' | ':' | ' '));
    if candidate.is_empty() || candidate.contains(' ') {
        return false;
    }

    if let Some(p) = sanitize_rel_path(candidate) {
        return allow_any || applied_set.contains(&p);
    }
    false
}

fn is_generic_edit_heading(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return false;
    }
    let clean = trimmed
        .trim_start_matches('#')
        .trim()
        .trim_matches(|c: char| matches!(c, '*' | '_' | '`' | ':' | ' '))
        .to_lowercase();

    matches!(
        clean.as_str(),
        "modifications"
            | "modification"
            | "modifications nécessaires"
            | "changements"
            | "changement"
            | "patch"
            | "diff"
            | "mise à jour"
            | "mises à jour"
            | "code"
            | "fichier modifié"
            | "fichiers modifiés"
            | "changes"
            | "edits"
            | "update"
            | "updates"
    )
}

fn is_code_leadin_clause(clause: &str) -> bool {
    let clean = clause
        .trim()
        .trim_start_matches(|c: char| matches!(c, '#' | '-' | '*' | '•' | '>' | ' ' | '\t'))
        .trim_matches(|c: char| matches!(c, '*' | '_' | '`'))
        .trim()
        .to_lowercase();

    if clean.is_empty() || clean.chars().count() > 140 {
        return false;
    }

    let prefixes = [
        "voici",
        "voilà",
        "ci-dessous",
        "modifications",
        "modification",
        "changements",
        "changement",
        "mise à jour",
        "patch",
        "diff",
        "remplace",
        "ajoute",
        "modifie",
        "applique",
        "édite",
        "crée",
        "code",
        "fichier",
        "here is",
        "here are",
        "below is",
        "below are",
        "updated",
        "changes",
        "replace",
        "apply",
    ];
    if prefixes.iter().any(|p| clean.starts_with(p)) {
        return true;
    }

    let needles = [
        "comme suit",
        "ci-dessous",
        "suivant",
        "modifications nécessaires",
        "changements nécessaires",
        "as follows",
        "following changes",
        "following code",
    ];
    needles.iter().any(|n| clean.contains(n))
}

/// Nettoie une ligne se terminant par `:` juste avant un bloc d'édition supprimé :
/// - `Some(new_line)` remplace la ligne
/// - `None` supprime complètement la ligne d'accroche orpheline
fn clean_dangling_colon_line(line: &str) -> Option<String> {
    let trimmed = line.trim_end();
    if !trimmed.ends_with(':') {
        return Some(line.to_string());
    }

    // Cas A : phrase d'accroche en fin de paragraphe (ex. "...en utilisant du texte. Voici les modifications nécessaires :")
    let mut best_boundary: Option<usize> = None;
    for sep in [". ", "! ", "? "] {
        if let Some(pos) = trimmed.rfind(sep) {
            best_boundary = Some(match best_boundary {
                Some(prev) => prev.max(pos),
                None => pos,
            });
        }
    }
    if let Some(pos) = best_boundary {
        let prefix_with_punct = &trimmed[..=pos];
        let after = trimmed[pos + 1..].trim();
        if is_code_leadin_clause(after) {
            return Some(prefix_with_punct.to_string());
        }
    }

    // Cas B : toute la ligne est une phrase d'accroche vers le code retiré
    if is_code_leadin_clause(trimmed) {
        return None;
    }

    // Cas C : phrase informative terminée par `:` pointant vers le bloc retiré
    let without_colon = trimmed.trim_end_matches(':').trim_end();
    if without_colon.is_empty() {
        return None;
    }
    let is_bullet = trimmed.trim_start().starts_with("- ")
        || trimmed.trim_start().starts_with("* ")
        || trimmed.trim_start().starts_with("• ");
    let ends_with_punct = without_colon
        .chars()
        .last()
        .map(|c| matches!(c, '.' | '!' | '?' | '…'))
        .unwrap_or(false);

    if is_bullet || ends_with_punct {
        Some(without_colon.to_string())
    } else {
        Some(format!("{without_colon}."))
    }
}

fn cleanup_before_removed_block(
    out_lines: &mut Vec<String>,
    applied_set: &HashSet<String>,
    allow_any_path_or_info: bool,
) {
    while out_lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out_lines.pop();
    }

    // Retirer au plus 2 en-têtes orphelins (ex. titre générique + titre de fichier)
    for _ in 0..2 {
        let should_pop = match out_lines.last() {
            Some(last) => {
                is_orphan_file_header_line(last, applied_set, allow_any_path_or_info)
                    || is_generic_edit_heading(last)
            }
            None => false,
        };
        if !should_pop {
            break;
        }
        out_lines.pop();
        while out_lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            out_lines.pop();
        }
    }

    if let Some(last) = out_lines.last() {
        let trimmed = last.trim_end();
        if trimmed.ends_with(':') {
            if allow_any_path_or_info {
                match clean_dangling_colon_line(last) {
                    Some(cleaned) => {
                        if let Some(slot) = out_lines.last_mut() {
                            *slot = cleaned;
                        }
                    }
                    None => {
                        out_lines.pop();
                    }
                }
            } else if is_code_leadin_clause(trimmed) {
                out_lines.pop();
            }
        }
    }

    while out_lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out_lines.pop();
    }
    // Garantir un saut de paragraphe entre ce qui précède et ce qui suit le bloc supprimé
    if !out_lines.is_empty() {
        out_lines.push(String::new());
    }
}

/// Nettoie la réponse finale pour l'affichage utilisateur en conservant les explications
/// et les commandes shell utiles, tout en masquant les tool_calls et blocs déjà appliqués.
pub fn strip_edit_blocks_for_display(text: &str, applied_files: &[String]) -> String {
    let tool_xml_re = Regex::new(r"<tool_call>[\s\S]*?</tool_call>").expect("valid xml regex");
    let no_xml = tool_xml_re.replace_all(text, "");

    let applied_set: HashSet<String> = applied_files
        .iter()
        .filter_map(|p| sanitize_rel_path(p).or_else(|| Some(p.trim().to_string())))
        .collect();

    let norm = no_xml.replace("\r\n", "\n");
    let lines: Vec<&str> = norm.split('\n').collect();
    let mut out_lines: Vec<String> = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let trimmed_start = lines[i].trim_start();
        let tick_count = trimmed_start.bytes().take_while(|&b| b == b'`').count();

        if tick_count >= 3 {
            let fence_start = i;
            let info = trimmed_start[tick_count..].trim();
            let mut body_lines: Vec<&str> = Vec::new();
            i += 1;
            let mut closed = false;
            while i < lines.len() {
                let close_trim = lines[i].trim();
                let close_ticks = close_trim.bytes().take_while(|&b| b == b'`').count();
                if close_ticks >= tick_count && close_trim[close_ticks..].trim().is_empty() {
                    closed = true;
                    break;
                }
                body_lines.push(lines[i]);
                i += 1;
            }
            if closed {
                i += 1;
            }

            let body = body_lines.join("\n");
            let info_low = info.to_lowercase();
            let is_tool_fence = info_low == "tool_call"
                || info_low == "json:tool_call"
                || (body.trim().starts_with("{\"name\"") && body.contains("\"arguments\""));
            let has_sr = body_has_any_search_or_replace_marker(&body);

            let (lang, info_path) = detect_path_from_info(info);
            let is_shell = is_shell_lang(&lang)
                && !info_path
                    .as_deref()
                    .map(is_shell_script_path)
                    .unwrap_or(false);

            let detected_path = info_path
                .or_else(|| detect_path_from_preceding_lines(&lines, fence_start))
                .or_else(|| detect_path_from_first_code_line(&body));

            let targets_applied = detected_path
                .as_ref()
                .map(|p| applied_set.contains(p))
                .unwrap_or(false);

            if is_tool_fence || has_sr || (!is_shell && targets_applied) {
                cleanup_before_removed_block(&mut out_lines, &applied_set, has_sr || targets_applied);
                continue;
            }

            if !body.trim().is_empty() {
                out_lines.push(lines[fence_start].trim_end().to_string());
                for b in body_lines {
                    out_lines.push(b.to_string());
                }
                out_lines.push("`".repeat(tick_count));
            }
            continue;
        }

        // Bloc SEARCH/REPLACE nu hors fence
        if is_search_marker(lines[i]) {
            cleanup_before_removed_block(&mut out_lines, &applied_set, true);
            i += 1;
            while i < lines.len() {
                if is_replace_marker(lines[i]) {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        let clean_line = lines[i].trim_end();
        if clean_line.is_empty() {
            if !out_lines.last().map(|l| l.is_empty()).unwrap_or(true) {
                out_lines.push(String::new());
            }
        } else {
            out_lines.push(clean_line.to_string());
        }
        i += 1;
    }

    // Dédupliquer les paragraphes consécutifs identiques
    let joined = out_lines.join("\n");
    let mut dedup_paras: Vec<String> = Vec::new();
    for para in joined.split("\n\n") {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        if dedup_paras.last().map(|prev| prev.as_str()) != Some(p) {
            dedup_paras.push(p.to_string());
        }
    }

    dedup_paras.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aiderstudio_patcher_test_{}_{}", name, Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_fuzzy_level1_exact_match() {
        let orig = "fn main() {\n    println!(\"hello\");\n}\n";
        let res = apply_search_replace_fuzzy(
            orig,
            "    println!(\"hello\");",
            "    println!(\"bonjour\");",
        )
        .unwrap();
        assert_eq!(res, "fn main() {\n    println!(\"bonjour\");\n}\n");
    }

    #[test]
    fn test_fuzzy_level2_trailing_whitespace() {
        let orig = "let a = 1;   \nlet b = 2;\t\nlet c = 3;\n";
        let res = apply_search_replace_fuzzy(orig, "let a = 1;\nlet b = 2;", "let a = 10;\nlet b = 20;")
            .unwrap();
        assert_eq!(res, "let a = 10;\nlet b = 20;\nlet c = 3;\n");
    }

    #[test]
    fn test_fuzzy_level3_flexible_indent_and_reindent() {
        let orig = "mod inner {\n        fn add(x: i32) -> i32 {\n            x + 1\n        }\n}\n";
        let search = "fn add(x: i32) -> i32 {\n    x + 1\n}";
        let replace = "fn add(x: i32) -> i32 {\n    x + 2\n}";
        let res = apply_search_replace_fuzzy(orig, search, replace).unwrap();
        assert_eq!(
            res,
            "mod inner {\n        fn add(x: i32) -> i32 {\n            x + 2\n        }\n}\n"
        );
    }

    #[test]
    fn test_fuzzy_level4_similarity_and_ambiguity_rejection() {
        let orig = "fn compute(alpha: i32, beta: i32) -> i32 {\n    let sum = alpha + beta;\n    sum * 2\n}\n";
        let search = "fn compute(alpha: i32, beta: i32) -> i32 {\n    let sum = alpha + bta;\n    sum * 2\n}";
        let replace = "fn compute(alpha: i32, beta: i32) -> i32 {\n    (alpha + beta) * 3\n}";
        let res = apply_search_replace_fuzzy(orig, search, replace).unwrap();
        assert!(res.contains("(alpha + beta) * 3"));

        // Rejet en cas de doublon identique au niveau 3 / 4
        let dup_orig = "    let v = 1;\n    return v;\n\n    let v = 1;\n    return v;\n";
        let err = apply_search_replace_fuzzy(dup_orig, "let v = 1;\nreturn v;", "let v = 2;\nreturn v;")
            .unwrap_err();
        assert!(err.closest_match.is_some());
        assert!(err.closest_line.is_some());
    }

    #[test]
    fn test_fuzzy_preserves_crlf() {
        let orig = "line1\r\nline2\r\nline3\r\n";
        let res = apply_search_replace_fuzzy(orig, "line2", "line2_mod").unwrap();
        assert_eq!(res, "line1\r\nline2_mod\r\nline3\r\n");
    }

    #[test]
    fn test_parse_backticks_and_multi_hunks() {
        let reply = r#"Mise à jour du fichier.

### `src/demo.ts`
```ts
<<<<<<< SEARCH
const a = 1;
=======
const a = 10;
>>>>>>> REPLACE
<<<<<<< SEARCH
const b = 2;
=======
const b = 20;
>>>>>>> REPLACE
```"#;

        let blocks = parse_edit_blocks(reply, &[], &[], "");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            EditBlock::SearchReplace { path, search, replace } => {
                assert_eq!(path, "src/demo.ts");
                assert_eq!(search, "const a = 1;");
                assert_eq!(replace, "const a = 10;");
            }
            _ => panic!("Expected SearchReplace"),
        }
        match &blocks[1] {
            EditBlock::SearchReplace { path, search, replace } => {
                assert_eq!(path, "src/demo.ts");
                assert_eq!(search, "const b = 2;");
                assert_eq!(replace, "const b = 20;");
            }
            _ => panic!("Expected SearchReplace"),
        }
    }

    #[tokio::test]
    async fn test_apply_edits_blocks_whole_file_when_truncated() {
        let dir = make_temp_dir("trunc");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/big.rs"), "fn keep_me() {}\n").unwrap();

        let mut truncated = HashSet::new();
        truncated.insert("src/big.rs".to_string());

        let reply = "### src/big.rs\n```rust\nfn truncated_overwrite() {}\n```";
        let outcome = apply_edits_from_response(&dir, reply, &[], &[], &truncated, "").await;

        assert!(outcome.applied_files.is_empty());
        assert_eq!(outcome.failures.len(), 1);
        assert!(outcome.failures[0].reason.contains("tronqué"));
        assert_eq!(
            std::fs::read_to_string(dir.join("src/big.rs")).unwrap(),
            "fn keep_me() {}\n"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_apply_edits_creates_new_file_and_strips_display() {
        let dir = make_temp_dir("newfile");
        let reply = r#"J'ai créé le module utilitaire.

### src/utils/math.ts
```ts
export function square(n: number): number {
  return n * n;
}
```

<tool_call>{"name":"list_files","arguments":{}}</tool_call>

```bash
npm run build
```"#;

        let outcome =
            apply_edits_from_response(&dir, reply, &[], &[], &HashSet::new(), "crée math.ts").await;
        assert_eq!(outcome.applied_files, vec!["src/utils/math.ts"]);
        assert!(dir.join("src/utils/math.ts").exists());

        let display = strip_edit_blocks_for_display(reply, &outcome.applied_files);
        assert!(display.contains("J'ai créé le module utilitaire."));
        assert!(display.contains("npm run build"));
        assert!(!display.contains("export function square"));
        assert!(!display.contains("<tool_call>"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ellipsis_level4_patch() {
        let orig = "fn alpha() {\n    let x = 1;\n    let y = 2;\n    x + y\n}\n";
        let search = "fn alpha() {\n...\n    x + y\n}";
        let replace = "fn alpha() {\n    42\n}";
        let res = apply_search_replace_fuzzy(orig, search, replace).unwrap();
        assert_eq!(res, "fn alpha() {\n    42\n}\n");
    }

    #[test]
    fn test_sanitize_rel_path_blocks_unsafe() {
        assert_eq!(sanitize_rel_path("`./src\\App.vue`").as_deref(), Some("src/App.vue"));
        assert_eq!(sanitize_rel_path("../secret.txt"), None);
        assert_eq!(sanitize_rel_path("/etc/passwd"), None);
        assert_eq!(sanitize_rel_path(".env.local"), None);
        assert_eq!(sanitize_rel_path(".git/config"), None);
        assert_eq!(sanitize_rel_path("https://example.com/a.js"), None);
    }

    #[test]
    fn test_strip_display_removes_trailing_leadin_sentence_user_case() {
        let reply = r#"Pour ajouter l'emote 🙀 à la place de l'image, nous devons modifier le fichier src/App.vue pour afficher cette emote en utilisant du texte. Voici les modifications nécessaires :

### src/App.vue
```vue
<<<<<<< SEARCH
<img src="./cat.png" />
=======
<span class="emote">🙀</span>
>>>>>>> REPLACE
```

Ces modifications remplacent l'image par l'emote 🙀 et affichent le titre "MinouAI" en blanc au centre de la page."#;

        let display = strip_edit_blocks_for_display(reply, &["src/App.vue".to_string()]);
        assert!(!display.contains("Voici les modifications nécessaires"));
        assert!(!display.contains("<<<<<<< SEARCH"));
        assert!(!display.ends_with(':'));
        assert!(!display.contains("\n:\n"));
        assert_eq!(
            display,
            "Pour ajouter l'emote 🙀 à la place de l'image, nous devons modifier le fichier src/App.vue pour afficher cette emote en utilisant du texte.\n\nCes modifications remplacent l'image par l'emote 🙀 et affichent le titre \"MinouAI\" en blanc au centre de la page."
        );
    }

    #[test]
    fn test_strip_display_removes_standalone_leadin_and_blank_line_header() {
        let reply = r#"Voici le code mis à jour :

### src/foo.ts

```ts
export const x = 42;
```

La constante vaut désormais 42."#;

        let display = strip_edit_blocks_for_display(reply, &["src/foo.ts".to_string()]);
        assert!(!display.contains("Voici le code"));
        assert!(!display.contains("src/foo.ts"));
        assert!(!display.contains("export const x = 42;"));
        assert_eq!(display, "La constante vaut désormais 42.");
    }

    #[test]
    fn test_strip_display_replaces_informative_dangling_colon() {
        let reply = r#"Nous mettons à jour l'en-tête dans src/App.vue :

```vue
<<<<<<< SEARCH
<h1>Ancien</h1>
=======
<h1>MinouAI</h1>
>>>>>>> REPLACE
```"#;

        let display = strip_edit_blocks_for_display(reply, &["src/App.vue".to_string()]);
        assert_eq!(display, "Nous mettons à jour l'en-tête dans src/App.vue.");
    }

    #[test]
    fn test_strip_display_keeps_leadin_for_kept_code_blocks() {
        let reply = r#"Voici la commande à lancer :

```bash
npm run dev
```"#;

        let display = strip_edit_blocks_for_display(reply, &[]);
        assert!(display.contains("Voici la commande à lancer :"));
        assert!(display.contains("```bash\nnpm run dev\n```"));
    }

    #[test]
    fn test_sanitize_and_extract_markdown_link_header() {
        assert_eq!(sanitize_rel_path("[App.vue](src/App.vue)"), None);
        assert_eq!(
            extract_path_from_text_line("### [App.vue](src/App.vue)").as_deref(),
            Some("src/App.vue")
        );
        assert_eq!(
            extract_path_from_text_line("Dans [index.css](src/index.css#L12) :").as_deref(),
            Some("src/index.css")
        );
    }

    #[tokio::test]
    async fn test_resolve_basename_against_project_files() {
        let dir = make_temp_dir("resolve_base");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/index.css"), "body {\n  background: #ffffff;\n}\n").unwrap();
        std::fs::write(dir.join("src/App.vue"), "<template><div/></template>\n").unwrap();

        let all_files = vec!["src/index.css".to_string(), "src/App.vue".to_string()];
        let reply = r#"### index.css
```css
<<<<<<< SEARCH
body {
  background: #ffffff;
}
=======
body {
  background: #2b2d31;
}
>>>>>>> REPLACE
```"#;

        let outcome = apply_edits_from_response(
            &dir,
            reply,
            &[],
            &all_files,
            &HashSet::new(),
            "mets un fond gris anthracite sur la page",
        )
        .await;
        assert_eq!(outcome.applied_files, vec!["src/index.css"]);
        let disk = std::fs::read_to_string(dir.join("src/index.css")).unwrap();
        assert!(disk.contains("#2b2d31"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tolerant_five_char_search_replace_markers() {
        let reply = r#"### src/index.css
```css
<<<<< SEARCH
body { background: white; }
=====
body { background: #2f3136; }
>>>>> REPLACE
```"#;
        let blocks = parse_edit_blocks(reply, &[], &[], "");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            EditBlock::SearchReplace {
                path,
                search,
                replace,
            } => {
                assert_eq!(path, "src/index.css");
                assert_eq!(search, "body { background: white; }");
                assert_eq!(replace, "body { background: #2f3136; }");
            }
            _ => panic!("Expected SearchReplace"),
        }
    }

    #[tokio::test]
    async fn test_single_target_implicit_whole_file_and_partial_guardrail() {
        let dir = make_temp_dir("implicit_guard");
        std::fs::create_dir_all(dir.join("src")).unwrap();

        // 1. Petit fichier -> remplacement sans en-tête accepté si cible unique de même langage
        std::fs::write(dir.join("src/index.css"), "body { color: black; }\n").unwrap();
        let targets = vec!["src/index.css".to_string()];
        let reply_small = "```css\nbody {\n  background: #2f3136;\n  color: #ffffff;\n}\n```";
        let out1 = apply_edits_from_response(
            &dir,
            reply_small,
            &targets,
            &targets,
            &HashSet::new(),
            "mets un fond gris anthracite",
        )
        .await;
        assert_eq!(out1.applied_files, vec!["src/index.css"]);
        assert!(std::fs::read_to_string(dir.join("src/index.css"))
            .unwrap()
            .contains("#2f3136"));

        // 2. Gros fichier (>= 15 lignes) -> extrait partiel sans en-tête refusé avec PatchFailure
        let big_css: String = (1..=20)
            .map(|i| format!(".rule-{i} {{ margin: {i}px; }}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.join("src/index.css"), &big_css).unwrap();
        let reply_partial = "```css\nbody { background: #2f3136; }\n```";
        let out2 = apply_edits_from_response(
            &dir,
            reply_partial,
            &targets,
            &targets,
            &HashSet::new(),
            "mets un fond gris anthracite",
        )
        .await;
        assert!(out2.applied_files.is_empty());
        assert_eq!(out2.failures.len(), 1);
        assert!(out2.failures[0].reason.contains("Bloc partiel"));
        assert_eq!(std::fs::read_to_string(dir.join("src/index.css")).unwrap(), big_css);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
