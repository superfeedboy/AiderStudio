<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import {
  defaultAgentRouting,
  defaultProvidersList,
  useStudioStore,
} from "../stores/studio";
import type {
  AgentModelSlot,
  AgentRoleKey,
  AgentRoutingConfig,
  AiderEditFormat,
  AppSettings,
  ProviderConfig,
  ThemePreference,
} from "../types/studio";

const store = useStudioStore();

function cloneProviders(list?: ProviderConfig[]): ProviderConfig[] {
  const source = list && list.length > 0 ? list : defaultProvidersList();
  return source.map((p) => ({
    ...p,
    availableModels: [...(p.availableModels ?? [])],
  }));
}

function cloneRouting(routing?: AgentRoutingConfig): AgentRoutingConfig {
  const base = defaultAgentRouting();
  if (!routing) return base;
  return {
    architect: { ...base.architect, ...(routing.architect ?? {}) },
    cartographer: { ...base.cartographer, ...(routing.cartographer ?? {}) },
    designer: { ...base.designer, ...(routing.designer ?? {}) },
    coder: { ...base.coder, ...(routing.coder ?? {}) },
    refactoring: { ...base.refactoring, ...(routing.refactoring ?? {}) },
    tester: { ...base.tester, ...(routing.tester ?? {}) },
    synthesizer: { ...base.synthesizer, ...(routing.synthesizer ?? {}) },
  };
}

const draft = reactive<AppSettings>({
  ...store.settings,
  aiderEditFormat: store.settings.aiderEditFormat || "auto",
  activeTarget: store.settings.activeTarget || "agent",
  providers: cloneProviders(store.settings.providers),
  agentRouting: cloneRouting(store.settings.agentRouting),
});

const revealedKeys = ref<Record<string, boolean>>({});
const validatingProvider = ref<Record<string, boolean>>({});
const providerFeedback = ref<
  Record<string, { ok?: string; error?: string }>
>({});

const expandedAgentRole = ref<AgentRoleKey | null>(null);
const agentModelSearch = ref("");

const providerBrandColors: Record<string, string> = {
  ollama: "#10b981",
  gemini: "#3b82f6",
  openai: "#22c55e",
  anthropic: "#f59e0b",
  deepseek: "#06b6d4",
  openrouter: "#a855f7",
  mistral: "#f97316",
};

const providerShortNames: Record<string, string> = {
  ollama: "Ollama",
  gemini: "Gemini",
  openai: "OpenAI",
  anthropic: "Claude",
  deepseek: "DeepSeek",
  openrouter: "OpenRouter",
  mistral: "Mistral",
};

function agentProviderColor(providerId: string): string {
  return providerBrandColors[providerId] || "#94a3b8";
}

function agentProviderShortName(providerId: string): string {
  if (providerShortNames[providerId]) return providerShortNames[providerId];
  const cfg = (draft.providers ?? []).find((p) => p.id === providerId);
  return cfg?.name || providerId;
}

watch(
  () => store.settingsOpen,
  (open) => {
    expandedAgentRole.value = null;
    agentModelSearch.value = "";
    if (open) {
      Object.assign(draft, {
        ...store.settings,
        aiderEditFormat: store.settings.aiderEditFormat || "auto",
        activeTarget: store.settings.activeTarget || "agent",
        providers: cloneProviders(store.settings.providers),
        agentRouting: cloneRouting(store.settings.agentRouting),
      });
      providerFeedback.value = {};
    }
  }
);

async function save() {
  await store.saveSettings({
    ...draft,
    aiderEditFormat: draft.aiderEditFormat || "auto",
    activeTarget: draft.activeTarget || "agent",
    providers: cloneProviders(draft.providers),
    agentRouting: cloneRouting(draft.agentRouting),
  });
  store.settingsOpen = false;
}

function onProviderKeyInput(p: ProviderConfig) {
  p.validated = false;
  if (providerFeedback.value[p.id]) {
    const next = { ...providerFeedback.value };
    delete next[p.id];
    providerFeedback.value = next;
  }
}

