<script setup lang="ts">
import { useStudioStore } from "../stores/studio";
import type { Project } from "../types/studio";
import AppLogo from "./AppLogo.vue";

const store = useStudioStore();

function handleDeleteProject(proj: Project) {
  const confirmMsg = `Voulez-vous vraiment supprimer le projet « ${proj.name} » ?\n(Les fichiers sur votre disque ne seront pas touchés, seules les sessions associées seront supprimées).`;
  if (window.confirm(confirmMsg)) {
    void store.deleteProject(proj.id);
  }
}
</script>

<template>
  <aside
    class="le-menu"
    :style="{ width: `${store.menuWidth}px` }"
  >
    <div class="le-menu-top">
      <div class="brand-row">
        <div class="brand-identity">
          <div class="brand-icon-box">
            <AppLogo :size="18" />
          </div>
          <span class="brand-name">AiderStudio</span>
        </div>
        <button
          class="icon-btn"
          title="Nouveau projet"
          @click="store.newProjectModalOpen = true"
        >
          + Projet
        </button>
      </div>

      <!-- Projects & Conversations Tree -->
      <div class="tree-scroll custom-scroll">
        <div
          v-for="project in store.projects"
          :key="project.id"
          class="project-group"
        >
          <div
            class="project-header"
            :class="{ 'is-active': store.activeProject?.id === project.id }"
          >
            <div class="project-header-left" :title="`${project.name} (${project.rootPath})`">
              <svg
                class="project-folder-svg"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.9"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" />
              </svg>
              <span class="project-name" :title="project.name">
                {{ project.name }}
              </span>
            </div>
            <div class="project-actions">
              <button
                class="mini-pill"
                title="Paramètres du projet (dossiers partagés, contexte, nom, suppression)"
                @click.stop="store.openProjectSettings(project)"
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M12 1l1.45 2.07c.88.29 1.72.69 2.5 1.2l2.44-.8 1.41 1.41-.8 2.44c.51.78.91 1.62 1.2 2.5L22 11v2l-2.07 1.45c-.29.88-.69 1.72-1.2 2.5l.8 2.44-1.41 1.41-2.44-.8c-.78.51-1.62.91-2.5 1.2L13 23h-2l-1.45-2.07c-.88-.29-1.72-.69-2.5-1.2l-2.44.8-1.41-1.41.8-2.44a8.99 8.99 0 0 1-1.2-2.5L2 13v-2l2.07-1.45c.29-.88.69-1.72 1.2-2.5l-.8-2.44 1.41-1.41 2.44.8c.78-.51 1.62-.91 2.5-1.2L11 1h2zm-1 3.08-1.09 1.56-.63.26a6.99 6.99 0 0 0-1.95.94l-.56.41-1.84-.6-1.41 1.41.6 1.84-.41.56a6.99 6.99 0 0 0-.94 1.95l-.26.63L2.55 12l1.56 1.09.26.63c.24.7.56 1.35.94 1.95l.41.56-.6 1.84 1.41 1.41 1.84-.6.56.41c.6.38 1.25.7 1.95.94l.63.26L11 20.92h2l1.09-1.56.63-.26c.7-.24 1.35-.56 1.95-.94l.56-.41 1.84.6 1.41-1.41-.6-1.84.41-.56c.38-.6.7-1.25.94-1.95l.26-.63 1.56-1.09-1.56-1.09-.26-.63a6.99 6.99 0 0 0-.94-1.95l-.41-.56.6-1.84-1.41-1.41-1.84.6-.56-.41a6.99 6.99 0 0 0-1.95-.94l-.63-.26L13 4.08h-2zM12 8a4 4 0 1 1 0 8 4 4 0 0 1 0-8zm0 2a2 2 0 1 0 0 4 2 2 0 0 0 0-4z"/>
                </svg>
              </button>
              <button
                class="mini-pill"
                title="Nouvelle conversation dans ce projet"
                @click.stop="store.createConversation(project.id)"
              >
                <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
                  <path d="M12 5v14M5 12h14" />
                </svg>
              </button>
            </div>
          </div>

          <div class="conv-list">
            <div
              v-for="conv in store.conversationsForProject(project.id)"
              :key="conv.id"
              class="conv-row"
              :class="{ 'is-selected': store.activeConversation?.id === conv.id }"
              @click="store.selectConversation(project.id, conv.id)"
            >
              <span class="conv-dot" />
              <span class="conv-title">{{ conv.title }}</span>
              <span
                v-if="conv.linesAdded > 0 || conv.linesRemoved > 0"
                class="conv-diff-badge"
              >
                +{{ conv.linesAdded }}
              </span>
              <button
                v-if="store.conversationsForProject(project.id).length > 1"
                class="conv-delete"
                title="Supprimer cette conversation"
                @click.stop="store.promptDeleteConversation(conv)"
              >
                ✕
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Stack health + Settings button -->
    <div class="le-menu-footer">
      <div class="stack-card">
        <div class="stack-row">
          <span class="stack-label">Ollama</span>
          <span
            class="stack-status"
            :class="store.stackStatus.ollamaRunning ? 'ok' : 'warn'"
          >
            {{ store.stackStatus.ollamaRunning ? "En ligne" : "Arrêté" }}
          </span>
        </div>
        <div class="stack-row">
          <span class="stack-label">Aider CLI</span>
          <span
            class="stack-status"
            :class="store.stackStatus.aiderInstalled ? 'ok' : 'muted'"
          >
            {{ store.stackStatus.aiderInstalled ? "Prêt" : "Non détecté" }}
          </span>
        </div>
        <button
          v-if="!store.stackStatus.ollamaRunning && store.stackStatus.ollamaInstalled"
          class="start-ollama-btn"
          @click="store.startOllama"
        >
          Démarrer Ollama
        </button>
      </div>

      <button class="mcp-menu-btn" @click="store.mcpModalOpen = true">
        <span class="mcp-menu-left">
          <span>🔌</span>
          <span>Serveurs MCP</span>
        </span>
        <span class="mcp-badge">{{ store.mcpServers.filter((s) => s.enabled).length }} actifs</span>
      </button>

      <button class="settings-btn" @click="store.settingsOpen = true">
        <span class="settings-btn-left">
          <svg
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <circle cx="12" cy="12" r="3" />
            <path
              d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.6.85 1 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"
            />
          </svg>
          <span>Paramètres</span>
        </span>
        <span class="settings-model">{{ store.settings.ollamaModel }}</span>
      </button>
    </div>
  </aside>
