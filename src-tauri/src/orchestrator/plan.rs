use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskRole {
    #[serde(rename = "coder")]
    Coder,
    #[serde(rename = "designer")]
    Designer,
    #[serde(rename = "refactoring")]
    Refactoring,
    #[serde(rename = "tester")]
    Tester,
    #[serde(rename = "terminal")]
    Terminal,
    #[serde(rename = "research")]
    Research,
    #[serde(rename = "general")]
    General,
}

impl TaskRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskRole::Coder => "coder",
            TaskRole::Designer => "designer",
            TaskRole::Refactoring => "refactoring",
            TaskRole::Tester => "tester",
            TaskRole::Terminal => "terminal",
            TaskRole::Research => "research",
            TaskRole::General => "general",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "coder" | "code" | "dev" | "backend" => TaskRole::Coder,
            "designer" | "design" | "ui" | "ux" | "css" | "frontend" => TaskRole::Designer,
            "refactoring" | "refactor" | "clean" => TaskRole::Refactoring,
            "tester" | "test" | "tests" | "qa" => TaskRole::Tester,
            "terminal" | "shell" | "bash" | "command" | "cmd" | "ops" => TaskRole::Terminal,
            "research" | "search" | "web" | "doc" | "docs" => TaskRole::Research,
            _ => TaskRole::General,
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            TaskRole::Coder => "Développeur",
            TaskRole::Designer => "Designer UI/UX",
            TaskRole::Refactoring => "Refactoring",
            TaskRole::Tester => "Testeur QA",
            TaskRole::Terminal => "Terminal / Shell",
            TaskRole::Research => "Recherche & Documentation",
            TaskRole::General => "Assistant Général",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskStep {
    pub id: String,
    pub title: String,
    pub role: String,
    pub goal: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskPlan {
    #[serde(default)]
    pub intent: String,
    pub summary: String,
    pub tasks: Vec<TaskStep>,
}

impl TaskPlan {
    /// Crée un plan de repli sûr (Safe Fallback) avec une seule tâche complète.
    pub fn fallback(user_prompt: &str) -> Self {
        let clean_prompt = user_prompt.trim();
        let summary = if clean_prompt.chars().count() > 80 {
            let truncated: String = clean_prompt.chars().take(77).collect();
            format!("{truncated}...")
        } else if clean_prompt.is_empty() {
            "Traitement de la requête".to_string()
        } else {
            clean_prompt.to_string()
        };

        Self {
            intent: "general_execution".to_string(),
            summary: format!("Exécution directe : {summary}"),
            tasks: vec![TaskStep {
                id: "task_1".to_string(),
                title: "Traitement complet de la demande".to_string(),
                role: "general".to_string(),
                goal: clean_prompt.to_string(),
                files: Vec::new(),
                tools: vec![
                    "read_file".to_string(),
                    "edit_file".to_string(),
                    "write_file".to_string(),
                    "delete_file".to_string(),
                    "run_command".to_string(),
                    "web_search".to_string(),
                    "fetch_url".to_string(),
                ],
                depends_on: Vec::new(),
            }],
        }
    }

    /// Valide le graphe (dépendances existantes, pas de cycle)
    /// et retourne les vagues d'exécution en parallèle (waves).
    /// Chaque vague est une liste d'IDs de tâches pouvant s'exécuter simultanément.
    pub fn compute_execution_waves(&self) -> Result<Vec<Vec<String>>, String> {
        if self.tasks.is_empty() {
            return Ok(Vec::new());
        }

        let mut task_map: HashMap<&str, &TaskStep> = HashMap::new();
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

        for task in &self.tasks {
            let id = task.id.trim();
            if id.is_empty() {
                return Err("Une tâche a un ID vide.".to_string());
            }
            if task_map.contains_key(id) {
                return Err(format!("ID de tâche en doublon : '{id}'"));
            }
            task_map.insert(id, task);
            in_degree.insert(id, 0);
            dependents.insert(id, Vec::new());
        }

        // Construire les dépendances
        for task in &self.tasks {
            let current_id = task.id.trim();
            for dep in &task.depends_on {
                let dep_id = dep.trim();
                if dep_id.is_empty() {
                    continue;
                }
                if dep_id == current_id {
                    return Err(format!(
                        "La tâche '{current_id}' ne peut pas dépendre d'elle-même."
                    ));
                }
                if !task_map.contains_key(dep_id) {
                    return Err(format!(
                        "La tâche '{current_id}' dépend d'un ID inconnu '{dep_id}'."
                    ));
                }
                // current_id a besoin de dep_id
                dependents.get_mut(dep_id).unwrap().push(current_id);
                *in_degree.get_mut(current_id).unwrap() += 1;
            }
        }

        // Algorithme de Kahn par vagues
        let mut waves: Vec<Vec<String>> = Vec::new();
        let mut ready_queue: VecDeque<&str> = VecDeque::new();

        for (&id, &deg) in &in_degree {
            if deg == 0 {
                ready_queue.push_back(id);
            }
        }

        let mut processed_count = 0;

        while !ready_queue.is_empty() {
            // Toutes les tâches actuellement dans ready_queue forment une vague parallèle
            let wave_size = ready_queue.len();
            let mut current_wave: Vec<String> = Vec::with_capacity(wave_size);
            let mut next_ready: Vec<&str> = Vec::new();

            for _ in 0..wave_size {
                let node = ready_queue.pop_front().unwrap();
                current_wave.push(node.to_string());
                processed_count += 1;

                for &dependent in &dependents[node] {
                    let deg = in_degree.get_mut(dependent).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        next_ready.push(dependent);
                    }
                }
            }

            // Trier les IDs de la vague pour un ordre déterministe
            current_wave.sort();
            waves.push(current_wave);

            for n in next_ready {
                ready_queue.push_back(n);
            }
        }

        if processed_count != self.tasks.len() {
            return Err("Cycle détecté dans le plan DAG (dépendances circulaires).".to_string());
        }

        Ok(waves)
    }

    /// Récupère une tâche par son identifiant
    pub fn get_task(&self, id: &str) -> Option<&TaskStep> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_role_loose() {
        assert_eq!(TaskRole::from_str_loose("coder"), TaskRole::Coder);
        assert_eq!(TaskRole::from_str_loose("Dev"), TaskRole::Coder);
        assert_eq!(TaskRole::from_str_loose("design"), TaskRole::Designer);
        assert_eq!(TaskRole::from_str_loose("CSS"), TaskRole::Designer);
        assert_eq!(TaskRole::from_str_loose("terminal"), TaskRole::Terminal);
        assert_eq!(TaskRole::from_str_loose("bash"), TaskRole::Terminal);
        assert_eq!(TaskRole::from_str_loose("web"), TaskRole::Research);
        assert_eq!(TaskRole::from_str_loose("unknown_role"), TaskRole::General);
    }

    #[test]
    fn test_plan_fallback() {
        let plan = TaskPlan::fallback("Corrige le binaire manquant dans tailwindcss");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].id, "task_1");
        assert!(plan.tasks[0].tools.contains(&"run_command".to_string()));
        assert!(plan.tasks[0].tools.contains(&"web_search".to_string()));

        let waves = plan.compute_execution_waves().unwrap();
        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0], vec!["task_1"]);
    }

    #[test]
    fn test_plan_dag_linear_sequential() {
        let plan = TaskPlan {
            intent: "fix".to_string(),
            summary: "Diagnostic then fix".to_string(),
            tasks: vec![
                TaskStep {
                    id: "step_1".to_string(),
                    title: "Diagnostic".to_string(),
                    role: "terminal".to_string(),
                    goal: "Check package.json".to_string(),
                    files: vec!["package.json".to_string()],
                    tools: vec!["read_file".to_string()],
                    depends_on: vec![],
                },
                TaskStep {
                    id: "step_2".to_string(),
                    title: "Correction".to_string(),
                    role: "coder".to_string(),
                    goal: "Fix dependency".to_string(),
                    files: vec!["package.json".to_string()],
                    tools: vec!["edit_file".to_string()],
                    depends_on: vec!["step_1".to_string()],
                },
                TaskStep {
                    id: "step_3".to_string(),
                    title: "Install".to_string(),
                    role: "terminal".to_string(),
                    goal: "Run pnpm install".to_string(),
                    files: vec![],
                    tools: vec!["run_command".to_string()],
                    depends_on: vec!["step_2".to_string()],
                },
            ],
        };

        let waves = plan.compute_execution_waves().unwrap();
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["step_1"]);
        assert_eq!(waves[1], vec!["step_2"]);
        assert_eq!(waves[2], vec!["step_3"]);
    }

    #[test]
    fn test_plan_dag_diamond_parallel() {
        // Step A -> Step B & Step C en parallèle -> Step D
        let plan = TaskPlan {
            intent: "feature".to_string(),
            summary: "Diamond DAG".to_string(),
            tasks: vec![
                TaskStep {
                    id: "init".to_string(),
                    title: "Init".to_string(),
                    role: "general".to_string(),
                    goal: "Prepare".to_string(),
                    files: vec![],
                    tools: vec![],
                    depends_on: vec![],
                },
                TaskStep {
                    id: "front".to_string(),
                    title: "Front UI".to_string(),
                    role: "designer".to_string(),
                    goal: "Build UI".to_string(),
                    files: vec![],
                    tools: vec![],
                    depends_on: vec!["init".to_string()],
                },
                TaskStep {
                    id: "back".to_string(),
                    title: "Back API".to_string(),
                    role: "coder".to_string(),
                    goal: "Build API".to_string(),
                    files: vec![],
                    tools: vec![],
                    depends_on: vec!["init".to_string()],
                },
                TaskStep {
                    id: "integrate".to_string(),
                    title: "Integrate".to_string(),
                    role: "tester".to_string(),
                    goal: "Test both".to_string(),
                    files: vec![],
                    tools: vec![],
                    depends_on: vec!["front".to_string(), "back".to_string()],
                },
            ],
        };

        let waves = plan.compute_execution_waves().unwrap();
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["init"]);
        assert_eq!(waves[1], vec!["back", "front"]); // triés par ordre alphabétique
        assert_eq!(waves[2], vec!["integrate"]);
    }

    #[test]
    fn test_plan_dag_cycle_detection() {
        let plan = TaskPlan {
            intent: "cycle".to_string(),
            summary: "Circular dependency".to_string(),
            tasks: vec![
                TaskStep {
                    id: "A".to_string(),
                    title: "A".to_string(),
                    role: "coder".to_string(),
                    goal: "".to_string(),
                    files: vec![],
                    tools: vec![],
                    depends_on: vec!["B".to_string()],
                },
                TaskStep {
                    id: "B".to_string(),
                    title: "B".to_string(),
                    role: "coder".to_string(),
                    goal: "".to_string(),
                    files: vec![],
                    tools: vec![],
                    depends_on: vec!["A".to_string()],
                },
            ],
        };

        assert!(plan.compute_execution_waves().is_err());
    }

    #[test]
    fn test_plan_dag_self_dependency() {
        let plan = TaskPlan {
            intent: "self".to_string(),
            summary: "Self dependency".to_string(),
            tasks: vec![TaskStep {
                id: "A".to_string(),
                title: "A".to_string(),
                role: "coder".to_string(),
                goal: "".to_string(),
                files: vec![],
                tools: vec![],
                depends_on: vec!["A".to_string()],
            }],
        };

        assert!(plan.compute_execution_waves().is_err());
    }

    #[test]
    fn test_plan_dag_unknown_dependency() {
        let plan = TaskPlan {
            intent: "unknown".to_string(),
            summary: "Unknown dependency".to_string(),
            tasks: vec![TaskStep {
                id: "A".to_string(),
                title: "A".to_string(),
                role: "coder".to_string(),
                goal: "".to_string(),
                files: vec![],
                tools: vec![],
                depends_on: vec!["non_existent".to_string()],
            }],
        };

        assert!(plan.compute_execution_waves().is_err());
    }
}
