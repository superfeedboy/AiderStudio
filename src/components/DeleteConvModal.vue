<script setup lang="ts">
import { useStudioStore } from "../stores/studio";

const store = useStudioStore();

function handleClose() {
  store.deleteConvModalOpen = false;
  store.convToDelete = null;
}
</script>

<template>
  <div
    v-if="store.deleteConvModalOpen && store.convToDelete"
    class="modal-backdrop"
    @click.self="handleClose"
  >
    <div class="modal-card halo-surface">
      <div class="modal-header">
        <div class="modal-title-wrap">
          <span class="modal-icon">🗑️</span>
          <div>
            <h2 class="modal-title">Supprimer la conversation</h2>
            <span class="modal-subtitle">« {{ store.convToDelete.title }} »</span>
          </div>
        </div>
        <button class="modal-close" @click="handleClose">✕</button>
      </div>

      <div class="modal-body">
        <p class="modal-intro">
          Avant de supprimer cette session, souhaitez-vous <strong>synthétiser les échanges et les fichiers créés/modifiés</strong> pour les intégrer automatiquement au <strong>contexte global du projet</strong> ?
        </p>

        <div class="conv-stats-box">
          <div class="stat-pill">
            <span class="stat-num">{{ store.convToDelete.messages.length }}</span>
            <span class="stat-lbl">messages</span>
          </div>
          <div class="stat-pill">
            <span class="stat-num">{{ store.convToDelete.modifiedFiles?.length ?? 0 }}</span>
            <span class="stat-lbl">fichiers touchés</span>
          </div>
          <div class="stat-pill">
            <span class="stat-num">+{{ store.convToDelete.linesAdded }}</span>
            <span class="stat-lbl">lignes écrites</span>
          </div>
        </div>

        <p class="modal-tip">
          💡 La synthèse conserve les requêtes clés, les résumés et les chemins de fichiers dans le contexte du projet, permettant aux futures conversations de s'appuyer sur ce travail.
        </p>
      </div>

      <div class="modal-actions">
        <button class="btn-cancel" @click="handleClose">
          Annuler
        </button>

        <button
          class="btn-delete-raw"
          title="Supprimer définitivement la session sans enrichir le projet"
          @click="store.confirmDeleteConversation(false)"
        >
          Supprimer sans synthétiser
        </button>

        <button
          class="btn-synth-delete"
          title="Résumer et intégrer au contexte global puis supprimer la session"
          @click="store.confirmDeleteConversation(true)"
        >
          🧠 Synthétiser & Supprimer
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 999;
  background: rgba(0, 0, 0, 0.65);
  backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
}

.modal-card {
  width: 100%;
  max-width: 520px;
  background: var(--table-bg);
  border: 1px solid var(--border-subtle);
  border-radius: 14px;
  box-shadow: var(--panel-shadow);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: modalPop 0.18s ease-out;
}

@keyframes modalPop {
  from {
    opacity: 0;
    transform: scale(0.96) translateY(6px);
  }
  to {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border-subtle);
  background: rgba(255, 255, 255, 0.02);
}

.modal-title-wrap {
  display: flex;
  align-items: center;
  gap: 10px;
}

.modal-icon {
  font-size: 20px;
}

.modal-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: var(--text-main);
}

.modal-subtitle {
  font-size: 12px;
  color: var(--accent);
  display: block;
  max-width: 340px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.modal-close {
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 14px;
  cursor: pointer;
  padding: 4px;
  border-radius: 6px;
}

.modal-close:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.modal-body {
  padding: 16px 18px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  font-size: 13px;
  line-height: 1.5;
  color: var(--text-main);
}

.modal-intro {
  margin: 0;
}

.conv-stats-box {
  display: flex;
  gap: 10px;
  padding: 10px 12px;
  background: var(--surface-elevated);
  border-radius: 9px;
  border: 1px solid var(--border-subtle);
}

.stat-pill {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
}

.stat-num {
  font-weight: 700;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  color: var(--accent);
}

.stat-lbl {
  font-size: 11px;
  color: var(--text-dim);
}

.modal-tip {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-muted);
  background: rgba(129, 140, 248, 0.06);
  padding: 8px 10px;
  border-radius: 7px;
  border-left: 3px solid var(--accent);
}

.modal-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px;
  border-top: 1px solid var(--border-subtle);
  background: rgba(255, 255, 255, 0.015);
}

.btn-cancel {
  border: 1px solid var(--border-subtle);
  background: transparent;
  color: var(--text-muted);
  border-radius: 8px;
  padding: 7px 12px;
  font-size: 12px;
  cursor: pointer;
}

.btn-cancel:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.btn-delete-raw {
  border: 1px solid rgba(239, 68, 68, 0.3);
  background: rgba(239, 68, 68, 0.08);
  color: #ef4444;
  border-radius: 8px;
  padding: 7px 12px;
  font-size: 12px;
  cursor: pointer;
}

.btn-delete-raw:hover {
  background: rgba(239, 68, 68, 0.16);
}

.btn-synth-delete {
  border: none;
  background: var(--accent);
  color: #fff;
  border-radius: 8px;
  padding: 7px 14px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  box-shadow: 0 2px 8px rgba(129, 140, 248, 0.3);
  transition: opacity 0.15s ease;
}

.btn-synth-delete:hover {
  opacity: 0.92;
}
</style>
