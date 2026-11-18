<script setup lang="ts">
import { reactive, watch } from "vue";
import { useStudioStore } from "../stores/studio";
import type {
  AiderEditFormat,
  AppSettings,
  ThemePreference,
} from "../types/studio";

const store = useStudioStore();
const draft = reactive<AppSettings>({
  ...store.settings,
  aiderEditFormat: store.settings.aiderEditFormat || "auto",
});

watch(
  () => store.settingsOpen,
  (open) => {
    if (open) {
      Object.assign(draft, store.settings);
      draft.aiderEditFormat = store.settings.aiderEditFormat || "auto";
    }
  }
);

async function save() {
  await store.saveSettings({
    ...draft,
    aiderEditFormat: draft.aiderEditFormat || "auto",
  });
  store.settingsOpen = false;
}

const themes: { value: ThemePreference; label: string }[] = [
  { value: "system", label: "Système" },
  { value: "dark", label: "Sombre" },
  { value: "light", label: "Clair" },
];

const editFormats: { value: AiderEditFormat; label: string }[] = [
  { value: "auto", label: "Auto (8K)" },
  { value: "whole", label: "Complet" },
  { value: "diff", label: "Diff" },
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

      <div class="modal-body custom-scroll">
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
          <div class="field-header">
            <label>Fenêtre de contexte cible (tokens)</label>
            <span class="context-pill">{{ draft.contextWindowTokens || 8192 }} tokens</span>
          </div>
          <div class="context-presets">
            <button
              v-for="size in [4096, 8192, 16384, 32768, 65536]"
              :key="size"
              type="button"
              class="preset-btn"
              :class="{ active: (draft.contextWindowTokens || 8192) === size }"
              @click="draft.contextWindowTokens = size"
            >
              {{ size >= 1024 ? `${size / 1024}K` : size }}
            </button>
          </div>
          <input
            v-model.number="draft.contextWindowTokens"
            type="number"
            min="1024"
            max="131072"
            step="1024"
            placeholder="8192"
          />
          <span class="field-hint">
            Transmis automatiquement comme <code>num_ctx</code> à votre serveur Ollama via Aider. Astuce RTX 4060Ti 8 Go + <code>q5_K_M</code> : activez <code>OLLAMA_FLASH_ATTENTION=1</code> et <code>OLLAMA_KV_CACHE_TYPE=q8_0</code> côté Ollama pour débloquer <strong>16K</strong> 100% en VRAM.
          </span>
        </div>

        <div class="field">
          <label>Format d'édition Aider</label>
          <div class="segmented">
            <button
              v-for="fmt in editFormats"
              :key="fmt.value"
              type="button"
              class="seg-btn"
              :class="{ active: (draft.aiderEditFormat || 'auto') === fmt.value }"
              @click="draft.aiderEditFormat = fmt.value"
            >
              {{ fmt.label }}
            </button>
          </div>
          <span class="field-hint">
            <strong>Auto (8K)</strong> : utilise <code>whole</code> pour les fichiers ≤ 250 lignes (zéro erreur diff et prompt ultra-léger sur Qwen 7B) et bascule en <code>diff</code> au-delà.
          </span>
        </div>

        <div class="field">
          <label>Binaire Aider CLI</label>
          <input
            v-model="draft.aiderPath"
            type="text"
            placeholder="aider"
          />
        </div>

        <!-- Stack Status & Download Proposals -->
        <div class="stack-probe-section">
          <label class="section-label">Diagnostic de la stack locale</label>
          <div class="probe-cards">
            <!-- OLLAMA PROBE -->
            <div class="probe-card" :class="store.stackStatus.ollamaInstalled ? 'is-installed' : 'is-missing'">
              <div class="probe-header">
                <span class="probe-title">Ollama</span>
                <span class="probe-status-tag" :class="store.stackStatus.ollamaRunning ? 'ok' : store.stackStatus.ollamaInstalled ? 'idle' : 'danger'">
                  {{ store.stackStatus.ollamaRunning ? 'En ligne' : store.stackStatus.ollamaInstalled ? 'Installé (arrêté)' : 'Non détecté' }}
                </span>
              </div>
              <p class="probe-desc">
                {{ store.stackStatus.ollamaInstalled ? 'Moteur d\'exécution de modèles LLM locaux (GPU/CPU).' : 'Ollama n\'a pas été trouvé sur votre machine.' }}
              </p>
              <div v-if="!store.stackStatus.ollamaInstalled" class="probe-action-row">
                <a href="https://ollama.com/download" target="_blank" rel="noopener noreferrer" class="download-link-btn">
                  📥 Télécharger Ollama (ollama.com)
                </a>
              </div>
            </div>

            <!-- AIDER PROBE -->
            <div class="probe-card" :class="store.stackStatus.aiderInstalled ? 'is-installed' : 'is-missing'">
              <div class="probe-header">
                <span class="probe-title">Aider CLI</span>
                <span class="probe-status-tag" :class="store.stackStatus.aiderInstalled ? 'ok' : 'danger'">
                  {{ store.stackStatus.aiderInstalled ? 'Prêt' : 'Non détecté' }}
                </span>
              </div>
              <p class="probe-desc">
                {{ store.stackStatus.aiderInstalled ? 'Pilote de code pair-programming en ligne de commande.' : 'Aider CLI n\'a pas été détecté dans votre environnement.' }}
              </p>
              <div v-if="!store.stackStatus.aiderInstalled" class="probe-action-row">
                <a href="https://aider.chat/docs/install.html" target="_blank" rel="noopener noreferrer" class="download-link-btn">
                  📥 Installer Aider (`pipx install aider-chat`)
                </a>
              </div>
            </div>
          </div>
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
  max-height: 70vh;
  overflow-y: auto;
  padding-right: 4px;
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

.field-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.context-pill {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--active-bg);
  color: var(--accent);
  font-weight: 600;
}

