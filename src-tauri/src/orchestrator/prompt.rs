use super::plan::TaskPlan;

/// Construit le prompt système pour l'Orchestrateur LLM.
/// Conçu pour être très concis et économe en tokens.
pub fn build_orchestrator_system_prompt(project_summary: &str) -> String {
    format!(
        r#"Langue de réponse obligatoire : Français (sauf code et identifiants techniques).

Tu es l'Orchestrateur Multi-Agent de >AIx STUDIO.
Ton rôle unique est d'analyser la demande de l'utilisateur à partir de l'instantané du projet (`WorkspaceSnapshot`) et de générer un plan d'action sous forme de DAG au format JSON STRICT.

INSTANTANÉ DU PROJET (arborescence, manifestes, scripts déclarés, chemins cassés éventuels) :
{}

RÔLES DISPONIBLES :
- "terminal" : Pour exécuter des commandes shell (installations, builds, migrations, vérifications d'environnement).
- "coder" : Pour diagnostiquer, créer, modifier ou corriger tout code ou configuration applicative.
- "designer" : Pour les interfaces UI/UX, layouts visuels et styles.
- "refactoring" : Pour nettoyer, réorganiser ou découper du code existant.
- "tester" : Pour écrire et exécuter des tests automatisés.
- "research" : Pour rechercher sur le Web (documentation récente, correctifs d'erreurs en ligne).
- "general" : Pour les questions d'architecture, d'analyse ou synthèses générales.

OUTILS DISPONIBLES POUR LES AGENTS :
- "read_file", "edit_file" (patch court), "write_file" (création/réécriture complète sans limite de lignes), "delete_file", "run_command", "web_search", "fetch_url", "search_code", "list_files"

RÈGLES D'ORCHESTRATION IMPÉRATIVES :
1. UNIFICATION DIAGNOSTIC + CORRECTION : Ne sépare JAMAIS « Diagnostic / Lecture » et « Correction / Écriture » en deux tâches distinctes ! Un agent "coder" ou "designer" sait lire les fichiers, diagnostiquer la cause racine (y compris fichiers manquants ou références cassées) et appliquer les créations/modifications dans la même session continue.
2. Si la tâche peut être menée par un seul agent spécialisé (la grande majorité des bugs, erreurs d'exécution, features ou installations) : crée UNE SEULE étape unifiée avec `depends_on: []`.
3. N'utilise un DAG multi-étapes que pour des chantiers véritablement multi-domaines ou parallélisables sur des fichiers disjoints. Dans ce cas, indique dans le `goal` de chaque étape tout ce qu'elle doit transmettre à l'étape suivante.
4. Si la demande concerne l'ajout ou la mise à jour d'une règle (mention `@rules` ou consigne projet) : planifie une tâche "coder" ciblant le fichier approprié dans `.agents/rules/*.md` ou `AGENTS.md`.
5. Si des chemins déclarés sont signalés introuvables dans l'instantané du projet ou si la structure du manifeste racine révèle des éléments manquants : mentionne explicitement dans le `goal` de vérifier la chaîne de démarrage complète du projet.
6. `validationCommand` (optionnel) : commande terminale non-interactive déterministe (sans serveur continu/watch bloquant) déduite des manifestes du projet pour valider le code après exécution (ex: script de compilation, d'analyse statique ou de test déduit du manifeste racine), ou `null` si aucune commande n'est applicable.
7. RÉPONDS UNIQUEMENT AVEC LE BLOC JSON SUIVANT, SANS AUCUN BAVARDAGE NI TEXTE AUTOUR :

```json
{{
  "intent": "bugfix|feature|install|refactor|doc|question",
  "summary": "Résumé concis du plan",
  "validationCommand": null,
  "tasks": [
    {{
      "id": "step_1",
      "title": "Titre court de l'étape",
      "role": "coder|designer|terminal|refactoring|tester|research|general",
      "goal": "Instructions très précises de ce que cet agent doit diagnostiquer, créer/modifier et vérifier",
      "files": ["fichier1.ext"],
      "tools": ["read_file", "write_file", "edit_file", "run_command"],
      "depends_on": []
    }}
  ]
}}
```"#,
        if project_summary.trim().is_empty() {
            "Aucun fichier spécifique fourni."
        } else {
            project_summary.trim()
        }
    )
}

/// Extrait et parse le JSON retourné par le LLM.
/// Si le JSON est invalide ou corrompu, retourne automatiquement un plan de repli sûr (fallback).
pub fn parse_orchestrator_response(raw_llm_output: &str, user_prompt: &str) -> (TaskPlan, bool) {
    let text = raw_llm_output.trim();

    // 1. Tenter d'extraire le contenu entre ```json et ```
    let json_candidate = if let Some(start) = text.find("```json") {
        let content_after = &text[start + 7..];
        if let Some(end) = content_after.find("```") {
            content_after[..end].trim()
        } else {
            content_after.trim()
        }
    } else if let Some(start) = text.find("```") {
        let content_after = &text[start + 3..];
        if let Some(end) = content_after.find("```") {
            content_after[..end].trim()
        } else {
            content_after.trim()
        }
    } else {
        // Recherche de la première accolade ouvrante et de la dernière fermante
        if let (Some(open_idx), Some(close_idx)) = (text.find('{'), text.rfind('}')) {
            if close_idx > open_idx {
                &text[open_idx..=close_idx]
            } else {
                text
            }
        } else {
            text
        }
    };

    // 2. Tenter la désérialisation serde_json
    match serde_json::from_str::<TaskPlan>(json_candidate) {
        Ok(mut plan) => {
            // Normaliser les tâches si nécessaire
            if plan.tasks.is_empty() {
                return (TaskPlan::fallback(user_prompt), true);
            }

            // Nettoyer les dépendances vides
            for task in &mut plan.tasks {
                task.depends_on.retain(|d| !d.trim().is_empty());
            }

            // Valider le DAG
            match plan.compute_execution_waves() {
                Ok(_) => (plan, false),
                Err(_) => {
                    // En cas de cycle ou d'ID manquant, basculer sur le repli
                    (TaskPlan::fallback(user_prompt), true)
                }
            }
        }
        Err(_) => {
            // Repli sûr en cas d'échec de parsing JSON
            (TaskPlan::fallback(user_prompt), true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_markdown_fenced_json() {
        let raw = r#"Voici mon plan d'action :
```json
{
  "intent": "install",
  "summary": "Installer les dépendances pnpm et configurer tailwindcss",
  "tasks": [
    {
      "id": "t1",
      "title": "Diagnostic environnement",
      "role": "terminal",
      "goal": "Lancer pnpm --version et inspecter package.json",
      "files": ["package.json"],
      "tools": ["read_file", "run_command"],
      "depends_on": []
    },
    {
      "id": "t2",
      "title": "Installation",
      "role": "terminal",
      "goal": "Exécuter pnpm add -D tailwindcss",
      "files": ["package.json"],
      "tools": ["run_command"],
      "depends_on": ["t1"]
    }
  ]
}
```
Bonne journée !"#;

        let (plan, is_fallback) = parse_orchestrator_response(raw, "installe tailwind");
        assert!(!is_fallback);
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].id, "t1");
        assert_eq!(plan.tasks[1].id, "t2");
        assert_eq!(plan.tasks[1].depends_on, vec!["t1"]);

        let waves = plan.compute_execution_waves().unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0], vec!["t1"]);
        assert_eq!(waves[1], vec!["t2"]);
    }

    #[test]
    fn test_parse_raw_json_without_fences() {
        let raw = r#"{
  "intent": "bugfix",
  "summary": "Fix syntax error",
  "tasks": [
    {
      "id": "task_fix",
      "title": "Fix syntax error",
      "role": "coder",
      "goal": "Repair broken brace",
      "files": ["src/main.rs"],
      "tools": ["edit_file"],
      "depends_on": []
    }
  ]
}"#;

        let (plan, is_fallback) = parse_orchestrator_response(raw, "fix syntax");
        assert!(!is_fallback);
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].id, "task_fix");
    }

    #[test]
    fn test_parse_corrupted_json_triggers_safe_fallback() {
        let raw = "Je n'ai pas pu générer de json valide { incomplete...";
        let (plan, is_fallback) = parse_orchestrator_response(raw, "mon prompt de test");
        assert!(is_fallback);
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].id, "task_1");
        assert_eq!(plan.tasks[0].goal, "mon prompt de test");
        assert!(plan.tasks[0].tools.contains(&"run_command".to_string()));
    }

    #[test]
    fn test_parse_circular_json_triggers_safe_fallback() {
        let raw = r#"```json
{
  "intent": "loop",
  "summary": "Circular",
  "tasks": [
    {
      "id": "a",
      "title": "A",
      "role": "coder",
      "goal": "",
      "files": [],
      "tools": [],
      "depends_on": ["b"]
    },
    {
      "id": "b",
      "title": "B",
      "role": "coder",
      "goal": "",
      "files": [],
      "tools": [],
      "depends_on": ["a"]
    }
  ]
}
```"#;

        let (plan, is_fallback) = parse_orchestrator_response(raw, "circular prompt");
        assert!(is_fallback);
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].id, "task_1");
    }
}
