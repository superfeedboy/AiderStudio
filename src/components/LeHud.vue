<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useStudioStore } from "../stores/studio";
import type { TokenUsageEvent } from "../types/studio";

const store = useStudioStore();
const dropdownOpen = ref(false);
const dropdownRef = ref<HTMLElement | null>(null);
const hoveredAgentStep = ref<TokenUsageEvent | null>(null);
const apiHovered = ref(false);

const conv = computed(() => store.activeConversation);
const artifacts = computed(() => conv.value?.artifacts ?? []);
const linesAdded = computed(() => conv.value?.linesAdded ?? 0);
const linesRemoved = computed(() => conv.value?.linesRemoved ?? 0);

const roleColors: Record<string, string> = {
  architect: "#8b5cf6",
  cartographer: "#06b6d4",
  designer: "#ec4899",
  coder: "#3b82f6",
  refactoring: "#f59e0b",
  tester: "#10b981",
  synthesizer: "#94a3b8",
};

const roleShortLabels: Record<string, string> = {
  architect: "Architecte",
  cartographer: "Cartographe",
  designer: "Designer",
  coder: "Codeur",
  refactoring: "Refactoring",
  tester: "Testeur",
  synthesizer: "Synthétiseur",
};

function resolveRoleKey(step: TokenUsageEvent): string {
  if (step.role && roleColors[step.role]) return step.role;
  const low = (step.subagent || "").toLowerCase();
  if (low.includes("architect")) return "architect";
  if (low.includes("cartograph")) return "cartographer";
  if (low.includes("design")) return "designer";
  if (low.includes("refact")) return "refactoring";
  if (low.includes("test") || low.includes("vérif")) return "tester";
  if (low.includes("synth")) return "synthesizer";
  return "coder";
}

function roleColor(step: TokenUsageEvent): string {
  return roleColors[resolveRoleKey(step)] || "#3b82f6";
}

function roleShortLabel(step: TokenUsageEvent): string {
  return roleShortLabels[resolveRoleKey(step)] || step.subagent;
}

function formatCompactTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(n >= 10_000 ? 0 : 1)}k`;
  return String(n);
}

function formatCtxLimit(limit: number): string {
  if (limit >= 1_000_000) {
    return `${(limit / 1_000_000).toFixed(limit % 1_000_000 === 0 ? 0 : 1)}M`;
  }
  if (limit >= 1024 && limit % 1024 === 0) {
    return `${limit / 1024}K`;
  }
  if (limit >= 1000) {
    return `${Math.round(limit / 1000)}K`;
  }
  return String(limit);
}

const stackedDenominator = computed<number>(() => {
  return Math.max(1, store.turnTotalTokens, store.contextTokensMax);
});

function segmentWidthPercent(step: TokenUsageEvent): number {
  const pct = (step.totalTokens / stackedDenominator.value) * 100;
  return Math.max(3, Math.min(100, pct));
}

function hoveredStepPercent(step: TokenUsageEvent): number {
  const limit =
    step.contextLimit && step.contextLimit > 0
      ? step.contextLimit
      : store.contextTokensMax;
  return Math.min(100, Math.round((step.totalTokens / Math.max(1, limit)) * 100));
}

const providerUsageRows = computed(() => {
  const map = store.apiUsage.byProvider ?? {};
  const provNames: Record<string, string> = {
    gemini: "Gemini",
    openai: "OpenAI",
    anthropic: "Anthropic",
    deepseek: "DeepSeek",
    openrouter: "OpenRouter",
    mistral: "Mistral",
  };
  return Object.entries(map).map(([id, entry]) => ({
    id,
    name: provNames[id] || id,
    ...entry,
  }));
});

const contextTooltip = computed(() => {
  const lines: string[] = [
    `Total tour actif : ${store.turnTotalTokens.toLocaleString()} tok`,
    `Contexte local estimé : ${store.localContextPercent}% de ${store.contextTokensMax.toLocaleString()} tok`,
  ];
  if (store.turnTokenSteps.length > 0) {
    lines.push("Détail par sous-agent :");
    for (const step of store.turnTokenSteps) {
      lines.push(
        `• ${roleShortLabel(step)} (${step.provider || "ollama"}${step.model ? `:${step.model}` : ""}) : ${step.totalTokens.toLocaleString()} tok (${step.promptTokens.toLocaleString()} in / ${step.completionTokens.toLocaleString()} out)`
      );
    }
  }
  return lines.join("\n");
});

function handleOutsideClick(ev: MouseEvent) {
  if (dropdownRef.value && !dropdownRef.value.contains(ev.target as Node)) {
    dropdownOpen.value = false;
  }
}

onMounted(() => document.addEventListener("click", handleOutsideClick));
onBeforeUnmount(() => document.removeEventListener("click", handleOutsideClick));

function selectArtifact(id: string) {
  dropdownOpen.value = false;
  store.openArtifact(id);
}
</script>

<template>
  <aside v-if="store.hudVisible" class="le-hud halo-hud" aria-label="Télémétrie et artefacts">
    <!-- Header row: HUD title + hide button -->
    <div class="hud-header">
      <span class="hud-badge">HUD</span>
      <button
        class="hud-close-btn"
        title="Masquer le HUD"
        aria-label="Masquer le HUD"
        @click="store.hudVisible = false"
      >
        <svg class="hud-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M18 6 6 18" />
          <path d="m6 6 12 12" />
        </svg>
      </button>
    </div>

    <!-- BARRE 1: CONSOMMATION API EXTERNES -->
    <div
      class="hud-api-section"
      @mouseenter="apiHovered = true"
      @mouseleave="apiHovered = false"
    >
      <div class="hud-context-header">
        <span class="hud-context-title">
          <svg class="hud-icon-sm" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z" />
          </svg>
          <span>API Externes</span>
        </span>
        <div class="hud-api-right">
          <button
            v-if="store.validatedExternalProviders.length === 0 && store.externalApiTokensTotal === 0"
            type="button"
            class="hud-config-link"
            title="Configurer une clé API externe"
            @click="store.openSettingsTab('providers')"
          >
            Configurer
          </button>
          <template v-else>
            <strong
              class="hud-context-percent"
              :class="{
                'is-warn': store.externalApiPercent >= 70 && store.externalApiPercent < 85,
                'is-danger': store.externalApiPercent >= 85,
              }"
            >
              {{ store.externalApiPercent }}%
            </strong>
            <button
              v-if="store.externalApiTokensTotal > 0"
              type="button"
              class="hud-reset-btn"
              title="Réinitialiser le compteur journalier API"
              aria-label="Réinitialiser le compteur journalier API"
              @click.stop="store.resetApiUsage()"
            >
              ↺
            </button>
          </template>
        </div>
      </div>

      <template v-if="store.validatedExternalProviders.length > 0 || store.externalApiTokensTotal > 0">
        <div class="context-bar-track">
          <div
            class="context-bar-fill"
            :style="{ width: `${store.externalApiPercent}%` }"
            :class="{
              'fill-ok': store.externalApiPercent < 70,
              'fill-warn': store.externalApiPercent >= 70 && store.externalApiPercent < 85,
              'fill-danger': store.externalApiPercent >= 85,
            }"
          />
        </div>
        <div class="hud-api-caption">
          <span>{{ formatCompactTokens(store.externalApiTokensTotal) }} / {{ formatCompactTokens(store.externalApiTokenLimit) }} tok</span>
          <span>{{ store.validatedExternalProviders.length }} actif(s)</span>
        </div>
      </template>

      <!-- Hover breakdown card for External APIs -->
      <div
        v-if="apiHovered && (providerUsageRows.length > 0 || store.validatedExternalProviders.length > 0)"
        class="hud-hover-card halo-hud"
      >
        <div class="hover-card-title">
          <span>Consommation du {{ store.apiUsage.date }}</span>
        </div>
        <div v-if="providerUsageRows.length === 0" class="hover-card-empty">
          0 token externe consommé aujourd'hui
        </div>
        <div
          v-for="row in providerUsageRows"
          :key="row.id"
          class="hover-card-row"
        >
          <span>{{ row.name }} ({{ row.calls }} app.)</span>
          <strong>{{ row.totalTokens.toLocaleString() }} tok</strong>
        </div>
      </div>
    </div>

    <div class="hud-divider" />

    <!-- BARRE 2: CONTEXTE HORIZONTAL STACKÉ MULTI-AGENTS -->
    <div
      class="hud-context-section"
      :title="store.turnTokenSteps.length === 0 ? contextTooltip : undefined"
    >
      <div class="hud-context-header">
        <span class="hud-context-title">
          <svg class="hud-icon-sm" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1-2.5-2.5Z" />
            <path d="M6 6h10" />
            <path d="M6 10h10" />
          </svg>
          <span>Contexte</span>
        </span>
        <strong
          v-if="store.turnTokenSteps.length > 0"
          class="hud-context-tokens"
        >
          {{ store.turnTotalTokens.toLocaleString() }} tok
        </strong>
        <strong
          v-else
          class="hud-context-percent"
          :class="{
            'is-warn': store.contextPercent >= 70 && store.contextPercent < 85,
            'is-danger': store.contextPercent >= 85,
          }"
        >
          {{ store.contextPercent }}%
        </strong>
      </div>

      <!-- Barre horizontale stackée par sous-agent (1 couleur par rôle) -->
      <div
        v-if="store.turnTokenSteps.length > 0"
        class="stacked-bar-track"
      >
        <div
          v-for="(step, idx) in store.turnTokenSteps"
          :key="`${step.subagent}-${idx}`"
          class="stacked-segment"
          :class="{
            'is-hovered': hoveredAgentStep && hoveredAgentStep.subagent === step.subagent,
          }"
          :style="{
            width: `${segmentWidthPercent(step)}%`,
            backgroundColor: roleColor(step),
          }"
          @mouseenter="hoveredAgentStep = step"
          @mouseleave="hoveredAgentStep = null"
        />
      </div>
      <!-- Repli à l'état initial avant tout appel sous-agent -->
      <div v-else class="context-bar-track">
        <div
          class="context-bar-fill"
          :style="{ width: `${store.contextPercent}%` }"
          :class="{
            'fill-ok': store.contextPercent < 70,
            'fill-warn': store.contextPercent >= 70 && store.contextPercent < 85,
            'fill-danger': store.contextPercent >= 85,
          }"
        />
      </div>

      <!-- Mini pastilles légende interactives (déclenchent aussi la bulle détaillée) -->
      <div v-if="store.turnTokenSteps.length > 0" class="stacked-legend">
        <button
          v-for="(step, idx) in store.turnTokenSteps"
          :key="`leg-${step.subagent}-${idx}`"
          type="button"
          class="legend-pill"
          :class="{
            'is-active': hoveredAgentStep && hoveredAgentStep.subagent === step.subagent,
          }"
          @mouseenter="hoveredAgentStep = step"
          @mouseleave="hoveredAgentStep = null"
        >
          <span
            class="legend-dot"
            :style="{ backgroundColor: roleColor(step) }"
          />
          <span class="legend-name">{{ roleShortLabel(step) }}</span>
          <span class="legend-val">{{ formatCompactTokens(step.totalTokens) }}</span>
        </button>
      </div>

      <!-- Infobulle détaillée au hover d'un segment ou d'une pastille -->
      <div
        v-if="hoveredAgentStep"
        class="hud-hover-card halo-hud"
      >
        <div class="hover-card-title">
          <span
            class="legend-dot"
            :style="{ backgroundColor: roleColor(hoveredAgentStep) }"
          />
          <strong>{{ roleShortLabel(hoveredAgentStep) }}</strong>
          <span class="hover-card-pct">
            {{ hoveredStepPercent(hoveredAgentStep) }}% ctx
          </span>
        </div>
        <div class="hover-card-model">
          {{ hoveredAgentStep.provider || "ollama" }} · {{ hoveredAgentStep.model || store.settings.ollamaModel }}
        </div>
        <div class="hover-card-row">
          <span>Total utilisé</span>
          <strong>{{ hoveredAgentStep.totalTokens.toLocaleString() }} tok</strong>
        </div>
        <div class="hover-card-sub">
          <span>{{ hoveredAgentStep.promptTokens.toLocaleString() }} in / {{ hoveredAgentStep.completionTokens.toLocaleString() }} out</span>
          <span>Fenêtre {{ formatCtxLimit(hoveredAgentStep.contextLimit || store.contextTokensMax) }}</span>
        </div>
      </div>

      <!-- Bulle de recommandation /compact si le contexte local >= 70% -->
      <div
        v-if="store.localContextPercent >= 70"
        class="compact-alert-bubble"
        :class="{ 'is-forced': store.localContextPercent >= 95 }"
      >
        <span class="compact-alert-msg">
          {{
            store.localContextPercent >= 95
              ? "Fenêtre locale saturée (95%) : /compact forcé !"
              : store.localContextPercent >= 80
                ? "Contexte local à 80% : /compact conseillé."
                : "Contexte local à 70% : pensez à /compact."
          }}
        </span>
        <button
          class="compact-action-btn"
          title="Exécuter un /compact pour résumer l'historique et libérer de la mémoire"
          @click="store.compactConversation()"
        >
          {{ store.localContextPercent >= 95 ? "Nettoyer" : "Compacter" }}
        </button>
      </div>
    </div>

    <div class="hud-divider" />

    <!-- Entity 1: Modified lines counter (clickable -> Review) -->
    <button
      class="hud-row hud-row-btn"
      title="Cliquer pour inspecter les fichiers modifiés dans le code"
      @click="store.openReview()"
    >
      <span class="hud-entity">
        <svg class="hud-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="6" cy="6" r="2.5" />
          <circle cx="6" cy="18" r="2.5" />
          <circle cx="18" cy="18" r="2.5" />
          <path d="M6 8.5v7" />
          <path d="M18 15.5V9a2 2 0 0 0-2-2h-4" />
        </svg>
        <span class="hud-label">Lignes</span>
      </span>
      <span class="hud-val diff-vals">
        <span class="add">+{{ linesAdded }}</span>
        <span class="del">-{{ linesRemoved }}</span>
      </span>
    </button>

    <div class="hud-divider" />

    <!-- Entity 2: CPU usage -->
    <div class="hud-row" title="Utilisation CPU">
      <span class="hud-entity">
        <svg class="hud-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <rect x="6" y="6" width="12" height="12" rx="2" />
          <rect x="9.5" y="9.5" width="5" height="5" rx="0.5" />
          <path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3" />
        </svg>
        <span class="hud-label">CPU</span>
      </span>
      <strong class="hud-val">{{ store.telemetry.cpuUsage.toFixed(0) }}%</strong>
    </div>

    <!-- Entity 3: GPU usage -->
    <div class="hud-row" title="Utilisation GPU">
      <span class="hud-entity">
        <svg class="hud-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <polygon points="13 2 4 14 11 14 10 22 20 10 13 10 13 2" />
        </svg>
        <span class="hud-label">GPU</span>
      </span>
      <strong class="hud-val" :class="{ gpu: store.telemetry.gpuUsage !== null }">
        {{
          store.telemetry.gpuUsage !== null
            ? `${store.telemetry.gpuUsage.toFixed(0)}%`
            : "N/A"
        }}
      </strong>
    </div>

    <!-- Entity 4: Memory usage -->
    <div
      class="hud-row"
      :title="`Mémoire : ${store.telemetry.memUsedGb.toFixed(1)} / ${store.telemetry.memTotalGb.toFixed(1)} Go (${store.telemetry.memPercent.toFixed(0)}%)`"
    >
      <span class="hud-entity">
        <svg class="hud-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="7" width="18" height="10" rx="2" />
          <path d="M7 11v2M11 11v2M15 11v2M7 17v2M12 17v2M17 17v2" />
        </svg>
        <span class="hud-label">MEM</span>
      </span>
      <strong class="hud-val">
        {{
          store.telemetry.memTotalGb > 0
            ? `${store.telemetry.memUsedGb.toFixed(1)}G`
            : "--"
        }}
      </strong>
    </div>

    <!-- Entity 5: System temperature -->
    <div class="hud-row" title="Température système">
      <span class="hud-entity">
        <svg class="hud-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 14.76V3.5a2.5 2.5 0 0 0-5 0v11.26a4.5 4.5 0 1 0 5 0Z" />
          <circle cx="11.5" cy="17.5" r="1.5" fill="currentColor" stroke="none" />
        </svg>
        <span class="hud-label">Temp</span>
      </span>
      <strong class="hud-val">
        {{
          store.telemetry.tempCelsius !== null
            ? `${store.telemetry.tempCelsius.toFixed(0)}°C`
            : "--°C"
        }}
      </strong>
    </div>

    <div class="hud-divider" />

    <!-- Entity 6: Artifacts dropdown -->
    <div ref="dropdownRef" class="art-dropdown-wrap">
      <button
        class="hud-row hud-row-btn hud-art-btn"
        :class="{ 'is-open': dropdownOpen }"
        title="Artefacts générés"
        @click.stop="dropdownOpen = !dropdownOpen"
      >
        <span class="hud-entity">
          <svg class="hud-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="m12 3 8 4.5v9L12 21l-8-4.5v-9L12 3Z" />
            <path d="M12 12 20 7.5M12 12v9M12 12 4 7.5" />
          </svg>
          <span class="hud-label">Artefacts ({{ artifacts.length }})</span>
        </span>
        <svg
          class="hud-chevron"
          :class="{ rotated: dropdownOpen }"
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

      <div v-if="dropdownOpen" class="art-menu halo-hud">
        <div v-if="artifacts.length === 0" class="art-empty">
          Aucun artefact créé
        </div>
        <button
          v-for="art in artifacts"
          :key="art.id"
          class="art-item"
          @click="selectArtifact(art.id)"
        >
          <span class="art-item-title">{{ art.title }}</span>
          <span class="art-item-lang">{{ art.language }}</span>
        </button>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.le-hud {
  position: absolute;
  top: 14px;
  right: 14px;
  z-index: 25;
  width: clamp(235px, calc((100% - 840px) / 2 - 16px), 320px);
  min-width: 235px;
  max-width: calc(100% - 28px);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  font-size: 12px;
  backdrop-filter: blur(12px);
}

.hud-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 2px 3px;
}

.hud-badge {
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--text-dim);
}

.hud-close-btn {
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 5px;
  width: 20px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
}

.hud-close-btn:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.hud-close-btn .hud-icon {
  width: 12px;
  height: 12px;
}

/* SECTION API EXTERNES & CONTEXTE */
.hud-api-section,
.hud-context-section {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding: 3px 2px;
}

.hud-context-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}

.hud-context-title {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-muted);
}

.hud-icon-sm {
  width: 12px;
  height: 12px;
  color: var(--text-dim);
}

.hud-api-right {
  display: flex;
  align-items: center;
  gap: 4px;
}

.hud-config-link {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  color: var(--accent);
  border-radius: 5px;
  padding: 1px 6px;
  font-size: 10px;
  font-weight: 600;
  cursor: pointer;
}

.hud-config-link:hover {
  border-color: var(--accent);
}

.hud-reset-btn {
  border: none;
  background: transparent;
  color: var(--text-dim);
  border-radius: 4px;
  width: 16px;
  height: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  cursor: pointer;
  padding: 0;
}

.hud-reset-btn:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.hud-api-caption {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 10px;
  color: var(--text-dim);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.hud-context-percent {
  font-size: 11px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  color: #10b981;
}

.hud-context-percent.is-warn {
  color: #f59e0b;
}

.hud-context-percent.is-danger {
  color: #ef4444;
}

.hud-context-tokens {
  font-size: 11px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  color: var(--text-main);
}

.context-bar-track {
  width: 100%;
  height: 6px;
  border-radius: 999px;
  background: var(--hover-bg);
  overflow: hidden;
}

.context-bar-fill {
  height: 100%;
  border-radius: 999px;
  transition: width 0.3s ease, background-color 0.3s ease;
}

.context-bar-fill.fill-ok {
  background: #10b981;
  box-shadow: 0 0 6px rgba(16, 185, 129, 0.4);
}

.context-bar-fill.fill-warn {
  background: #f59e0b;
  box-shadow: 0 0 6px rgba(245, 158, 11, 0.4);
}

.context-bar-fill.fill-danger {
  background: #ef4444;
  box-shadow: 0 0 6px rgba(239, 68, 68, 0.6);
}

/* Barre horizontale stackée multi-couleurs */
.stacked-bar-track {
  display: flex;
  flex-direction: row;
  align-items: stretch;
  width: 100%;
  height: 8px;
  border-radius: 999px;
  background: var(--hover-bg);
  overflow: hidden;
  gap: 1px;
}

.stacked-segment {
  height: 100%;
  min-width: 4px;
  flex-shrink: 1;
  cursor: pointer;
  transition: filter 0.14s ease, transform 0.14s ease, opacity 0.14s ease;
}

.stacked-segment:hover,
.stacked-segment.is-hovered {
  filter: brightness(1.22);
}

.stacked-legend {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 1px;
}

.legend-pill {
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  color: var(--text-muted);
  border-radius: 5px;
  padding: 2px 5px;
  font-size: 9.5px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  cursor: pointer;
  transition: border-color 0.12s ease, background-color 0.12s ease;
}

.legend-pill:hover,
.legend-pill.is-active {
  border-color: var(--accent);
  color: var(--text-main);
  background: var(--hover-bg);
}

.legend-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.legend-name {
  font-weight: 500;
}

.legend-val {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 9px;
  color: var(--text-dim);
}

/* Floating detail card anchored below bar inside HUD */
.hud-hover-card {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  z-index: 50;
  padding: 8px 9px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 11px;
  box-shadow: 0 10px 24px rgba(0, 0, 0, 0.35);
  pointer-events: none;
}

.hover-card-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  color: var(--text-main);
}

.hover-card-pct {
  margin-left: auto;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 10px;
  color: var(--accent);
  font-weight: 600;
}

.hover-card-model {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 10px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hover-card-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 10.5px;
  color: var(--text-muted);
}

.hover-card-row strong {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  color: var(--text-main);
}

.hover-card-sub {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 9.5px;
  color: var(--text-dim);
  padding-top: 2px;
  border-top: 1px solid var(--border-subtle);
}

.hover-card-empty {
  font-size: 10.5px;
  color: var(--text-dim);
}

.compact-alert-bubble {
  margin-top: 4px;
  padding: 6px 7px;
  border-radius: 7px;
  border: 1px solid rgba(245, 158, 11, 0.35);
  background: rgba(245, 158, 11, 0.1);
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 10.5px;
  color: var(--text-main);
}

.compact-alert-bubble.is-forced {
  border-color: rgba(239, 68, 68, 0.4);
  background: rgba(239, 68, 68, 0.12);
}

.compact-alert-msg {
  line-height: 1.3;
}

.compact-action-btn {
  align-self: flex-end;
  border: none;
  background: var(--accent);
  color: #fff;
  border-radius: 5px;
  padding: 2px 7px;
  font-size: 10.5px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.15s ease;
}

.compact-action-btn:hover {
  opacity: 0.9;
}

.hud-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  padding: 4px 6px;
  border-radius: 7px;
  min-height: 25px;
}

.hud-row-btn {
  width: 100%;
  border: 1px solid transparent;
  background: transparent;
  color: inherit;
  cursor: pointer;
  font: inherit;
  text-align: left;
  transition: background-color 0.14s ease, border-color 0.14s ease;
}

.hud-row-btn:hover {
  background: var(--hover-bg);
  border-color: var(--border-subtle);
}

.hud-entity {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  color: var(--text-muted);
}

.hud-icon {
  width: 13.5px;
  height: 13.5px;
  flex-shrink: 0;
  opacity: 0.88;
}

.hud-label {
  font-size: 11.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.hud-val {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-main);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.hud-val.gpu {
  color: var(--accent-2);
}

.diff-vals {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}

.diff-vals .add {
  color: var(--diff-add-bar);
}

.diff-vals .del {
  color: var(--diff-del-bar);
}

.hud-divider {
  height: 1px;
  margin: 2px 4px;
  background: var(--border-subtle);
}

.art-dropdown-wrap {
  position: relative;
}

.hud-art-btn {
  background: var(--active-bg);
  color: var(--accent);
}

.hud-art-btn .hud-entity {
  color: var(--accent);
  font-weight: 500;
}

.hud-art-btn:hover,
.hud-art-btn.is-open {
  background: var(--active-bg);
  border-color: var(--accent);
}

.hud-chevron {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
  transition: transform 0.15s ease;
}

.hud-chevron.rotated {
  transform: rotate(180deg);
}

.art-menu {
  position: absolute;
  right: 0;
  top: calc(100% + 6px);
  width: 220px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  z-index: 40;
}

.art-empty {
  padding: 8px;
  color: var(--text-muted);
  text-align: center;
  font-size: 12px;
}

.art-item {
  border: none;
  background: transparent;
  color: var(--text-main);
  border-radius: 7px;
  padding: 6px 8px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  cursor: pointer;
  font-size: 12px;
  text-align: left;
}

.art-item:hover {
  background: var(--hover-bg);
}

.art-item-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.art-item-lang {
  color: var(--accent-2);
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
}
</style>
