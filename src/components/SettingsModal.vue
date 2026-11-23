<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
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
  StackStatus,
  ThemePreference,
} from "../types/studio";

const store = useStudioStore();

function cloneProviders(list?: ProviderConfig[]): ProviderConfig[] {
  const source = list && list.length > 0 ? list : defaultProvidersList();
  return source.map((p) => ({
    ...p,
    availableModels: [...(p.availableModels ?? [])],
    color: p.color || null,
  }));
}

function cloneSlot(slot?: AgentModelSlot): AgentModelSlot {
  return {
    provider: slot?.provider || "ollama",
    model: slot?.model || "",
    fallbacks: (slot?.fallbacks ?? []).map((f) => ({
      provider: f.provider,
      model: f.model,
    })),
  };
}

function cloneRouting(routing?: AgentRoutingConfig): AgentRoutingConfig {
  const base = defaultAgentRouting();
  if (!routing) return base;
  return {
    architect: cloneSlot(routing.architect ?? routing.synthesizer ?? base.architect),
    coder: cloneSlot(routing.coder ?? routing.designer ?? routing.refactoring ?? base.coder),
    reviewer: cloneSlot(routing.reviewer ?? routing.tester ?? base.reviewer),
  };
}

const draft = reactive<AppSettings>({
  ...store.settings,
  aiderEditFormat: store.settings.aiderEditFormat || "auto",
  activeTarget: store.settings.activeTarget || "agent",
  providers: cloneProviders(store.settings.providers),
  agentRouting: cloneRouting(store.settings.agentRouting),
  hudContextView: store.settings.hudContextView || "agent",
  hudShowTokenUsage: store.hudShowTokenUsageEffective,
  hudShowLocalResources: store.hudShowLocalResourcesEffective,
  hudShowGlobalDiff: store.hudShowGlobalDiffEffective,
  hudAutoArtifactEvolutionReport: store.hudAutoArtifactEvolutionReportEffective,
});

const providersSubTab = ref<"local" | "external">("local");

const revealedKeys = ref<Record<string, boolean>>({});
const validatingProvider = ref<Record<string, boolean>>({});
const providerFeedback = ref<Record<string, { ok?: string; error?: string }>>({});

const testingOllama = ref(false);
const ollamaFeedback = ref<{ ok?: string; error?: string }>({});

const expandedProviderId = ref<string | null>(null);
const expandedAgentRole = ref<AgentRoleKey | null>(null);
const agentModelSearch = ref("");
const noCostPresetFeedback = ref("");

const providerBrandColors: Record<string, string> = {
  ollama: "#10b981",
  gemini: "#3b82f6",
  openai: "#22c55e",
  anthropic: "#f59e0b",
  deepseek: "#06b6d4",
  openrouter: "#a855f7",
  groq: "#f43f5e",
  cerebras: "#ec4899",
  mistral: "#f97316",
};

const providerShortNames: Record<string, string> = {
  ollama: "Ollama",
  gemini: "Gemini",
  openai: "OpenAI",
  anthropic: "Claude",
  deepseek: "DeepSeek",
  openrouter: "OpenRouter",
  groq: "Groq",
  cerebras: "Cerebras",
  mistral: "Mistral",
};

const freeTierProviderIds = new Set(["gemini", "openrouter", "groq", "cerebras"]);

function getProviderColor(provIdOrConfig: string | ProviderConfig): string {
  if (typeof provIdOrConfig === "object") {
    return provIdOrConfig.color || providerBrandColors[provIdOrConfig.id] || "#94a3b8";
  }
  const cfg = (draft.providers ?? []).find((p) => p.id === provIdOrConfig);
  if (cfg?.color) return cfg.color;
  return providerBrandColors[provIdOrConfig] || "#94a3b8";
}

function onProviderColorChange(p: ProviderConfig, ev: Event) {
  const target = ev.target as HTMLInputElement;
  p.color = target.value;
}

function agentProviderColor(providerId: string): string {
  return getProviderColor(providerId);
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
    expandedProviderId.value = null;
    agentModelSearch.value = "";
    noCostPresetFeedback.value = "";
    ollamaFeedback.value = {};
    if (open) {
      Object.assign(draft, {
        ...store.settings,
        aiderEditFormat: store.settings.aiderEditFormat || "auto",
        activeTarget: store.settings.activeTarget || "agent",
        providers: cloneProviders(store.settings.providers),
        agentRouting: cloneRouting(store.settings.agentRouting),
        hudContextView: store.settings.hudContextView || "agent",
        hudShowTokenUsage: store.hudShowTokenUsageEffective,
        hudShowLocalResources: store.hudShowLocalResourcesEffective,
        hudShowGlobalDiff: store.hudShowGlobalDiffEffective,
        hudAutoArtifactEvolutionReport: store.hudAutoArtifactEvolutionReportEffective,
      });
      providerFeedback.value = {};
    }
  }
);

async function onSelectTheme(t: ThemePreference) {
  draft.theme = t;
  store.settings.theme = t;
  await store.setTheme(t);
  await store.saveSettings({ ...draft });
}

async function onToggleHudSetting<K extends keyof AppSettings>(key: K, value: AppSettings[K]) {
  (draft as Record<string, unknown>)[key] = value;
  (store.settings as Record<string, unknown>)[key] = value;
  await store.saveSettings({ ...draft });
  if (key === "hudShowLocalResources" && value === true) {
    void store.refreshTelemetry();
  }
}

async function save() {
  await store.saveSettings({
    ...draft,
    aiderEditFormat: draft.aiderEditFormat || "auto",
    activeTarget: draft.activeTarget || "agent",
    providers: cloneProviders(draft.providers),
    agentRouting: cloneRouting(draft.agentRouting),
    hudContextView: draft.hudContextView || "agent",
    hudShowTokenUsage: draft.hudShowTokenUsage,
    hudShowLocalResources: draft.hudShowLocalResources,
    hudShowGlobalDiff: draft.hudShowGlobalDiff ?? true,
    hudAutoArtifactEvolutionReport: draft.hudAutoArtifactEvolutionReport ?? true,
  });
  store.settingsOpen = false;
}

