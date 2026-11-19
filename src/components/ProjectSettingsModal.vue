<script setup lang="ts">
import { ref, watch } from "vue";
import { useStudioStore } from "../stores/studio";

const store = useStudioStore();

const draftName = ref("");
const draftContext = ref("");
const draftSharedDirs = ref<string[]>([]);
const activeTab = ref<"general" | "dirs" | "context" | "danger">("general");
const isConfirmingDelete = ref(false);

watch(
  () => store.projectSettingsModalOpen,
  (open) => {
    if (open && store.projectToEdit) {
      draftName.value = store.projectToEdit.name;
      draftContext.value = store.projectToEdit.globalContext || "";
      draftSharedDirs.value = [...(store.projectToEdit.sharedDirectories || [])];
      activeTab.value = "general";
      isConfirmingDelete.value = false;
    }
  }
);

async function handleAddSharedDir() {
  const chosen = await store.pickProjectDirectory();
  if (chosen && !draftSharedDirs.value.includes(chosen)) {
    draftSharedDirs.value.push(chosen);
  }
}

function handleRemoveSharedDir(index: number) {
  draftSharedDirs.value.splice(index, 1);
}

function clearGlobalContext() {
  draftContext.value = "";
}

async function handleSave() {
  if (!store.projectToEdit) return;
  const nameToSave = draftName.value.trim() || store.projectToEdit.name;
  await store.updateProject(
    store.projectToEdit.id,
    nameToSave,
    draftContext.value,
    draftSharedDirs.value
  );
}

function handleDeleteProject() {
  if (!store.projectToEdit) return;
  const proj = store.projectToEdit;
  store.projectSettingsModalOpen = false;
  void store.deleteProject(proj.id);
}
</script>