async function validateProvider(p: ProviderConfig) {
  const key = p.apiKey.trim();
  if (!key || validatingProvider.value[p.id]) return;

  validatingProvider.value = { ...validatingProvider.value, [p.id]: true };
  providerFeedback.value = {
    ...providerFeedback.value,
    [p.id]: {},
  };

  try {
    const updated = await store.validateProviderKey(
      p.id,
      key,
      p.baseUrl ?? null
    );
    p.apiKey = updated.apiKey;
    p.validated = updated.validated;
    p.availableModels = [...(updated.availableModels ?? [])];
    providerFeedback.value = {
      ...providerFeedback.value,
      [p.id]: {
        ok: `Clé valide · ${p.availableModels.length} modèle(s) détecté(s)`,
      },
    };
  } catch (err: any) {
    p.validated = false;
    const msg =
      typeof err === "string"
        ? err
        : err?.message || "Validation impossible";
    providerFeedback.value = {
      ...providerFeedback.value,
      [p.id]: { error: msg },
    };
  } finally {
    validatingProvider.value = { ...validatingProvider.value, [p.id]: false };
  }
}

const validatedProvidersInDraft = computed<ProviderConfig[]>(() =>
  (draft.providers ?? []).filter((p) => p.validated && p.apiKey.trim().length > 0)
);

const ollamaModelList = computed<string[]>(() => {
  const set = new Set<string>();
  if (draft.ollamaModel.trim()) set.add(draft.ollamaModel.trim());
  for (const m of store.stackStatus.availableModels) {
    if (m.trim()) set.add(m.trim());
  }
  set.add("qwen2.5-coder:3b");
  set.add("qwen2.5-coder:7b");
  set.add("qwen2.5-coder:1.5b");
  return Array.from(set);
});

function modelsForProvider(providerId: string): string[] {
  if (providerId === "ollama") return ollamaModelList.value;
  const prov = (draft.providers ?? []).find((p) => p.id === providerId);
  return prov?.availableModels ?? [];
}

function getAgentSlot(role: AgentRoleKey): AgentModelSlot {
  if (!draft.agentRouting) {
    draft.agentRouting = defaultAgentRouting();
  }
  return draft.agentRouting[role];
}

function onAgentProviderChange(role: AgentRoleKey, nextProv: string) {
  const slot = getAgentSlot(role);
  slot.provider = nextProv;
  agentModelSearch.value = "";
  if (nextProv === "ollama") {
    slot.model = "";
  } else {
    const models = modelsForProvider(nextProv);
    slot.model = models[0] ?? "";
  }
}

function toggleAgentPicker(role: AgentRoleKey) {
  agentModelSearch.value = "";
  expandedAgentRole.value = expandedAgentRole.value === role ? null : role;
}

function filteredModelsForAgent(providerId: string): string[] {
  const list = modelsForProvider(providerId);
  const q = agentModelSearch.value.trim().toLowerCase();
  if (!q) return list;
  return list.filter((m) => m.toLowerCase().includes(q));
}

function selectAgentModel(role: AgentRoleKey, model: string) {
  const slot = getAgentSlot(role);
  slot.model = model;
  expandedAgentRole.value = null;
  agentModelSearch.value = "";
}

function formatLimitLabel(limit: number): string {
  if (limit >= 1_000_000) return `${(limit / 1_000_000).toFixed(limit % 1_000_000 === 0 ? 0 : 1)}M`;
  if (limit >= 1000) return `${Math.round(limit / 1000)}K`;
  return String(limit);
}

const tabs: { id: "general" | "providers" | "agents"; label: string }[] = [
  { id: "general", label: "Général" },
  { id: "providers", label: "Fournisseurs" },
  { id: "agents", label: "Agents" },
];

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

