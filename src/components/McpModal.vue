<script setup lang="ts">
import { ref } from "vue";
import { useStudioStore } from "../stores/studio";
import type { McpServerConfig, McpToolInfo } from "../types/studio";

const store = useStudioStore();

const showAddForm = ref(false);
const newServerName = ref("");
const newServerCommand = ref("");
const newServerArgs = ref("");
const testingId = ref<string | null>(null);
const testResults = ref<Record<string, { ok: boolean; tools?: McpToolInfo[]; error?: string }>>({});

function initNewServer(preset?: "sqlite" | "postgres") {
  if (preset === "sqlite") {
    newServerName.value = "SQLite Projet";
    newServerCommand.value = "npx";
    newServerArgs.value = "-y @modelcontextprotocol/server-sqlite --db-path ./project.db";
  } else if (preset === "postgres") {
    newServerName.value = "PostgreSQL";
    newServerCommand.value = "npx";
    newServerArgs.value = "-y @modelcontextprotocol/server-postgres postgresql://localhost/mydb";
  } else {
    newServerName.value = "";
    newServerCommand.value = "";
    newServerArgs.value = "";
  }
  showAddForm.value = true;
}

async function handleSaveNewServer() {
  if (!newServerName.value.trim() || !newServerCommand.value.trim()) return;

  const args = newServerArgs.value
    .trim()
    .split(/\s+/)
    .filter((a) => a.length > 0);

  const server: McpServerConfig = {
    id: `mcp-${Date.now()}`,
    name: newServerName.value.trim(),
    command: newServerCommand.value.trim(),
    args,
    enabled: true,
    projectId: store.activeProjectId,
  };

  await store.saveMcpServer(server);
  showAddForm.value = false;
}

async function testConnection(server: McpServerConfig) {
  testingId.value = server.id;
  try {
    const tools = await store.testMcpServer(server);
    testResults.value[server.id] = { ok: true, tools };
  } catch (err) {
    testResults.value[server.id] = { ok: false, error: String(err) };
  } finally {
    testingId.value = null;
  }
}

async function removeServer(id: string) {
  await store.deleteMcpServer(id);
  delete testResults.value[id];
}

async function toggleEnabled(server: McpServerConfig) {
  server.enabled = !server.enabled;
  await store.saveMcpServer(server);
}
</script>