<template>
  <div
    v-if="store.projectSettingsModalOpen"
    class="modal-backdrop"
    @click.self="store.projectSettingsModalOpen = false"
  >
    <div class="modal-card halo-hud">
      <!-- Header -->
      <div class="modal-header">
        <div class="modal-title-group">
          <svg class="settings-gear-icon" viewBox="0 0 24 24" fill="currentColor">
            <path d="M12 1l1.45 2.07c.88.29 1.72.69 2.5 1.2l2.44-.8 1.41 1.41-.8 2.44c.51.78.91 1.62 1.2 2.5L22 11v2l-2.07 1.45c-.29.88-.69 1.72-1.2 2.5l.8 2.44-1.41 1.41-2.44-.8c-.78.51-1.62.91-2.5 1.2L13 23h-2l-1.45-2.07c-.88-.29-1.72-.69-2.5-1.2l-2.44.8-1.41-1.41.8-2.44a8.99 8.99 0 0 1-1.2-2.5L2 13v-2l2.07-1.45c.29-.88.69-1.72 1.2-2.5l-.8-2.44 1.41-1.41 2.44.8c.78-.51 1.62-.91 2.5-1.2L11 1h2zm-1 3.08-1.09 1.56-.63.26a6.99 6.99 0 0 0-1.95.94l-.56.41-1.84-.6-1.41 1.41.6 1.84-.41.56a6.99 6.99 0 0 0-.94 1.95l-.26.63L2.55 12l1.56 1.09.26.63c.24.7.56 1.35.94 1.95l.41.56-.6 1.84 1.41 1.41 1.84-.6.56.41c.6.38 1.25.7 1.95.94l.63.26L11 20.92h2l1.09-1.56.63-.26c.7-.24 1.35-.56 1.95-.94l.56-.41 1.84.6 1.41-1.41-.6-1.84.41-.56c.38-.6.7-1.25.94-1.95l.26-.63 1.56-1.09-1.56-1.09-.26-.63a6.99 6.99 0 0 0-.94-1.95l-.41-.56.6-1.84-1.41-1.41-1.84.6-.56-.41a6.99 6.99 0 0 0-1.95-.94l-.63-.26L13 4.08h-2zM12 8a4 4 0 1 1 0 8 4 4 0 0 1 0-8zm0 2a2 2 0 1 0 0 4 2 2 0 0 0 0-4z"/>
          </svg>
          <h2>Paramètres du projet · <span class="proj-highlight">{{ store.projectToEdit?.name }}</span></h2>
        </div>
        <button class="close-x" title="Fermer" @click="store.projectSettingsModalOpen = false">✕</button>
      </div>

      <!-- Navigation Tabs -->
      <div class="settings-tabs">
        <button
          class="tab-btn"
          :class="{ active: activeTab === 'general' }"
          @click="activeTab = 'general'"
        >
          <svg class="tab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 20h9M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z" />
          </svg>
          Général & Nom
        </button>
        <button
          class="tab-btn"
          :class="{ active: activeTab === 'dirs' }"
          @click="activeTab = 'dirs'"
        >
          <svg class="tab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
          </svg>
          Dossiers partagés
          <span v-if="draftSharedDirs.length > 0" class="tab-badge">{{ draftSharedDirs.length }}</span>
        </button>
        <button
          class="tab-btn"
          :class="{ active: activeTab === 'context' }"
          @click="activeTab = 'context'"
        >
          <svg class="tab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1-2.5-2.5Z" />
            <path d="M6 6h10" />
            <path d="M6 10h10" />
          </svg>
          Contexte global
        </button>
        <button
          class="tab-btn tab-danger"
          :class="{ active: activeTab === 'danger' }"
          @click="activeTab = 'danger'"
        >
          <svg class="tab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
          </svg>
          Danger
        </button>
      </div>

      <!-- Tab Contents -->
      <div class="tab-body custom-scroll">
        <!-- TAB 1: Général & Nom -->
        <div v-if="activeTab === 'general'" class="settings-pane">
          <div class="field">
            <label class="field-label">Nom du projet</label>
            <input
              v-model="draftName"
              type="text"
              class="settings-input"
              placeholder="Ex: MonSuperProjet"
            />
            <span class="field-hint">Ce nom s'affiche dans le menu latéral et vos onglets de sessions.</span>
          </div>

          <div class="field">
            <label class="field-label">Emplacement racine du projet sur le disque</label>
            <div class="path-display-box" :title="store.projectToEdit?.rootPath">
              <svg class="path-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
              </svg>
              <span class="path-text">{{ store.projectToEdit?.rootPath }}</span>
            </div>
            <span class="field-hint">Dépôt Git suivi par AiderStudio et espace de création de vos fichiers.</span>
          </div>

          <!-- Optimization & .aiderignore status card -->
          <div class="field aiderignore-card">
            <div class="aiderignore-header">
              <div class="aiderignore-info">
                <span class="card-tag" :class="store.hasAiderignore ? 'tag-ok' : 'tag-warn'">
                  {{ store.hasAiderignore ? "✓ Protection Contexte Active" : "⚠️ Optimisation Requise" }}
                </span>
                <span class="aiderignore-title">Fichier .aiderignore / .gitignore (Protection 8K)</span>
              </div>
              <button
                v-if="!store.hasAiderignore"
                class="create-ignore-btn"
                @click="store.ensureAiderignore(store.projectToEdit?.id)"
              >
                Générer .aiderignore
              </button>
            </div>
            <p class="aiderignore-desc">
              {{ store.hasAiderignore
                ? "Le fichier .aiderignore (en complément de .gitignore) exclut node_modules, target, dist, lockfiles et logs de la repo-map et des outils du moteur natif."
                : "Sans .aiderignore, les dossiers volumineux non ignorés par Git peuvent encombrer la repo-map et la recherche du moteur natif sur les modèles 8K." }}
            </p>
          </div>
        </div>

        <!-- TAB 2: Dossiers partagés -->
        <div v-if="activeTab === 'dirs'" class="settings-pane">
          <div class="pane-header-row">
            <div>
              <label class="field-label">Dossiers partagés à l'agent</label>
              <p class="field-hint">
                Répertoires additionnels mis à disposition de l'agent (documentation, assets partagés, base SQLite, micro-services annexes).
              </p>
            </div>
            <button class="add-dir-btn" @click="handleAddSharedDir">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                <path d="M12 5v14M5 12h14" />
              </svg>
              Ajouter un dossier
            </button>
          </div>

          <div v-if="draftSharedDirs.length === 0" class="empty-dirs-box">
            <svg class="empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
              <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
            </svg>
            <p>Aucun dossier supplémentaire partagé pour l'instant.</p>
            <span class="empty-sub">Cliquez sur « Ajouter un dossier » pour sélectionner un dossier de votre disque.</span>
          </div>

          <div v-else class="dirs-list">
            <div
              v-for="(dirPath, idx) in draftSharedDirs"
              :key="idx"
              class="dir-item"
            >
              <div class="dir-item-left" :title="dirPath">
                <svg class="dir-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
                </svg>
                <span class="dir-path">{{ dirPath }}</span>
              </div>
              <button
                class="remove-dir-btn"
                title="Retirer ce dossier des partages"
                @click="handleRemoveSharedDir(idx)"
              >
                ✕
              </button>
            </div>
          </div>
        </div>

        <!-- TAB 3: Contexte global -->
        <div v-if="activeTab === 'context'" class="settings-pane">
          <div class="field">
            <div class="field-header-row">
              <label class="field-label">Contexte global du projet</label>
              <button
                v-if="draftContext.trim().length > 0"
                class="clear-context-btn"
                title="Vider le contexte"
                @click="clearGlobalContext"
              >
                Vider le contexte
              </button>
            </div>
            <textarea
              v-model="draftContext"
              rows="9"
              class="settings-textarea custom-scroll"
              placeholder="Règles d'architecture, technologies, dépendances requises, conventions de nommage... Ce contenu est injecté au modèle à chaque requête."
            />
            <span class="field-hint">
              Ce texte est partagé entre toutes les conversations de ce projet.
            </span>
          </div>
        </div>

        <!-- TAB 4: Zone de danger -->
        <div v-if="activeTab === 'danger'" class="settings-pane">
          <div class="danger-box">
            <div class="danger-header">
              <svg class="danger-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z" />
                <line x1="12" y1="9" x2="12" y2="13" />
                <line x1="12" y1="17" x2="12.01" y2="17" />
              </svg>
              <div>
                <h4>Supprimer le projet de l'interface</h4>
                <p>
                  Cette action retire définitivement le projet « {{ store.projectToEdit?.name }} » ainsi que tout l'historique de ses conversations d'AiderStudio. Vos fichiers source sur le disque resteront intacts.
                </p>
              </div>
            </div>
            <div v-if="!isConfirmingDelete">
              <button
                class="btn-danger-action"
                @click="isConfirmingDelete = true"
              >
                Supprimer le projet
              </button>
            </div>
            <div v-else class="danger-confirm-row">
              <span class="danger-confirm-warning">
                ⚠️ Confirmez-vous le retrait définitif de ce projet d'AiderStudio ?
              </span>
              <div class="danger-btns">
                <button class="btn-danger-action" @click="handleDeleteProject">
                  Oui, supprimer définitivement
                </button>
                <button class="btn-secondary" @click="isConfirmingDelete = false">
                  Annuler
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Footer -->
      <div class="modal-footer">
        <button class="btn-secondary" @click="store.projectSettingsModalOpen = false">
          Annuler
        </button>
        <button class="btn-primary" @click="handleSave">
          Enregistrer les modifications
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
  background: rgba(0, 0, 0, 0.58);
  backdrop-filter: blur(6px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}

