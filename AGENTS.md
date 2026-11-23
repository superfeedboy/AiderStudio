# AGENTS.md — Directives & Architecture pour >AIx STUDIO

Ce fichier définit les directives fondamentales, l'architecture globale et les standards de développement pour tout agent IA ou contributeur travaillant sur le projet **>AIx STUDIO**.

---

## 1. Vue d'ensemble du Projet

**>AIx STUDIO** est un IDE desktop natif de pair-programming IA agnostique et multi-agents (macOS Universal & Windows x64).
- **Frontend** : Vue 3.5 (Composition API avec `<script setup lang="ts">`), Pinia 2.3, Vite 6, Highlight.js.
- **Backend / Desktop** : Tauri v2, Rust 2021 (moteur multi-agents 100 % natif, zéro dépendance Python/Aider).
- **Communication & LLM** : Moteur hybride combinant Ollama en local et fournisseurs Cloud (Gemini, OpenAI, Anthropic, DeepSeek, OpenRouter, Mistral), streaming unifié SSE/NDJSON, Model Context Protocol (MCP).

---

## 2. Commandes de Build & Vérification

Avant de valider ou proposer un changement, **toujours exécuter ou vérifier ces étapes** :

### Frontend (Vue 3 / TypeScript)
```bash
# Vérification des types TypeScript et build de production Vite
npm run build
# ou uniquement le type-checking
npx vue-tsc --noEmit
```

### Backend (Rust / Tauri v2)
```bash
# Vérification de compilation Rust
cargo check --manifest-path src-tauri/Cargo.toml

# Linter Rust avec alertes strictes
cargo clippy --manifest-path src-tauri/Cargo.toml
```

---

## 3. Règles Fondamentales pour les Agents

1. **Zéro Régression de Types** :
   - TypeScript strict partout. **Interdiction d'utiliser `any`** (privilégier `unknown`, `never`, des types d'union stricts ou des interfaces dédiées dans `src/types/`).
2. **Architecture Rust Robuste** :
   - Toujours gérer les erreurs via `Result<T, E>` et `anyhow` côté Rust.
   - Ne jamais faire de `.unwrap()` ou `.expect()` susceptible de paniquer en production dans les commandes Tauri ou les tâches asynchrones Tokio.
3. **Édition Chirurgicale** :
   - Préserver rigoureusement le style de code existant et l'indentation.
   - Ne pas supprimer de code, de commentaires utiles ou de typages sans justification explicite.
4. **Structure Modulaire** :
   - Les règles détaillées sont compartimentées dans le dossier `.agents/rules/` :
     - `.agents/rules/frontend-vue.md` : standards Vue 3 & Pinia.
     - `.agents/rules/backend-rust-tauri.md` : standards Tauri v2 & Rust Tokio.
     - `.agents/rules/validation.md` : chaîne de compilation et contrôle qualité.
     - `.agents/rules/git-workflow.md` : conventions de commit et gestion des versions.