<template>
  <div
    v-if="store.mcpModalOpen"
    class="modal-backdrop"
    @click.self="store.mcpModalOpen = false"
  >
    <div class="modal-card halo-hud">
      <div class="modal-header">
        <div class="header-left">
          <span class="mcp-icon">🔌</span>
          <h2>Serveurs MCP (Model Context Protocol)</h2>
        </div>
        <button class="close-x" @click="store.mcpModalOpen = false">✕</button>
      </div>

      <p class="mcp-desc">
        Connectez vos bases SQL (SQLite, Postgres, MySQL) ou outils locaux à l'IA via le standard MCP. L'IA pourra interroger les tables, exécuter des requêtes et manipuler les données.
      </p>

      <!-- Preset Add Buttons -->
      <div class="preset-row">
        <button class="preset-btn" @click="initNewServer('sqlite')">
          <span>🗄️</span> Ajouter SQLite MCP
        </button>
        <button class="preset-btn" @click="initNewServer('postgres')">
          <span>🐘</span> Ajouter PostgreSQL MCP
        </button>
        <button class="preset-btn custom" @click="initNewServer()">
          <span>➕</span> Serveur Personnalisé
        </button>
      </div>

      <!-- Add Form -->
      <div v-if="showAddForm" class="add-form halo-surface">
        <div class="form-title">Nouveau serveur MCP</div>
        <div class="form-row">
          <div class="form-field flex-1">
            <label>Nom du serveur</label>
            <input v-model="newServerName" placeholder="Ex: SQLite Local" />
          </div>
          <div class="form-field flex-1">
            <label>Commande</label>
            <input v-model="newServerCommand" placeholder="Ex: npx, uvx, python" />
          </div>
        </div>
        <div class="form-field">
          <label>Arguments (séparés par des espaces)</label>
          <input
            v-model="newServerArgs"
            placeholder="Ex: -y @modelcontextprotocol/server-sqlite --db-path ./project.db"
          />
        </div>
        <div class="form-actions">
          <button class="btn-cancel" @click="showAddForm = false">Annuler</button>
          <button
            class="btn-save"
            :disabled="!newServerName.trim() || !newServerCommand.trim()"
            @click="handleSaveNewServer"
          >
            Enregistrer le serveur
          </button>
        </div>
      </div>

      <!-- Existing Servers List -->
      <div class="servers-list custom-scroll">
        <div v-if="store.mcpServers.length === 0" class="servers-empty">
          Aucun serveur MCP configuré pour le moment. Cliquez sur un preset ci-dessus pour connecter une base SQL.
        </div>

        <div
          v-for="server in store.mcpServers"
          :key="server.id"
          class="server-card"
          :class="{ disabled: !server.enabled }"
        >
          <div class="server-top">
            <div class="server-meta">
              <span class="server-title">{{ server.name }}</span>
              <code class="server-cmd">{{ server.command }} {{ server.args.join(" ") }}</code>
            </div>

            <div class="server-actions">
              <button
                class="test-btn"
                :disabled="testingId === server.id"
                @click="testConnection(server)"
              >
                {{ testingId === server.id ? "Test en cours..." : "Tester" }}
              </button>
              <button
                class="toggle-btn"
                :class="{ on: server.enabled }"
                @click="toggleEnabled(server)"
              >
                {{ server.enabled ? "Actif" : "Inactif" }}
              </button>
              <button class="delete-btn" title="Supprimer" @click="removeServer(server.id)">
                ✕
              </button>
            </div>
          </div>

          <!-- Test result output -->
          <div
            v-if="testResults[server.id]"
            class="test-box"
            :class="{ success: testResults[server.id].ok, error: !testResults[server.id].ok }"
          >
            <div v-if="testResults[server.id].ok">
              <span class="ok-title">✅ Connexion réussie ! Outils disponibles ({{ testResults[server.id].tools?.length }}) :</span>
              <ul class="tools-list">
                <li v-for="t in testResults[server.id].tools" :key="t.name">
                  <code>{{ t.name }}</code> — {{ t.description }}
                </li>
              </ul>
            </div>
            <div v-else>
              <span class="err-title">❌ Échec de la connexion :</span>
              <p class="err-detail">{{ testResults[server.id].error }}</p>
            </div>
          </div>
        </div>
      </div>

      <div class="modal-footer">
        <button class="btn-primary" @click="store.mcpModalOpen = false">
          Terminer
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
  background: rgba(0, 0, 0, 0.55);
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
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  overflow: hidden;
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.mcp-icon {
  font-size: 18px;
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

.mcp-desc {
  margin: 0;
  font-size: 12.5px;
  color: var(--text-muted);
  line-height: 1.5;
}

.preset-row {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.preset-btn {
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  color: var(--text-main);
  border-radius: 8px;
  padding: 6px 12px;
  font-size: 12px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  transition: all 0.14s ease;
}

.preset-btn:hover {
  border-color: var(--accent);
  background: var(--hover-bg);
}

.preset-btn.custom {
  color: var(--accent);
}

.add-form {
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  border-radius: 10px;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.form-title {
  font-weight: 600;
  font-size: 13px;
  color: var(--accent);
}

.form-row {
  display: flex;
  gap: 10px;
}

.form-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.form-field.flex-1 {
  flex: 1;
}

.form-field label {
  font-size: 11px;
  color: var(--text-dim);
}

.form-field input {
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  border-radius: 6px;
  padding: 6px 9px;
  font-size: 12px;
  outline: none;
}

.form-field input:focus {
  border-color: var(--accent);
}

.form-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.btn-cancel {
  border: none;
  background: transparent;
  color: var(--text-muted);
  padding: 5px 10px;
  font-size: 12px;
  cursor: pointer;
}

.btn-save {
  border: none;
  background: var(--accent);
  color: #fff;
  border-radius: 6px;
  padding: 5px 12px;
  font-size: 12px;
  cursor: pointer;
}

.btn-save:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.servers-list {
  flex: 1;
  max-height: 280px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.servers-empty {
  padding: 18px;
  text-align: center;
  font-size: 12px;
  color: var(--text-dim);
  border: 1px dashed var(--border-subtle);
  border-radius: 8px;
}

.server-card {
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  border-radius: 9px;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.server-card.disabled {
  opacity: 0.6;
}

.server-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.server-meta {
  display: flex;
  flex-direction: column;
  min-width: 0;
  gap: 2px;
}

.server-title {
  font-size: 13px;
  font-weight: 600;
}

.server-cmd {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.server-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.test-btn {
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  color: var(--text-muted);
  border-radius: 6px;
  padding: 4px 8px;
  font-size: 11px;
  cursor: pointer;
}

.test-btn:hover {
  border-color: var(--accent);
  color: var(--text-main);
}

.toggle-btn {
  border: 1px solid var(--border-subtle);
  background: transparent;
  color: var(--text-dim);
  border-radius: 6px;
  padding: 4px 8px;
  font-size: 11px;
  cursor: pointer;
}

.toggle-btn.on {
  background: var(--active-bg);
  color: var(--accent);
  border-color: var(--accent);
}

.delete-btn {
  border: none;
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  padding: 4px 6px;
  font-size: 12px;
}

.delete-btn:hover {
  color: var(--diff-del-bar);
}

.test-box {
  padding: 8px 10px;
  border-radius: 7px;
  font-size: 11.5px;
}

.test-box.success {
  background: rgba(16, 185, 129, 0.1);
  border: 1px solid rgba(16, 185, 129, 0.25);
  color: #10b981;
}

.test-box.error {
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid rgba(239, 68, 68, 0.25);
  color: #ef4444;
}

.ok-title,
.err-title {
  font-weight: 600;
}

.tools-list {
  margin: 4px 0 0;
  padding-left: 18px;
  color: var(--text-muted);
}

.tools-list code {
  color: var(--accent);
}

.err-detail {
  margin: 3px 0 0;
  color: var(--text-muted);
}

.modal-footer {
  display: flex;
  justify-content: flex-end;
  padding-top: 6px;
}

.btn-primary {
  border: none;
  background: var(--accent);
  color: #fff;
  border-radius: 8px;
  padding: 7px 16px;
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
}
</style>