const agentRoles: {
  key: AgentRoleKey;
  label: string;
  desc: string;
  color: string;
}[] = [
  {
    key: "architect",
    label: "Architecte & Orchestrateur",
    desc: "Analyse de la demande, routage des rôles et spécifications techniques",
    color: "#8b5cf6",
  },
  {
    key: "cartographer",
    label: "Cartographe",
    desc: "Ciblage des fichiers pertinents et réduction du bruit de contexte",
    color: "#06b6d4",
  },
  {
    key: "designer",
    label: "Designer UI/UX",
    desc: "Interfaces modernes, CSS global, mise en page et responsive",
    color: "#ec4899",
  },
  {
    key: "coder",
    label: "Codeur",
    desc: "Implémentation de fonctionnalités, correctifs et intégrations",
    color: "#3b82f6",
  },
  {
    key: "refactoring",
    label: "Refactoring",
    desc: "Nettoyage, découpage, typage strict et optimisation",
    color: "#f59e0b",
  },
  {
    key: "tester",
    label: "Testeur",
    desc: "Génération de tests unitaires/d'intégration et vérifications",
    color: "#10b981",
  },
  {
    key: "synthesizer",
    label: "Synthétiseur",
    desc: "Résumé concis du travail accompli en fin de tour",
    color: "#94a3b8",
  },
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

      <!-- Segmented 3-Tab Switcher -->
      <div class="settings-tabs">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          class="settings-tab-btn"
          :class="{ active: store.settingsActiveTab === tab.id }"
          @click="store.settingsActiveTab = tab.id"
        >
          <span>{{ tab.label }}</span>
          <span
            v-if="tab.id === 'providers' && validatedProvidersInDraft.length > 0"
            class="tab-count"
          >
            {{ validatedProvidersInDraft.length }}
          </span>
        </button>
      </div>

      <!-- TAB 1: GÉNÉRAL -->
      <div
        v-if="store.settingsActiveTab === 'general'"
        class="modal-body custom-scroll"
      >
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
          <label>Modèle Ollama par défaut (local / repli)</label>
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
            <label>Fenêtre de contexte cible Ollama (tokens)</label>
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
            <strong>Auto (8K)</strong> : utilise <code>whole</code> pour les fichiers ≤ 250 lignes sur Ollama (≤ 120 lignes sur cloud) et bascule en <code>diff</code> au-delà.
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

        <div class="stack-probe-section">
          <label class="section-label">Diagnostic de la stack locale</label>
          <div class="probe-cards">
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

      <!-- TAB 2: FOURNISSEURS API EXTERNES -->
      <div
        v-else-if="store.settingsActiveTab === 'providers'"
        class="modal-body custom-scroll"
      >
        <p class="tab-intro">
          Ajoutez et validez vos clés API pour activer les fournisseurs dans le sélecteur de prompt et les assigner à vos sous-agents.
        </p>

        <div class="provider-list">
          <div
            v-for="p in draft.providers"
            :key="p.id"
            class="provider-card"
            :class="{
              'is-validated': p.validated,
              'is-error': Boolean(providerFeedback[p.id]?.error),
            }"
          >
            <div class="provider-card-header">
              <div class="provider-title-wrap">
                <span class="provider-name">{{ p.name }}</span>
                <span class="provider-id-tag">{{ p.id }}</span>
              </div>
              <span
                class="probe-status-tag"
                :class="
                  providerFeedback[p.id]?.error
                    ? 'danger'
                    : p.validated
                      ? 'ok'
                      : 'muted'
                "
              >
                {{
                  providerFeedback[p.id]?.error
                    ? "Erreur"
                    : p.validated
                      ? "Validée"
                      : "Non configurée"
                }}
              </span>
            </div>

            <div class="provider-key-row">
              <div class="key-input-wrap">
                <input
                  v-model="p.apiKey"
                  :type="revealedKeys[p.id] ? 'text' : 'password'"
                  :placeholder="`Clé API ${p.name}...`"
                  autocomplete="off"
                  @input="onProviderKeyInput(p)"
                />
                <button
                  type="button"
                  class="reveal-btn"
                  :title="revealedKeys[p.id] ? 'Masquer' : 'Afficher'"
                  @click="revealedKeys[p.id] = !revealedKeys[p.id]"
                >
                  {{ revealedKeys[p.id] ? "Masquer" : "Voir" }}
                </button>
              </div>
              <button
                type="button"
                class="validate-btn"
                :disabled="!p.apiKey.trim() || validatingProvider[p.id]"
                @click="validateProvider(p)"
              >
                {{ validatingProvider[p.id] ? "Test..." : "Valider" }}
              </button>
            </div>

            <div
              v-if="providerFeedback[p.id]?.ok"
              class="provider-msg is-ok"
            >
              ✓ {{ providerFeedback[p.id].ok }}
            </div>
            <div
              v-else-if="providerFeedback[p.id]?.error"
              class="provider-msg is-err"
            >
              ⚠ {{ providerFeedback[p.id].error }}
            </div>

            <div class="provider-limit-row">
              <span class="limit-label">Quota journalier HUD :</span>
              <div class="limit-presets">
                <button
                  v-for="lim in [100000, 500000, 1000000, 5000000]"
                  :key="lim"
                  type="button"
                  class="preset-btn"
                  :class="{ active: p.dailyTokenLimit === lim }"
                  @click="p.dailyTokenLimit = lim"
                >
                  {{ formatLimitLabel(lim) }}
                </button>
              </div>
              <input
                v-model.number="p.dailyTokenLimit"
                type="number"
                min="10000"
                step="50000"
                class="limit-input"
              />
            </div>

            <div
              v-if="p.validated && p.availableModels.length > 0"
              class="provider-models-preview"
            >
              <span
                v-for="mdl in p.availableModels.slice(0, 8)"
                :key="mdl"
                class="model-preview-chip"
              >
                {{ mdl }}
              </span>
              <span
                v-if="p.availableModels.length > 8"
                class="model-preview-more"
              >
                +{{ p.availableModels.length - 8 }} autres
              </span>
            </div>
          </div>
        </div>
      </div>

      <!-- TAB 3: AGENTS -->
      <div
        v-else
        class="modal-body custom-scroll"
      >
        <p class="tab-intro">
          Assignez un fournisseur et un modèle précis à chaque sous-agent pour le mode <strong>Agent</strong>. Cliquez sur un sous-agent pour personnaliser son moteur.
        </p>

        <div class="agents-list">
          <div
            v-for="role in agentRoles"
            :key="role.key"
            class="agent-row-card"
            :class="{ 'is-expanded': expandedAgentRole === role.key }"
            :style="{ '--role-color': role.color }"
          >
            <div class="agent-row-header">
              <div class="agent-role-info">
                <div class="agent-role-title">
                  <span
                    class="role-dot"
                    :style="{ backgroundColor: role.color }"
                  />
                  <strong>{{ role.label }}</strong>
                </div>
                <span class="agent-role-desc">{{ role.desc }}</span>
              </div>

              <button
                type="button"
                class="agent-picker-trigger"
                :class="{ active: expandedAgentRole === role.key }"
                @click="toggleAgentPicker(role.key)"
              >
                <span
                  class="agent-prov-pill"
                  :style="{ '--prov-color': agentProviderColor(getAgentSlot(role.key).provider) }"
                >
                  <span
                    class="agent-prov-dot"
                    :style="{ backgroundColor: agentProviderColor(getAgentSlot(role.key).provider) }"
                  />
                  <span>{{ agentProviderShortName(getAgentSlot(role.key).provider) }}</span>
                </span>
                <span class="agent-trigger-model">
                  {{
                    getAgentSlot(role.key).provider === "ollama" && !getAgentSlot(role.key).model
                      ? `Défaut (${draft.ollamaModel})`
                      : getAgentSlot(role.key).model || "Choisir..."
                  }}
                </span>
                <svg
                  class="agent-chevron"
                  :class="{ rotated: expandedAgentRole === role.key }"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path d="m6 9 6 6 6-6" />
                </svg>
              </button>
            </div>

            <!-- Inline designed accordion panel -->
            <div
              v-if="expandedAgentRole === role.key"
              class="agent-picker-panel"
            >
              <div class="picker-section-label">Fournisseur</div>
              <div class="agent-provider-pills">
                <button
                  type="button"
                  class="provider-choice-pill"
                  :class="{ active: getAgentSlot(role.key).provider === 'ollama' }"
                  @click="onAgentProviderChange(role.key, 'ollama')"
                >
                  <span
                    class="agent-prov-dot"
                    :style="{ backgroundColor: agentProviderColor('ollama') }"
                  />
                  <span>Ollama (Local)</span>
                </button>
                <button
                  v-for="vp in validatedProvidersInDraft"
                  :key="vp.id"
                  type="button"
                  class="provider-choice-pill"
                  :class="{ active: getAgentSlot(role.key).provider === vp.id }"
                  @click="onAgentProviderChange(role.key, vp.id)"
                >
                  <span
                    class="agent-prov-dot"
                    :style="{ backgroundColor: agentProviderColor(vp.id) }"
                  />
                  <span>{{ vp.name }}</span>
                </button>
              </div>

              <div class="picker-section-row">
                <span class="picker-section-label">Modèle</span>
                <input
                  v-if="modelsForProvider(getAgentSlot(role.key).provider).length > 5"
                  v-model="agentModelSearch"
                  type="text"
                  class="agent-search-input"
                  placeholder="Filtrer les modèles..."
                />
              </div>

              <div class="agent-model-grid custom-scroll">
                <button
                  v-if="getAgentSlot(role.key).provider === 'ollama'"
                  type="button"
                  class="agent-model-chip"
                  :class="{ active: getAgentSlot(role.key).model === '' }"
                  @click="selectAgentModel(role.key, '')"
                >
                  <span>Par défaut ({{ draft.ollamaModel }})</span>
                  <span
                    v-if="getAgentSlot(role.key).model === ''"
                    class="chip-check"
                  >
                    ✓
                  </span>
                </button>
                <button
                  v-for="mdl in filteredModelsForAgent(getAgentSlot(role.key).provider)"
                  :key="mdl"
                  type="button"
                  class="agent-model-chip"
                  :class="{ active: getAgentSlot(role.key).model === mdl }"
                  @click="selectAgentModel(role.key, mdl)"
                >
                  <span class="chip-model-name">{{ mdl }}</span>
                  <span
                    v-if="getAgentSlot(role.key).model === mdl"
                    class="chip-check"
                  >
                    ✓
                  </span>
                </button>
              </div>

              <div class="agent-custom-row">
                <input
                  v-model="getAgentSlot(role.key).model"
                  type="text"
                  class="agent-custom-input"
                  placeholder="Ou saisir un ID de modèle personnalisé..."
                />
              </div>
            </div>
          </div>
        </div>
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
  max-width: 640px;
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
  font-size: 14px;
}

