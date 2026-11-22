# Règle Backend : Rust & Tauri v2

Cette règle s'applique à tout le code sous `src-tauri/` (`.rs`, `Cargo.toml`, configuration Tauri).

## 1. Principes Fondamentaux Rust
- **Rust Edition 2021**.
- **Gestion des erreurs avec `Result` & `anyhow`** :
  - Ne jamais utiliser `.unwrap()` ou `.expect()` en production qui pourrait faire crasher le runtime de l'IDE.
  - Renvoyer des erreurs serialisables (`Result<T, String>` pour les commandes Tauri ou des types d'erreurs dédiés avec `serde`).
- **Asynchronisme avec Tokio** :
  - Privilégier les streams non bloquants (`tokio::sync`, `tokio::process`, `tokio::time`).
  - Pour les tâches de streaming SSE / NDJSON, utiliser des canaux (`mpsc`) ou des événements Tauri (`app.emit(...)`).

## 2. Commandes Tauri v2
- Déclarer les commandes avec l'attribut `#[tauri::command]` :
  ```rust
  #[tauri::command]
  pub async fn executer_action(param: String) -> Result<CustomResponse, String> {
      service::traiter(param).await.map_err(|e| e.to_string())
  }
  ```
- N'oubliez pas d'enregistrer chaque commande dans le `generate_handler![...]` de `lib.rs` / `main.rs`.
- Vérifier la configuration des permissions Tauri v2 dans `src-tauri/capabilities/` si de nouvelles APIs natives ou plugins sont appelés.

## 3. Performance & Ressources
- Minimiser les clones inutiles de structures volumineuses (`Arc<T>`, références empruntées `&str`, slices).
- Surveiller l'utilisation mémoire lors des traitements de fichiers volumineux (chunking, streaming).
