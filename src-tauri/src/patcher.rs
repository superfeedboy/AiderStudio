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

/// Normalise les alias de noms d'outils émis par différents modèles locaux ou cloud.
pub fn normalize_tool_name(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "read_file" | "read" | "view_file" => Some("read_file"),
        "write_file" | "create_file" | "write" => Some("write_file"),
        "edit_file" | "patch_file" | "replace_in_file" => Some("edit_file"),
        "delete_file" | "remove_file" => Some("delete_file"),
        "run_command" | "bash" | "shell" | "exec" | "execute_command" | "terminal" => {
            Some("run_command")
        }
        "search_code" | "grep" | "ripgrep" => Some("search_code"),
        "list_files" | "ls" => Some("list_files"),
        "web_search" | "search_web" => Some("web_search"),
        "fetch_url" | "read_url" => Some("fetch_url"),
        "mcp_call" => Some("mcp_call"),
        _ => None,
    }
}

/// Normalise les alias de clés d'arguments d'outils (ex. `cmd` -> `command`, `file_path` -> `path`).
pub fn normalize_tool_arguments(
    canonical_name: &str,
    raw_args: serde_json::Value,
) -> serde_json::Value {
    let resolved = match raw_args {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
                parsed
            } else {
                match canonical_name {
                    "run_command" => serde_json::json!({ "command": trimmed }),
                    "web_search" => serde_json::json!({ "query": trimmed }),
                    "fetch_url" => serde_json::json!({ "url": trimmed }),
                    "read_file" | "delete_file" => serde_json::json!({ "path": trimmed }),
                    _ => serde_json::json!({}),
                }
            }
        }
        other => other,
    };

    let serde_json::Value::Object(mut map) = resolved else {
        return serde_json::json!({});
    };

    match canonical_name {
        "run_command" => {
            let has_cmd = map
                .get("command")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_cmd {
                let mut found_cmd: Option<String> = None;
                for alias in ["command", "cmd", "script", "shell", "command_line", "commandLine"] {
                    if let Some(val) = map.get(alias) {
                        if let Some(s) = val.as_str() {
                            if !s.trim().is_empty() {
                                found_cmd = Some(s.trim().to_string());
                                break;
                            }
                        } else if let Some(arr) = val.as_array() {
                            let joined: Vec<String> = arr
                                .iter()
                                .filter_map(|x| x.as_str().map(str::trim))
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                                .collect();
                            if !joined.is_empty() {
                                found_cmd = Some(joined.join(" "));
                                break;
                            }
                        }
                    }
                }
                if let Some(c) = found_cmd {
                    map.insert("command".to_string(), serde_json::Value::String(c));
                }
            }
        }
        "read_file" | "write_file" | "edit_file" | "delete_file" => {
            let has_path = map
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_path {
                for alias in ["file_path", "filePath", "file", "filename", "target", "target_file"] {
                    if let Some(s) = map.get(alias).and_then(|v| v.as_str()) {
                        if !s.trim().is_empty() {
                            map.insert(
                                "path".to_string(),
                                serde_json::Value::String(s.trim().to_string()),
                            );
                            break;
                        }
                    }
                }
            }
            if canonical_name == "write_file" && !map.contains_key("content") {
                for alias in ["text", "code", "body", "contents", "file_content", "new_content"] {
                    if let Some(s) = map.get(alias).and_then(|v| v.as_str()) {
                        map.insert(
                            "content".to_string(),
                            serde_json::Value::String(s.to_string()),
                        );
                        break;
                    }
                }
            }
        }
        "web_search" => {
            if !map.contains_key("query") {
                for alias in ["q", "search", "keywords"] {
                    if let Some(s) = map.get(alias).and_then(|v| v.as_str()) {
                        map.insert(
                            "query".to_string(),
                            serde_json::Value::String(s.to_string()),
                        );
                        break;
                    }
                }
            }
        }
        "search_code" => {
            if !map.contains_key("query") {
                for alias in ["pattern", "regex", "search", "text"] {
                    if let Some(s) = map.get(alias).and_then(|v| v.as_str()) {
                        map.insert(
                            "query".to_string(),
                            serde_json::Value::String(s.to_string()),
                        );
                        break;
                    }
                }
            }
            if !map.contains_key("path_filter") {
                for alias in ["path", "dir", "directory"] {
                    if let Some(s) = map.get(alias).and_then(|v| v.as_str()) {
                        map.insert(
                            "path_filter".to_string(),
                            serde_json::Value::String(s.to_string()),
                        );
                        break;
                    }
                }
            }
        }
        _ => {}
    }

    serde_json::Value::Object(map)
}