async function testOllama() {
  const url = draft.ollamaUrl.trim();
  if (!url || testingOllama.value) return;

  testingOllama.value = true;
  ollamaFeedback.value = {};

  try {
    let res: StackStatus;
    if (typeof store.testOllamaConnection === "function") {
      res = await store.testOllamaConnection(url);
    } else {
      res = await invoke<StackStatus>("check_stack_status", { url });
      store.stackStatus = res;
    }
    if (res.ollamaRunning) {
      ollamaFeedback.value = {
        ok: `Connexion établie · ${res.availableModels.length} modèle(s) détecté(s)`,
      };
      if (res.availableModels.length > 0 && !draft.ollamaModel) {
        draft.ollamaModel = res.availableModels[0];
      }
    } else {
      ollamaFeedback.value = {
        error: "Serveur Ollama injoignable ou arrêté sur cette adresse URL.",
      };
    }
  } catch (err: unknown) {
    const msg =
      typeof err === "string"
        ? err
        : (err as { message?: string })?.message || "Erreur de connexion";
    ollamaFeedback.value = { error: msg };
  } finally {
    testingOllama.value = false;
  }
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
  } catch (err: unknown) {
    p.validated = false;
    const msg =
      typeof err === "string"
        ? err
        : (err as { message?: string })?.message || "Validation impossible";
    providerFeedback.value = {
      ...providerFeedback.value,
      [p.id]: { error: msg },
    };
  } finally {
    validatingProvider.value = { ...validatingProvider.value, [p.id]: false };
  }
}

function toggleProviderAccordion(id: string) {
  expandedProviderId.value = expandedProviderId.value === id ? null : id;
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
  return Array.from(set);
});

function modelsForProvider(providerId: string): string[] {
  if (providerId === "ollama") return ollamaModelList.value;
  const prov = (draft.providers ?? []).find((p) => p.id === providerId);
  return prov?.availableModels ?? [];
}

const configuredProvidersList = computed<{ id: string; name: string; color: string }[]>(() => {
  const list: { id: string; name: string; color: string }[] = [
    { id: "ollama", name: "Ollama (Local)", color: providerBrandColors["ollama"] || "#10b981" },
  ];
  for (const p of validatedProvidersInDraft.value) {
    list.push({
      id: p.id,
      name: p.name,
      color: getProviderColor(p),
    });
  }
  return list;
});

const maxFallbacksPerAgent = computed<number>(() => {
  return Math.max(1, configuredProvidersList.value.length);
});

function getAgentSlot(role: AgentRoleKey): AgentModelSlot {
  if (!draft.agentRouting) {
    draft.agentRouting = defaultAgentRouting();
  }
  return draft.agentRouting[role] ?? draft.agentRouting.coder;
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
  agentModelSearch.value = "";
}

function getAgentFallbacks(role: AgentRoleKey): AgentModelSlot[] {
  const slot = getAgentSlot(role);
  if (!slot.fallbacks) {
    slot.fallbacks = [];
  }
  return slot.fallbacks;
}

function canAddFallback(role: AgentRoleKey): boolean {
  const fbs = getAgentFallbacks(role);
  if (fbs.length >= maxFallbacksPerAgent.value) return false;
  if (fbs.length === 0) return true;
  const last = fbs[fbs.length - 1];
  return Boolean(last.provider);
}

function addAgentFallback(role: AgentRoleKey) {
  const fbs = getAgentFallbacks(role);
  if (fbs.length >= maxFallbacksPerAgent.value) return;

  const used = new Set([getAgentSlot(role).provider, ...fbs.map((f) => f.provider)]);
  const cand = configuredProvidersList.value.find((cp) => !used.has(cp.id)) || configuredProvidersList.value[0];
  const nextProv = cand ? cand.id : "ollama";
  const models = modelsForProvider(nextProv);

  fbs.push({
    provider: nextProv,
    model: nextProv === "ollama" ? "" : (models[0] ?? ""),
  });
}

function removeAgentFallback(role: AgentRoleKey, index: number) {
  const fbs = getAgentFallbacks(role);
  fbs.splice(index, 1);
}

function onFallbackProviderChange(fb: AgentModelSlot, nextProv: string) {
  fb.provider = nextProv;
  if (nextProv === "ollama") {
    fb.model = "";
  } else {
    const models = modelsForProvider(nextProv);
    fb.model = models[0] ?? "";
  }
}

function pickFreeModel(provId: string, preferredSubstrings: string[]): string {
  const cfg = (draft.providers ?? []).find((p) => p.id === provId);
  if (!cfg || !cfg.validated || cfg.apiKey.trim().length === 0) return "";
  const pool =
    provId === "openrouter"
      ? (cfg.availableModels ?? []).filter((m) =>
          m.toLowerCase().endsWith(":free")
        )
      : (cfg.availableModels ?? []);
  if (pool.length === 0) return "";
  for (const sub of preferredSubstrings) {
    const lowSub = sub.toLowerCase();
    const match = pool.find((m) => m.toLowerCase().includes(lowSub));
    if (match) return match;
  }
  return pool[0] ?? "";
}

function assignFirstAvailableFreeSlot(
  candidates: { prov: string; prefs: string[] }[]
): AgentModelSlot {
  for (const c of candidates) {
    const mdl = pickFreeModel(c.prov, c.prefs);
    if (mdl) {
      return { provider: c.prov, model: mdl, fallbacks: [{ provider: "ollama", model: "" }] };
    }
  }
  return { provider: "ollama", model: "", fallbacks: [] };
}