.modal-card {
  width: 100%;
  max-width: 640px;
  max-height: 85vh;
  padding: 20px 22px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  border-radius: 12px;
  background: var(--surface-elevated);
  border: 1px solid var(--border-subtle);
  box-shadow: var(--panel-shadow);
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 4px;
}

.modal-title-group {
  display: flex;
  align-items: center;
  gap: 8px;
}

.settings-gear-icon {
  width: 18px;
  height: 18px;
  color: var(--accent);
  flex-shrink: 0;
}

.modal-header h2 {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: var(--text-main);
}

.proj-highlight {
  color: var(--accent);
}

.close-x {
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 15px;
  cursor: pointer;
  padding: 4px 6px;
  border-radius: 6px;
  transition: all 0.12s ease;
}

.close-x:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.settings-tabs {
  display: flex;
  gap: 6px;
  border-bottom: 1px solid var(--border-subtle);
  padding-bottom: 6px;
}

.tab-btn {
  border: none;
  background: transparent;
  color: var(--text-muted);
  padding: 6px 10px;
  font-size: 12px;
  font-weight: 500;
  border-radius: 7px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  transition: all 0.12s ease;
}

.tab-btn:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.tab-btn.active {
  background: var(--active-bg);
  color: var(--accent);
  font-weight: 600;
}

