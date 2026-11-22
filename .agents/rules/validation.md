# Règle : Validation Continue & Contrôle Qualité

Avant d'affirmer qu'une fonctionnalité ou un correctif est terminé, l'agent doit valider le code avec les commandes de contrôle qualité du projet.

## 1. Validation Frontend
```bash
# Vérification des types Vue / TypeScript
npm run build
```
Si le build échoue ou remonte des erreurs de types dans `vue-tsc`, les corriger immédiatement.

## 2. Validation Backend
```bash
# Vérification de compilation Rust
cargo check --manifest-path src-tauri/Cargo.toml

# Vérification des avertissements Clippy
cargo clippy --manifest-path src-tauri/Cargo.toml
```

## 3. Checklist Obligatoire
- [ ] Aucun warning ou erreur de type TypeScript introduit.
- [ ] Le code compile sans avertissement bloquant en Rust.
- [ ] Les imports inutilisés ou fichiers résiduels sont nettoyés.
- [ ] Les contrats d'interface (IPC TypeScript <-> Commandes Rust) sont alignés (noms des payloads et types).
