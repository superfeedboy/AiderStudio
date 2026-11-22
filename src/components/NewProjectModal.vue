<script setup lang="ts">
import { ref } from "vue";
import { useStudioStore } from "../stores/studio";

const store = useStudioStore();
const name = ref("");
const rootPath = ref("");
const globalContext = ref("");
const initRules = ref(true);
const initialPrompt = ref("");
const isPicking = ref(false);

async function browseFolder() {
  if (isPicking.value) return;
  isPicking.value = true;
  try {
    const selected = await store.pickProjectDirectory();
    if (selected) {
      rootPath.value = selected;
      if (!name.value.trim()) {
        const base = selected.split(/[/\\]/).filter(Boolean).pop();
        if (base) name.value = base;
      }
    }
  } finally {
    isPicking.value = false;
  }
}

function handlePathClick() {
  if (!rootPath.value.trim()) {
    void browseFolder();
  }
}

async function submit() {
  if (!name.value.trim() || !rootPath.value.trim()) return;
  await store.createProject(
    name.value,
    rootPath.value,
    globalContext.value,
    initRules.value,
    initialPrompt.value
  );
  name.value = "";
  rootPath.value = "";
  globalContext.value = "";
  initialPrompt.value = "";
  initRules.value = true;
}
</script>

<template>
  <div
    v-if="store.newProjectModalOpen"
    class="modal-backdrop"
    @click.self="store.newProjectModalOpen = false"
  >
    <div class="modal-card halo-hud">
      <div class="modal-header">
        <h2>Nouveau projet</h2>
        <button class="close-x" @click="store.newProjectModalOpen = false">
          ✕
        </button>
      </div>

      <div class="modal-body">
        <div class="field">
          <label>Répertoire du projet (Finder / Explorateur)</label>
          <div class="folder-picker-row">
            <input
              v-model="rootPath"
              type="text"
              placeholder="Cliquez sur Parcourir pour choisir un dossier..."
              @click="handlePathClick"
            />
            <button
              type="button"
              class="browse-btn"
              :disabled="isPicking"
              @click="browseFolder"
            >
              <svg
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                aria-hidden="true"
              >
                <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
              </svg>
              <span>{{ isPicking ? "Ouverture..." : "Parcourir…" }}</span>
            </button>
          </div>
        </div>
        <div class="field">
          <label>Nom du projet</label>
          <input v-model="name" type="text" placeholder="Ex: MonAppVue" />
        </div>
        <div class="field-checkbox">
          <label class="checkbox-label">
            <input v-model="initRules" type="checkbox" />
            <span class="checkbox-text">
              <strong>Initialiser AGENTS.md & structure .agents/rules/</strong>
              <small>Génère les standards de code et directives IA adaptés à votre stack</small>
            </span>
          </label>
        </div>
        <div v-if="initRules" class="field">
          <label>Prompt initial / Idée du projet (optionnel)</label>
          <textarea
            v-model="initialPrompt"
            rows="2"
            placeholder="Ex: Application desktop de streaming audio en Vue 3 et Tauri v2 avec SQLite..."
          />
          <span class="field-help">
            L'IA utilisera ce prompt pour pré-remplir les règles et le fichier AGENTS.md.
          </span>
        </div>
        <div class="field">
          <label>Contexte global additionnel (optionnel)</label>
          <textarea
            v-model="globalContext"
            rows="2"
            placeholder="Directives spécifiques supplémentaires..."
          />
        </div>
      </div>

      <div class="modal-footer">
        <button
          class="btn-secondary"
          @click="store.newProjectModalOpen = false"
        >
          Annuler
        </button>
        <button
          class="btn-primary"
          :disabled="!name.trim() || !rootPath.trim()"
          @click="submit"
        >
          Créer le projet
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 60;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(5px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}

.modal-card {
  width: 100%;
  max-width: 460px;
  padding: 18px 20px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.modal-header h2 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}

.close-x {
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}

.modal-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.field label {
  font-size: 12.5px;
  color: var(--text-muted);
}

.folder-picker-row {
  display: flex;
  align-items: stretch;
  gap: 8px;
}

.folder-picker-row input {
  flex: 1;
  min-width: 0;
}

.browse-btn {
  border: 1px solid var(--border-subtle);
  background: var(--active-bg);
  color: var(--accent);
  border-radius: 9px;
  padding: 0 12px;
  font-size: 12.5px;
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
}

.browse-btn:hover:not(:disabled) {
  border-color: var(--accent);
}

.browse-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.field input,
.field textarea {
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  border-radius: 9px;
  padding: 8px 10px;
  font-size: 13px;
  outline: none;
}

.field input:focus,
.field textarea:focus {
  border-color: var(--accent);
}

.field-checkbox {
  padding: 8px 10px;
  background: var(--hover-bg);
  border-radius: 9px;
  border: 1px solid var(--border-subtle);
}

.checkbox-label {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  cursor: pointer;
}

.checkbox-label input[type="checkbox"] {
  margin-top: 3px;
  accent-color: var(--accent);
}

.checkbox-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 13px;
}

.checkbox-text small {
  color: var(--text-muted);
  font-size: 11px;
}

.field-help {
  font-size: 11px;
  color: var(--text-muted);
  margin-top: 2px;
}

.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.btn-secondary {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  border-radius: 9px;
  padding: 7px 14px;
  font-size: 13px;
  cursor: pointer;
}

.btn-primary {
  border: none;
  background: var(--accent);
  color: #ffffff;
  border-radius: 9px;
  padding: 7px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}

.btn-primary:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