.settings-tabs {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px;
  padding: 3px;
  border-radius: 10px;
  background: var(--discussion-bg);
  border: 1px solid var(--border-subtle);
}

.settings-tab-btn {
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 7px;
  padding: 6px 10px;
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  transition: all 0.14s ease;
}

.settings-tab-btn:hover {
  color: var(--text-main);
}

.settings-tab-btn.active {
  background: var(--active-bg);
  color: var(--accent);
  font-weight: 600;
}

.tab-count {
  font-size: 10px;
  font-weight: 700;
  padding: 0 5px;
  border-radius: 999px;
  background: rgba(16, 185, 129, 0.2);
  color: #10b981;
}

.tab-intro {
  margin: 0;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.45;
}

.modal-body {
  display: flex;
  flex-direction: column;
  gap: 13px;
  max-height: 66vh;
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
  color: var(--text-main);
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
  color: var(--text-main);
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
  color: var(--text-main);
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

.probe-status-tag.muted {
  background: var(--hover-bg);
  color: var(--text-dim);
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

/* Fournisseurs Tab */
.provider-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.provider-card {
  padding: 11px 12px;
  border-radius: 10px;
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.provider-card.is-validated {
  border-color: rgba(16, 185, 129, 0.35);
}

.provider-card.is-error {
  border-color: rgba(239, 68, 68, 0.4);
}

.provider-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.provider-title-wrap {
  display: flex;
  align-items: center;
  gap: 7px;
}

.provider-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
}

.provider-id-tag {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 10px;
  color: var(--text-dim);
  background: var(--discussion-bg);
  padding: 1px 6px;
  border-radius: 4px;
  border: 1px solid var(--border-subtle);
}

.provider-key-row {
  display: flex;
  gap: 8px;
  align-items: center;
}

.key-input-wrap {
  flex: 1;
  display: flex;
  align-items: center;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  border-radius: 8px;
  padding-right: 4px;
}

.key-input-wrap:focus-within {
  border-color: var(--accent);
}

.key-input-wrap input {
  flex: 1;
  border: none;
  background: transparent;
  color: var(--text-main);
  padding: 7px 9px;
  font-size: 12.5px;
  outline: none;
  min-width: 0;
}

.reveal-btn {
  border: none;
  background: transparent;
  color: var(--text-dim);
  font-size: 11px;
  padding: 3px 6px;
  border-radius: 5px;
  cursor: pointer;
}

.reveal-btn:hover {
  color: var(--text-main);
  background: var(--hover-bg);
}

.validate-btn {
  border: none;
  background: var(--accent);
  color: #ffffff;
  border-radius: 8px;
  padding: 7px 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.validate-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.provider-msg {
  font-size: 11px;
  line-height: 1.35;
}

.provider-msg.is-ok {
  color: #10b981;
}

.provider-msg.is-err {
  color: #ef4444;
}

.provider-limit-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}

.limit-label {
  font-size: 11px;
  color: var(--text-muted);
}

.limit-presets {
  display: flex;
  gap: 4px;
}

.limit-input {
  width: 96px;
  margin-left: auto;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-main);
  border-radius: 6px;
  padding: 3px 7px;
  font-size: 11.5px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.provider-models-preview {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.model-preview-chip {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--discussion-bg);
  border: 1px solid var(--border-subtle);
  color: var(--text-muted);
}

.model-preview-more {
  font-size: 10px;
  color: var(--text-dim);
  padding: 1px 4px;
}

/* Agents Tab */
.agents-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.agent-row-card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--border-subtle);
  border-left: 3px solid var(--role-color, var(--accent));
  background: var(--hover-bg);
  transition: border-color 0.14s ease, background-color 0.14s ease;
}