/// Extrait zéro, un ou plusieurs appels d'outils normalisés à partir d'une valeur JSON.
pub fn parse_json_tool_call_value(val: &serde_json::Value) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    match val {
        serde_json::Value::Array(arr) => {
            for item in arr {
                out.extend(parse_json_tool_call_value(item));
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(arr) = map
                .get("tool_calls")
                .or_else(|| map.get("calls"))
                .and_then(|v| v.as_array())
            {
                for item in arr {
                    out.extend(parse_json_tool_call_value(item));
                }
                if !out.is_empty() {
                    return out;
                }
            }

            if let Some(fn_obj) = map.get("function").and_then(|v| v.as_object()) {
                if let Some(raw_name) = fn_obj.get("name").and_then(|v| v.as_str()) {
                    if let Some(canon) = normalize_tool_name(raw_name) {
                        let raw_args = fn_obj
                            .get("arguments")
                            .or_else(|| fn_obj.get("parameters"))
                            .or_else(|| fn_obj.get("args"))
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        out.push((canon.to_string(), normalize_tool_arguments(canon, raw_args)));
                        return out;
                    }
                }
            }

            let raw_name_opt = map
                .get("name")
                .or_else(|| map.get("tool"))
                .or_else(|| map.get("tool_name"))
                .or_else(|| map.get("action"))
                .and_then(|v| v.as_str());

            if let Some(raw_name) = raw_name_opt {
                if let Some(canon) = normalize_tool_name(raw_name) {
                    if canon == "mcp_call"
                        && map.contains_key("server")
                        && map.contains_key("tool")
                    {
                        let inner_args = map
                            .get("arguments")
                            .or_else(|| map.get("parameters"))
                            .or_else(|| map.get("args"))
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        out.push((
                            "mcp_call".to_string(),
                            serde_json::json!({
                                "server": map.get("server").cloned().unwrap_or_else(|| serde_json::json!("")),
                                "tool": map.get("tool").cloned().unwrap_or_else(|| serde_json::json!("")),
                                "arguments": inner_args
                            }),
                        ));
                        return out;
                    }

                    let raw_args = if let Some(explicit) = map
                        .get("arguments")
                        .or_else(|| map.get("parameters"))
                        .or_else(|| map.get("args"))
                        .or_else(|| map.get("input"))
                    {
                        explicit.clone()
                    } else {
                        let mut flat = map.clone();
                        flat.remove("name");
                        flat.remove("tool");
                        flat.remove("tool_name");
                        flat.remove("action");
                        flat.remove("type");
                        flat.remove("id");
                        serde_json::Value::Object(flat)
                    };
                    out.push((canon.to_string(), normalize_tool_arguments(canon, raw_args)));
                }
            }
        }
        _ => {}
    }
    out
}

/// Échappe les caractères de contrôle bruts (`\n`, `\r`, `\t`, etc.) à l'intérieur des littéraux de chaîne JSON.
pub fn escape_raw_control_chars_in_json_strings(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;

    for c in raw.chars() {
        if !in_string {
            if c == '"' {
                in_string = true;
            }
            out.push(c);
            continue;
        }

        if escaped {
            if (c as u32) < 0x20 {
                match c {
                    '\r' => {
                        // Conserve `escaped = true` pour absorber un éventuel `\n` (CRLF) après `\`
                        continue;
                    }
                    '\n' => {
                        out.push('n');
                    }
                    '\t' => {
                        out.push('t');
                    }
                    _ => {
                        // Remplace un caractère de contrôle inattendu après `\` par un échappement neutre
                        out.push('\\');
                    }
                }
                escaped = false;
            } else {
                out.push(c);
                escaped = false;
            }
            continue;
        }

        match c {
            '\\' => {
                escaped = true;
                out.push('\\');
            }
            '"' => {
                in_string = false;
                out.push('"');
            }
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {}
            _ => out.push(c),
        }
    }

    out
}

/// Supprime les virgules finales (`trailing commas`) avant `}` ou `]` hors chaînes JSON.
pub fn remove_json_trailing_commas(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0usize;

    while i < len {
        let c = chars[i];
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }

        if c == ',' {
            let mut j = i + 1;
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            if j < len && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }

        out.push(c);
        i += 1;
    }

    out
}

fn strip_outer_markdown_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    if !trimmed.starts_with("```") {
        return trimmed;
    }
    let Some(first_nl) = trimmed.find('\n') else {
        return trimmed;
    };
    let body = &trimmed[first_nl + 1..];
    if let Some(last_fence) = body.rfind("\n```") {
        let after = body[last_fence + 4..].trim();
        if after.is_empty() {
            return body[..last_fence].trim();
        }
    }
    if let Some(stripped) = body.strip_suffix("```") {
        return stripped.trim();
    }
    trimmed
}

/// Répare automatiquement les erreurs courantes de syntaxe JSON (caractères de contrôle non échappés,
/// virgules finales, balise Markdown englobante) puis valide via `serde_json` avant écriture sur disque.
pub fn repair_and_validate_json_str(raw: &str) -> Result<String, String> {
    let candidate = strip_outer_markdown_fence(raw);
    if candidate.is_empty() {
        return Err("Contenu JSON vide".to_string());
    }

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(candidate) {
        return serde_json::to_string_pretty(&val)
            .map(|s| format!("{s}\n"))
            .map_err(|e| e.to_string());
    }

    // Remplace les retours ligne/tabulations bruts dans les chaînes par des séquences sûres + retire les virgules finales
    let repaired = remove_json_trailing_commas(&escape_raw_control_chars_in_json_strings(candidate));
    match serde_json::from_str::<serde_json::Value>(&repaired) {
        Ok(val) => serde_json::to_string_pretty(&val)
            .map(|s| format!("{s}\n"))
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Analyse un bloc brut (contenu d'une balise `<tool_call>` ou d'un bloc JSON) pour en extraire tous les appels d'outils.
pub fn extract_json_tool_calls_from_block(raw: &str) -> Vec<(String, serde_json::Value)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let parsed = parse_json_tool_call_value(&val);
        if !parsed.is_empty() {
            return parsed;
        }
    }

    let mut out = Vec::new();
    let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<serde_json::Value>();
    for item in stream {
        match item {
            Ok(val) => out.extend(parse_json_tool_call_value(&val)),
            Err(_) => break,
        }
    }
    if !out.is_empty() {
        return out;
    }

    // Repli déterministe si le LLM a laissé des retours ligne/tabulations bruts ou des virgules finales dans son JSON d'outil
    let repaired = remove_json_trailing_commas(&escape_raw_control_chars_in_json_strings(trimmed));
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&repaired) {
        let parsed = parse_json_tool_call_value(&val);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    let repaired_stream =
        serde_json::Deserializer::from_str(&repaired).into_iter::<serde_json::Value>();
    for item in repaired_stream {
        match item {
            Ok(val) => out.extend(parse_json_tool_call_value(&val)),
            Err(_) => break,
        }
    }
    out
}

