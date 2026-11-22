use super::plan::TaskPlan;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledSubWave {
    pub is_parallel: bool,
    pub task_ids: Vec<String>,
}

/// Découpe une vague de tâches indépendantes en sous-vagues d'exécution sécurisées :
/// - Vérifie que deux tâches exécutées en parallèle ne modifient pas les mêmes fichiers (pas de conflit d'écriture).
/// - Respecte la limite de concurrence (ex: 1 pour Ollama local, 3 pour API Cloud).
pub fn partition_wave_into_safe_batches(
    wave: &[String],
    plan: &TaskPlan,
    max_concurrency: usize,
) -> Vec<ScheduledSubWave> {
    if wave.is_empty() {
        return Vec::new();
    }

    let concurrency_limit = max_concurrency.max(1);

    // Si la limite de concurrence est 1, toutes les tâches sont exécutées séquentiellement
    if concurrency_limit == 1 || wave.len() == 1 {
        return wave
            .iter()
            .map(|id| ScheduledSubWave {
                is_parallel: false,
                task_ids: vec![id.clone()],
            })
            .collect();
    }

    let mut batches: Vec<ScheduledSubWave> = Vec::new();
    let mut remaining: Vec<String> = wave.to_vec();

    while !remaining.is_empty() {
        let mut current_batch_ids: Vec<String> = Vec::new();
        let mut touched_files: HashSet<String> = HashSet::new();
        let mut next_remaining: Vec<String> = Vec::new();

        for task_id in remaining {
            if current_batch_ids.len() >= concurrency_limit {
                next_remaining.push(task_id);
                continue;
            }

            let task_files: Vec<String> = if let Some(task) = plan.get_task(&task_id) {
                // Seuls les rôles qui écrivent des fichiers ("coder", "designer", "refactoring")
                // posent un risque de conflit. "research" ou "terminal" en lecture seule ne bloquent pas.
                let is_writer_role = match task.role.trim().to_lowercase().as_str() {
                    "coder" | "designer" | "refactoring" => true,
                    _ => false,
                };

                if is_writer_role {
                    task.files
                        .iter()
                        .map(|f| f.trim().to_lowercase())
                        .filter(|f| !f.is_empty())
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            // Vérifier s'il y a collision de fichier avec les tâches déjà sélectionnées dans ce sous-lot
            let has_collision = task_files.iter().any(|f| touched_files.contains(f));

            if !has_collision {
                for f in task_files {
                    touched_files.insert(f);
                }
                current_batch_ids.push(task_id);
            } else {
                // Collision détectée : reporter cette tâche au sous-lot suivant
                next_remaining.push(task_id);
            }
        }

        let is_parallel = current_batch_ids.len() > 1;
        batches.push(ScheduledSubWave {
            is_parallel,
            task_ids: current_batch_ids,
        });

        remaining = next_remaining;
    }

    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::plan::TaskStep;

    #[test]
    fn test_single_concurrency_serializes_all() {
        let plan = TaskPlan {
            intent: "test".to_string(),
            summary: "test".to_string(),
            tasks: vec![
                TaskStep {
                    id: "t1".to_string(),
                    title: "T1".to_string(),
                    role: "coder".to_string(),
                    goal: "".to_string(),
                    files: vec!["a.ts".to_string()],
                    tools: vec![],
                    depends_on: vec![],
                },
                TaskStep {
                    id: "t2".to_string(),
                    title: "T2".to_string(),
                    role: "coder".to_string(),
                    goal: "".to_string(),
                    files: vec!["b.ts".to_string()],
                    tools: vec![],
                    depends_on: vec![],
                },
            ],
        };

        let wave = vec!["t1".to_string(), "t2".to_string()];
        // Concurrency = 1 (mode local Ollama)
        let batches = partition_wave_into_safe_batches(&wave, &plan, 1);
        assert_eq!(batches.len(), 2);
        assert!(!batches[0].is_parallel);
        assert_eq!(batches[0].task_ids, vec!["t1"]);
        assert!(!batches[1].is_parallel);
        assert_eq!(batches[1].task_ids, vec!["t2"]);
    }

    #[test]
    fn test_disjoint_files_run_in_parallel() {
        let plan = TaskPlan {
            intent: "test".to_string(),
            summary: "test".to_string(),
            tasks: vec![
                TaskStep {
                    id: "t1".to_string(),
                    title: "T1".to_string(),
                    role: "research".to_string(),
                    goal: "Search doc".to_string(),
                    files: vec![],
                    tools: vec!["web_search".to_string()],
                    depends_on: vec![],
                },
                TaskStep {
                    id: "t2".to_string(),
                    title: "T2".to_string(),
                    role: "terminal".to_string(),
                    goal: "Check package.json".to_string(),
                    files: vec!["package.json".to_string()],
                    tools: vec!["run_command".to_string()],
                    depends_on: vec![],
                },
            ],
        };

        let wave = vec!["t1".to_string(), "t2".to_string()];
        // Concurrency = 3 (mode Cloud)
        let batches = partition_wave_into_safe_batches(&wave, &plan, 3);
        assert_eq!(batches.len(), 1);
        assert!(batches[0].is_parallel);
        assert_eq!(batches[0].task_ids, vec!["t1", "t2"]);
    }

    #[test]
    fn test_file_collision_splits_into_sequential_sub_batches() {
        let plan = TaskPlan {
            intent: "test".to_string(),
            summary: "test".to_string(),
            tasks: vec![
                TaskStep {
                    id: "t1".to_string(),
                    title: "T1".to_string(),
                    role: "coder".to_string(),
                    goal: "Edit index".to_string(),
                    files: vec!["src/index.ts".to_string()],
                    tools: vec![],
                    depends_on: vec![],
                },
                TaskStep {
                    id: "t2".to_string(),
                    title: "T2".to_string(),
                    role: "coder".to_string(),
                    goal: "Also edit index".to_string(),
                    files: vec!["src/index.ts".to_string()],
                    tools: vec![],
                    depends_on: vec![],
                },
                TaskStep {
                    id: "t3".to_string(),
                    title: "T3".to_string(),
                    role: "designer".to_string(),
                    goal: "Edit CSS".to_string(),
                    files: vec!["src/style.css".to_string()],
                    tools: vec![],
                    depends_on: vec![],
                },
            ],
        };

        let wave = vec!["t1".to_string(), "t2".to_string(), "t3".to_string()];
        let batches = partition_wave_into_safe_batches(&wave, &plan, 3);
        // t1 et t3 n'ont pas de collision -> sous-lot 1 (parallèle)
        // t2 entre en collision avec t1 sur src/index.ts -> sous-lot 2 (séquentiel)
        assert_eq!(batches.len(), 2);
        assert!(batches[0].is_parallel);
        assert_eq!(batches[0].task_ids, vec!["t1", "t3"]);
        assert!(!batches[1].is_parallel);
        assert_eq!(batches[1].task_ids, vec!["t2"]);
    }

    #[test]
    fn test_concurrency_capacity_chunking() {
        let plan = TaskPlan {
            intent: "test".to_string(),
            summary: "test".to_string(),
            tasks: (1..=5)
                .map(|i| TaskStep {
                    id: format!("t{i}"),
                    title: format!("T{i}"),
                    role: "research".to_string(),
                    goal: "".to_string(),
                    files: vec![],
                    tools: vec![],
                    depends_on: vec![],
                })
                .collect(),
        };

        let wave = vec![
            "t1".to_string(),
            "t2".to_string(),
            "t3".to_string(),
            "t4".to_string(),
            "t5".to_string(),
        ];
        // Concurrency limit = 2
        let batches = partition_wave_into_safe_batches(&wave, &plan, 2);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].task_ids, vec!["t1", "t2"]);
        assert_eq!(batches[1].task_ids, vec!["t3", "t4"]);
        assert_eq!(batches[2].task_ids, vec!["t5"]);
    }
}
