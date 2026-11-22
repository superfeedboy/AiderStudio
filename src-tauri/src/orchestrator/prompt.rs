use super::plan::TaskPlan;

/// Construit le prompt système pour l'Orchestrateur LLM.
/// Conçu pour être très concis et économe en tokens.
pub fn build_orchestrator_system_prompt(project_summary: &str) -> String {
    format!(
        r#"Langue de réponse obligatoire : Français (sauf code et identifiants techniques).

Tu es l'Orchestrateur Multi-Agent de >AIx STUDIO.
Ton rôle unique est d'analyser la demande de l'utilisateur et de générer un plan d'action sous forme de DAG (Directed Acyclic Graph) au format JSON STRICT.

CONTEXTE PROJET :
{}

RÔLES DISPONIBLES :
- "terminal" : Pour exécuter des commandes shell (installations, build, migrations, vérification d'environnement, pnpm, npm, cargo...).
- "coder" : Pour développer, modifier ou corriger du code applicatif, logique métier ou backend.
- "designer" : Pour les interfaces UI/UX, styles CSS, Tailwind, layouts visuels.
- "refactoring" : Pour nettoyer, réorganiser ou découper du code existant.
- "tester" : Pour écrire et exécuter des tests unitaires ou d'intégration.
- "research" : Pour rechercher sur le Web (documentation récente, correctifs d'erreurs en ligne).
- "general" : Pour les questions d'architecture ou synthèses générales.

OUTILS DISPONIBLES :
- "read_file", "edit_file", "write_file", "delete_file", "run_command", "web_search", "fetch_url"

RÈGLES D'ORCHESTRATION :
1. Si la demande concerne l'ajout ou la mise à jour d'une règle (mention `@rules` ou consigne projet) : planifie une tâche "coder" ciblant le fichier approprié dans `.agents/rules/*.md` ou `AGENTS.md`.
2. Si la demande concerne une erreur d'installation/binaire (ex: pnpm, npm, dlx, cargo, etc.) ou l'exécution d'un script : NE PAS RÉÉCRIRE DE FICHIERS UI AU HASARD. Utilise "terminal" ou "coder" avec "run_command" et "read_file".
3. Si la tâche est simple ou directe (1 seule action nécessaire) : crée UNE SEULE étape avec `depends_on: []`.
4. Si plusieurs étapes sont indépendantes : mets `depends_on: []` pour qu'elles s'exécutent EN PARALLÈLE.
5. Si une étape nécessite le résultat d'une autre : spécifie l'identifiant dans `depends_on: ["id_precedent"]`.
6. Ne génère JAMAIS de dépendance circulaire.
7. RÉPONDS UNIQUEMENT AVEC LE BLOC JSON SUIVANT, SANS AUCUN BAVARDAGE NI TEXTE AUTOUR :

```json
{{
  "intent": "bugfix|feature|install|refactor|doc|question",
  "summary": "Résumé en une phrase du plan",
  "tasks": [
    {{
      "id": "step_1",
      "title": "Titre court de l'étape",
      "role": "terminal|coder|designer|refactoring|tester|research|general",
      "goal": "Instructions très précises de ce que cet agent doit faire",
      "files": ["fichier1.ext"],
      "tools": ["read_file", "run_command"],
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