function applyNoCostPreset() {
  if (!draft.agentRouting) {
    draft.agentRouting = defaultAgentRouting();
  }
  draft.agentRouting.architect = assignFirstAvailableFreeSlot([
    { prov: "gemini", prefs: ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"] },
    { prov: "openrouter", prefs: ["deepseek-r1:free", ":free"] },
    { prov: "cerebras", prefs: ["qwen-3-235b", "llama-3.3-70b"] },
    { prov: "groq", prefs: ["qwen3-32b", "llama-3.3-70b"] },
  ]);
  draft.agentRouting.coder = assignFirstAvailableFreeSlot([
    { prov: "openrouter", prefs: ["qwen3-coder:free", "deepseek-chat", ":free"] },
    { prov: "cerebras", prefs: ["qwen-3-235b", "qwen-3-32b"] },
    { prov: "groq", prefs: ["qwen3-32b", "llama-3.3-70b"] },
    { prov: "gemini", prefs: ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-2.0-flash"] },
  ]);

  const fastFreeCandidates = [
    { prov: "groq", prefs: ["llama-3.3-70b", "qwen3-32b", "llama-3.1-8b"] },
    { prov: "cerebras", prefs: ["llama-3.3-70b", "qwen-3-32b"] },
    { prov: "gemini", prefs: ["gemini-2.5-flash", "gemini-2.0-flash"] },
  ];
  draft.agentRouting.reviewer = assignFirstAvailableFreeSlot(fastFreeCandidates);

  const usedFreeProviders = new Set(
    [draft.agentRouting.architect, draft.agentRouting.coder, draft.agentRouting.reviewer]
      .map((s) => s.provider)
      .filter((p) => freeTierProviderIds.has(p))
  );
  if (usedFreeProviders.size > 0) {
    noCostPresetFeedback.value = `Préréglage appliqué (${usedFreeProviders.size} fournisseur(s) gratuit(s) actif(s) + repli Ollama local). Cliquez sur Enregistrer.`;
  } else {
    noCostPresetFeedback.value =
      "Aucun fournisseur gratuit validé détecté : tous les rôles restent sur Ollama local. Validez Gemini, OpenRouter (:free), Groq ou Cerebras dans Fournisseurs.";
  }
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
  facets?: { label: string; kind: "code" | "ui" | "refacto" }[];
}[] = [
  {
    key: "architect",
    label: "Architecte & Synthétiseur",
    desc: "Planification DAG initiale, routage et synthèse finale en boucle fermée",
    color: "#8b5cf6",
  },
  {
    key: "coder",
    label: "Codeur + Designer UI/UX",
    desc: "Implémentation atomique en un prompt : logique applicative, interfaces modernes et refactoring",
    color: "#3b82f6",
    facets: [
      { label: "Code", kind: "code" },
      { label: "UI/UX", kind: "ui" },
      { label: "Refacto", kind: "refacto" },
    ],
  },
  {
    key: "reviewer",
    label: "Reviewer & QA",
    desc: "Revue de diff Git, validation build/tests et contrôle qualité",
    color: "#10b981",
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
        <h2>Paramètres >AIx STUDIO</h2>
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
        <!-- Thème de l'interface (largeur compacte + changement instantané) -->
        <div class="field">
          <label>Thème de l'interface</label>
          <div class="segmented segmented-compact">
            <button
              v-for="t in themes"
              :key="t.value"
              type="button"
              class="seg-btn"
              :class="{ active: draft.theme === t.value }"
              @click="onSelectTheme(t.value)"
            >
              {{ t.label }}
            </button>
          </div>
          <span class="field-hint">
            Bascule instantanée entre le mode sombre, clair ou aligné sur votre système d'exploitation.
          </span>
        </div>

        <!-- Section Switchs HUD & Affichage -->
        <div class="hud-settings-section">
          <div class="section-title-wrap">
            <span class="section-label">Affichage & Télémétrie HUD</span>
            <span class="field-hint">
              Activez ou masquez les composants de télémétrie affichés en temps réel en haut à droite.
            </span>
          </div>

          <div class="hud-settings-group">
            <!-- 1. Fenêtre de contexte (Globale / Par agent) -->
            <div class="hud-setting-row">
              <div class="hud-setting-info">
                <span class="hud-setting-title">Fenêtre de contexte</span>
                <span class="hud-setting-desc">
                  Mode d'affichage de la jauge de tokens du tour actif.
                </span>
              </div>
              <div class="segmented segmented-compact">
                <button
                  type="button"
                  class="seg-btn"
                  :class="{ active: (draft.hudContextView || 'agent') === 'agent' }"
                  @click="onToggleHudSetting('hudContextView', 'agent')"
                >
                  Par agent
                </button>
                <button
                  type="button"
                  class="seg-btn"
                  :class="{ active: draft.hudContextView === 'global' }"
                  @click="onToggleHudSetting('hudContextView', 'global')"
                >
                  Globale
                </button>
              </div>
            </div>

            <!-- 2. Consommation de tokens -->
            <div class="hud-setting-row">
              <div class="hud-setting-info">
                <span class="hud-setting-title">Consommation de tokens API</span>
                <span class="hud-setting-desc">
                  Affiche la jauge journalière et le détail de consommation des clés externes.
                </span>
              </div>
              <label class="switch-toggle" title="Activer/Désactiver la consommation de tokens">
                <input
                  :checked="Boolean(draft.hudShowTokenUsage)"
                  type="checkbox"
                  @change="onToggleHudSetting('hudShowTokenUsage', ($event.target as HTMLInputElement).checked)"
                />
                <span class="switch-slider" />
              </label>
            </div>

            <!-- 3. Ressources locales -->
            <div class="hud-setting-row">
              <div class="hud-setting-info">
                <span class="hud-setting-title">Ressources locales (CPU / GPU / RAM / Temp)</span>
                <span class="hud-setting-desc">
                  Télémétrie matérielle de la machine locale exécutant les modèles.
                </span>
              </div>
              <label class="switch-toggle" title="Activer/Désactiver l'état des ressources locales">
                <input
                  :checked="Boolean(draft.hudShowLocalResources)"
                  type="checkbox"
                  @change="onToggleHudSetting('hudShowLocalResources', ($event.target as HTMLInputElement).checked)"
                />
                <span class="switch-slider" />
              </label>
            </div>

            <!-- 4. Diff global -->
            <div class="hud-setting-row">
              <div class="hud-setting-info">
                <span class="hud-setting-title">Diff global (+Lignes / -Lignes)</span>
                <span class="hud-setting-desc">
                  Indicateur du volume de modifications de code dans la conversation active.
                </span>
              </div>
              <label class="switch-toggle" title="Activer/Désactiver le diff global">
                <input
                  :checked="Boolean(draft.hudShowGlobalDiff)"
                  type="checkbox"
                  @change="onToggleHudSetting('hudShowGlobalDiff', ($event.target as HTMLInputElement).checked)"
                />
                <span class="switch-slider" />
              </label>
            </div>

            <!-- 5. Rapport d'évolution automatique -->
            <div class="hud-setting-row">
              <div class="hud-setting-info">
                <span class="hud-setting-title">Artefact — Rapport d'évolution automatique</span>
                <span class="hud-setting-desc">
                  Génère et met à jour en continu le Journal d'évolution du projet dans les artefacts.
                </span>
              </div>
              <label class="switch-toggle" title="Activer/Désactiver le rapport d'évolution automatique">
                <input
                  :checked="Boolean(draft.hudAutoArtifactEvolutionReport)"
                  type="checkbox"
                  @change="onToggleHudSetting('hudAutoArtifactEvolutionReport', ($event.target as HTMLInputElement).checked)"
                />
                <span class="switch-slider" />
              </label>
            </div>
          </div>
        </div>
      </div>

      <!-- TAB 2: FOURNISSEURS (Local & Externes) -->
      <div
        v-else-if="store.settingsActiveTab === 'providers'"
        class="modal-body custom-scroll"
      >
        <!-- Sous-onglets Local vs Externes -->
        <div class="providers-subtabs">
          <button
            type="button"
            class="subtab-btn"
            :class="{ active: providersSubTab === 'local' }"
            @click="providersSubTab = 'local'"
          >
            <svg class="subtab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <rect x="2" y="3" width="20" height="14" rx="2" />
              <line x1="8" y1="21" x2="16" y2="21" />
              <line x1="12" y1="17" x2="12" y2="21" />
            </svg>
            <span>Serveur Local (Ollama)</span>
            <span
              class="subtab-status-dot"
              :class="{ ok: store.stackStatus.ollamaRunning, idle: store.stackStatus.ollamaInstalled && !store.stackStatus.ollamaRunning }"
              :title="store.stackStatus.ollamaRunning ? 'Ollama en ligne' : 'Ollama hors ligne'"
            />
          </button>
          <button
            type="button"
            class="subtab-btn"
            :class="{ active: providersSubTab === 'external' }"
            @click="providersSubTab = 'external'"
          >
            <svg class="subtab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z" />
            </svg>
            <span>Fournisseurs Cloud (API)</span>
            <span v-if="validatedProvidersInDraft.length > 0" class="tab-count">
              {{ validatedProvidersInDraft.length }}
            </span>
          </button>
        </div>

        <!-- SOUS-ONGLET 1: LOCAL (OLLAMA) -->
        <div v-if="providersSubTab === 'local'" class="local-provider-pane">
          <p class="tab-intro">
            Paramétrez un serveur local Ollama, sur votre machine ou votre LAN. Celui-ci pourra être sélectionné pour toutes les tâches ou seulement certaines dans un mode multi-agentique. Il servira également d'IA de repli.
          </p>

          <!-- 2. Input URL API Ollama + Bouton test d'accès -->
          <div class="field">
            <label>URL API Ollama</label>
            <div class="input-with-button">
              <input
                v-model="draft.ollamaUrl"
                type="text"
                placeholder="http://127.0.0.1:11434"
                @input="ollamaFeedback = {}"
              />
              <button
                type="button"
                class="validate-btn"
                :disabled="!draft.ollamaUrl.trim() || testingOllama"
                @click="testOllama"
              >
                {{ testingOllama ? "Test..." : "Tester l'accès" }}
              </button>
            </div>
            <div v-if="ollamaFeedback.ok" class="provider-msg is-ok">
              ✓ {{ ollamaFeedback.ok }}
            </div>
            <div v-else-if="ollamaFeedback.error" class="provider-msg is-err">
              ⚠ {{ ollamaFeedback.error }}
            </div>
          </div>

          <!-- 4. Section des modèles -->
          <div class="field">
            <label>Modèle Ollama actif / repli</label>

            <!-- Cas A: Non validé / serveur hors-ligne -->
            <div v-if="!store.stackStatus.ollamaRunning" class="ollama-notice-box is-muted">
              Validez l'accès à l'API Ollama pour sélectionner un modèle.
            </div>

            <!-- Cas B: Validé mais aucun modèle présent sur Ollama -->
            <div
              v-else-if="store.stackStatus.availableModels.length === 0"
              class="ollama-notice-box is-recom"
            >
              <span>Pour commencer, nous vous préconisons de tester le modèle :</span>
              <div class="recom-model-action">
                <code>qwen2.5-coder:7b-instruct:q5_K_M</code>
                <button
                  type="button"
                  class="preset-btn"
                  @click="draft.ollamaModel = 'qwen2.5-coder:7b-instruct:q5_K_M'"
                >
                  Utiliser ce modèle
                </button>
              </div>
            </div>

            <!-- Cas C: Liste des modèles disponibles -->
            <div v-else class="ollama-models-container">
              <div class="model-chips">
                <button
                  v-for="m in store.stackStatus.availableModels"
                  :key="m"
                  type="button"
                  class="model-chip"
                  :class="{ active: draft.ollamaModel === m }"
                  @click="draft.ollamaModel = m"
                >
                  <span v-if="draft.ollamaModel === m" class="chip-check">✓</span>
                  <span>{{ m }}</span>
                </button>
              </div>
              <input
                v-model="draft.ollamaModel"
                type="text"
                class="model-input-custom"
                placeholder="Ou saisir un identifiant précis (ex: qwen2.5-coder:7b)..."
              />
            </div>
          </div>

          <!-- 5. Sélection de la fenêtre de contexte (affichée une fois un modèle sélectionné) -->
          <div v-if="draft.ollamaModel.trim().length > 0" class="field">
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
              Transmis automatiquement comme <code>num_ctx</code> à votre serveur Ollama. Astuce : activez <code>OLLAMA_FLASH_ATTENTION=1</code> et <code>OLLAMA_KV_CACHE_TYPE=q8_0</code> côté Ollama pour débloquer <strong>16K</strong> 100% en VRAM avec un modèle quantifié en <code>q5_K_M</code>.
            </span>
          </div>

          <!-- 6. Format d'édition -->
          <div class="field">
            <label>Format d'édition (Moteur natif)</label>
            <div class="segmented segmented-compact">
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

          <!-- Options système & diagnostic -->
          <label class="checkbox-row">
            <input v-model="draft.autoStartOllama" type="checkbox" />
            <span>Démarrer Ollama automatiquement si arrêté</span>
          </label>

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

              <div class="probe-card is-installed">
                <div class="probe-header">
                  <span class="probe-title">Moteur Natif Rust</span>
                  <span class="probe-status-tag ok">Prêt</span>
                </div>
                <p class="probe-desc">
                  Moteur pair-programming intégré (streaming multi-fournisseurs, patch fuzzy, outils & MCP, zéro dépendance Python/Aider).
                </p>
              </div>
            </div>
          </div>
        </div>

        <!-- SOUS-ONGLET 2: EXTERNES (DROPDOWNS COLLAPSIBLES + COULEURS PERSONNALISABLES) -->
        <div v-else class="external-provider-pane">
          <p class="tab-intro">
            Cliquez sur un fournisseur pour afficher ses modèles et ses réglages. Modifiez la pastille de couleur pour personnaliser son apparence.
          </p>

          <div class="provider-list">
            <div
              v-for="p in draft.providers"
              :key="p.id"
              class="provider-accordion"
              :class="{
                'is-expanded': expandedProviderId === p.id,
                'is-validated': p.validated,
                'is-error': Boolean(providerFeedback[p.id]?.error),
              }"
            >
              <!-- En-tête cliquable pour déplier/replier -->
              <div
                class="provider-accordion-header"
                @click="toggleProviderAccordion(p.id)"
              >
                <div class="provider-title-wrap">
                  <!-- Pastille de couleur personnalisable (déclenche le sélecteur natif) -->
                  <label
                    class="color-picker-label"
                    :title="`Changer la couleur pour ${p.name}`"
                    @click.stop
                  >
                    <span
                      class="color-dot-swatch"
                      :style="{ backgroundColor: getProviderColor(p) }"
                    />
                    <input
                      type="color"
                      :value="getProviderColor(p)"
                      class="color-input-hidden"
                      @input="onProviderColorChange(p, $event)"
                    />
                  </label>

                  <span class="provider-name">{{ p.name }}</span>
                  <span class="provider-id-tag">{{ p.id }}</span>
                  <span
                    v-if="freeTierProviderIds.has(p.id)"
                    class="free-tier-badge"
                    title="Fournisseur proposant un tier gratuit ou des modèles :free"
                  >
                    Tier gratuit
                  </span>
                </div>

                <div class="provider-header-right">
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
                  <svg
                    class="accordion-chevron"
                    :class="{ rotated: expandedProviderId === p.id }"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                  >
                    <path d="m6 9 6 6 6-6" />
                  </svg>
                </div>
              </div>

              <!-- Corps du dropdown : visible uniquement si déplié -->
              <div
                v-if="expandedProviderId === p.id"
                class="provider-accordion-body"
              >
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

                <!-- Modèles possibles : affichés uniquement dans le dropdown -->
                <div
                  v-if="p.validated && p.availableModels.length > 0"
                  class="provider-models-section"
                >
                  <label class="models-label">
                    Modèles disponibles ({{ p.availableModels.length }}) :
                  </label>
                  <div class="provider-models-preview">
                    <span
                      v-for="mdl in p.availableModels"
                      :key="mdl"
                      class="model-preview-chip"
                    >
                      {{ mdl }}
                    </span>
                  </div>
                </div>
                <div v-else-if="!p.validated" class="models-hint-muted">
                  Validez votre clé API pour détecter et afficher la liste des modèles de ce fournisseur.
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- TAB 3: AGENTS (DROPDOWNS ET GESTION DES ROUTES DE FALLBACKS) -->
      <div
        v-else
        class="modal-body custom-scroll"
      >
        <p class="tab-intro">
          Assignez un moteur principal et personnalisez la <strong>route de fallbacks</strong> de chaque agent. Cliquez sur un rôle pour le déplier.
        </p>

        <div class="agents-toolbar">
          <span class="agents-fallback-hint">
            En cas de quota 429 ou d'indisponibilité, l'agent bascule automatiquement sur les replis définis.
          </span>
          <button
            type="button"
            class="no-cost-preset-btn"
            @click="applyNoCostPreset"
          >
            ⚡ Préréglage No-Cost
          </button>
        </div>
        <div v-if="noCostPresetFeedback" class="no-cost-feedback">
          {{ noCostPresetFeedback }}
        </div>

        <div class="agents-list">
          <!-- PILIER 1 : CARTOGRAPHE DÉTERMINISTE (MOTEUR NATIF RUST) -->
          <div
            class="agent-row-card is-native-pillar"
            style="--role-color: #06b6d4"
          >
            <div class="agent-row-header is-static">
              <div class="agent-role-info">
                <div class="agent-role-title">
                  <span
                    class="role-dot"
                    style="background-color: #06b6d4"
                  />
                  <strong>Cartographe Déterministe</strong>
                  <span class="native-rust-badge">Moteur Natif Rust</span>
                </div>
                <span class="agent-role-desc">
                  Indexation AST &amp; lexicale instantanée, graphe d'imports et ciblage déterministe des fichiers (Stage 1)
                </span>
              </div>

              <div class="agent-summary-pills">
                <span
                  class="agent-prov-pill"
                  style="--prov-color: #06b6d4"
                >
                  <span
                    class="agent-prov-dot"
                    style="background-color: #06b6d4"
                  />
                  <span>Rust Index</span>
                </span>
                <span class="agent-trigger-model">0 token · &lt; 15 ms</span>
              </div>
            </div>
          </div>

          <div
            v-for="role in agentRoles"
            :key="role.key"
            class="agent-row-card"
            :class="{ 'is-expanded': expandedAgentRole === role.key }"
            :style="{ '--role-color': role.color }"
          >
            <!-- En-tête cliquable de l'agent -->
            <div class="agent-row-header" @click="toggleAgentPicker(role.key)">
              <div class="agent-role-info">
                <div class="agent-role-title">
                  <span
                    class="role-dot"
                    :style="{ backgroundColor: role.color }"
                  />
                  <strong>{{ role.label }}</strong>
                  <span
                    v-for="facet in role.facets"
                    :key="facet.kind"
                    class="coder-facet-pill"
                    :class="`is-${facet.kind}`"
                  >
                    {{ facet.label }}
                  </span>
                </div>
                <span class="agent-role-desc">{{ role.desc }}</span>
              </div>

              <div class="agent-summary-pills">
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
                      ? `Défaut (${draft.ollamaModel || 'qwen'})`
                      : getAgentSlot(role.key).model || "Choisir..."
                  }}
                </span>
                <span
                  v-if="getAgentFallbacks(role.key).length > 0"
                  class="agent-fallbacks-count-badge"
                  :title="`${getAgentFallbacks(role.key).length} repli(s) configuré(s)`"
                >
                  +{{ getAgentFallbacks(role.key).length }} repli(s)
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
              </div>
            </div>

            <!-- Panneau déplié de l'agent -->
            <div
              v-if="expandedAgentRole === role.key"
              class="agent-picker-panel"
            >
              <!-- SECTION 1: MOTEUR PRINCIPAL -->
              <div class="agent-section-block">
                <div class="picker-section-label">Moteur Principal</div>
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
                  <span class="picker-section-label">Modèle principal</span>
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
                    <span>Par défaut ({{ draft.ollamaModel || 'qwen' }})</span>
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

              <!-- SECTION 2: ROUTE DE FALLBACKS SPÉCIFIQUE -->
              <div class="fallbacks-section">
                <div class="fallback-route-header">
                  <div class="fallback-route-title">
                    <span class="picker-section-label">Route de repli (Fallbacks)</span>
                    <span class="fallback-route-count">
                      ({{ getAgentFallbacks(role.key).length }} / {{ maxFallbacksPerAgent }})
                    </span>
                  </div>
                  <span class="field-hint">
                    Cascade exécutée en cas d'erreur de quota (429) ou de défaillance.
                  </span>
                </div>

                <!-- Liste ordonnée des replis -->
                <div v-if="getAgentFallbacks(role.key).length > 0" class="fallback-chain">
                  <div
                    v-for="(fb, fbIdx) in getAgentFallbacks(role.key)"
                    :key="`fb-${role.key}-${fbIdx}`"
                    class="fallback-item-card"
                  >
                    <div class="fallback-step-badge">
                      Repli #{{ fbIdx + 1 }}
                    </div>

                    <!-- Sélecteur de fournisseur pour ce repli -->
                    <select
                      :value="fb.provider"
                      class="fallback-prov-select"
                      @change="onFallbackProviderChange(fb, ($event.target as HTMLSelectElement).value)"
                    >
                      <option
                        v-for="cp in configuredProvidersList"
                        :key="cp.id"
                        :value="cp.id"
                      >
                        {{ cp.name }}
                      </option>
                    </select>

                    <!-- Sélecteur de modèle pour ce repli -->
                    <select
                      v-if="modelsForProvider(fb.provider).length > 0"
                      v-model="fb.model"
                      class="fallback-model-select"
                    >
                      <option v-if="fb.provider === 'ollama'" value="">
                        Défaut ({{ draft.ollamaModel || 'qwen' }})
                      </option>
                      <option
                        v-for="mdl in modelsForProvider(fb.provider)"
                        :key="mdl"
                        :value="mdl"
                      >
                        {{ mdl }}
                      </option>
                    </select>
                    <input
                      v-else
                      v-model="fb.model"
                      type="text"
                      class="fallback-model-input"
                      placeholder="Modèle..."
                    />

                    <!-- Bouton supprimer ce repli -->
                    <button
                      type="button"
                      class="fallback-remove-btn"
                      title="Supprimer ce repli de la route"
                      @click="removeAgentFallback(role.key, fbIdx)"
                    >
                      ✕
                    </button>
                  </div>
                </div>

                <div v-else class="fallback-empty-hint">
                  Aucun repli spécifique configuré : l'agent utilisera la cascade de repli globale automatique vers Ollama.
                </div>

                <!-- Bouton ajouter un nouveau repli -->
                <button
                  v-if="canAddFallback(role.key)"
                  type="button"
                  class="add-fallback-btn"
                  @click="addAgentFallback(role.key)"
                >
                  + Ajouter un repli (Repli #{{ getAgentFallbacks(role.key).length + 1 }})
                </button>
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
  background: rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(6px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
}

.modal-card {
  width: 100%;
  max-width: 660px;
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

/* Compact segmented controller for themes and HUD context */
.segmented {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  border: 1px solid var(--border-subtle);
  border-radius: 9px;
  overflow: hidden;
  background: var(--discussion-bg);
}

.segmented.segmented-compact {
  max-width: 280px;
  width: fit-content;
  display: inline-flex;
  align-self: flex-start;
}

.segmented.segmented-compact .seg-btn {
  flex: 1;
  text-align: center;
  padding: 5px 10px;
  font-size: 12px;
}

.seg-btn {
  border: none;
  background: transparent;
  padding: 7px 10px;
  font-size: 12.5px;
  cursor: pointer;
  color: var(--text-muted);
  transition: all 0.12s ease;
}

.seg-btn.active {
  background: var(--active-bg);
  color: var(--accent);
  font-weight: 600;
}

/* HUD Switches Section */
.hud-settings-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 6px;
}

.section-title-wrap {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.section-label {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-main);
}

.hud-settings-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 14px;
  background: var(--discussion-bg);
  border: 1px solid var(--border-subtle);
  border-radius: 10px;
}

.hud-setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 4px 0;
}