/// Indique si un bloc Markdown clôturé correspond à un appel d'outil (et ne doit donc pas être affiché ni écrit comme un fichier).
pub fn is_tool_call_fence(info: &str, body: &str) -> bool {
    let info_low = info.trim().to_ascii_lowercase();
    if matches!(info_low.as_str(), "tool_call" | "json:tool_call" | "tool") {
        return true;
    }
    if !extract_json_tool_calls_from_block(body).is_empty() {
        return true;
    }

    // Filet anti-fuite pour les blocs JSON d'appels d'outils tronqués/incomplets
    let body_trim = body.trim();
    if (body_trim.starts_with('{') || body_trim.starts_with('['))
        && Regex::new(
            r#"^\s*[\[{]\s*"(?:name|tool|tool_name|function)"\s*:\s*"(?:read_file|write_file|create_file|edit_file|delete_file|run_command|search_code|list_files|web_search|fetch_url|mcp_call|bash|shell|exec|execute_command|patch_file)""#,
        )
        .map(|re| re.is_match(body_trim))
        .unwrap_or(false)
    {
        return true;
    }

    false
}

fn find_fenced_byte_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut pos = 0usize;
    let mut in_fence: Option<(usize, usize)> = None;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let tick_count = trimmed.bytes().take_while(|&b| b == b'`').count();
        if let Some((start_pos, open_ticks)) = in_fence {
            let close_trim = line.trim();
            let close_ticks = close_trim.bytes().take_while(|&b| b == b'`').count();
            if close_ticks >= open_ticks && close_trim[close_ticks..].trim().is_empty() {
                ranges.push((start_pos, pos + line.len()));
                in_fence = None;
            }
        } else if tick_count >= 3 {
            in_fence = Some((pos, tick_count));
        }
        pos += line.len();
    }
    if let Some((start_pos, _)) = in_fence {
        ranges.push((start_pos, text.len()));
    }
    ranges
}

fn collect_bare_json_tool_spans(text: &str) -> Vec<(usize, usize, Vec<(String, serde_json::Value)>)> {
    let fence_ranges = find_fenced_byte_ranges(text);
    let mut spans = Vec::new();
    let mut skip_until = 0usize;

    let is_inside_fence = |idx: usize| -> bool {
        fence_ranges.iter().any(|&(s, e)| idx >= s && idx < e)
    };

    for (idx, ch) in text.char_indices() {
        if idx < skip_until || is_inside_fence(idx) {
            continue;
        }
        if ch != '{' && ch != '[' {
            continue;
        }

        let prev_ok = if idx == 0 {
            true
        } else {
            text[..idx]
                .chars()
                .next_back()
                .map(|c| c.is_whitespace() || c == ':' || c == '>')
                .unwrap_or(true)
        };
        if !prev_ok {
            continue;
        }

        let open_ch = ch;
        let close_ch = if open_ch == '{' { '}' } else { ']' };
        let mut depth = 0usize;
        let mut in_str = false;
        let mut escaped = false;
        let mut end_idx: Option<usize> = None;

        for (rel_off, c) in text[idx..].char_indices() {
            if in_str {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_str = false;
                }
            } else if c == '"' {
                in_str = true;
            } else if c == open_ch {
                depth += 1;
            } else if c == close_ch {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    end_idx = Some(idx + rel_off + c.len_utf8());
                    break;
                }
            }
        }

        if let Some(end) = end_idx {
            let slice = &text[idx..end];
            let parsed_val = serde_json::from_str::<serde_json::Value>(slice).or_else(|_| {
                let repaired = remove_json_trailing_commas(
                    &escape_raw_control_chars_in_json_strings(slice),
                );
                serde_json::from_str::<serde_json::Value>(&repaired)
            });
            if let Ok(val) = parsed_val {
                let calls = parse_json_tool_call_value(&val);
                if !calls.is_empty() {
                    spans.push((idx, end, calls));
                    skip_until = end;
                }
            }
        }
    }

    spans
}

/// Extrait les appels d'outils JSON nus présents en dehors des blocs de code clôturés.
pub fn extract_bare_json_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
    collect_bare_json_tool_spans(text)
        .into_iter()
        .flat_map(|(_, _, calls)| calls)
        .collect()
}