.tab-btn.tab-danger.active {
  color: #ef4444;
  background: rgba(239, 68, 68, 0.12);
}

.tab-icon {
  width: 14px;
  height: 14px;
}

.tab-badge {
  font-size: 10px;
  padding: 1px 5px;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  font-weight: 700;
}

.tab-body {
  flex: 1;
  min-height: 240px;
  max-height: 52vh;
  overflow-y: auto;
  padding: 6px 2px;
}

.settings-pane {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-header-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.field-label {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-main);
}

.field-hint {
  font-size: 11.5px;
  color: var(--text-muted);
  line-height: 1.4;
}

.settings-input {
  width: 100%;
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-main);
  font-size: 13px;
  outline: none;
  transition: border-color 0.15s ease, background-color 0.15s ease;
}

.settings-input:focus {
  border-color: var(--accent);
}

.settings-textarea {
  width: 100%;
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-main);
  font-family: inherit;
  font-size: 12.5px;
  line-height: 1.5;
  resize: vertical;
  outline: none;
  transition: border-color 0.15s ease, background-color 0.15s ease;
}

.settings-textarea:focus {
  border-color: var(--accent);
}

:root[data-theme="light"] .settings-input,
:root[data-theme="light"] .settings-textarea {
  background: #ffffff;
  border-color: rgba(15, 23, 42, 0.18);
  color: var(--text-main);
}

:root[data-theme="light"] .path-display-box,
:root[data-theme="light"] .dir-item {
  background: rgba(15, 23, 42, 0.04);
  border-color: rgba(15, 23, 42, 0.12);
}

.path-display-box {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 8px;
  background: var(--hover-bg);
  border: 1px solid var(--border-subtle);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
  color: var(--text-muted);
  overflow: hidden;
}

.path-icon {
  width: 15px;
  height: 15px;
  flex-shrink: 0;
  color: var(--accent);
}

