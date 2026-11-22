use crate::models::ToolExecutionResult;
use regex::Regex;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Décode une chaîne encodée en Base64 standard ou URL-safe (sans dépendance externe).
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let clean = input.trim().trim_end_matches('=');
    let mut buffer = 0u32;
    let mut bits_collected = 0;
    let mut output = Vec::with_capacity((clean.len() * 3) / 4);

    for byte in clean.bytes() {
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b' ' | b'\r' | b'\n' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | (val as u32);
        bits_collected += 6;
        if bits_collected >= 8 {
            bits_collected -= 8;
            output.push((buffer >> bits_collected) as u8);
            buffer &= (1 << bits_collected) - 1;
        }
    }
    Some(output)
}

/// Décode les percent-encodings URL (%20, %3A, %2F, etc.).
pub fn percent_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.bytes().peekable();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex_str = [c1, c2];
                if let Ok(s) = std::str::from_utf8(&hex_str) {
                    if let Ok(val) = u8::from_str_radix(s, 16) {
                        bytes.push(val);
                        continue;
                    }
                }
                bytes.push(b'%');
                bytes.push(c1);
                bytes.push(c2);
            } else {
                bytes.push(b'%');
                if let Some(c1) = h1 {
                    bytes.push(c1);
                }
            }
        } else if b == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

/// Décode les entités HTML courantes et numériques.
pub fn decode_html_entities(input: &str) -> String {
    let mut res = input
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&bull;", "•")
        .replace("&hellip;", "…");

    // Décodage des entités numériques décimales : &#([0-9]+);
    if res.contains("&#") {
        let dec_re = Regex::new(r"&#([0-9]{1,6});").unwrap();
        res = dec_re
            .replace_all(&res, |caps: &regex::Captures| {
                if let Ok(num) = caps[1].parse::<u32>() {
                    if let Some(ch) = char::from_u32(num) {
                        return ch.to_string();
                    }
                }
                caps[0].to_string()
            })
            .to_string();

        let hex_re = Regex::new(r"&#x([0-9a-fA-F]{1,6});").unwrap();
        res = hex_re
            .replace_all(&res, |caps: &regex::Captures| {
                if let Ok(num) = u32::from_str_radix(&caps[1], 16) {
                    if let Some(ch) = char::from_u32(num) {
                        return ch.to_string();
                    }
                }
                caps[0].to_string()
            })
            .to_string();
    }

    res
}

/// Extrait l'URL réelle cachée dans un lien Bing redirect `https://www.bing.com/ck/a?...&u=a1<base64>&...`.
pub fn decode_bing_u_param(href: &str) -> Option<String> {
    let u_pos = href.find("u=a1")?;
    let rest = &href[u_pos + 4..];
    let end_pos = rest.find('&').unwrap_or(rest.len());
    let token = &rest[..end_pos];
    let bytes = base64_decode(token)?;
    let decoded = String::from_utf8(bytes).ok()?;
    if decoded.starts_with("http://") || decoded.starts_with("https://") {
        Some(decoded)
    } else {
        None
    }
}

/// Extrait l'URL réelle cachée dans un lien DuckDuckGo redirect `//duckduckgo.com/l/?uddg=<url>&...`.
pub fn decode_ddg_uddg_param(href: &str) -> Option<String> {
    let uddg_pos = href.find("uddg=")?;
    let rest = &href[uddg_pos + 5..];
    let end_pos = rest.find('&').unwrap_or(rest.len());
    let token = &rest[..end_pos];
    let decoded = percent_decode(token);
    if decoded.starts_with("http://") || decoded.starts_with("https://") {
        Some(decoded)
    } else {
        None
    }
}

/// Nettoie une balise HTML pour n'en garder que le texte brut lisible.
pub fn strip_html_tags(html: &str) -> String {
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let text = tag_re.replace_all(html, " ");
    let decoded = decode_html_entities(&text);
    let ws_re = Regex::new(r"\s+").unwrap();
    ws_re.replace_all(&decoded, " ").trim().to_string()
}

