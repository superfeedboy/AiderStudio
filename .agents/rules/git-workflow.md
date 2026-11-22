# Règle : Workflow Git & Conventions de Commit

Directives pour la gestion du versioning et les messages de commit sur **>AIx STUDIO**.

## 1. Conventions de Commits (Conventional Commits)
Format attendu :
```
<type>(<scope>): <description courte et impérative>

[corps optionnel détaillant la motivation et les changements clés]
```

### Types autorisés :
- `feat` : Nouvelle fonctionnalité utilisateur ou agentique.
- `fix` : Correction d'un bug ou d'une régression.
- `refactor` : Modification du code sans changer son comportement externe.
- `perf` : Amélioration de la vitesse ou réduction de l'empreinte mémoire.
- `style` : Formatage, retouches UI cosmétiques sans changement de logique.
- `chore` : Mise à jour des dépendances, configuration du build ou de l'outillage.
- `docs` : Documentation, README, guides internes.

### Exemples de scopes pertinents :
- `feat(engine)` : moteur multi-agents Rust ou streaming.
- `fix(ipc)` : communication Tauri frontend <-> backend.
- `feat(ui)` : interface Vue 3, tiroir de code, discussion.
- `fix(ollama)` : détection ou streaming de modèles locaux.
- `chore(deps)` : montée de version Tauri ou packages NPM.

## 2. Intégrité du Dépôt
- Ne jamais commiter de fichiers sensibles (`.env`, clés d'API, configurations personnelles).
- Ne jamais commiter les répertoires générés (`dist/`, `target/`, `node_modules/`).
- S'assurer que le `.gitignore` est scrupuleusement respecté.