/// Supprime les objets/tableaux JSON d'appels d'outils nus présents hors des fences Markdown.
pub fn strip_bare_json_tool_calls_from_prose(text: &str) -> String {
    let spans = collect_bare_json_tool_spans(text);
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (start, end, _) in spans {
        if start >= cursor {
            out.push_str(&text[cursor..start]);
            cursor = end;
        }
    }
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    out
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
            | ("js" | "javascript" | "jsx" | "mjs" | "cjs", "js" | "jsx" | "mjs" | "cjs")
            | ("vue", "vue")
            | ("svelte", "svelte")
            | ("rs" | "rust", "rs")
            | ("py" | "python", "py")
            | ("go" | "golang", "go")
            | ("c" | "h", "c" | "h")
            | ("cpp" | "cxx" | "cc" | "hpp", "cpp" | "cxx" | "cc" | "hpp")
            | ("cs" | "csharp", "cs")
            | ("java", "java")
            | ("kt" | "kotlin" | "kts", "kt" | "kts")
            | ("php", "php")
            | ("rb" | "ruby", "rb")
            | ("swift", "swift")
            | ("zig", "zig")
            | ("lua", "lua")
            | ("dart", "dart")
            | ("ex" | "exs" | "elixir", "ex" | "exs")
            | ("sh" | "bash" | "zsh", "sh" | "bash" | "zsh")
            | ("sql", "sql")
            | ("json" | "jsonc", "json" | "jsonc")
            | ("toml", "toml")
            | ("xml" | "svg", "xml" | "svg")
            | ("yaml" | "yml", "yaml" | "yml")
            | ("md" | "markdown", "md")
    )
}

/// Indique si l'extension du fichier correspond à un actif binaire (image, police, archive, binaire compilé, base SQLite...)
/// dans lequel aucun texte brut ou code Markdown ne doit être écrit.
pub fn is_binary_or_non_text_path(path: &str) -> bool {
    let ext = path
        .trim()
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "ico"
            | "bmp"
            | "tiff"
            | "avif"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "eot"
            | "mp3"
            | "mp4"
            | "wav"
            | "ogg"
            | "webm"
            | "mov"
            | "avi"
            | "pdf"
            | "zip"
            | "tar"
            | "gz"
            | "tgz"
            | "7z"
            | "rar"
            | "exe"
            | "dll"
            | "so"
            | "dylib"
            | "bin"
            | "dmg"
            | "pkg"
            | "sqlite"
            | "sqlite3"
            | "db"
            | "wasm"
            | "pyc"
            | "class"
            | "o"
            | "a"
    )
}

/// Vérifie si un chemin candidat extrait d'un en-tête ou de la prose ressemble à un vrai chemin relatif
/// de code projet et non à une mention de bibliothèque/runtime (ex: `Vue.js`, `Node.js`, `FastAPI.py`, `Axum.rs`)
/// ou à une chaîne de propriétés d'objet (ex: `build.rollupOptions.external`).
fn is_plausible_code_path(path: &str) -> bool {
    let clean = path.trim().replace('\\', "/");
    let clean = clean.strip_prefix("./").unwrap_or(&clean);
    if clean.is_empty() || is_binary_or_non_text_path(clean) {
        return false;
    }

    // Tout chemin contenant un séparateur de sous-dossier (ex: `src/App.vue`, `lib/math.rs`, `pkg/api.go`)
    // est un chemin de code légitime.
    if clean.contains('/') {
        return true;
    }

    // Fichiers manifestes ou scripts racine standards (sans extension ou avec syntaxe standard)
    let low = clean.to_lowercase();
    if matches!(
        low.as_str(),
        "makefile"
            | "justfile"
            | "dockerfile"
            | "gemfile"
            | "procfile"
            | "rakefile"
            | "cmakelists.txt"
            | "requirements.txt"
            | "cargo.toml"
            | "readme.md"
            | "license"
            | "licence"
    ) {
        return true;
    }

    let dot_count = clean.matches('.').count();
    if dot_count == 0 {
        return false;
    }

    // Chaîne de propriétés ou identifiants avec plusieurs points sans être un format de config/test reconnu
    // (ex: rejette `build.rollupOptions.external`, `app.state.counter`)
    if dot_count > 1
        && !low.contains(".config.")
        && !low.ends_with(".d.ts")
        && !low.contains(".test.")
        && !low.contains(".spec.")
    {
        return false;
    }

    // À la racine du projet, les fichiers réels sont en minuscules (ex: `main.rs`, `index.html`, `app.py`).
    // Les mentions de frameworks ou bibliothèques dans la prose commencent par une majuscule
    // (ex: `Vue.js`, `Node.js`, `React.js`, `Three.js`, `FastAPI.py`, `Axum.rs`, `Tailwind.css`).
    // Les composants capitalisés résident légitimement dans un sous-dossier (ex: `src/App.vue`).
    let first_char = clean.chars().next().unwrap_or(' ');
    if first_char.is_ascii_uppercase() {
        return false;
    }

    true
}

fn accept_extracted_candidate_path(p: String) -> Option<String> {
    if is_plausible_code_path(&p) {
        Some(p)
    } else {
        None
    }
}

