<script setup lang="ts">
import { ref, watch } from "vue";
import { useStudioStore } from "../stores/studio";

const store = useStudioStore();
const globalContextDraft = ref("");

watch(
  () => store.contextModalOpen,
  (open) => {
    if (open && store.activeProject) {
      globalContextDraft.value = store.activeProject.globalContext;
    }
  }
);

async function save() {
  if (!store.activeProject) return;
  await store.updateProjectContext(
    store.activeProject.id,
    globalContextDraft.value
  );
  store.contextModalOpen = false;
}

async function clearGlobal() {
  if (!store.activeProject) return;
  globalContextDraft.value = "";
  await store.clearGlobalContext(store.activeProject.id);
}

async function clearConv() {
  if (!store.activeConversation) return;
  await store.clearConversationContext(store.activeConversation.id);
}
</script>

<template>
  <div
    v-if="store.contextModalOpen"
    class="modal-backdrop"
    @click.self="store.contextModalOpen = false"
  >
    <div class="modal-card halo-hud">
      <div class="modal-header">
        <h2>Contextes & Mémoire · {{ store.activeProject?.name }}</h2>
        <button class="close-x" @click="store.contextModalOpen = false">✕</button>
      </div>

      <div class="ctx-grid">
        <div class="field">
          <div class="field-header">
            <label>Contexte global du projet (partagé entre toutes les conversations)</label>
            <button class="clear-btn" title="Vider le contexte global" @click="clearGlobal">
              Vider
            </button>
          </div>
          <textarea
            v-model="globalContextDraft"
            rows="6"
            placeholder="Définissez les règles, objectifs ou spécifications architecturales..."
            class="custom-scroll"
          />
        </div>

        <div class="field">
          <div class="field-header">
            <label>
              Mémoire de la conversation active ({{ store.activeConversation?.title }})
            </label>
            <button
              v-if="store.activeConversation?.conversationContext"
              class="clear-btn"
              title="Vider la mémoire de cette conversation"
              @click="clearConv"
            >
              Vider
            </button>
          </div>
          <pre class="conv-ctx-box custom-scroll">{{
            store.activeConversation?.conversationContext ||
            "La mémoire se construira automatiquement au fil de vos prompts."
          }}</pre>
        </div>
      </div>

      <div class="modal-footer">
        <button class="btn-secondary" @click="store.contextModalOpen = false">
          Fermer
        </button>
        <button class="btn-primary" @click="save">
          Enregistrer le contexte global
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
  max-width: 580px;
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

.ctx-grid {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.clear-btn {
  border: 1px solid var(--border-subtle);
  background: transparent;
  color: var(--text-dim);
  border-radius: 5px;
  padding: 1px 6px;
  font-size: 11px;
  cursor: pointer;
}

.clear-btn:hover {
  color: var(--diff-del-bar);
  border-color: var(--diff-del-bar);
}

.field label {
  font-size: 12.5px;
  color: var(--text-muted);
  font-weight: 500;
}

.field textarea {
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  border-radius: 10px;
  padding: 10px;
  font-size: 12.5px;
  line-height: 1.5;
  resize: vertical;
  outline: none;
}

.field textarea:focus {
  border-color: var(--accent);
}

.conv-ctx-box {
  margin: 0;
  max-height: 140px;
  overflow-y: auto;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  border-radius: 10px;
  padding: 10px;
  font-size: 12px;
  color: var(--text-muted);
  white-space: pre-wrap;
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