</template>

<style scoped>
.le-menu {
  flex-shrink: 0;
  min-width: 195px;
  max-width: 380px;
  height: 100%;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  background: transparent; /* Shares "la table" surface */
  padding: 6px 4px 6px 10px;
  overflow: hidden;
}

.le-menu-top {
  display: flex;
  flex-direction: column;
  min-height: 0;
  flex: 1;
}

.brand-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 4px 10px;
  gap: 8px;
}

.brand-identity {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.brand-icon-box {
  width: 28px;
  height: 28px;
  border-radius: 8px;
  background: var(--active-bg);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.brand-name {
  font-weight: 600;
  font-size: 13.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.icon-btn {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  border-radius: 8px;
  padding: 4px 8px;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}

.icon-btn:hover {
  border-color: var(--accent);
}

.tree-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-right: 4px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.project-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.project-header {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  padding: 5px 6px;
  border-radius: 8px;
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 600;
  transition: background-color 0.12s ease;
  overflow: hidden;
}

.project-header:hover {
  background: var(--hover-bg);
}

.project-header.is-active {
  color: var(--text-main);
}

.project-header-left {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
}

.project-folder-svg {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  color: var(--accent);
}

.project-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12.5px;
}

.project-actions {
  position: absolute;
  right: 0;
  top: 0;
  bottom: 0;
  display: flex;
  align-items: center;
  gap: 4px;
  padding-left: 28px;
  padding-right: 6px;
  background: linear-gradient(
    to right,
    transparent 0%,
    var(--table-bg) 24px,
    var(--table-bg) 100%
  );
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.18s ease;
}

.project-header:hover .project-actions {
  opacity: 1;
  pointer-events: auto;
}

.mini-pill {
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  color: var(--text-muted);
  border-radius: 6px;
  width: 22px;
  height: 22px;
  padding: 0;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: all 0.12s ease;
  line-height: 1;
}

.mini-pill:hover {
  background: var(--active-bg);
  color: var(--accent);
  border-color: var(--accent);
}

.mini-pill.pill-danger:hover {
  background: rgba(239, 68, 68, 0.12);
  color: #ef4444;
  border-color: rgba(239, 68, 68, 0.3);
}

.conv-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-left: 6px;
}

.conv-row {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 7px 8px;
  border-radius: 9px;
  cursor: pointer;
  color: var(--text-muted);
  font-size: 12.5px;
  transition: background-color 0.14s ease, color 0.14s ease;
}

.conv-row:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.conv-row.is-selected {
  background: var(--active-bg);
  color: var(--text-main);
  font-weight: 500;
}

.conv-dot {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background: var(--text-dim);
  flex-shrink: 0;
}

.conv-row.is-selected .conv-dot {
  background: var(--accent);
}

.conv-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.conv-diff-badge {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  color: var(--diff-add-bar);
  flex-shrink: 0;
}

.conv-delete {
  opacity: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 11px;
  cursor: pointer;
  padding: 0 2px;
}

.conv-row:hover .conv-delete {
  opacity: 0.75;
}

.conv-delete:hover {
  opacity: 1 !important;
  color: var(--diff-del-bar);
}

.le-menu-footer {
  padding-top: 8px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.stack-card {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  border-radius: 10px;
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12px;
}

.stack-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.stack-label {
  color: var(--text-muted);
}

.stack-status {
  font-weight: 500;
}

.stack-status.ok {
  color: var(--diff-add-bar);
}

.stack-status.warn {
  color: #f59e0b;
}

.stack-status.muted {
  color: var(--text-dim);
}

.start-ollama-btn {
  margin-top: 4px;
  border: 1px solid var(--border-subtle);
  background: var(--active-bg);
  color: var(--accent);
  border-radius: 7px;
  padding: 4px 8px;
  font-size: 12px;
  cursor: pointer;
}

.mcp-menu-btn {
  width: 100%;
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  border-radius: 10px;
  padding: 8px 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  cursor: pointer;
  font-size: 12.5px;
  color: var(--text-main);
  transition: all 0.14s ease;
  margin-bottom: 6px;
}

.mcp-menu-btn:hover {
  border-color: #10b981;
  background: rgba(16, 185, 129, 0.08);
}

.mcp-menu-left {
  display: flex;
  align-items: center;
  gap: 7px;
  font-weight: 500;
}

.mcp-badge {
  font-size: 11px;
  padding: 2px 6px;
  border-radius: 5px;
  background: rgba(16, 185, 129, 0.16);
  color: #10b981;
  font-weight: 500;
}

.settings-btn {
  width: 100%;
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  border-radius: 10px;
  padding: 8px 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  cursor: pointer;
  font-size: 12.5px;
  transition: border-color 0.14s ease;
}

.settings-btn:hover {
  border-color: var(--accent);
}

.settings-btn-left {
  display: flex;
  align-items: center;
  gap: 7px;
}

.settings-model {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 96px;
}
</style>