.path-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.aiderignore-card {
  margin-top: 14px;
  padding: 12px;
  border-radius: 9px;
  border: 1px solid var(--border-subtle);
  background: rgba(129, 140, 248, 0.04);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.aiderignore-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.aiderignore-info {
  display: flex;
  align-items: center;
  gap: 8px;
}

.card-tag {
  font-size: 10px;
  font-weight: 700;
  padding: 2px 7px;
  border-radius: 999px;
}

.tag-ok {
  background: rgba(16, 185, 129, 0.2);
  color: #10b981;
  border: 1px solid rgba(16, 185, 129, 0.35);
}

.tag-warn {
  background: rgba(245, 158, 11, 0.2);
  color: #f59e0b;
  border: 1px solid rgba(245, 158, 11, 0.35);
}

.aiderignore-title {
  font-size: 12px;
  font-weight: 600;
}

.aiderignore-desc {
  margin: 0;
  font-size: 11.5px;
  color: var(--text-muted);
  line-height: 1.45;
}

.create-ignore-btn {
  border: 1px solid var(--accent);
  background: var(--active-bg);
  color: var(--accent);
  font-size: 11px;
  font-weight: 600;
  padding: 4px 9px;
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.create-ignore-btn:hover {
  background: var(--accent);
  color: #fff;
}

.pane-header-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.add-dir-btn {
  border: 1px solid var(--accent);
  background: var(--accent);
  color: #fff;
  border-radius: 7px;
  padding: 6px 10px;
  font-size: 11.5px;
  font-weight: 600;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  cursor: pointer;
  white-space: nowrap;
  transition: opacity 0.15s ease;
}

.add-dir-btn:hover {
  opacity: 0.9;
}

.empty-dirs-box {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 28px 14px;
  border: 1px dashed var(--border-subtle);
  border-radius: 9px;
  color: var(--text-muted);
  gap: 6px;
}

.empty-icon {
  width: 28px;
  height: 28px;
  color: var(--text-dim);
}

.empty-dirs-box p {
  margin: 0;
  font-size: 12.5px;
  color: var(--text-main);
}

.empty-sub {
  font-size: 11px;
  color: var(--text-muted);
}

.dirs-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.dir-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 7px 10px;
  border-radius: 8px;
  background: var(--hover-bg);
  border: 1px solid var(--border-subtle);
}

.dir-item-left {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  flex: 1;
}

.dir-icon {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  color: var(--accent);
}

.dir-path {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11.5px;
  color: var(--text-main);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.remove-dir-btn {
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 12px;
  padding: 2px 6px;
  border-radius: 5px;
  transition: all 0.12s ease;
}

.remove-dir-btn:hover {
  background: rgba(239, 68, 68, 0.15);
  color: #ef4444;
}

.clear-context-btn {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  color: var(--text-muted);
  border-radius: 6px;
  padding: 2px 8px;
  font-size: 11px;
  cursor: pointer;
  transition: all 0.12s ease;
}

.clear-context-btn:hover {
  color: #ef4444;
  border-color: rgba(239, 68, 68, 0.4);
}

.danger-box {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 14px 16px;
  border-radius: 9px;
  border: 1px solid rgba(239, 68, 68, 0.3);
  background: rgba(239, 68, 68, 0.05);
}

.danger-header {
  display: flex;
  gap: 12px;
}

.danger-icon {
  width: 22px;
  height: 22px;
  color: #ef4444;
  flex-shrink: 0;
}

.danger-header h4 {
  margin: 0 0 4px;
  font-size: 13px;
  font-weight: 600;
  color: #ef4444;
}

.danger-header p {
  margin: 0;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.45;
}

.btn-danger-action {
  align-self: flex-start;
  border: 1px solid #ef4444;
  background: #ef4444;
  color: #fff;
  border-radius: 7px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.15s ease;
}

.btn-danger-action:hover {
  opacity: 0.9;
}

.danger-confirm-row {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px;
  background: rgba(239, 68, 68, 0.12);
  border: 1px solid rgba(239, 68, 68, 0.35);
  border-radius: 8px;
}

.danger-confirm-warning {
  font-size: 12px;
  font-weight: 600;
  color: #ef4444;
}

.danger-btns {
  display: flex;
  align-items: center;
  gap: 8px;
}

.modal-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  padding-top: 8px;
  border-top: 1px solid var(--border-subtle);
}

.btn-secondary {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  color: var(--text-main);
  border-radius: 7px;
  padding: 6px 14px;
  font-size: 12.5px;
  cursor: pointer;
  transition: all 0.12s ease;
}

.btn-secondary:hover {
  background: var(--active-bg);
}

.btn-primary {
  border: none;
  background: var(--accent);
  color: #fff;
  border-radius: 7px;
  padding: 6px 16px;
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.15s ease;
}

.btn-primary:hover {
  opacity: 0.9;
}
</style>