/// Parse le HTML de DuckDuckGo (version standard ou lite).
pub fn parse_duckduckgo_html(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // 1. Essai DDG Lite (tables)
    let lite_link_re = Regex::new(r#"<a[^>]*class=["']result-link["'][^>]*href=["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap();
    let lite_snip_re = Regex::new(r#"<td[^>]*class=["']result-snippet["'][^>]*>(.*?)</td>"#).unwrap();

    let links: Vec<(String, String)> = lite_link_re
        .captures_iter(html)
        .map(|c| (c[1].to_string(), strip_html_tags(&c[2]).trim().to_string()))
        .collect();
    let snippets: Vec<String> = lite_snip_re
        .captures_iter(html)
        .map(|c| strip_html_tags(&c[1]).trim().to_string())
        .collect();

    for (i, (raw_url, title)) in links.into_iter().enumerate() {
        if results.len() >= max_results {
            break;
        }
        let clean_url = decode_ddg_uddg_param(&raw_url).unwrap_or(raw_url);
        let snippet = snippets.get(i).cloned().unwrap_or_default();
        if !title.is_empty() && (clean_url.starts_with("http://") || clean_url.starts_with("https://")) {
            results.push(SearchResult {
                title,
                url: clean_url,
                snippet,
            });
        }
    }

    if !results.is_empty() {
        return results;
    }

    // 2. Essai DDG HTML standard
    let std_link_re = Regex::new(r#"<a[^>]*class=["'][^"']*result__a[^"']*["'][^>]*href=["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap();
    let std_snip_re = Regex::new(r#"<a[^>]*class=["'][^"']*result__snippet[^"']*["'][^>]*>(.*?)</a>"#).unwrap();

    let std_links: Vec<(String, String)> = std_link_re
        .captures_iter(html)
        .map(|c| (c[1].to_string(), strip_html_tags(&c[2]).trim().to_string()))
        .collect();
    let std_snips: Vec<String> = std_snip_re
        .captures_iter(html)
        .map(|c| strip_html_tags(&c[1]).trim().to_string())
        .collect();

    for (i, (raw_url, title)) in std_links.into_iter().enumerate() {
        if results.len() >= max_results {
            break;
        }
        let clean_url = decode_ddg_uddg_param(&raw_url).unwrap_or(raw_url);
        let snippet = std_snips.get(i).cloned().unwrap_or_default();
        if !title.is_empty() && (clean_url.starts_with("http://") || clean_url.starts_with("https://")) {
            results.push(SearchResult {
                title,
                url: clean_url,
                snippet,
            });
        }
    }

    results
}

/// Parse le HTML des résultats Bing (`<li class="b_algo">...`).
pub fn parse_bing_html(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let algo_re = Regex::new(r#"(?s)<li[^>]*class=["'][^"']*b_algo[^"']*["'][^>]*>(.*?)</li>"#).unwrap();
    let title_link_re = Regex::new(r#"(?s)<h2><a[^>]*href=["']([^"']+)["'][^>]*>(.*?)</a></h2>"#).unwrap();
    let snippet_re = Regex::new(r#"(?s)<p[^>]*>(.*?)</p>"#).unwrap();

    for cap in algo_re.captures_iter(html) {
        if results.len() >= max_results {
            break;
        }
        let block = &cap[1];
        if let Some(tl_cap) = title_link_re.captures(block) {
            let raw_href = tl_cap[1].replace("&amp;", "&");
            let raw_title = &tl_cap[2];
            let title = strip_html_tags(raw_title).trim().to_string();

            let url = decode_bing_u_param(&raw_href).unwrap_or(raw_href);
            let snippet = if let Some(sn_cap) = snippet_re.captures(block) {
                strip_html_tags(&sn_cap[1]).trim().to_string()
            } else {
                String::new()
            };

            if !title.is_empty() && (url.starts_with("http://") || url.starts_with("https://")) {
                results.push(SearchResult {
                    title,
                    url,
                    snippet,
                });
            }
        }
    }

    results
}

/// Nettoie une page web complète en texte structuré / markdown lisible pour le LLM.
pub fn clean_html_to_markdown_or_text(html: &str, max_chars: usize) -> String {
    // 1. Suppression des scripts, styles, svg, canvas, head, nav, footer
    let mut cleaned = html.to_string();
    let tags_to_remove = ["script", "style", "svg", "canvas", "noscript", "head", "nav", "footer"];
    for tag in tags_to_remove {
        let pattern = format!(r"(?is)<{tag}[^>]*>.*?</{tag}>");
        if let Ok(re) = Regex::new(&pattern) {
            cleaned = re.replace_all(&cleaned, "").to_string();
        }
    }

    // 2. Suppression des commentaires HTML
    let re_comments = Regex::new(r"(?s)<!--.*?-->").unwrap();
    cleaned = re_comments.replace_all(&cleaned, "").to_string();

    // 3. Conversion des titres h1-h6
    for level in 1..=6 {
        let pattern = format!(r"(?is)<h{level}[^>]*>(.*?)</h{level}>");
        if let Ok(re) = Regex::new(&pattern) {
            let prefix = "#".repeat(level);
            cleaned = re
                .replace_all(&cleaned, |caps: &regex::Captures| {
                    let content = strip_html_tags(&caps[1]);
                    format!("\n\n{prefix} {content}\n")
                })
                .to_string();
        }
    }

    // 4. Conversion des listes li
    let re_li = Regex::new(r"(?is)<li[^>]*>(.*?)</li>").unwrap();
    cleaned = re_li
        .replace_all(&cleaned, |caps: &regex::Captures| {
            let content = caps[1].trim();
            format!("\n- {content}")
        })
        .to_string();

    // 5. Conversion des liens hypertextes avec libellé pertinent
    let re_a = Regex::new(r#"(?is)<a[^>]*href=["'](https?://[^"']+)["'][^>]*>(.*?)</a>"#).unwrap();
    cleaned = re_a
        .replace_all(&cleaned, |caps: &regex::Captures| {
            let href = &caps[1];
            let text = strip_html_tags(&caps[2]).trim().to_string();
            if text.is_empty() || text.len() > 100 {
                text
            } else {
                format!("[{text}]({href})")
            }
        })
        .to_string();

    // 6. Remplacement des balises de blocs par des sauts de ligne
    let re_blocks = Regex::new(r"(?is)<(p|div|tr|article|section|header|blockquote|pre)[^>]*>").unwrap();
    cleaned = re_blocks.replace_all(&cleaned, "\n").to_string();
    let re_br = Regex::new(r"(?is)<br\s*/?>").unwrap();
    cleaned = re_br.replace_all(&cleaned, "\n").to_string();

    // 7. Suppression de toutes les balises restantes
    let re_remaining_tags = Regex::new(r"<[^>]+>").unwrap();
    cleaned = re_remaining_tags.replace_all(&cleaned, " ").to_string();

    // 8. Décodage entités HTML
    cleaned = decode_html_entities(&cleaned);

    // 9. Normalisation des espaces et sauts de ligne multiples
    let ws_spaces = Regex::new(r"[ \t]+").unwrap();
    let mut normalized_lines = Vec::new();
    let mut prev_blank = false;
    for line in cleaned.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank {
                normalized_lines.push(String::new());
                prev_blank = true;
            }
        } else {
            let single_spaced = ws_spaces.replace_all(trimmed, " ").to_string();
            normalized_lines.push(single_spaced);
            prev_blank = false;
        }
    }
    let mut full_text = normalized_lines.join("\n").trim().to_string();

    if full_text.chars().count() > max_chars {
        let mut truncated: String = full_text.chars().take(max_chars).collect();
        truncated.push_str(&format!(
            "\n\n[... Contenu tronqué à {max_chars} caractères sur {} au total ...]",
            full_text.chars().count()
        ));
        full_text = truncated;
    }

    full_text
}

/// Effectue une recherche web via moteur DuckDuckGo Lite / HTML.
async fn query_duckduckgo(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    let url = "https://lite.duckduckgo.com/lite/";
    let form_body = format!("q={}", percent_encode(query));
    let resp = client
        .post(url)
        .header("Referer", "https://lite.duckduckgo.com/")
        .header("Origin", "https://lite.duckduckgo.com")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_body)
        .send()
        .await
        .map_err(|e| format!("Erreur réseau DDG : {e}"))?;

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Erreur lecture réponse DDG : {e}"))?;

    // Si DDG renvoie la page de challenge anti-bot
    if html.contains("anomaly.js") || html.contains("challenge-form") {
        return Err("Challenge anti-bot DDG détecté".to_string());
    }

    let results = parse_duckduckgo_html(&html, max_results);
    if results.is_empty() {
        Err("Aucun résultat DDG extrait".to_string())
    } else {
        Ok(results)
    }
}

/// Effectue une recherche web via Bing HTML (fallback robuste sans clé).
async fn query_bing(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    let encoded_query = percent_encode(query);
    let url = format!("https://www.bing.com/search?q={encoded_query}&setlang=en");
    let resp = client
        .get(&url)
        .header("Accept-Language", "en-US,en;q=0.9,fr;q=0.8")
        .send()
        .await
        .map_err(|e| format!("Erreur réseau Bing : {e}"))?;

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Erreur lecture réponse Bing : {e}"))?;

    let results = parse_bing_html(&html, max_results);
    if results.is_empty() {
        Err("Aucun résultat Bing extrait".to_string())
    } else {
        Ok(results)
    }
}

/// Fallback encyclopédique Wikipédia (API publique garantie sans bot check).
async fn query_wikipedia(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    let encoded_query = percent_encode(query);
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={encoded_query}&format=json"
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Erreur réseau Wikipédia : {e}"))?;

    let json_val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Erreur parsing JSON Wikipédia : {e}"))?;

    let mut results = Vec::new();
    if let Some(items) = json_val.pointer("/query/search").and_then(|v| v.as_array()) {
        for it in items {
            if results.len() >= max_results {
                break;
            }
            let title = it.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
            let snippet = it.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
            if !title.is_empty() {
                let clean_snippet = strip_html_tags(snippet);
                let page_url = format!("https://en.wikipedia.org/wiki/{}", percent_encode(title));
                results.push(SearchResult {
                    title: title.to_string(),
                    url: page_url,
                    snippet: clean_snippet,
                });
            }
        }
    }

    if results.is_empty() {
        Err("Aucun article Wikipédia trouvé".to_string())
    } else {
        Ok(results)
    }
}

/// Encode minimalement les caractères non-alphanumériques pour une URL de requête.
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

/// Outil `web_search` : effectue une recherche web en chaîne avec fallback sans clé.
pub async fn web_search(args: &serde_json::Value) -> Result<ToolExecutionResult, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Paramètre 'query' obligatoire pour web_search".to_string())?;

    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 10) as usize;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("Impossible d'initialiser le client HTTP : {e}"))?;

    // 1. Essai DuckDuckGo Lite
    let ddg_res = query_duckduckgo(&client, query, max_results).await;
    let mut final_results = match ddg_res {
        Ok(res) if !res.is_empty() => res,
        _ => {
            // 2. Fallback Bing HTML
            match query_bing(&client, query, max_results).await {
                Ok(res) if !res.is_empty() => res,
                _ => {
                    // 3. Fallback Wikipédia
                    query_wikipedia(&client, query, max_results).await.unwrap_or_default()
                }
            }
        }
    };

    if final_results.is_empty() {
        return Ok(ToolExecutionResult {
            tool: "web_search".to_string(),
            success: true,
            message: format!("Aucun résultat pertinent trouvé en ligne pour la requête : \"{query}\"."),
            path: None,
        });
    }

    if final_results.len() > max_results {
        final_results.truncate(max_results);
    }

    let mut out = format!("### Résultats de recherche web ({}) pour : \"{query}\"\n\n", final_results.len());
    for (idx, item) in final_results.into_iter().enumerate() {
        out.push_str(&format!("{}. [{}]({})\n", idx + 1, item.title, item.url));
        if !item.snippet.is_empty() {
            out.push_str(&format!("   {}\n", item.snippet));
        }
        out.push('\n');
    }

    Ok(ToolExecutionResult {
        tool: "web_search".to_string(),
        success: true,
        message: out.trim_end().to_string(),
        path: None,
    })
}

/// Outil `fetch_url` : télécharge et convertit en texte épuré une page web ou documentation.
pub async fn fetch_url(args: &serde_json::Value) -> Result<ToolExecutionResult, String> {
    let raw_url = args
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Paramètre 'url' obligatoire pour fetch_url".to_string())?;

    let url = if !raw_url.starts_with("http://") && !raw_url.starts_with("https://") {
        format!("https://{raw_url}")
    } else {
        raw_url.to_string()
    };

    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(12000)
        .clamp(500, 50000) as usize;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(18))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("Impossible d'initialiser le client HTTP : {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Erreur lors de la récupération de l'URL '{url}' : {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Ok(ToolExecutionResult {
            tool: "fetch_url".to_string(),
            success: false,
            message: format!("Le serveur a répondu avec le statut HTTP {} ({}) pour '{url}'.", status.as_u16(), status),
            path: None,
        });
    }

    // Vérifier si le contenu est binaire
    if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
        let ct_lower = ct.to_lowercase();
        if ct_lower.starts_with("image/")
            || ct_lower.starts_with("video/")
            || ct_lower.starts_with("audio/")
            || ct_lower.contains("pdf")
            || ct_lower.contains("zip")
            || ct_lower.contains("octet-stream")
        {
            return Ok(ToolExecutionResult {
                tool: "fetch_url".to_string(),
                success: false,
                message: format!("L'URL cible renvoie un contenu binaire ({ct}) non exploitable en texte."),
                path: None,
            });
        }
    }

    let raw_body = resp
        .text()
        .await
        .map_err(|e| format!("Erreur lecture du contenu pour '{url}' : {e}"))?;

    let is_html = raw_body.contains("<html") || raw_body.contains("<body") || raw_body.contains("<!DOCTYPE");
    let content = if is_html {
        clean_html_to_markdown_or_text(&raw_body, max_chars)
    } else {
        let mut text = raw_body.trim().to_string();
        if text.chars().count() > max_chars {
            let mut trunc: String = text.chars().take(max_chars).collect();
            trunc.push_str(&format!("\n\n[... Contenu texte brut tronqué à {max_chars} caractères ...]"));
            text = trunc;
        }
        text
    };

    let formatted = format!("### Contenu extrait de : {url}\n\n{content}");

    Ok(ToolExecutionResult {
        tool: "fetch_url".to_string(),
        success: true,
        message: formatted,
        path: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_decode() {
        assert_eq!(base64_decode("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(base64_decode("aGVsbG8"), Some(b"hello".to_vec()));
        assert_eq!(
            base64_decode("aHR0cHM6Ly9mci53aWtpcGVkaWEub3Jn"),
            Some(b"https://fr.wikipedia.org".to_vec())
        );
    }

    #[test]
    fn test_percent_decode_and_encode() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("https%3A%2F%2Fexample.com%2Fpath%3Fq%3Dtest"), "https://example.com/path?q=test");
        assert_eq!(percent_encode("hello world"), "hello+world");
        assert_eq!(percent_encode("foo&bar"), "foo%26bar");
    }

    #[test]
    fn test_decode_html_entities() {
        let input = "&lt;div&gt;Hello &quot;World&quot; &amp; friends &mdash; &#39;test&#39; &#65; &#x42;&lt;/div&gt;";
        let output = decode_html_entities(input);
        assert_eq!(output, "<div>Hello \"World\" & friends — 'test' A B</div>");
    }

    #[test]
    fn test_decode_bing_u_param() {
        let href = "https://www.bing.com/ck/a?!&&p=abc&u=a1aHR0cHM6Ly9mci53aWtpcGVkaWEub3Jn&ntb=1";
        let decoded = decode_bing_u_param(href);
        assert_eq!(decoded, Some("https://fr.wikipedia.org".to_string()));
    }

    #[test]
    fn test_decode_ddg_uddg_param() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Ftailwindcss.com%2Fdocs&rut=123";
        let decoded = decode_ddg_uddg_param(href);
        assert_eq!(decoded, Some("https://tailwindcss.com/docs".to_string()));
    }

    #[test]
    fn test_clean_html_to_markdown_or_text() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head><title>Test Page</title><style>.test { color: red; }</style></head>
            <body>
                <header><nav><a href="/home">Home</a></nav></header>
                <h1>Guide Tailwind CSS</h1>
                <p>Tailwind est un <strong>framework</strong> utilitaire.</p>
                <ul>
                    <li>Installation via npm</li>
                    <li>Configuration de <a href="https://tailwindcss.com">Tailwind</a></li>
                </ul>
                <script>console.log("secret");</script>
                <footer>Footer unneeded</footer>
            </body>
            </html>
        "#;
        let text = clean_html_to_markdown_or_text(html, 1000);
        assert!(text.contains("# Guide Tailwind CSS"));
        assert!(text.contains("Tailwind est un framework utilitaire."));
        assert!(text.contains("- Installation via npm"));
        assert!(text.contains("[Tailwind](https://tailwindcss.com)"));
        assert!(!text.contains("console.log"));
        assert!(!text.contains("Footer unneeded"));
    }

    #[test]
    fn test_parse_bing_html_mock() {
        let mock = r#"
            <li class="b_algo">
                <h2><a href="https://www.bing.com/ck/a?!&u=a1aHR0cHM6Ly9leGFtcGxlLmNvbS9kb2Nz&ntb=1">Exemple Titre <b>Gras</b></a></h2>
                <p>Voici la description ou le <i>snippet</i> utile.</p>
            </li>
        "#;
        let results = parse_bing_html(mock, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Exemple Titre Gras");
        assert_eq!(results[0].url, "https://example.com/docs");
        assert_eq!(results[0].snippet, "Voici la description ou le snippet utile.");
    }
}