.agent-row-card.is-expanded {
  border-color: var(--accent);
  border-left-color: var(--role-color, var(--accent));
  background: var(--surface-elevated);
}

.agent-row-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.agent-role-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.agent-role-title {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12.5px;
  color: var(--text-main);
}

.role-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.agent-role-desc {
  font-size: 11px;
  color: var(--text-dim);
  line-height: 1.3;
}

.agent-picker-trigger {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-main);
  border-radius: 9px;
  padding: 5px 9px 5px 6px;
  cursor: pointer;
  max-width: 255px;
  flex-shrink: 0;
  transition: all 0.14s ease;
}

.agent-picker-trigger:hover,
.agent-picker-trigger.active {
  border-color: var(--accent);
  background: var(--active-bg);
}

.agent-prov-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 7px;
  border-radius: 6px;
  background: var(--hover-bg);
  border: 1px solid var(--border-subtle);
  font-size: 10.5px;
  font-weight: 600;
  color: var(--text-main);
  flex-shrink: 0;
}

.agent-prov-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.agent-trigger-model {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.agent-chevron {
  width: 13px;
  height: 13px;
  color: var(--text-dim);
  flex-shrink: 0;
  transition: transform 0.15s ease;
}

.agent-chevron.rotated {
  transform: rotate(180deg);
}

.agent-picker-panel {
  border-top: 1px solid var(--border-subtle);
  padding-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.picker-section-label {
  font-size: 10.5px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-dim);
}

.picker-section-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.agent-provider-pills {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.provider-choice-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-muted);
  border-radius: 8px;
  padding: 5px 10px;
  font-size: 11.5px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.12s ease;
}