.hud-setting-row:not(:last-child) {
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  padding-bottom: 8px;
}

.hud-setting-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
}

.hud-setting-title {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--text-main);
}

.hud-setting-desc {
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.35;
}

/* Switch toggle element */
.switch-toggle {
  position: relative;
  display: inline-block;
  width: 40px;
  height: 22px;
  flex-shrink: 0;
  cursor: pointer;
}

.switch-toggle input {
  opacity: 0;
  width: 0;
  height: 0;
}

.switch-slider {
  position: absolute;
  inset: 0;
  background-color: var(--hover-bg);
  transition: 0.2s;
  border-radius: 22px;
  border: 1px solid var(--border-subtle);
}

.switch-slider:before {
  position: absolute;
  content: "";
  height: 16px;
  width: 16px;
  left: 2px;
  bottom: 2px;
  background-color: var(--text-muted);
  transition: 0.2s;
  border-radius: 50%;
}

.switch-toggle input:checked + .switch-slider {
  background-color: var(--accent);
  border-color: var(--accent);
}

.switch-toggle input:checked + .switch-slider:before {
  background-color: #ffffff;
  transform: translateX(18px);
}

/* Sub-tabs for Fournisseurs */
.providers-subtabs {
  display: flex;
  gap: 6px;
  padding: 3px;
  background: var(--discussion-bg);
  border: 1px solid var(--border-subtle);
  border-radius: 9px;
  margin-bottom: 2px;
}