.context-presets {
  display: flex;
  gap: 6px;
  margin-bottom: 2px;
}

.preset-btn {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  color: var(--text-muted);
  border-radius: 6px;
  padding: 3px 8px;
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.12s ease;
}

.preset-btn:hover {
  background: var(--active-bg);
  color: var(--accent);
}

.preset-btn.active {
  background: var(--accent);
  color: #fff;
  border-color: var(--accent);
  font-weight: 600;
}

.field-hint {
  font-size: 11px;
  color: var(--text-dim);
  line-height: 1.4;
}

.stack-probe-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 4px;
}

.section-label {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-main);
}

.probe-cards {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.probe-card {
  padding: 10px 12px;
  border-radius: 9px;
  background: var(--hover-bg);
  border: 1px solid var(--border-subtle);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.probe-card.is-missing {
  border-color: rgba(245, 158, 11, 0.3);
}

.probe-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.probe-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-main);
}

.probe-status-tag {
  font-size: 10.5px;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: 5px;
}

.probe-status-tag.ok {
  background: rgba(16, 185, 129, 0.18);
  color: #10b981;
}

.probe-status-tag.idle {
  background: rgba(245, 158, 11, 0.18);
  color: #f59e0b;
}

.probe-status-tag.danger {
  background: rgba(239, 68, 68, 0.18);
  color: #ef4444;
}

.probe-desc {
  margin: 0;
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.35;
}

.probe-action-row {
  margin-top: 4px;
}

.download-link-btn {
  display: inline-block;
  font-size: 11px;
  font-weight: 600;
  color: var(--accent);
  text-decoration: none;
  background: var(--active-bg);
  border: 1px solid var(--accent);
  padding: 4px 8px;
  border-radius: 6px;
  transition: opacity 0.15s ease;
}

.download-link-btn:hover {
  opacity: 0.85;
}
</style>
