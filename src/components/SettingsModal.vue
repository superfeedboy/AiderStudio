<script setup lang="ts">
import { reactive, watch } from "vue";
import { useStudioStore } from "../stores/studio";
import type { AppSettings, ThemePreference } from "../types/studio";

const store = useStudioStore();
const draft = reactive<AppSettings>({ ...store.settings });

watch(
  () => store.settingsOpen,
  (open) => {
    if (open) Object.assign(draft, store.settings);
  }
);

async function save() {
  await store.saveSettings({ ...draft });
  store.settingsOpen = false;
}

const themes: { value: ThemePreference; label: string }[] = [
  { value: "system", label: "Système" },
  { value: "dark", label: "Sombre" },
  { value: "light", label: "Clair" },
];
</script>

<template>
  <div
    v-if="store.settingsOpen"
    class="modal-backdrop"
    @click.self="store.settingsOpen = false"
  >
    <div class="modal-card halo-hud">
      <div class="modal-header">
        <h2>Paramètres AiderStudio</h2>
        <button class="close-x" @click="store.settingsOpen = false">✕</button>
      </div>

      <div class="modal-body">
        <div class="field">
          <label>Thème de l'interface</label>
          <div class="segmented">
            <button
              v-for="t in themes"
              :key="t.value"
              type="button"
              class="seg-btn"
              :class="{ active: draft.theme === t.value }"
              @click="draft.theme = t.value"
            >
              {{ t.label }}
            </button>
          </div>
        </div>

        <div class="field">
          <label>Modèle Ollama (code)</label>
          <input
            v-model="draft.ollamaModel"
            type="text"
            placeholder="qwen2.5-coder:3b"
          />
          <div v-if="store.stackStatus.availableModels.length > 0" class="model-chips">
            <button
              v-for="m in store.stackStatus.availableModels"
              :key="m"
              type="button"
              class="model-chip"
              @click="draft.ollamaModel = m"
            >
              {{ m }}
            </button>
          </div>
        </div>

        <div class="field">
          <label>URL API Ollama</label>
          <input
            v-model="draft.ollamaUrl"
            type="text"
            placeholder="http://127.0.0.1:11434"
          />
        </div>

        <div class="field">
          <label>Binaire Aider CLI</label>
          <input
            v-model="draft.aiderPath"
            type="text"
            placeholder="aider"
          />
        </div>

        <label class="checkbox-row">
          <input v-model="draft.autoStartOllama" type="checkbox" />
          <span>Démarrer Ollama automatiquement si arrêté</span>
        </label>
      </div>

      <div class="modal-footer">
        <button class="btn-secondary" @click="store.settingsOpen = false">
          Annuler
        </button>
        <button class="btn-primary" @click="save">Enregistrer</button>
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
  gap: 16px;
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
  font-size: 14px;
}

.modal-body {
  display: flex;
  flex-direction: column;
  gap: 13px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field label {
  font-size: 12.5px;
  color: var(--text-muted);
  font-weight: 500;
}

.field input {
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  border-radius: 9px;
  padding: 8px 10px;
  font-size: 13px;
  outline: none;
}

.field input:focus {
  border-color: var(--accent);
}

.segmented {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  border: 1px solid var(--border-subtle);
  border-radius: 9px;
  overflow: hidden;
  background: var(--discussion-bg);
}

.seg-btn {
  border: none;
  background: transparent;
  padding: 7px 10px;
  font-size: 12.5px;
  cursor: pointer;
  color: var(--text-muted);
}

.seg-btn.active {
  background: var(--active-bg);
  color: var(--accent);
  font-weight: 600;
}

.model-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.model-chip {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  border-radius: 6px;
  padding: 2px 8px;
  font-size: 11.5px;
  cursor: pointer;
}

.checkbox-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  cursor: pointer;
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
</style>
