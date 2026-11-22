# Règle Frontend : Vue 3.5, TypeScript & Pinia

Cette règle s'applique à tout le code sous `src/` (`.vue`, `.ts`).

## 1. Composants Vue (`.vue`)
- **Composition API obligatoire** : toujours utiliser `<script setup lang="ts">`.
- **Props et Événements typés** :
  ```ts
  const props = defineProps<{
    modelValue: string;
    disabled?: boolean;
  }>();

  const emit = defineEmits<{
    (e: 'update:modelValue', value: string): void;
    (e: 'submit'): void;
  }>();
  ```
- **Réactivité propre** :
  - Utiliser `ref()` pour les primitives et les états réassignables.
  - Utiliser `computed()` pour les valeurs dérivées (garder les computeds purs, sans effets de bord).
  - Éviter les mutations directes profondes non maîtrisées.
- **Styles** :
  - Privilégier les styles scopés `<style scoped>` pour éviter toute collision CSS entre composants.

## 2. TypeScript Strict
- Aucun `any` toléré. Si un type dynamique est nécessaire, utiliser `unknown` avec des *type guards* ou `Record<string, unknown>`.
- Placer les interfaces et modèles de données réutilisables dans `src/types/`.

## 3. Gestion d'État (Pinia)
- Définir les stores dans `src/stores/` selon la syntaxe Setup Store :
  ```ts
  import { defineStore } from 'pinia';
  import { ref, computed } from 'vue';

  export const useMyStore = defineStore('myStore', () => {
    // state
    const items = ref<string[]>([]);
    // getters
    const count = computed(() => items.value.length);
    // actions
    function addItem(item: string) {
      items.value.push(item);
    }
    return { items, count, addItem };
  });
  ```

## 4. Communication IPC avec Tauri
- Importer les APIs de communication depuis `@tauri-apps/api/core` ou plugins dédiés (`@tauri-apps/plugin-shell`, etc.).
- Toujours typer les retours de `invoke()` :
  ```ts
  import { invoke } from '@tauri-apps/api/core';
  const result = await invoke<CommandResponse>('ma_commande', { payload });
  ```
- Encapsuler les appels IPC dans des blocs `try / catch` avec notification ou retour utilisateur en cas d'erreur.