.provider-choice-pill:hover {
  border-color: var(--accent);
  color: var(--text-main);
}

.provider-choice-pill.active {
  background: var(--active-bg);
  border-color: var(--accent);
  color: var(--accent);
  font-weight: 600;
}

.agent-search-input {
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-main);
  border-radius: 6px;
  padding: 3px 8px;
  font-size: 11px;
  outline: none;
  width: 180px;
}

.agent-search-input:focus {
  border-color: var(--accent);
}

.agent-model-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  max-height: 132px;
  overflow-y: auto;
  padding: 2px 0;
}

.agent-model-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-muted);
  border-radius: 7px;
  padding: 5px 9px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  cursor: pointer;
  max-width: 100%;
  transition: all 0.12s ease;
}

.agent-model-chip:hover {
  border-color: var(--accent);
  color: var(--text-main);
  background: var(--hover-bg);
}

.agent-model-chip.active {
  background: var(--active-bg);
  border-color: var(--accent);
  color: var(--accent);
  font-weight: 600;
}

.chip-model-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip-check {
  color: var(--accent);
  font-weight: 700;
}

.agent-custom-row {
  display: flex;
}

.agent-custom-input {
  flex: 1;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-main);
  border-radius: 7px;
  padding: 5px 9px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  outline: none;
}

.agent-custom-input:focus {
  border-color: var(--accent);
}
</style>
