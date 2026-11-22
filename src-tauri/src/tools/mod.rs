pub mod approval;
pub mod fs;
pub mod shell;
pub mod web;

use crate::models::ToolExecutionResult;
use std::path::Path;

pub fn build_native_tool_definitions(has_mcp: bool) -> Vec<serde_json::Value> {
    let mut defs = vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Lit le contenu d'un fichier du projet (optionnellement sur une plage de lignes pour économiser le contexte).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Chemin relatif exact du fichier (ex: src/App.vue)" },
                        "start_line": { "type": "integer", "description": "Première ligne à lire (1-indexé, optionnel)" },
                        "end_line": { "type": "integer", "description": "Dernière ligne à lire (inclus, optionnel, max +250 lignes)" }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "search_code",
                "description": "Recherche un motif ou identifiant dans les fichiers source du projet et retourne les lignes correspondantes.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Motif texte ou identifiant à rechercher" },
                        "path_filter": { "type": "string", "description": "Préfixe de chemin optionnel (ex: src/components/)" }
                    },
                    "required": ["query"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_files",
                "description": "Liste les chemins relatifs des fichiers indexés du projet.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Crée un nouveau fichier ou remplace intégralement le contenu d'un fichier existant sans aucune limite de lignes. Utiliser cet outil pour toute création de nouveau fichier ou réécriture complète.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Chemin relatif exact du fichier à créer ou écrire" },
                        "content": { "type": "string", "description": "Contenu complet et opérationnel du fichier" }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Patch court : modifie chirurgicalement un fichier existant via un ou plusieurs blocs SEARCH/REPLACE exacts et atomiques. Ne jamais utiliser pour créer un fichier inexistant (utiliser write_file).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Chemin relatif exact du fichier existant" },
                        "search": { "type": "string", "description": "Extrait exact existant à remplacer (patch court)" },
                        "replace": { "type": "string", "description": "Nouveau code de remplacement" },
                        "edits": {
                            "type": "array",
                            "description": "Liste ordonnée d'éditions {search, replace} à appliquer atomiquement",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "search": { "type": "string", "description": "Extrait exact existant à remplacer" },
                                    "replace": { "type": "string", "description": "Nouveau texte de remplacement" }
                                },
                                "required": ["search", "replace"]
                            }
                        }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "delete_file",
                "description": "Supprime un fichier obsolète du projet (une confirmation explicite est demandée à l'utilisateur dans l'interface).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Chemin relatif exact du fichier à supprimer" },
                        "reason": { "type": "string", "description": "Raison brève expliquant à l'utilisateur pourquoi ce fichier est supprimé (optionnel)" }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Exécute une commande shell dans la racine du projet (installation de dépendances, scripts, builds, tests). Une confirmation est demandée pour les commandes destructrices.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Commande shell à exécuter" },
                        "timeout_secs": { "type": "integer", "description": "Délai maximal en secondes (défaut 90, borné entre 5 et 300)" }
                    },
                    "required": ["command"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Recherche des informations actualisées sur le web (documentation, erreurs de compilation/dépendances, paquets, solutions) sans nécessiter de clé API.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Mots-clés ciblés de la recherche web" },
                        "max_results": { "type": "integer", "description": "Nombre maximal de résultats à extraire (défaut 5, borné entre 1 et 10)" }
                    },
                    "required": ["query"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "fetch_url",
                "description": "Télécharge et extrait le contenu textuel/markdown épuré d'une page web ou documentation via son URL.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "URL complète de la page web ou documentation à lire (http:// ou https://)" },
                        "max_chars": { "type": "integer", "description": "Nombre maximal de caractères à extraire (défaut 12000, max 50000)" }
                    },
                    "required": ["url"]
                }
            }
        }),
    ];

    if has_mcp {
        defs.push(serde_json::json!({
            "type": "function",
            "function": {
                "name": "mcp_call",
                "description": "Appelle un outil exposé par un serveur MCP actif.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "server": { "type": "string", "description": "Nom du serveur MCP" },
                        "tool": { "type": "string", "description": "Nom de l'outil MCP" },
                        "arguments": { "type": "object", "description": "Arguments JSON de l'outil" }
                    },
                    "required": ["server", "tool", "arguments"]
                }
            }
        }));
    }

    defs
}

pub async fn execute_local_tool(
    project_root: &Path,
    name: &str,
    args: &serde_json::Value,
) -> Result<ToolExecutionResult, String> {
    match name {
        "write_file" | "create_file" => fs::write_file(project_root, name, args).await,
        "edit_file" => fs::edit_file(project_root, args).await,
        "read_file" => fs::read_file(project_root, args).await,
        "search_code" => fs::search_code(project_root, args).await,
        "list_files" => fs::list_files(project_root).await,
        "delete_file" => fs::delete_file(project_root, args).await,
        "run_command" => shell::run_command_simple(project_root, args).await,
        "web_search" => web::web_search(args).await,
        "fetch_url" => web::fetch_url(args).await,
        other => Err(format!("Outil inconnu : {}", other)),
    }
}