.subtab-btn {
  flex: 1;
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 7px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  transition: all 0.14s ease;
}

.subtab-btn:hover {
  color: var(--text-main);
}

.subtab-btn.active {
  background: var(--active-bg);
  color: var(--accent);
  font-weight: 600;
}

.subtab-icon {
  width: 14px;
  height: 14px;
}

.subtab-status-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--text-dim);
}

.subtab-status-dot.ok {
  background: #10b981;
}

.subtab-status-dot.idle {
  background: #f59e0b;
}

.local-provider-pane,
.external-provider-pane {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.input-with-button {
  display: flex;
  gap: 8px;
}

.input-with-button input {
  flex: 1;
}

/* Ollama notice boxes */
.ollama-notice-box {
  padding: 10px 12px;
  border-radius: 8px;
  font-size: 12px;
  border: 1px solid var(--border-subtle);
}

.ollama-notice-box.is-muted {
  background: var(--hover-bg);
  color: var(--text-muted);
}

.ollama-notice-box.is-recom {
  background: rgba(59, 130, 246, 0.08);
  border-color: rgba(59, 130, 246, 0.25);
  color: var(--text-main);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.recom-model-action {
  display: flex;
  align-items: center;
  gap: 8px;
}

.recom-model-action code {
  font-size: 11.5px;
  background: var(--discussion-bg);
  padding: 2px 6px;
  border-radius: 4px;
  border: 1px solid var(--border-subtle);
}

.ollama-models-container {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.model-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.model-chip {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  color: var(--text-muted);
  border-radius: 6px;
  padding: 4px 9px;
  font-size: 11.5px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  transition: all 0.12s ease;
}

.model-chip:hover {
  border-color: var(--accent);
  color: var(--text-main);
}

.model-chip.active {
  background: var(--active-bg);
  border-color: var(--accent);
  color: var(--accent);
  font-weight: 600;
}

.model-input-custom {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11.5px;
}

/* External Provider Accordion */
.provider-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.provider-accordion {
  border-radius: 10px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  overflow: hidden;
  transition: border-color 0.14s ease;
}

.provider-accordion.is-expanded {
  border-color: var(--border-main);
}

.provider-accordion.is-validated {
  border-left: 3px solid #10b981;
}

.provider-accordion.is-error {
  border-left: 3px solid #ef4444;
}

.provider-accordion-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px;
  cursor: pointer;
  user-select: none;
  transition: background-color 0.12s ease;
}

.provider-accordion-header:hover {
  background: var(--hover-bg);
}

.provider-title-wrap {
  display: flex;
  align-items: center;
  gap: 8px;
}

.color-picker-label {
  display: inline-flex;
  align-items: center;
  cursor: pointer;
  position: relative;
}

.color-dot-swatch {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  display: inline-block;
  border: 1.5px solid rgba(255, 255, 255, 0.35);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  transition: transform 0.14s ease;
}

.color-picker-label:hover .color-dot-swatch {
  transform: scale(1.25);
}

.color-input-hidden {
  position: absolute;
  opacity: 0;
  width: 0;
  height: 0;
  pointer-events: none;
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
  background: var(--hover-bg);
  padding: 1px 6px;
  border-radius: 4px;
  border: 1px solid var(--border-subtle);
}

.provider-header-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.accordion-chevron {
  width: 15px;
  height: 15px;
  color: var(--text-muted);
  transition: transform 0.16s ease;
}

.accordion-chevron.rotated {
  transform: rotate(180deg);
}

.provider-accordion-body {
  padding: 12px 14px;
  border-top: 1px solid var(--border-subtle);
  display: flex;
  flex-direction: column;
  gap: 10px;
  background: var(--hover-bg);
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

.provider-models-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.models-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted);
}

.provider-models-preview {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.model-preview-chip {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 10px;
  padding: 2px 6px;
  border-radius: 4px;
  background: var(--discussion-bg);
  border: 1px solid var(--border-subtle);
  color: var(--text-muted);
}

.models-hint-muted {
  font-size: 11px;
  color: var(--text-dim);
  font-style: italic;
}

/* Agents Tab & Fallback Routes */
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
  background: var(--discussion-bg);
  transition: border-color 0.14s ease, background-color 0.14s ease;
}

.agent-row-card.is-expanded {
  border-color: var(--accent);
  border-left-color: var(--role-color, var(--accent));
  background: var(--surface-elevated, var(--hover-bg));
}

.agent-row-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  cursor: pointer;
  user-select: none;
}