fn extract_path_from_text_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let low = trimmed.to_lowercase();
    if low.starts_with("$ ")
        || low.starts_with("> ")
        || low.starts_with("% ")
        || low.starts_with("cd ")
        || low.starts_with("git ")
        || low.starts_with("npm ")
        || low.starts_with("pnpm ")
        || low.starts_with("yarn ")
        || low.starts_with("bun ")
        || low.starts_with("npx ")
        || low.starts_with("cargo ")
        || low.starts_with("pip ")
        || low.starts_with("python ")
        || low.starts_with("python3 ")
        || low.starts_with("go ")
        || low.starts_with("make ")
        || low.starts_with("docker ")
        || low.starts_with("composer ")
    {
        return None;
    }

    // Support d'un en-tête/mention au format lien Markdown [NomFichier](chemin/relatif.ext#L10)
    if let Ok(md_link_re) = Regex::new(r"\[[^\]]+\]\(([^)\s]+)\)") {
        for cap in md_link_re.captures_iter(trimmed) {
            if let Some(href_m) = cap.get(1) {
                let raw_href = href_m.as_str().split('#').next().unwrap_or("");
                if let Some(p) = sanitize_rel_path(raw_href).and_then(accept_extracted_candidate_path) {
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
        if let Some(p) = sanitize_rel_path(direct_clean).and_then(accept_extracted_candidate_path) {
            return Some(p);
        }
    }

    if let Some(cap) = path_re.captures(trimmed) {
        if let Some(m) = cap.get(1) {
            if let Some(p) = sanitize_rel_path(m.as_str()).and_then(accept_extracted_candidate_path) {
                return Some(p);
            }
        }
    }
    None
}

fn is_explicit_file_label_line(line: &str) -> bool {
    let low = line
        .trim()
        .trim_start_matches(['#', '-', '*', '>', ' ', '\t'])
        .trim_start_matches(['*', '_', '`'])
        .to_lowercase();
    [
        "fichier",
        "file",
        "chemin",
        "path",
        "cible",
        "target",
        "créer",
        "creer",
        "create",
        "modifier",
        "update",
    ]
    .iter()
    .any(|kw| {
        low.starts_with(kw)
            && low[kw.len()..]
                .chars()
                .next()
                .map(|c| c.is_whitespace() || c == ':')
                .unwrap_or(false)
    })
}

fn is_standalone_header_or_link(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.contains("](") {
        return true;
    }
    let direct_clean = trimmed
        .trim_start_matches(['#', '-', '*', '>', ' ', '\t'])
        .trim()
        .trim_matches(['`', '\'', '"', '*', '_', ':'])
        .trim();
    !direct_clean.is_empty() && !direct_clean.contains(' ')
}

fn path_matches_existing_files(
    candidate: &str,
    allowed_targets: &[String],
    all_project_files: &[String],
) -> bool {
    let clean = candidate.trim().replace('\\', "/");
    let clean = clean.strip_prefix("./").unwrap_or(&clean);
    let lower = clean.to_lowercase();
    let suffix = format!("/{lower}");
    let is_basename = !lower.contains('/');

    let check_slice = |list: &[String]| -> bool {
        list.iter().any(|p| {
            let p_low = p.to_lowercase();
            p_low == lower
                || p_low.ends_with(&suffix)
                || (is_basename && p_low.rsplit('/').next().unwrap_or("") == lower)
        })
    };

    check_slice(allowed_targets) || check_slice(all_project_files)
}

fn is_likely_dotted_property_or_symbol(
    candidate: &str,
    allowed_targets: &[String],
    all_project_files: &[String],
) -> bool {
    if path_matches_existing_files(candidate, allowed_targets, all_project_files) {
        return false;
    }
    if candidate.contains('/') || candidate.contains('\\') {
        return false;
    }
    let segments: Vec<&str> = candidate.split('.').collect();
    if segments.len() >= 3 {
        // Ex. `build.rollupOptions.external` : chaîne de propriétés camelCase sans `/`
        if segments.iter().any(|seg| {
            seg.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && seg.chars().any(|c| c.is_ascii_uppercase())
        }) {
            return true;
        }
    }
    false
}

fn detect_path_from_preceding_lines(
    lines: &[&str],
    fence_line_idx: usize,
    allowed_targets: &[String],
    all_project_files: &[String],
) -> Option<String> {
    let mut checked = 0usize;
    let mut idx = fence_line_idx;
    while idx > 0 && checked < 2 {
        idx -= 1;
        let line = lines[idx].trim();
        if line.is_empty() {
            continue;
        }
        checked += 1;
        if let Some(p) = extract_path_from_text_line(line) {
            if is_standalone_header_or_link(line)
                || is_explicit_file_label_line(line)
                || path_matches_existing_files(&p, allowed_targets, all_project_files)
            {
                return Some(p);
            }
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

/// Si aucun chemin n'est explicitement spécifié pour un bloc de code et qu'il n'y a pas d'en-tête,
/// vérifie si l'espace de travail comporte un et un seul fichier correspondant à ce langage
/// (ex. un unique `src/main.rs`, ou un unique `app.py`, ou un unique `src/App.vue`).
/// Cette inférence est 100 % déterministe et agnostique à toute stack.
fn unique_workspace_target_for_lang(
    lang: &str,
    all_project_files: &[String],
    already_seen: &HashSet<String>,
) -> Option<String> {
    let clean_lang = lang.trim();
    if clean_lang.is_empty() {
        return None;
    }
    let candidates: Vec<&String> = all_project_files
        .iter()
        .filter(|f| !already_seen.contains(*f) && lang_matches_path(clean_lang, f))
        .collect();

    if candidates.len() == 1 {
        Some(candidates[0].clone())
    } else {
        None
    }
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
    _user_prompt: &str,
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
            let body_trim = body.trim();
            if is_tool_call_fence(&info_low, body_trim) {
                continue;
            }

            let (lang, info_path) = detect_path_from_info(info);
            let has_sr_markers = body_has_search_replace_pair(&body);

            let explicit_or_context_path = info_path
                .or_else(|| {
                    detect_path_from_preceding_lines(
                        &lines,
                        fence_start_idx,
                        allowed_targets,
                        all_project_files,
                    )
                })
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
                if let Some(fb) = unique_workspace_target_for_lang(
                    &lang,
                    all_project_files,
                    &seen_whole_paths,
                ) {
                    detected_path = Some(fb);
                    implicit_target = true;
                }
            }

            let Some(raw_path) = detected_path else {
                continue;
            };
            let path = resolve_project_rel_path(&raw_path, allowed_targets, all_project_files);

            if !is_plausible_code_path(&path)
                || (!has_sr_markers
                    && is_likely_dotted_property_or_symbol(
                        &path,
                        allowed_targets,
                        all_project_files,
                    ))
            {
                continue;
            }

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
                let detected = detect_path_from_preceding_lines(
                    &lines,
                    sr_start_idx,
                    allowed_targets,
                    all_project_files,
                )
                .or_else(|| {
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

    if search_norm.trim().is_empty() {
        if orig_norm.trim().is_empty() {
            return Ok(restore_eol(rep_norm, had_crlf));
        }
        return Err(make_err(
            "Le bloc SEARCH est vide ou ne contient que des espaces pour un fichier existant non vide. Utilise `write_file` pour réécrire le fichier complet.",
            None,
            None,
        ));
    }

    // Niveau 1 : Correspondance exacte
    if orig_norm.contains(&search_norm) {
        let updated = orig_norm.replacen(&search_norm, &rep_norm, 1);
        return Ok(restore_eol(updated, had_crlf));
    }

    // Niveau 1b : Correspondance du fichier entier (en ignorant les sauts de ligne / espaces aux extrémités)
    let search_trimmed = search_norm.trim();
    if orig_norm.trim() == search_trimmed {
        return Ok(restore_eol(rep_norm.trim().to_string(), had_crlf));
    }

    // Niveau 1c : Correspondance exacte du bloc émondé si unique dans le fichier
    if !search_trimmed.is_empty()
        && search_trimmed.len() >= 6
        && orig_norm.contains(search_trimmed)
        && orig_norm.matches(search_trimmed).count() == 1
    {
        let updated = orig_norm.replacen(search_trimmed, rep_norm.trim(), 1);
        return Ok(restore_eol(updated, had_crlf));
    }

    let orig_lines: Vec<&str> = orig_norm.split('\n').collect();
    let search_trimmed_lines: Vec<&str> = search_trimmed.split('\n').collect();
    let search_lines: Vec<&str> = search_norm.split('\n').collect();

    let eff_search_lines: Vec<&str> = if search_lines.len() > orig_lines.len()
        && search_trimmed_lines.len() <= orig_lines.len()
    {
        search_trimmed_lines
    } else {
        search_lines
    };
    let m = eff_search_lines.len();

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
        let matched = (0..m).all(|k| orig_lines[i + k].trim_end() == eff_search_lines[k].trim_end());
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
    let (ded_search, search_indent) = dedent_lines(&eff_search_lines);
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

    // Niveau 3b : Tolérance syntaxique (guillemets simples/doubles et ponctuation de fin de ligne ',' ou ';')
    let normalize_syntax_line = |line: &str| -> String {
        let mut s = line.trim().replace('"', "'");
        while s.ends_with(';') || s.ends_with(',') {
            s.pop();
        }
        s.trim_end().to_string()
    };
    let norm_search: Vec<String> = eff_search_lines.iter().map(|l| normalize_syntax_line(l)).collect();
    let mut level3b_matches = Vec::new();
    for i in 0..=orig_lines.len().saturating_sub(m) {
        let matched = (0..m).all(|k| {
            normalize_syntax_line(orig_lines[i + k]) == norm_search[k]
        });
        if matched {
            level3b_matches.push(i);
        }
    }
    if level3b_matches.len() == 1 {
        let idx = level3b_matches[0];
        let rep_lines = rep_norm.split('\n').map(|s| s.to_string()).collect();
        let updated = replace_line_range(&orig_lines, idx, idx + m, rep_lines);
        return Ok(restore_eol(updated, had_crlf));
    }

    // Niveau 4a : Gestion des ellipses "..." isolées sur une ligne
    if let Some(ellipsis_pos) = eff_search_lines.iter().position(|l| l.trim() == "...") {
        if rep_norm.split('\n').any(|l| l.trim() == "...") {
            return Err(make_err(
                "Le bloc REPLACE ne doit pas contenir de ligne '...' (fournissez le code complet).",
                None,
                None,
            ));
        }
        let prefix = &eff_search_lines[..ellipsis_pos];
        let suffix = &eff_search_lines[ellipsis_pos + 1..];
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
        if m < orig_lines.len() {
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
                if is_binary_or_non_text_path(&path) {
                    continue;
                }
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

                let raw_final_content = if body_has_search_replace_pair(&content) {
                    extract_clean_replace_code(&content)
                } else {
                    content
                };

                let final_content = if path.to_lowercase().ends_with(".json") {
                    match repair_and_validate_json_str(&raw_final_content) {
                        Ok(valid) => valid,
                        Err(err_msg) => {
                            outcome.failures.push(PatchFailure {
                                path: path.clone(),
                                search_snippet: String::new(),
                                closest_match: None,
                                closest_line: None,
                                reason: format!(
                                    "Contenu JSON invalide pour '{}' ({}). Réécris un JSON strictement valide avec write_file.",
                                    path, err_msg
                                ),
                            });
                            continue;
                        }
                    }
                } else {
                    raw_final_content
                };

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
                if is_binary_or_non_text_path(&path) {
                    continue;
                }
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
                        let final_patched = if path.to_lowercase().ends_with(".json") {
                            match repair_and_validate_json_str(&patched) {
                                Ok(valid) => valid,
                                Err(err_msg) => {
                                    outcome.failures.push(PatchFailure {
                                        path: path.clone(),
                                        search_snippet: search,
                                        closest_match: None,
                                        closest_line: None,
                                        reason: format!(
                                            "Contenu JSON invalide pour '{}' ({}). Réécris un JSON strictement valide avec write_file.",
                                            path, err_msg
                                        ),
                                    });
                                    continue;
                                }
                            }
                        } else {
                            patched
                        };
                        if let Some(parent) = target_path.parent() {
                            let _ = tokio::fs::create_dir_all(parent).await;
                        }
                        if let Err(e) = tokio::fs::write(&target_path, &final_patched).await {
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
        let mut add_args = vec!["add", "-A", "--", "."];
        add_args.extend(crate::git_diff::GIT_EXCLUDE_PATHSPECS);
        let _ = tokio::process::Command::new("git")
            .args(&add_args)
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
    let tool_xml_re = Regex::new(r"<tool_call(?:\s[^>]*)?>[\s\S]*?</tool_call>").expect("valid xml regex");
    let no_xml = tool_xml_re.replace_all(text, "");
    let open_xml_re = Regex::new(r"<tool_call(?:\s[^>]*)?>[\s\S]*$").expect("valid open xml regex");
    let no_xml = open_xml_re.replace_all(&no_xml, "");
    let fn_xml_re = Regex::new(r"<function=[^>]+>[\s\S]*?</function>").expect("valid fn xml regex");
    let no_xml = fn_xml_re.replace_all(&no_xml, "");
    let open_fn_re = Regex::new(r"<function=[^>]+>[\s\S]*$").expect("valid open fn regex");
    let no_xml = open_fn_re.replace_all(&no_xml, "");
    let mistral_re = Regex::new(r"\[TOOL_CALLS\]").expect("valid mistral regex");
    let no_xml = mistral_re.replace_all(&no_xml, "");
    let native_tool_re = Regex::new(r"\[Appel d'outils?(?:\s+natifs?)?\s*:[^\]]+\]").expect("valid native tool regex");
    let no_xml = native_tool_re.replace_all(&no_xml, "");
    let no_bare = strip_bare_json_tool_calls_from_prose(&no_xml);

    let applied_set: HashSet<String> = applied_files
        .iter()
        .filter_map(|p| sanitize_rel_path(p).or_else(|| Some(p.trim().to_string())))
        .collect();

    let norm = no_bare.replace("\r\n", "\n");
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
            let is_tool_fence = is_tool_call_fence(&info_low, &body);
            let has_sr = body_has_any_search_or_replace_marker(&body);

            let (lang, info_path) = detect_path_from_info(info);
            let is_shell = is_shell_lang(&lang)
                && !info_path
                    .as_deref()
                    .map(is_shell_script_path)
                    .unwrap_or(false);

            let detected_path = info_path
                .or_else(|| detect_path_from_preceding_lines(&lines, fence_start, applied_files, &[]))
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
    fn test_fuzzy_level3b_quotes_and_trailing_punctuation() {
        let orig = "module.exports = {\n  content: [\n    \"./index.html\",\n    \"./src/**/*.{vue,js,ts}\",\n  ],\n};\n";
        // Search has single quotes and misses a trailing semicolon
        let search = "content: [\n    './index.html',\n    './src/**/*.{vue,js,ts}'\n  ]";
        let replace = "content: [\n    './index.html',\n    './src/**/*.{vue,js,ts,jsx,tsx}'\n  ]";
        let res = apply_search_replace_fuzzy(orig, search, replace).unwrap();
        assert!(res.contains("./src/**/*.{vue,js,ts,jsx,tsx}"));
    }

    #[test]
    fn test_fuzzy_level1b_trimmed_file_match() {
        let orig = "\n\nmodule.exports = {\n  darkMode: 'class'\n};\n\n";
        let search = "module.exports = {\n  darkMode: 'class'\n};";
        let replace = "module.exports = {\n  darkMode: 'media'\n};";
        let res = apply_search_replace_fuzzy(orig, search, replace).unwrap();
        assert!(res.contains("darkMode: 'media'"));
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

    #[test]
    fn test_universal_json_tool_call_filtering_and_package_json_preservation() {
        let pkg_json = r#"{"name": "my-site", "version": "1.0.0", "scripts": {"build": "vite build"}}"#;
        assert!(!is_tool_call_fence("json", pkg_json));

        let indented_tool = r#"{
  "name": "run_command",
  "arguments": {
    "cmd": "pnpm init"
  }
}"#;
        assert!(is_tool_call_fence("json", indented_tool));
        let parsed = extract_json_tool_calls_from_block(indented_tool);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "run_command");
        assert_eq!(parsed[0].1["command"], "pnpm init");

        let reply = r#"Je prépare le projet :

```json
{
  "tool": "run_command",
  "parameters": {
    "command": "pnpm init"
  }
}
```

{"name":"write_file","file_path":"index.html","code":"<h1>Hello</h1>"}

Tout est prêt."#;
        let blocks = parse_edit_blocks(reply, &[], &[], "initialise le site");
        assert!(blocks.is_empty());

        let cleaned = strip_edit_blocks_for_display(reply, &[]);
        assert!(!cleaned.contains("pnpm init"));
        assert!(!cleaned.contains("write_file"));
        assert!(cleaned.contains("Tout est prêt."));
    }

    #[test]
    fn test_repair_and_validate_json_str_control_chars_and_trailing_commas() {
        let broken_pkg = "{\n  \"name\": \"site-demo\",\n  \"description\": \"Ligne 1\nLigne 2\tavec tab\",\n  \"scripts\": {\n    \"build\": \"vite build\",\n  },\n}";
        let repaired = repair_and_validate_json_str(broken_pkg).expect("should repair broken json");
        let parsed: serde_json::Value = serde_json::from_str(&repaired).expect("valid pretty json");
        assert_eq!(parsed["name"], "site-demo");
        assert_eq!(parsed["scripts"]["build"], "vite build");
        assert_eq!(parsed["description"], "Ligne 1\nLigne 2\tavec tab");

        let wrapped = "```json\n{\"ok\": true,}\n```";
        let repaired_wrapped = repair_and_validate_json_str(wrapped).expect("should unwrap fence");
        let parsed_wrapped: serde_json::Value =
            serde_json::from_str(&repaired_wrapped).expect("valid wrapped json");
        assert_eq!(parsed_wrapped["ok"], true);
    }

    #[test]
    fn test_extract_json_tool_calls_with_raw_newlines_in_content() {
        let raw_block = "{\n  \"name\": \"write_file\",\n  \"arguments\": {\n    \"path\": \"index.html\",\n    \"content\": \"<!DOCTYPE html>\n<html>\n<body>Bonjour</body>\n</html>\"\n  }\n}";
        assert!(is_tool_call_fence("json", raw_block));
        let calls = extract_json_tool_calls_from_block(raw_block);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "write_file");
        assert_eq!(calls[0].1["path"], "index.html");
        assert_eq!(
            calls[0].1["content"],
            "<!DOCTYPE html>\n<html>\n<body>Bonjour</body>\n</html>"
        );
    }

    #[test]
    fn test_reject_binary_and_runtime_paths_in_patcher() {
        assert!(is_binary_or_non_text_path("assets/logo.png"));
        assert!(is_binary_or_non_text_path("favicon.ico"));
        assert!(!is_binary_or_non_text_path("assets/logo.svg"));
        assert_eq!(extract_path_from_text_line("### assets/logo.png"), None);
        assert_eq!(extract_path_from_text_line("### Node.js"), None);
        assert_eq!(extract_path_from_text_line("### Vue.js"), None);
        assert_eq!(extract_path_from_text_line("### FastAPI.py"), None);
        assert_eq!(extract_path_from_text_line("### Axum.rs"), None);
        assert_eq!(extract_path_from_text_line("### Gin.go"), None);
        assert_eq!(extract_path_from_text_line("### Three.js"), None);
        assert_eq!(
            extract_path_from_text_line("### src/App.vue").as_deref(),
            Some("src/App.vue")
        );
        assert_eq!(
            extract_path_from_text_line("### src/main.rs").as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(
            extract_path_from_text_line("### lib/server.py").as_deref(),
            Some("lib/server.py")
        );
    }

    #[test]
    fn test_reject_dotted_property_or_prose_mention_as_phantom_file() {
        let reply_prose = r#"Pour corriger l'erreur Rollup, ajoute `/tailwind.css` dans `build.rollupOptions.external` :

```js
export default {
  build: {
    rollupOptions: {
      external: ['/tailwind.css']
    }
  }
}
```"#;
        let blocks = parse_edit_blocks(reply_prose, &[], &[], "corrige le build");
        assert!(
            blocks.is_empty(),
            "Prose mentioning build.rollupOptions.external must not create a phantom file: {:?}",
            blocks
        );

        let reply_header_dotted = r#"### build.rollupOptions.external
```js
external: ['/tailwind.css']
```"#;
        let blocks2 = parse_edit_blocks(reply_header_dotted, &[], &[], "corrige");
        assert!(
            blocks2.is_empty(),
            "Explicit header with camelCase dotted property must not create a phantom file"
        );

        let reply_real = r#"### tailwind.config.cjs
```js
module.exports = { content: ['./index.html'] };
```

### src/App.vue
```vue
<template><div>OK</div></template>
```"#;
        let blocks3 = parse_edit_blocks(reply_real, &[], &[], "init");
        assert_eq!(blocks3.len(), 2);
        match &blocks3[0] {
            EditBlock::WholeFile { path, .. } => assert_eq!(path, "tailwind.config.cjs"),
            _ => panic!("Expected WholeFile for tailwind.config.cjs"),
        }
        match &blocks3[1] {
            EditBlock::WholeFile { path, .. } => assert_eq!(path, "src/App.vue"),
            _ => panic!("Expected WholeFile for src/App.vue"),
        }
    }
}