.agent-row-header.is-static {
  cursor: default;
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
  flex-wrap: wrap;
  gap: 7px;
  font-size: 12.5px;
  color: var(--text-main);
}

.native-rust-badge {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 7px;
  border-radius: 999px;
  color: #22d3ee;
  background: rgba(6, 182, 212, 0.14);
  border: 1px solid rgba(6, 182, 212, 0.35);
}

.coder-facet-pill {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: 999px;
  border: 1px solid transparent;
  letter-spacing: 0.01em;
}

.coder-facet-pill.is-code {
  color: #60a5fa;
  background: rgba(59, 130, 246, 0.14);
  border-color: rgba(59, 130, 246, 0.35);
}

.coder-facet-pill.is-ui {
  color: #f472b6;
  background: rgba(236, 72, 153, 0.14);
  border-color: rgba(236, 72, 153, 0.35);
}

.coder-facet-pill.is-refacto {
  color: #fbbf24;
  background: rgba(245, 158, 11, 0.14);
  border-color: rgba(245, 158, 11, 0.35);
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

.agent-summary-pills {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  flex-shrink: 0;
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
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.agent-fallbacks-count-badge {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: 999px;
  background: rgba(59, 130, 246, 0.15);
  color: var(--accent);
  border: 1px solid rgba(59, 130, 246, 0.3);
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
  gap: 12px;
}

.agent-section-block {
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
  max-height: 120px;
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

/* Fallbacks route styling */
.fallbacks-section {
  padding-top: 10px;
  border-top: 1px dashed var(--border-subtle);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.fallback-route-header {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.fallback-route-title {
  display: flex;
  align-items: center;
  gap: 6px;
}

.fallback-route-count {
  font-size: 11px;
  font-weight: 600;
  color: var(--accent);
}

.fallback-chain {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.fallback-item-card {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 8px;
  background: var(--hover-bg);
  border: 1px solid var(--border-subtle);
}

.fallback-step-badge {
  font-size: 10.5px;
  font-weight: 600;
  padding: 2px 7px;
  border-radius: 5px;
  background: var(--discussion-bg);
  color: var(--text-main);
  white-space: nowrap;
}

.fallback-prov-select,
.fallback-model-select {
  padding: 4px 8px;
  border-radius: 6px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-main);
  font-size: 11.5px;
  outline: none;
}

.fallback-model-select {
  flex: 1;
  min-width: 0;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.fallback-model-input {
  flex: 1;
  min-width: 0;
  padding: 4px 8px;
  border-radius: 6px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  color: var(--text-main);
  font-size: 11.5px;
  outline: none;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.fallback-remove-btn {
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 12px;
  padding: 4px 6px;
  border-radius: 4px;
  transition: color 0.12s ease;
}

.fallback-remove-btn:hover {
  color: #ef4444;
  background: rgba(239, 68, 68, 0.1);
}

.fallback-empty-hint {
  font-size: 11px;
  color: var(--text-dim);
  font-style: italic;
  padding: 2px 0;
}

.add-fallback-btn {
  align-self: flex-start;
  border: 1px dashed var(--border-subtle);
  background: transparent;
  color: var(--accent);
  padding: 5px 11px;
  border-radius: 6px;
  font-size: 11.5px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  transition: all 0.14s ease;
}

.add-fallback-btn:hover {
  background: var(--active-bg);
  border-color: var(--accent);
}

/* Footer & Toolbar */
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

.checkbox-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  cursor: pointer;
}

.stack-probe-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 4px;
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

.free-tier-badge {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: 4px;
  background: rgba(16, 185, 129, 0.14);
  color: #10b981;
  border: 1px solid rgba(16, 185, 129, 0.28);
}

.agents-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 9px;
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
}

.agents-fallback-hint {
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.35;
}

.no-cost-preset-btn {
  flex-shrink: 0;
  border: 1px solid rgba(16, 185, 129, 0.4);
  background: rgba(16, 185, 129, 0.14);
  color: #10b981;
  border-radius: 8px;
  padding: 6px 11px;
  font-size: 11.5px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s ease;
}

.no-cost-preset-btn:hover {
  background: rgba(16, 185, 129, 0.22);
  border-color: #10b981;
}

.no-cost-feedback {
  font-size: 11px;
  color: #10b981;
  padding: 0 2px;
}
</style>
