<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import hljs from "highlight.js";
import "highlight.js/styles/github-dark.css";
import { useStudioStore } from "../stores/studio";
import type {
  AgentTraceStep,
  AttachedFile,
  ChatMessage,
  FileChangeSummary,
  ProcessStep,
  ProjectRuleItem,
} from "../types/studio";
import AppLogo from "./AppLogo.vue";
import LeHud from "./LeHud.vue";
import MarkdownInline from "./MarkdownInline.vue";
import PromptComposer from "./PromptComposer.vue";
import { tokenizeInline, type InlineToken, type ResolvedFileTarget } from "../utils/markdownInline";

const store = useStudioStore();
const scrollRef = ref<HTMLElement | null>(null);
const composerRef = ref<InstanceType<typeof PromptComposer> | null>(null);
const showStreamLogs = ref(false);
const nowTick = ref(Date.now());
const copiedBlockKey = ref<string | null>(null);
const copiedPromptId = ref<string | null>(null);
const previewModalImage = ref<string | null>(null);
const isDraggingOver = ref(false);

// Files dropdown open state per message ID
const openFilesDropdown = ref<Record<string, boolean>>({});
const liveTab = ref<"thinking" | "stream">("thinking");

watch(
  () => store.isProcessing,
  (processing) => {
    if (processing) {
      liveTab.value = "thinking";
    }
  }
);

watch(
  () => store.targetScrollMsgId,
  (msgId) => {
    if (msgId) {
      scrollToMessage(msgId);
      store.targetScrollMsgId = "";
    }
  }
);

function ensureElementVisibleInStream(el: HTMLElement | null) {
  const container = scrollRef.value;
  if (!container || !el) return;

  const containerRect = container.getBoundingClientRect();
  const elRect = el.getBoundingClientRect();
  const bottomMargin = 20;
  const topMargin = 16;

  if (elRect.bottom > containerRect.bottom - bottomMargin) {
    const needed = elRect.bottom - (containerRect.bottom - bottomMargin);
    const maxShift = Math.max(0, elRect.top - (containerRect.top + topMargin));
    container.scrollTop += Math.min(needed, maxShift);
  } else if (elRect.top < containerRect.top + topMargin) {
    container.scrollTop -= containerRect.top + topMargin - elRect.top;
  }
}

async function toggleFilesDropdown(msgId: string) {
  const willOpen = !openFilesDropdown.value[msgId];
  openFilesDropdown.value[msgId] = willOpen;

  if (!willOpen) return;

  await nextTick();
  ensureElementVisibleInStream(document.getElementById(`diff-dropdown-${msgId}`));
}

// Thinking state per message
const openThinkingIds = ref<Set<string>>(new Set());

async function toggleThinking(msgId: string) {
  const willOpen = !openThinkingIds.value.has(msgId);
  if (willOpen) {
    openThinkingIds.value.add(msgId);
  } else {
    openThinkingIds.value.delete(msgId);
    return;
  }

  await nextTick();
  ensureElementVisibleInStream(document.getElementById(`thinking-block-${msgId}`));
}

// Process Timeline state & helpers (fermé par défaut en mode dropdown)
const openProcessTimelineIds = ref<Set<string>>(new Set());
const openStepDetailIds = ref<Set<string>>(new Set());
const isLiveProcessTimelineOpen = ref<boolean>(false);

function isProcessTimelineOpen(msgId: string): boolean {
  return openProcessTimelineIds.value.has(msgId);
}

async function toggleProcessTimeline(msgId: string) {
  if (openProcessTimelineIds.value.has(msgId)) {
    openProcessTimelineIds.value.delete(msgId);
    return;
  }
  openProcessTimelineIds.value.add(msgId);
  await nextTick();
  ensureElementVisibleInStream(document.getElementById(`process-timeline-${msgId}`));
}

function toggleLiveProcessTimeline() {
  isLiveProcessTimelineOpen.value = !isLiveProcessTimelineOpen.value;
}

function getLiveTimelineBadgeLabel(steps: ProcessStep[]): string {
  for (let i = steps.length - 1; i >= 0; i--) {
    if (steps[i].status === "running") {
      return `Exécution en cours : ${steps[i].tool}`;
    }
  }
  if (steps.length > 0) {
    return `Exécution en cours : ${steps[steps.length - 1].tool}`;
  }
  return "Exécution en cours : analyse";
}

function isStepDetailOpen(msgId: string, stepId: string): boolean {
  return openStepDetailIds.value.has(`${msgId}::${stepId}`);
}

function toggleStepDetail(msgId: string, stepId: string) {
  const key = `${msgId}::${stepId}`;
  if (openStepDetailIds.value.has(key)) {
    openStepDetailIds.value.delete(key);
  } else {
    openStepDetailIds.value.add(key);
  }
}

function getMessageProcessSteps(msg: ChatMessage): ProcessStep[] {
  if (msg.processSteps && msg.processSteps.length > 0) {
    return msg.processSteps;
  }
  if (msg.thinking && msg.thinking.includes("### Contrôles & Outils exécutés en local")) {
    const lines = msg.thinking.split("\n");
    const steps: ProcessStep[] = [];
    let inTools = false;
    let idx = 1;
    for (const l of lines) {
      if (l.startsWith("### Contrôles & Outils exécutés en local")) {
        inTools = true;
        continue;
      }
      if (inTools && l.startsWith("### ")) {
        break;
      }
      if (inTools && l.trim().startsWith("• 🛠️")) {
        const text = l.replace("• 🛠️", "").trim();
        const isErr = text.toLowerCase().includes("erreur") || text.toLowerCase().includes("échec");
        steps.push({
          id: `legacy-${idx++}`,
          tool: text.startsWith("`") ? "run_command" : "tool",
          title: text.length > 80 ? text.slice(0, 80) + "..." : text,
          status: isErr ? "error" : "success",
          detail: text,
        });
      }
    }
    if (steps.length > 0) return steps;
  }
  return [];
}

function countProcessSuccess(steps: ProcessStep[]): number {
  return steps.filter((s) => s.status === "success").length;
}

function countProcessError(steps: ProcessStep[]): number {
  return steps.filter((s) => s.status === "error").length;
}

function hasProcessError(steps: ProcessStep[]): boolean {
  return steps.some((s) => s.status === "error");
}

function formatProcessSubtitle(steps: ProcessStep[]): string {
  const fileSteps = steps.filter((s) =>
    ["write_file", "create_file", "edit_file", "patch_file", "delete_file"].includes(s.tool)
  ).length;
  const cmdSteps = steps.filter((s) =>
    ["run_command", "bash", "shell", "exec", "execute_command"].includes(s.tool)
  ).length;
  const otherSteps = steps.length - fileSteps - cmdSteps;

  const parts: string[] = [];
  if (fileSteps > 0) parts.push(`${fileSteps} fichier${fileSteps > 1 ? "s" : ""} modifié${fileSteps > 1 ? "s" : ""}`);
  if (cmdSteps > 0) parts.push(`${cmdSteps} commande${cmdSteps > 1 ? "s" : ""} shell`);
  if (otherSteps > 0) parts.push(`${otherSteps} autre${otherSteps > 1 ? "s" : ""} action${otherSteps > 1 ? "s" : ""}`);

  return parts.length > 0 ? parts.join(" · ") : `${steps.length} action${steps.length > 1 ? "s" : ""}`;
}

function getProcessTotalDuration(steps: ProcessStep[]): string | null {
  const totalMs = steps.reduce((acc, s) => acc + (s.durationMs || 0), 0);
  if (totalMs <= 0) return null;
  return formatStepDuration(totalMs);
}

function formatStepDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function getProcessToolCategory(tool: string): "terminal" | "create" | "edit" | "search" | "mcp" | "file" {
  if (["run_command", "bash", "shell", "exec", "execute_command"].includes(tool)) return "terminal";
  if (["write_file", "create_file"].includes(tool)) return "create";
  if (["edit_file", "patch_file"].includes(tool)) return "edit";
  if (["search_code", "search_web", "web_search"].includes(tool)) return "search";
  if (tool.startsWith("mcp") || tool.includes("::")) return "mcp";
  return "file";
}

// Slash commands system (/)
interface SlashCommandItem {
  id: string;
  name: string;
  command: string;
  description: string;
  icon: string;
  needsArg?: boolean;
  argPlaceholder?: string;
}

const slashOpen = ref(false);
const slashQuery = ref("");
const slashActiveIndex = ref(0);

const availableSlashCommands: SlashCommandItem[] = [
  {
    id: "slash-rules",
    name: "rules",
    command: "/rules",
    description: "Afficher ou ajouter des règles projet (AGENTS.md & .agents/rules/)",
    icon: "📜",
    needsArg: false,
    argPlaceholder: "<consigne optionnelle>",
  },
  {
    id: "slash-plan",
    name: "plan",
    command: "/plan",
    description: "Générer un plan d'action structuré avant de coder",
    icon: "🗺️",
    needsArg: true,
    argPlaceholder: "<votre objectif ou feature>",
  },
  {
    id: "slash-compact",
    name: "compact",
    command: "/compact",
    description: "Nettoyer et alléger le contexte LLM de la session active",
    icon: "🧹",
    needsArg: false,
  },
  {
    id: "slash-diff",
    name: "diff",
    command: "/diff",
    description: "Ouvrir le panneau de revue des modifications Git",
    icon: "🔍",
    needsArg: false,
  },
  {
    id: "slash-test",
    name: "test",
    command: "/test",
    description: "Exécuter les vérifications de validation (build & tests)",
    icon: "🧪",
    needsArg: false,
    argPlaceholder: "<commande optionnelle>",
  },
  {
    id: "slash-mcp",
    name: "mcp",
    command: "/mcp",
    description: "Ouvrir la gestion des serveurs et outils MCP",
    icon: "🔌",
    needsArg: false,
  },
  {
    id: "slash-clear",
    name: "clear",
    command: "/clear",
    description: "Vider les messages de la discussion active",
    icon: "🗑️",
    needsArg: false,
  },
  {
    id: "slash-help",
    name: "help",
    command: "/help",
    description: "Afficher l'aide des commandes et raccourcis",
    icon: "❓",
    needsArg: false,
  },
];

const filteredSlashCommands = computed<SlashCommandItem[]>(() => {
  const q = slashQuery.value.trim().toLowerCase();
  if (!q) return availableSlashCommands;
  return availableSlashCommands.filter(
    (c) =>
      c.name.toLowerCase().includes(q) ||
      c.command.toLowerCase().includes(q) ||
      c.description.toLowerCase().includes(q)
  );
});

// Mention system (@)
interface MentionItem {
  id: string;
  name: string;
  description: string;
  category: "file" | "tool" | "plugin" | "mcp" | "rule";
  tag: string;
}

const mentionOpen = ref(false);
const mentionQuery = ref("");
const mentionActiveIndex = ref(0);
const mentionCategory = ref<"all" | "file" | "tool" | "plugin" | "mcp" | "rule">("all");
const projectFiles = ref<string[]>([]);
const projectRules = ref<ProjectRuleItem[]>([]);

const builtInTools: MentionItem[] = [
  { id: "tool-write", name: "write_file", description: "Créer ou écraser un fichier local", category: "tool", tag: "@tool:write_file" },
  { id: "tool-create", name: "create_file", description: "Créer un nouveau fichier dans le projet", category: "tool", tag: "@tool:create_file" },
  { id: "tool-read", name: "read_file", description: "Lire le contenu d'un fichier", category: "tool", tag: "@tool:read_file" },
  { id: "tool-search", name: "search_code", description: "Rechercher un motif dans le code du projet", category: "tool", tag: "@tool:search_code" },
  { id: "tool-cmd", name: "run_command", description: "Exécuter une commande shell locale", category: "tool", tag: "@tool:run_command" },
  { id: "tool-list", name: "list_files", description: "Lister les fichiers d'un dossier", category: "tool", tag: "@tool:list_files" },
  { id: "tool-diff", name: "git_diff", description: "Inspecter les modifications Git", category: "tool", tag: "@tool:git_diff" },
];

const builtInPlugins: MentionItem[] = [
  { id: "plugin-codegen", name: "code-generator", description: "Génération de code assistée", category: "plugin", tag: "@plugin:code-generator" },
  { id: "plugin-test", name: "test-runner", description: "Exécution et écriture de tests", category: "plugin", tag: "@plugin:test-runner" },
  { id: "plugin-lint", name: "linter", description: "Vérification et formatage de syntaxe", category: "plugin", tag: "@plugin:linter" },
  { id: "plugin-git", name: "git", description: "Gestion des branches et commits", category: "plugin", tag: "@plugin:git" },
  { id: "plugin-ollama", name: "ollama", description: "Modèle LLM local Ollama", category: "plugin", tag: "@plugin:ollama" },
];

const mcpMentions = computed<MentionItem[]>(() => {
  const items: MentionItem[] = [];
  for (const s of store.mcpServers) {
    if (!s.enabled) continue;
    if (s.serverType === "sqlite") {
      items.push({
        id: `mcp-${s.id}-read_query`,
        name: `${s.name}: read_query`,
        description: `Exécuter un SELECT SQL sur ${s.name}`,
        category: "mcp",
        tag: `@mcp:${s.name}:read_query`,
      });
      items.push({
        id: `mcp-${s.id}-list_tables`,
        name: `${s.name}: list_tables`,
        description: `Lister les tables SQL sur ${s.name}`,
        category: "mcp",
        tag: `@mcp:${s.name}:list_tables`,
      });
    } else if (s.serverType === "postgres") {
      items.push({
        id: `mcp-${s.id}-query`,
        name: `${s.name}: execute_query`,
        description: `Requête SQL PostgreSQL sur ${s.name}`,
        category: "mcp",
        tag: `@mcp:${s.name}:execute_query`,
      });
    } else {
      items.push({
        id: `mcp-${s.id}-tool`,
        name: `${s.name}`,
        description: `Outil MCP stdio (${s.command})`,
        category: "mcp",
        tag: `@mcp:${s.name}`,
      });
    }
  }
  return items;
});

async function refreshProjectFiles() {
  projectFiles.value = await store.listProjectFiles();
  projectRules.value = await store.listProjectRules();
}

async function refreshProjectRules() {
  projectRules.value = await store.listProjectRules();
}

const allMentions = computed<MentionItem[]>(() => {
  const fileItems: MentionItem[] = projectFiles.value.map((f) => ({
    id: `file-${f}`,
    name: f,
    description: "Fichier du projet",
    category: "file",
    tag: `@file:${f}`,
  }));

  const ruleItems: MentionItem[] = [
    {
      id: "rule-add",
      name: "rules",
      description: "Ajouter ou modifier une règle (.agents/rules/ ou AGENTS.md)",
      category: "rule",
      tag: "@rules",
    },
    ...projectRules.value.map((r) => ({
      id: `rule-${r.name}`,
      name: r.name,
      description: r.description || `Règle projet ${r.relativePath}`,
      category: "rule" as const,
      tag: `@rule:${r.name}`,
    })),
  ];

  return [
    ...ruleItems,
    ...fileItems,
    ...builtInTools,
    ...builtInPlugins,
    ...mcpMentions.value,
  ];
});

const targetPickerRef = ref<HTMLElement | null>(null);
const targetMenuOpen = ref(false);
const hoveredProvider = ref<string | null>(null);

const ollamaModelChoices = computed(() => {
  const list = new Set<string>();
  if (store.settings.ollamaModel) list.add(store.settings.ollamaModel);
  for (const m of store.stackStatus.availableModels) {
    if (m) list.add(m);
  }
  list.add("qwen2.5-coder:3b");
  list.add("qwen2.5-coder:7b");
  list.add("qwen2.5-coder:1.5b");
  return Array.from(list);
});

const providerColors: Record<string, string> = {
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

function providerDotColor(id: string): string {
  return providerColors[id] || "#94a3b8";
}

const roleTraceColors: Record<string, string> = {
  architect: "#8b5cf6",
  cartographer: "#06b6d4",
  designer: "#ec4899",
  coder: "#3b82f6",
  refactoring: "#f59e0b",
  tester: "#10b981",
  reviewer: "#10b981",
  terminal: "#f97316",
  research: "#14b8a6",
  synthesizer: "#94a3b8",
};

const providerShortLabels: Record<string, string> = {
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

function roleTraceDotColor(role: string): string {
  const base = role.split("_step_")[0];
  return roleTraceColors[base] || "#3b82f6";
}

function providerShortName(id: string): string {
  return providerShortLabels[id] || id;
}

function formatCompactTok(n?: number): string {
  if (!n || n <= 0) return "";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(n >= 10_000 ? 0 : 1)}k`;
  return String(n);
}

interface TraceGroup {
  id: string;
  kind: "phase" | "wave";
  waveNumber?: number;
  totalWaves?: number;
  isParallel?: boolean;
  label: string;
  steps: AgentTraceStep[];
}

function roleIcon(role: string): string {
  const base = role.split("_step_")[0].toLowerCase();
  switch (base) {
    case "terminal":
      return "💻";
    case "research":
      return "🌐";
    case "coder":
      return "🛠️";
    case "designer":
      return "🎨";
    case "refactoring":
      return "⚡";
    case "tester":
      return "🧪";
    case "reviewer":
      return "🛡️";
    case "architect":
      return "📐";
    case "cartographer":
      return "🗺️";
    case "synthesizer":
      return "📝";
    default:
      return "🤖";
  }
}

interface CoderFacetBadge {
  key: "code" | "ui" | "refacto";
  label: string;
  icon: string;
}

function isCoderFamilyRole(role: string): boolean {
  const base = role.split("_step_")[0].toLowerCase();
  return base === "coder" || base === "designer" || base === "refactoring";
}

function stripFacetTags(summary: string): string {
  return (summary || "").replace(/^(?:\[(?:Code|UI\/UX|Refacto)\]\s*)+/i, "").trim();
}

function getCoderTraceFacets(step: AgentTraceStep, msg?: ChatMessage): CoderFacetBadge[] {
  if (!isCoderFamilyRole(step.role)) return [];
  const rawSummary = step.summary || "";
  const lowText = `${step.roleLabel} ${rawSummary}`.toLowerCase();
  const baseRole = step.role.split("_step_")[0].toLowerCase();

  let hasCode = /\[code\]/i.test(rawSummary) || baseRole === "coder";
  let hasUi = /\[ui\/ux\]/i.test(rawSummary) || baseRole === "designer";
  let hasRefacto = /\[refacto\]/i.test(rawSummary) || baseRole === "refactoring";

  const diffPaths = (msg?.turnDiffs ?? []).map((d) => d.path.toLowerCase());
  for (const p of diffPaths) {
    if (/\.(css|scss|sass|less|svg)$/.test(p) || p.includes("tailwind.config")) {
      hasUi = true;
    } else if (/\.(vue|svelte|jsx|tsx|html|astro)$/.test(p)) {
      hasUi = true;
      hasCode = true;
    } else if (/\.(rs|ts|js|py|go|java|kt|cs|php|c|cpp)$/.test(p)) {
      hasCode = true;
    }
  }

  if (/\b(ui|ux|design|redesign|refonte|style|css|layout|scroll|theme|thème|responsive)\b/i.test(lowText)) {
    hasUi = true;
  }
  if (/\b(refacto|nettoyage|découpage|typage|clean|restructur)\b/i.test(lowText)) {
    hasRefacto = true;
  }
  if (/\b(store|api|persiste|localstorage|logique|code|bug|corrige)\b/i.test(lowText)) {
    hasCode = true;
  }

  const out: CoderFacetBadge[] = [];
  if (hasCode) out.push({ key: "code", label: "Code", icon: "🛠️" });
  if (hasUi) out.push({ key: "ui", label: "UI/UX", icon: "🎨" });
  if (hasRefacto) out.push({ key: "refacto", label: "Refacto", icon: "⚡" });
  if (out.length === 0) out.push({ key: "code", label: "Code", icon: "🛠️" });
  return out;
}

function getProcessStepFacets(step: ProcessStep): CoderFacetBadge[] {
  const tool = (step.tool || "").toLowerCase();
  if (tool !== "write_file" && tool !== "edit_file") return [];
  const p = (step.title || "").toLowerCase();
  const detail = (step.detail || "").toLowerCase();
  let hasCode = false;
  let hasUi = false;
  let hasRefacto = false;

  if (/\.(css|scss|sass|less|svg)$/.test(p) || p.includes("tailwind.config")) {
    hasUi = true;
  } else if (/\.(vue|svelte|jsx|tsx|html|astro)$/.test(p)) {
    hasUi = true;
    hasCode = true;
  } else if (p.includes(".")) {
    hasCode = true;
  }
  if (tool === "edit_file" && /\b(refacto|clean|rename)\b/i.test(detail)) {
    hasRefacto = true;
  }

  const out: CoderFacetBadge[] = [];
  if (hasCode) out.push({ key: "code", label: "Code", icon: "🛠️" });
  if (hasUi) out.push({ key: "ui", label: "UI/UX", icon: "🎨" });
  if (hasRefacto) out.push({ key: "refacto", label: "Refacto", icon: "⚡" });
  return out;
}

const traceGroupsCache = new WeakMap<AgentTraceStep[], TraceGroup[]>();

function groupTraceSteps(steps?: AgentTraceStep[]): TraceGroup[] {
  if (!steps || steps.length === 0) return [];
  const cached = traceGroupsCache.get(steps);
  if (cached) return cached;

  const groups: TraceGroup[] = [];
  let currentWaveNumber: number | null = null;
  let currentWaveSteps: AgentTraceStep[] = [];
  let currentTotalWaves = 1;

  function flushWave() {
    if (currentWaveSteps.length > 0 && currentWaveNumber !== null) {
      const isParallel = currentWaveSteps.length > 1;
      groups.push({
        id: `wave-${currentWaveNumber}`,
        kind: "wave",
        waveNumber: currentWaveNumber,
        totalWaves: currentTotalWaves,
        isParallel,
        label: isParallel
          ? `Vague ${currentWaveNumber}/${currentTotalWaves} · ${currentWaveSteps.length} tâches en parallèle`
          : `Vague ${currentWaveNumber}/${currentTotalWaves} · Séquentielle`,
        steps: [...currentWaveSteps],
      });
      currentWaveSteps = [];
      currentWaveNumber = null;
    }
  }

  for (const step of steps) {
    const match = step.roleLabel.match(/Vague\s+(\d+)(?:\/(\d+))?/i);
    if (match) {
      const waveNum = parseInt(match[1], 10);
      const totalW = match[2] ? parseInt(match[2], 10) : 1;

      if (currentWaveNumber !== null && currentWaveNumber !== waveNum) {
        flushWave();
      }

      currentWaveNumber = waveNum;
      currentTotalWaves = totalW;
      currentWaveSteps.push(step);
    } else {
      flushWave();
      groups.push({
        id: `phase-${step.role}`,
        kind: "phase",
        label: step.roleLabel,
        steps: [step],
      });
    }
  }

  flushWave();
  traceGroupsCache.set(steps, groups);
  return groups;
}

const providerGroups = computed(() => {
  const groups: {
    id: string;
    name: string;
    color: string;
    models: string[];
  }[] = [
    {
      id: "ollama",
      name: "Ollama (Local)",
      color: providerColors.ollama,
      models: ollamaModelChoices.value,
    },
  ];
  for (const p of store.validatedExternalProviders) {
    groups.push({
      id: p.id,
      name: p.name,
      color: providerDotColor(p.id),
      models: p.availableModels ?? [],
    });
  }
  return groups;
});

function toggleTargetMenu() {
  if (store.isProcessing) return;
  targetMenuOpen.value = !targetMenuOpen.value;
  if (!targetMenuOpen.value) {
    hoveredProvider.value = null;
  }
}

async function selectTarget(target: string) {
  targetMenuOpen.value = false;
  hoveredProvider.value = null;
  await store.setActiveTarget(target);
}

function openTargetSettings() {
  targetMenuOpen.value = false;
  hoveredProvider.value = null;
  store.openSettingsTab("providers");
}

function isTargetSelected(providerId: string, modelName: string): boolean {
  const cur = (store.settings.activeTarget || "agent").trim();
  return cur === `${providerId}:${modelName}`;
}

function handleTargetOutsideClick(ev: MouseEvent) {
  if (
    targetPickerRef.value &&
    !targetPickerRef.value.contains(ev.target as Node)
  ) {
    targetMenuOpen.value = false;
    hoveredProvider.value = null;
  }
}

const filteredMentions = computed<MentionItem[]>(() => {
  const q = mentionQuery.value.toLowerCase().trim();
  let list = allMentions.value;
  if (mentionCategory.value !== "all") {
    list = list.filter((item) => item.category === mentionCategory.value);
  }
  if (!q) return list.slice(0, 30);
  return list
    .filter(
      (item) =>
        item.name.toLowerCase().includes(q) ||
        item.tag.toLowerCase().includes(q) ||
        item.description.toLowerCase().includes(q)
    )
    .slice(0, 30);
});

let tickTimer: number | null = null;
let streamObserver: MutationObserver | null = null;
const isAutoScrollPinned = ref(true);
let lastScrollTop = 0;
let scrollBottomRafId: number | null = null;
let clearProgrammaticRafId: number | null = null;
let isProgrammaticScroll = false;

function cancelPendingAutoScroll() {
  if (scrollBottomRafId !== null) {
    cancelAnimationFrame(scrollBottomRafId);
    scrollBottomRafId = null;
  }
  if (clearProgrammaticRafId !== null) {
    cancelAnimationFrame(clearProgrammaticRafId);
    clearProgrammaticRafId = null;
  }
}

function unpinAutoScroll() {
  isAutoScrollPinned.value = false;
  isProgrammaticScroll = false;
  cancelPendingAutoScroll();
}

function onStreamWheel(ev: WheelEvent) {
  if (ev.deltaY < 0) {
    unpinAutoScroll();
  } else if (ev.deltaY > 0 && scrollRef.value) {
    const el = scrollRef.value;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    if (distanceFromBottom <= 24) {
      isAutoScrollPinned.value = true;
    }
  }
}

function scrollToBottom(force = false) {
  if (force) {
    isAutoScrollPinned.value = true;
  } else if (!isAutoScrollPinned.value) {
    return;
  }

  nextTick(() => {
    if (!force && !isAutoScrollPinned.value) return;
    const container = scrollRef.value;
    if (!container) return;

    cancelPendingAutoScroll();
    isProgrammaticScroll = true;
    container.scrollTop = container.scrollHeight;
    lastScrollTop = container.scrollTop;

    scrollBottomRafId = requestAnimationFrame(() => {
      scrollBottomRafId = null;
      if (!force && !isAutoScrollPinned.value) {
        isProgrammaticScroll = false;
        return;
      }
      const el = scrollRef.value;
      if (el) {
        el.scrollTop = el.scrollHeight;
        lastScrollTop = el.scrollTop;
      }
      clearProgrammaticRafId = requestAnimationFrame(() => {
        clearProgrammaticRafId = null;
        isProgrammaticScroll = false;
      });
    });
  });
}

onMounted(() => {
  void refreshProjectFiles();
  document.addEventListener("click", handleTargetOutsideClick);
  tickTimer = window.setInterval(() => {
    nowTick.value = Date.now();
  }, 30_000);

  // Smooth autoscroll during live LLM streaming
  if (scrollRef.value) {
    scrollRef.value.addEventListener("scroll", onStreamScroll, { passive: true });
    scrollRef.value.addEventListener("wheel", onStreamWheel, { passive: true });
    streamObserver = new MutationObserver(() => {
      if (store.isProcessing && isAutoScrollPinned.value) {
        scrollToBottom();
      }
    });
    streamObserver.observe(scrollRef.value, {
      childList: true,
      subtree: true,
      characterData: true,
    });
  }
});

onBeforeUnmount(() => {
  scrollRef.value?.removeEventListener("scroll", onStreamScroll);
  scrollRef.value?.removeEventListener("wheel", onStreamWheel);
  document.removeEventListener("click", handleTargetOutsideClick);
  cancelPendingAutoScroll();
  if (scrollDebounceTimer !== null) {
    cancelAnimationFrame(scrollDebounceTimer);
    scrollDebounceTimer = null;
  }
  if (tickTimer !== null) {
    clearInterval(tickTimer);
    tickTimer = null;
  }
  if (streamObserver) {
    streamObserver.disconnect();
    streamObserver = null;
  }
});

watch(
  () => store.activeProjectId,
  () => {
    void refreshProjectFiles();
  }
);

const conv = computed(() => store.activeConversation);
const messages = computed(() => conv.value?.messages ?? []);
const hasStarted = computed(() => messages.value.length > 0);
const activeToolApprovals = computed(() =>
  store.pendingToolApprovals.filter(
    (req) =>
      req.conversationId === store.activeConversationId ||
      req.conversationId === store.processingConversationId
  )
);

interface ConversationTurn {
  id: string;
  assistantMsgIds: string[];
  items: { msg: ChatMessage; globalIndex: number }[];
}

const conversationTurns = computed<ConversationTurn[]>(() => {
  const msgs = messages.value;
  const turns: ConversationTurn[] = [];
  let currentTurn: ConversationTurn | null = null;

  for (let i = 0; i < msgs.length; i++) {
    const msg = msgs[i];
    if (msg.role === "user") {
      if (!currentTurn || currentTurn.assistantMsgIds.length > 0) {
        currentTurn = {
          id: `turn-${msg.id}`,
          assistantMsgIds: [],
          items: [{ msg, globalIndex: i }],
        };
        turns.push(currentTurn);
      } else {
        currentTurn.items.push({ msg, globalIndex: i });
      }
    } else {
      if (!currentTurn) {
        currentTurn = {
          id: `turn-${msg.id}`,
          assistantMsgIds: [msg.id],
          items: [{ msg, globalIndex: i }],
        };
        turns.push(currentTurn);
      } else {
        currentTurn.assistantMsgIds.push(msg.id);
        currentTurn.items.push({ msg, globalIndex: i });
      }
    }
  }

  return turns;
});

// Gestion des bulles de question longues (collapsible 5-6 lignes max)
const expandedQuestionIds = ref<Set<string>>(new Set());

function isQuestionCollapsible(_msgId: string, content: string): boolean {
  if (!content) return false;
  return content.split("\n").length >= 6 || content.length > 220;
}

function toggleQuestionExpand(msgId: string) {
  if (expandedQuestionIds.value.has(msgId)) {
    expandedQuestionIds.value.delete(msgId);
  } else {
    expandedQuestionIds.value.add(msgId);
  }
}

function handleBubbleClick(msgId: string, isCollapsible: boolean, _ev: MouseEvent) {
  if (!isCollapsible) return;
  const selection = window.getSelection();
  if (selection && selection.toString().trim().length > 0) {
    return;
  }
  toggleQuestionExpand(msgId);
}


const isRenamingTitle = ref(false);
const titleDraft = ref("");
const titleInputRef = ref<HTMLInputElement | null>(null);

watch(
  () => store.activeConversationId,
  () => {
    isRenamingTitle.value = false;
    lastScrollTop = 0;
    scrollToBottom(true);
  }
);

function startRenameTitle() {
  if (!conv.value) return;
  titleDraft.value = conv.value.title || "Nouvelle session";
  isRenamingTitle.value = true;
  nextTick(() => {
    if (titleInputRef.value) {
      titleInputRef.value.focus();
      titleInputRef.value.select();
    }
  });
}

async function commitRenameTitle() {
  if (!isRenamingTitle.value) return;
  isRenamingTitle.value = false;
  const targetConv = conv.value;
  const trimmed = titleDraft.value.trim();
  if (!targetConv || !trimmed || trimmed === targetConv.title) return;
  await store.renameConversation(targetConv.id, trimmed);
}

function cancelRenameTitle() {
  isRenamingTitle.value = false;
}

const lastAssistantIndex = computed(() => {
  for (let i = messages.value.length - 1; i >= 0; i--) {
    if (messages.value[i].role === "assistant") return i;
  }
  return -1;
});

// Auto-scroll to bottom when messages or live output update
watch(
  [
    () => messages.value.length,
    () => store.liveContent,
    () => store.liveThinking,
    () => store.isProcessing,
  ],
  () => {
    scrollToBottom();
  },
  { flush: "post" }
);

type MarkdownBlock =
  | { kind: "heading"; level: 1 | 2 | 3; tokens: InlineToken[] }
  | { kind: "paragraph"; tokens: InlineToken[] }
  | { kind: "quote"; tokens: InlineToken[] }
  | { kind: "ul"; items: { indent: number; tokens: InlineToken[] }[] }
  | { kind: "ol"; items: InlineToken[][] }
  | {
      kind: "table";
      headers: InlineToken[][];
      rows: InlineToken[][][];
      alignments?: ("left" | "center" | "right")[];
    }
  | { kind: "code"; lang: string; body: string; filename?: string; streaming?: boolean };

function parseMarkdownProse(prose: string): MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  const lines = prose.split("\n");
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    if (!trimmed) {
      i++;
      continue;
    }

    // Headings
    const headingMatch = /^(#{1,3})\s+(.+)$/.exec(trimmed);
    if (headingMatch) {
      const level = headingMatch[1].length as 1 | 2 | 3;
      blocks.push({
        kind: "heading",
        level,
        tokens: tokenizeInline(headingMatch[2].trim()),
      });
      i++;
      continue;
    }

    // Blockquote
    if (trimmed.startsWith(">")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        quoteLines.push(lines[i].trim().replace(/^>\s?/, ""));
        i++;
      }
      blocks.push({
        kind: "quote",
        tokens: tokenizeInline(quoteLines.join(" ")),
      });
      continue;
    }

    // Markdown Table
    if (line.includes("|") && i + 1 < lines.length) {
      const nextLine = lines[i + 1].trim();
      const isSep = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)+\|?$/.test(nextLine);
      if (isSep) {
        const splitRow = (rowStr: string): string[] => {
          let s = rowStr.trim();
          if (s.startsWith("|")) s = s.slice(1);
          if (s.endsWith("|")) s = s.slice(0, -1);
          return s.split("|").map((c) => c.trim());
        };

        const rawHeaders = splitRow(line);
        const rawSeps = splitRow(nextLine);
        const alignments: ("left" | "center" | "right")[] = rawSeps.map((sep) => {
          const leftColon = sep.startsWith(":");
          const rightColon = sep.endsWith(":");
          if (leftColon && rightColon) return "center";
          if (rightColon) return "right";
          return "left";
        });

        const headers = rawHeaders.map((h) => tokenizeInline(h.replace(/\\n/g, " ")));
        const rows: InlineToken[][][] = [];
        i += 2;

        while (i < lines.length) {
          const rowLine = lines[i].trim();
          if (!rowLine || !rowLine.includes("|")) break;
          const cells = splitRow(rowLine);
          if (cells.length === 0 || (cells.length === 1 && !cells[0])) break;
          rows.push(cells.map((c) => tokenizeInline(c.replace(/\\n/g, " "))));
          i++;
        }

        blocks.push({
          kind: "table",
          headers,
          rows,
          alignments,
        });
        continue;
      }
    }

    // Unordered list
    if (/^\s*[-*•◦▪]\s+/.test(line)) {
      const items: { indent: number; tokens: InlineToken[] }[] = [];
      while (i < lines.length && /^\s*[-*•◦▪]\s+/.test(lines[i])) {
        const leadingSpaces = lines[i].match(/^(\s*)/)?.[1].length ?? 0;
        const text = lines[i].replace(/^\s*[-*•◦▪]\s+/, "");
        items.push({
          indent: Math.min(4, Math.floor(leadingSpaces / 2)),
          tokens: tokenizeInline(text),
        });
        i++;
      }
      blocks.push({ kind: "ul", items });
      continue;
    }

    // Ordered list
    if (/^\s*\d+\.\s+/.test(line)) {
      const items: InlineToken[][] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
        const text = lines[i].replace(/^\s*\d+\.\s+/, "");
        items.push(tokenizeInline(text));
        i++;
      }
      blocks.push({ kind: "ol", items });
      continue;
    }

    // Paragraph
    const paraLines: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !/^#{1,3}\s+/.test(lines[i].trim()) &&
      !lines[i].trim().startsWith(">") &&
      !/^\s*[-*•◦▪]\s+/.test(lines[i]) &&
      !/^\s*\d+\.\s+/.test(lines[i]) &&
      !(
        lines[i].includes("|") &&
        i + 1 < lines.length &&
        /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)+\|?$/.test(lines[i + 1].trim())
      )
    ) {
      paraLines.push(lines[i].trim());
      i++;
    }
    blocks.push({
      kind: "paragraph",
      tokens: tokenizeInline(paraLines.join(" ")),
    });
  }

  return blocks;
}

function guessFilename(lang: string, body: string, explicitHint?: string): string {
  if (explicitHint && explicitHint.trim()) {
    const clean = explicitHint.trim().replace(/^["':\s]+|["':\s]+$/g, "");
    if (clean) {
      const parts = clean.split("/");
      return parts[parts.length - 1];
    }
  }

  // Check first line for comment with filename (e.g. // src/App.vue or # file.py)
  const firstLine = (body.trim().split("\n")[0] || "").trim();
  const commentMatch = /^(?:\/\/|#|\/\*|<!--)\s*([a-zA-Z0-9_\-./]+\.[a-zA-Z0-9]+)/.exec(firstLine);
  if (commentMatch) {
    const parts = commentMatch[1].split("/");
    return parts[parts.length - 1];
  }

  const cleanLang = (lang || "").toLowerCase().trim();
  const canonicalExtMap: Record<string, string> = {
    vue: "vue",
    svelte: "svelte",
    ts: "ts",
    typescript: "ts",
    tsx: "tsx",
    js: "js",
    javascript: "js",
    jsx: "jsx",
    rs: "rs",
    rust: "rs",
    py: "py",
    python: "py",
    go: "go",
    golang: "go",
    java: "java",
    c: "c",
    cpp: "cpp",
    cs: "cs",
    csharp: "cs",
    php: "php",
    rb: "rb",
    ruby: "rb",
    swift: "swift",
    kt: "kt",
    kotlin: "kt",
    html: "html",
    css: "css",
    scss: "scss",
    sass: "sass",
    less: "less",
    sql: "sql",
    json: "json",
    yaml: "yaml",
    yml: "yaml",
    toml: "toml",
    xml: "xml",
    sh: "sh",
    bash: "sh",
    zsh: "sh",
    md: "md",
    markdown: "md",
  };

  const ext = canonicalExtMap[cleanLang] || cleanLang || "txt";
  return `snippet.${ext}`;
}

const HLJS_LANG_ALIASES: Record<string, string> = {
  vue: "xml",
  html: "xml",
  htm: "xml",
  svelte: "xml",
  svg: "xml",
  ts: "typescript",
  tsx: "typescript",
  mts: "typescript",
  cts: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  rs: "rust",
  py: "python",
  sh: "bash",
  zsh: "bash",
  shell: "bash",
  console: "bash",
  yml: "yaml",
  md: "markdown",
};

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

const codeHighlightCache = new Map<string, string>();
const MAX_CODE_CACHE_ENTRIES = 600;

function renderCodeHtml(code: string, lang: string, isStreaming = false): string {
  if (!code) return "";
  const cleanLang = (lang || "").trim().toLowerCase();
  const cacheKey = !isStreaming ? `${cleanLang}:::${code}` : null;
  if (cacheKey && codeHighlightCache.has(cacheKey)) {
    return codeHighlightCache.get(cacheKey)!;
  }

  const mappedLang = HLJS_LANG_ALIASES[cleanLang] || cleanLang;
  let result = "";
  try {
    if (
      mappedLang &&
      mappedLang !== "code" &&
      mappedLang !== "txt" &&
      mappedLang !== "text" &&
      hljs.getLanguage(mappedLang)
    ) {
      result = hljs.highlight(code, { language: mappedLang, ignoreIllegals: true }).value;
    } else if (
      isStreaming ||
      !cleanLang ||
      cleanLang === "code" ||
      cleanLang === "txt" ||
      cleanLang === "text" ||
      code.length > 20_000
    ) {
      result = escapeHtml(code);
    } else {
      result = hljs.highlightAuto(code).value;
    }
  } catch {
    result = escapeHtml(code);
  }

  if (cacheKey) {
    if (codeHighlightCache.size >= MAX_CODE_CACHE_ENTRIES) {
      const firstKey = codeHighlightCache.keys().next().value;
      if (firstKey !== undefined) codeHighlightCache.delete(firstKey);
    }
    codeHighlightCache.set(cacheKey, result);
  }

  return result;
}

function downloadCodeBlock(code: string, filename: string) {
  try {
    const blob = new Blob([code], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    URL.revokeObjectURL(url);
  } catch {
    // Ignore download errors
  }
}

function parseFenceInfo(info: string): { lang: string; hint: string } {
  const trimmed = info.trim();
  if (!trimmed) return { lang: "code", hint: "" };

  const colonIdx = trimmed.indexOf(":");
  if (colonIdx > 0) {
    const left = trimmed.slice(0, colonIdx).trim();
    const right = trimmed.slice(colonIdx + 1).trim();
    if (left && !/\s/.test(left)) {
      return { lang: left, hint: right };
    }
  }

  const spaceMatch = /^([a-zA-Z0-9_\-+#.]+)\s+(.+)$/.exec(trimmed);
  if (spaceMatch) {
    return { lang: spaceMatch[1], hint: spaceMatch[2].trim() };
  }

  if (/^[a-zA-Z0-9_\-./\\]+\.[a-zA-Z0-9]{1,10}$/.test(trimmed)) {
    const ext = trimmed.split(".").pop() || "code";
    return { lang: ext, hint: trimmed };
  }

  return { lang: trimmed, hint: "" };
}

function extractPrecedingPathHint(proseLines: string[]): string | undefined {
  for (let i = proseLines.length - 1; i >= 0; i--) {
    const line = proseLines[i].trim();
    if (!line) continue;
    const isHeadingOrPathLine =
      line.startsWith("#") || /^[`*_\s]*[a-zA-Z0-9_\-./\\]+\.[a-zA-Z0-9]{1,10}[`*_\s:]*$/.test(line);
    const match =
      /(?:^|[\s"'\*#`:(])([a-zA-Z0-9_\-./\\]+\.[a-zA-Z0-9]{1,10})(?:[\s"'\*`:).,;]|$)/.exec(
        line
      );
    if (match && match[1]) {
      const candidate = match[1];
      if (isHeadingOrPathLine || candidate.includes("/") || candidate.includes("\\")) {
        return candidate;
      }
    }
    break;
  }
  return undefined;
}

const KNOWN_TOOL_NAMES = new Set([
  "read_file",
  "read",
  "view_file",
  "write_file",
  "create_file",
  "write",
  "edit_file",
  "patch_file",
  "replace_in_file",
  "delete_file",
  "remove_file",
  "run_command",
  "bash",
  "shell",
  "exec",
  "execute_command",
  "terminal",
  "search_code",
  "grep",
  "ripgrep",
  "list_files",
  "ls",
  "web_search",
  "search_web",
  "fetch_url",
  "read_url",
  "mcp_call",
]);

function isKnownToolName(name: string): boolean {
  const lower = name.trim().toLowerCase();
  return KNOWN_TOOL_NAMES.has(lower) || lower.startsWith("mcp_") || lower.startsWith("mcp:");
}

function isKnownToolNamePrefix(partial: string): boolean {
  const lower = partial.trim().toLowerCase();
  if (!lower) return true;
  if ("mcp_".startsWith(lower) || "mcp:".startsWith(lower) || lower.startsWith("mcp_") || lower.startsWith("mcp:")) {
    return true;
  }
  for (const name of KNOWN_TOOL_NAMES) {
    if (name.startsWith(lower)) return true;
  }
  return false;
}

function isToolCallJsonObject(text: string, allowIncomplete = false): boolean {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return false;

  if (/"tool_calls"\s*:\s*\[/.test(trimmed)) return true;
  if (/"function"\s*:\s*\{\s*"name"\s*:/.test(trimmed)) return true;

  // Clés de payload d'outil pouvant apparaître AVANT `"name"` (ordre de clés variable selon les modèles)
  if (/^\[?\s*\{\s*"(?:arguments|parameters|args|tool_calls|function|tool_name)"\s*:/.test(trimmed)) {
    return true;
  }

  const toolNameMatch = /"tool_name"\s*:\s*"([^"]+)"/.exec(trimmed);
  if (toolNameMatch && (isKnownToolName(toolNameMatch[1]) || /"(?:arguments|parameters|args|input)"\s*:/.test(trimmed))) {
    return true;
  }

  const nameMatch = /"(?:name|tool)"\s*:\s*"([^"]+)"/.exec(trimmed);
  if (nameMatch) {
    const isKnown = isKnownToolName(nameMatch[1]);
    const hasArgs = /"(?:arguments|parameters|args|input)"\s*:/.test(trimmed);
    if (isKnown && (hasArgs || allowIncomplete)) return true;
    if (hasArgs && !/"version"\s*:/.test(trimmed) && !/"dependencies"\s*:/.test(trimmed)) {
      return true;
    }
  }

  if (allowIncomplete) {
    // 1. Tout début du JSON avant même que le nom de l'outil soit complet
    if (
      trimmed.length <= 28 &&
      /^\[?\s*\{\s*(?:"(?:n|na|nam|name|t|to|too|tool|tool_n|tool_na|tool_nam|tool_name|tool_c|tool_ca|tool_cal|tool_call|tool_calls|a|ar|arg|args|argu|argum|argume|argumen|argument|arguments|p|pa|par|para|param|parame|paramet|paramete|parameter|parameters|f|fu|fun|func|funct|functi|functio|function)?"?\s*:?\s*"?)?$/.test(
        trimmed
      )
    ) {
      return true;
    }
    // 2. Nom d'outil en cours de frappe entre guillemets (ex: `{"name":"write_`, `{"name":"run_c`)
    const partialNameMatch = /^\[?\s*\{\s*"(?:name|tool|tool_name)"\s*:\s*"([a-z_:-]*)$/i.exec(trimmed);
    if (partialNameMatch && isKnownToolNamePrefix(partialNameMatch[1])) {
      return true;
    }
  }

  return false;
}

function isToolCallCodeBlock(lang: string, body: string, isStreamingUnclosed = false): boolean {
  const langLow = (lang || "").trim().toLowerCase();
  if (langLow === "tool_call" || langLow === "json:tool_call" || langLow === "tool") {
    return true;
  }
  if (langLow === "" || langLow === "json" || langLow === "text" || langLow === "txt") {
    if (isStreamingUnclosed && body.trim() === "") return true;
    return isToolCallJsonObject(body, isStreamingUnclosed);
  }
  return false;
}

function stripBareJsonToolCalls(raw: string, isStreaming = false): string {
  if (!raw || (!raw.includes("{") && !raw.includes("["))) return raw;

  let out = "";
  let i = 0;
  let inFence = false;
  let fenceTicks = 0;

  while (i < raw.length) {
    const atLineStart = i === 0 || raw[i - 1] === "\n";
    if (atLineStart) {
      let j = i;
      while (j < raw.length && (raw[j] === " " || raw[j] === "\t")) j++;
      if (raw[j] === "`") {
        let ticks = 0;
        while (j + ticks < raw.length && raw[j + ticks] === "`") ticks++;
        if (ticks >= 3) {
          const lineEnd = raw.indexOf("\n", j + ticks);
          const nextIdx = lineEnd === -1 ? raw.length : lineEnd + 1;
          const restOfLine = raw.slice(j + ticks, lineEnd === -1 ? raw.length : lineEnd).trim();
          if (!inFence && !restOfLine.includes("`")) {
            inFence = true;
            fenceTicks = ticks;
          } else if (inFence && ticks >= fenceTicks && restOfLine === "") {
            inFence = false;
            fenceTicks = 0;
          }
          out += raw.slice(i, nextIdx);
          i = nextIdx;
          continue;
        }
      }
    }

    if (!inFence && (raw[i] === "{" || raw[i] === "[")) {
      const prevCh = i > 0 ? raw[i - 1] : "\n";
      if (prevCh === "\n" || prevCh === " " || prevCh === "\t" || prevCh === "\r") {
        const openCh = raw[i];
        const closeCh = openCh === "{" ? "}" : "]";
        let depth = 0;
        let inStr = false;
        let escaped = false;
        let endIdx = -1;

        for (let k = i; k < raw.length; k++) {
          const ch = raw[k];
          if (inStr) {
            if (escaped) {
              escaped = false;
            } else if (ch === "\\") {
              escaped = true;
            } else if (ch === "\"") {
              inStr = false;
            }
          } else {
            if (ch === "\"") {
              inStr = true;
            } else if (ch === openCh) {
              depth++;
            } else if (ch === closeCh) {
              depth--;
              if (depth === 0) {
                endIdx = k + 1;
                break;
              }
            }
          }
        }

        if (endIdx !== -1) {
          const candidate = raw.slice(i, endIdx);
          if (isToolCallJsonObject(candidate, false)) {
            i = endIdx;
            continue;
          }
        } else if (isStreaming) {
          const candidate = raw.slice(i);
          if (isToolCallJsonObject(candidate, true)) {
            break;
          }
        }
      }
    }

    out += raw[i];
    i++;
  }

  return out;
}

function sanitizeMessageContent(raw: string, isStreaming = false): string {
  if (!raw) return "";
  let text = raw
    .replace(/<tool_call(?:\s[^>]*)?>[\s\S]*?<\/tool_call>/g, "")
    .replace(/<tool_call(?:\s[^>]*)?>[\s\S]*$/g, "")
    .replace(/<function=[^>]+>[\s\S]*?<\/function>/g, "")
    .replace(/<function=[^>]+>[\s\S]*$/g, "")
    .replace(/\[TOOL_CALLS\]/g, "")
    .replace(/\[Appel d'outils?(?:\s+natifs?)?\s*:[^\]]+\]/gi, "");

  if (isStreaming) {
    text = text
      .replace(/<(?:t(?:o(?:o(?:l(?:_(?:c(?:a(?:l(?:l)?)?)?)?)?)?)?)?|f(?:u(?:n(?:c(?:t(?:i(?:o(?:n)?)?)?)?)?)?)?)$/i, "")
      .replace(/\[T(?:O(?:O(?:L(?:_(?:C(?:A(?:L(?:L(?:S)?)?)?)?)?)?)?)?)?$/, "")
      .replace(/\n\s*`{1,3}(?:j(?:s(?:o(?:n)?)?)?)?$/i, "");
  }

  // Filtrer les blocs Markdown clôturés ou ouverts en direct contenant des appels d'outils
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const filteredLines: string[] = [];
  let i = 0;

  while (i < lines.length) {
    const openMatch = /^\s*(`{3,})(.*)$/.exec(lines[i]);
    if (openMatch && !openMatch[2].includes("`")) {
      const openTicks = openMatch[1].length;
      const { lang } = parseFenceInfo(openMatch[2]);
      const codeLines: string[] = [];
      let closed = false;
      const fenceStartIdx = i;
      i++;

      while (i < lines.length) {
        const closeMatch = /^\s*(`{3,})\s*$/.exec(lines[i]);
        if (closeMatch && closeMatch[1].length >= openTicks) {
          closed = true;
          i++;
          break;
        }
        codeLines.push(lines[i]);
        i++;
      }

      const body = codeLines.join("\n");
      if (isToolCallCodeBlock(lang, body, isStreaming && !closed)) {
        // Ignorer complètement ce bloc de code car c'est un appel d'outil
        continue;
      }

      // Conserver le bloc de code normal
      filteredLines.push(lines[fenceStartIdx]);
      for (const cl of codeLines) {
        filteredLines.push(cl);
      }
      if (closed) {
        filteredLines.push("`".repeat(openTicks));
      }
      continue;
    }

    filteredLines.push(lines[i]);
    i++;
  }

  const withoutFencedTools = filteredLines.join("\n");
  return stripBareJsonToolCalls(withoutFencedTools, isStreaming)
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function sanitizeLiveContent(raw: string): string {
  return sanitizeMessageContent(raw, true);
}

const markdownBlocksCache = new Map<string, MarkdownBlock[]>();
const MAX_MD_CACHE_ENTRIES = 400;

function parseMarkdownMessage(content: string, isStreaming = false): MarkdownBlock[] {
  if (!content) return [];
  const sanitized = sanitizeMessageContent(content, isStreaming);
  if (!sanitized) return [];
  if (!isStreaming && markdownBlocksCache.has(sanitized)) {
    return markdownBlocksCache.get(sanitized)!;
  }

  const blocks: MarkdownBlock[] = [];
  const normalized = sanitized.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  let proseLines: string[] = [];
  let i = 0;

  const flushProse = () => {
    if (proseLines.length > 0) {
      blocks.push(...parseMarkdownProse(proseLines.join("\n")));
      proseLines = [];
    }
  };

  while (i < lines.length) {
    const openMatch = /^\s*(`{3,})(.*)$/.exec(lines[i]);
    if (openMatch && !openMatch[2].includes("`")) {
      const openTicks = openMatch[1].length;
      const { lang, hint } = parseFenceInfo(openMatch[2]);
      const precedingHint = hint || extractPrecedingPathHint(proseLines);

      flushProse();

      const codeLines: string[] = [];
      let closed = false;
      i++;

      while (i < lines.length) {
        const closeMatch = /^\s*(`{3,})\s*$/.exec(lines[i]);
        if (closeMatch && closeMatch[1].length >= openTicks) {
          closed = true;
          i++;
          break;
        }
        codeLines.push(lines[i]);
        i++;
      }

      if (!closed && isStreaming && codeLines.length > 0) {
        if (/^\s*`{1,2}\s*$/.test(codeLines[codeLines.length - 1])) {
          codeLines.pop();
        }
      }

      const body = codeLines.join("\n");
      if (isToolCallCodeBlock(lang, body, isStreaming && !closed)) {
        continue;
      }
      const filename = guessFilename(lang, body, precedingHint);
      blocks.push({
        kind: "code",
        lang,
        body,
        filename,
        streaming: isStreaming && !closed,
      });
      continue;
    }

    proseLines.push(lines[i]);
    i++;
  }

  flushProse();

  if (!isStreaming) {
    if (markdownBlocksCache.size >= MAX_MD_CACHE_ENTRIES) {
      const firstKey = markdownBlocksCache.keys().next().value;
      if (firstKey !== undefined) markdownBlocksCache.delete(firstKey);
    }
    markdownBlocksCache.set(content, blocks);
  }

  return blocks;
}

const liveContentBlocks = computed<MarkdownBlock[]>(() =>
  parseMarkdownMessage(sanitizeLiveContent(store.liveContent), true)
);

function formatRelativeTime(dateString: string): string {
  const ts = new Date(dateString).getTime();
  if (Number.isNaN(ts)) return "à l'instant";

  const diffSec = Math.max(0, Math.floor((nowTick.value - ts) / 1000));
  if (diffSec < 45) return "à l'instant";
  if (diffSec < 90) return "il y a 1 minute";

  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `il y a ${diffMin} minutes`;

  const diffHours = Math.floor(diffMin / 60);
  const remMinutes = diffMin % 60;
  if (diffHours < 24) {
    if (remMinutes === 0) return `il y a ${diffHours}h`;
    return `il y a ${diffHours}h${remMinutes.toString().padStart(2, "0")}`;
  }

  const diffDays = Math.floor(diffHours / 24);
  if (diffDays === 1) return "il y a 1 jour";
  if (diffDays < 30) return `il y a ${diffDays} jours`;

  const diffMonths = Math.floor(diffDays / 30);
  return `il y a ${diffMonths} mois`;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} o`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} Ko`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} Mo`;
}

function messageDiffStats(
  msg: ChatMessage,
  _idx: number
): { filesChanged: number; linesAdded: number; linesRemoved: number } | null {
  if (
    (msg.filesChanged && msg.filesChanged > 0) ||
    (msg.linesAdded && msg.linesAdded > 0) ||
    (msg.linesRemoved && msg.linesRemoved > 0)
  ) {
    return {
      filesChanged: msg.filesChanged ?? 0,
      linesAdded: msg.linesAdded ?? 0,
      linesRemoved: msg.linesRemoved ?? 0,
    };
  }
  return null;
}

function getFileListForMessage(msg: ChatMessage, _idx: number): FileChangeSummary[] {
  if (msg.filesSummary && msg.filesSummary.length > 0) {
    return msg.filesSummary;
  }
  if (msg.turnDiffs && msg.turnDiffs.length > 0) {
    return msg.turnDiffs.map((d) => ({
      path: d.path,
      added: d.added,
      removed: d.removed,
      modified: d.modified,
      status: d.status ?? null,
    }));
  }
  return [];
}

function canUndoMessageFiles(idx: number): boolean {
  const msg = conv.value?.messages[idx];
  return (
    idx === lastAssistantIndex.value &&
    ((msg?.turnDiffs?.length ?? 0) > 0 || (conv.value?.modifiedFiles.length ?? 0) > 0)
  );
}

const activeVisibleMsgId = ref<string>("");

function formatTimelineTime(isoString?: string): string {
  if (!isoString) return "";
  try {
    const d = new Date(isoString);
    const now = new Date();
    const timeStr = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    const isToday =
      d.getDate() === now.getDate() &&
      d.getMonth() === now.getMonth() &&
      d.getFullYear() === now.getFullYear();
    if (isToday) {
      return timeStr;
    }
    const dateStr = d.toLocaleDateString([], { day: "numeric", month: "short" });
    return `${timeStr} · ${dateStr}`;
  } catch {
    return isoString;
  }
}

function cleanResponseSnippet(text: string): string {
  const cleaned = text
    .replace(/```[\s\S]*?```/g, "")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/^[#*-]+\s*/gm, "")
    .replace(/\n+/g, " ")
    .trim();
  if (cleaned.length > 220) {
    return cleaned.slice(0, 217) + "...";
  }
  return cleaned || "Réponse générée.";
}

const assistantTimelineSteps = computed(() => {
  const msgs = messages.value;
  const steps: {
    msgId: string;
    isCurrent: boolean;
    isCompaction: boolean;
    question: string;
    answerSnippet: string;
    timestamp: string;
    commitId?: string;
  }[] = [];

  for (let i = 0; i < msgs.length; i++) {
    const msg = msgs[i];
    if (msg.role !== "assistant") continue;

    let userQuestion = "";
    let userTime = msg.createdAt;
    for (let j = i - 1; j >= 0; j--) {
      if (msgs[j].role === "user") {
        userQuestion = msgs[j].content;
        userTime = msgs[j].createdAt;
        break;
      }
    }

    const isCompaction =
      Boolean(msg.stageSummary?.toLowerCase().includes("compact")) ||
      Boolean(msg.content.toLowerCase().includes("contexte compacté")) ||
      msg.id.startsWith("compact-") ||
      Boolean(i > 0 && msgs[i - 1].content.trim() === "/compact");

    const isCurrent = activeVisibleMsgId.value
      ? activeVisibleMsgId.value === msg.id
      : i === lastAssistantIndex.value;

    steps.push({
      msgId: msg.id,
      isCurrent,
      isCompaction,
      question: userQuestion,
      answerSnippet: cleanResponseSnippet(msg.content),
      timestamp: userTime || msg.createdAt,
      commitId: msg.commitId,
    });
  }

  return steps;
});

let scrollDebounceTimer: number | null = null;
function onStreamScroll() {
  const container = scrollRef.value;
  if (container) {
    const currentTop = container.scrollTop;
    const distanceFromBottom =
      container.scrollHeight - currentTop - container.clientHeight;

    if (currentTop < lastScrollTop - 2 && distanceFromBottom > 12) {
      unpinAutoScroll();
    } else if (!isProgrammaticScroll && distanceFromBottom <= 24) {
      isAutoScrollPinned.value = true;
    }
    lastScrollTop = currentTop;
  }

  if (scrollDebounceTimer) cancelAnimationFrame(scrollDebounceTimer);
  scrollDebounceTimer = requestAnimationFrame(() => {
    const container = scrollRef.value;
    if (!container) return;
    const containerRect = container.getBoundingClientRect();
    const targetY = containerRect.top + containerRect.height * 0.35;

    // Refermer automatiquement les bulles de question ouvertes sorties de l'écran
    if (expandedQuestionIds.value.size > 0) {
      expandedQuestionIds.value.forEach((msgId) => {
        const bubbleEl = container.querySelector<HTMLElement>(`[data-user-bubble-id="${msgId}"]`);
        if (!bubbleEl) return;
        const rect = bubbleEl.getBoundingClientRect();
        if (rect.bottom < containerRect.top || rect.top > containerRect.bottom) {
          expandedQuestionIds.value.delete(msgId);
        }
      });
    }

    const assistantArticles = container.querySelectorAll<HTMLElement>(".assistant-article");
    let closestId = "";
    let minDistance = Infinity;

    assistantArticles.forEach((el) => {
      const rect = el.getBoundingClientRect();
      const mid = rect.top + rect.height * 0.3;
      const dist = Math.abs(mid - targetY);
      if (dist < minDistance) {
        minDistance = dist;
        closestId = el.getAttribute("data-msg-id") || "";
      }
    });

    if (closestId) {
      activeVisibleMsgId.value = closestId;
    }
  });
}

function scrollToMessage(msgId: string) {
  const container = scrollRef.value;
  if (!container) return;
  const turn = container.querySelector<HTMLElement>(`[data-turn-assistant-id~="${msgId}"]`);
  const assistantEl = container.querySelector<HTMLElement>(`[data-msg-id="${msgId}"]`);
  const target = turn || assistantEl;
  if (target) {
    unpinAutoScroll();
    const containerH = container.clientHeight;
    const targetH = target.offsetHeight;
    if (targetH <= containerH * 0.85) {
      target.scrollIntoView({ behavior: "smooth", block: "center" });
    } else {
      target.scrollIntoView({ behavior: "smooth", block: "start" });
    }
    target.classList.remove("pulse-highlight");
    void target.offsetWidth;
    target.classList.add("pulse-highlight");
    setTimeout(() => {
      target.classList.remove("pulse-highlight");
    }, 1800);
  }
}

function getAvailableDiffPaths(msg?: ChatMessage): string[] {
  const paths: string[] = [];
  const seen = new Set<string>();
  const addPath = (p?: string) => {
    const clean = (p || "").trim();
    if (clean && !seen.has(clean)) {
      seen.add(clean);
      paths.push(clean);
    }
  };
  if (msg?.turnDiffs) {
    for (const f of msg.turnDiffs) {
      addPath(f.path);
    }
  }
  if (msg?.filesSummary) {
    for (const f of msg.filesSummary) {
      addPath(f.path);
    }
  }
  for (const f of conv.value?.modifiedFiles ?? []) {
    addPath(f.path);
  }
  return paths;
}

function normalizeReviewPath(p: string): string {
  return p.replace(/\\/g, "/").replace(/^\.\//, "").trim();
}

function handleOpenFileInReview(
  payload: ResolvedFileTarget,
  messageId?: string | null
) {
  if (messageId) {
    const scopedMsg = conv.value?.messages.find((m) => m.id === messageId);
    const targetNorm = normalizeReviewPath(payload.path);
    const inTurnDiffs = Boolean(
      scopedMsg?.turnDiffs?.some((f) => normalizeReviewPath(f.path) === targetNorm)
    );
    if (inTurnDiffs) {
      store.openReview(payload.path, payload.line, messageId);
      return;
    }
  }
  store.openReview(payload.path, payload.line, null);
}

function getFileStatus(file: FileChangeSummary): "created" | "modified" | "deleted" {
  if (file.status === "created" || file.status === "modified" || file.status === "deleted") {
    return file.status;
  }
  if ((file.modified ?? 0) === 0 && file.removed === 0 && file.added > 0) return "created";
  if ((file.modified ?? 0) === 0 && file.added === 0 && file.removed > 0) return "deleted";
  return "modified";
}

function getFileStatusLabel(file: FileChangeSummary): string {
  const status = getFileStatus(file);
  if (status === "created") return "CRÉÉ";
  if (status === "deleted") return "SUPPRIMÉ";
  return "MODIFIÉ";
}

function getFilesBreakdown(
  msg: ChatMessage,
  idx: number
): { created: number; modified: number; deleted: number } {
  const files = getFileListForMessage(msg, idx);
  let created = 0;
  let modified = 0;
  let deleted = 0;
  for (const f of files) {
    const s = getFileStatus(f);
    if (s === "created") created++;
    else if (s === "deleted") deleted++;
    else modified++;
  }
  return { created, modified, deleted };
}

async function copyPrompt(content: string, msgId: string) {
  try {
    await navigator.clipboard.writeText(content);
    copiedPromptId.value = msgId;
    window.setTimeout(() => {
      if (copiedPromptId.value === msgId) {
        copiedPromptId.value = null;
      }
    }, 1600);
  } catch {
    // Ignore clipboard failure
  }
}

function rerunPrompt(content: string) {
  composerRef.value?.setPrompt(content);
  scrollToBottom(true);
}

async function confirmUndoFile(filePath: string, messageId?: string | null) {
  const scopedMsg = messageId
    ? conv.value?.messages.find((m) => m.id === messageId)
    : undefined;
  const hasStepTree = Boolean(messageId && scopedMsg?.preTurnTree);
  const stepLabel = scopedMsg?.commitId || "cette étape";
  const ok = window.confirm(
    hasStepTree
      ? `Voulez-vous restaurer "${filePath}" tel qu'il était avant ${stepLabel} ?`
      : `Voulez-vous annuler toutes les modifications apportées au fichier "${filePath}" et le restaurer dans son état d'origine ?`
  );
  if (!ok) return;
  await store.undoFile(filePath, undefined, hasStepTree ? messageId : null);
}

async function confirmUndoAll(messageId?: string | null) {
  const scopedMsg = messageId
    ? conv.value?.messages.find((m) => m.id === messageId)
    : undefined;
  const hasStepTree = Boolean(messageId && scopedMsg?.preTurnTree);
  const stepLabel = scopedMsg?.commitId || "cette étape";
  const count = hasStepTree
    ? (scopedMsg?.turnDiffs?.length ?? scopedMsg?.filesSummary?.length ?? 0)
    : (conv.value?.modifiedFiles.length ?? 0);
  const ok = window.confirm(
    hasStepTree
      ? `Voulez-vous annuler TOUTES les modifications de ${stepLabel} (${count} fichier${count > 1 ? "s" : ""}) et restaurer ces fichiers à leur état précédant cette étape ?\nCette action est irréversible.`
      : `Voulez-vous annuler TOUTES les modifications (${count} fichier${count > 1 ? "s" : ""}) et restaurer le projet dans son état d'origine ?\nCette action est irréversible.`
  );
  if (!ok) return;
  await store.undoAll(undefined, hasStepTree ? messageId : null);
}


function handleDragOver(ev: DragEvent) {
  ev.preventDefault();
  isDraggingOver.value = true;
}

function handleDragLeave() {
  isDraggingOver.value = false;
}

function handleDrop(ev: DragEvent) {
  ev.preventDefault();
  isDraggingOver.value = false;
  if (ev.dataTransfer?.files && ev.dataTransfer.files.length > 0) {
    composerRef.value?.addFiles(ev.dataTransfer.files);
  }
}

async function executeSlashCommand(command: string, args: string): Promise<boolean> {
  const norm = command.toLowerCase().trim();
  const conv = store.activeConversation;

  switch (norm) {
    case "/compact":
      await store.compactConversation();
      return true;

    case "/clear":
      await store.clearConversation();
      return true;

    case "/diff":
    case "/review":
      store.openReview(undefined, null, null);
      return true;

    case "/mcp":
      store.mcpModalOpen = true;
      return true;

    case "/rules": {
      if (args.trim()) {
        const prompt = `@rules ${args.trim()}`;
        await store.sendPrompt(prompt, []);
        return true;
      }
      const rules = await store.listProjectRules();
      const now = new Date().toISOString();
      let content = "📜 **Directives actives du projet** :\n\n";
      if (rules.length === 0) {
        content += "Aucun fichier `AGENTS.md` ou `.agents/rules/*.md` détecté dans ce projet.\nUtilisez `/rules <consigne>` ou `@rules <consigne>` pour en créer.";
      } else {
        content += rules
          .map((r) => `- **${r.name}** (\`${r.relativePath}\`) : ${r.description}`)
          .join("\n");
        content += "\n\n💡 *Pour ajouter ou modifier une règle, écrivez `/rules <votre consigne>`.*";
      }
      conv?.messages.push({
        id: "rules-" + Date.now(),
        role: "assistant",
        content,
        stageSummary: "Consultation des règles (/rules)",
        createdAt: now,
        filesChanged: 0,
        linesAdded: 0,
        linesRemoved: 0,
      });
      return true;
    }

    case "/plan": {
      const planGoal = args.trim();
      const prompt = planGoal
        ? `[MODE PLANIFICATION] Analyse la demande suivante et génère un plan d'action structuré par étapes (sans modifier de code immédiatement) : ${planGoal}`
        : "[MODE PLANIFICATION] Analyse l'état du projet et propose un plan d'action structuré pour les prochaines étapes.";
      await store.sendPrompt(prompt, []);
      return true;
    }

    case "/test": {
      const testGoal = args.trim();
      const prompt = testGoal
        ? `[TEST & VALIDATION] Exécute la vérification ou les tests suivants : ${testGoal}`
        : "[TEST & VALIDATION] Exécute la vérification de compilation et les tests du projet (build & test) via l'outil terminal et synthétise le résultat.";
      await store.sendPrompt(prompt, []);
      return true;
    }

    case "/help": {
      const now = new Date().toISOString();
      const helpContent = `### 💡 Guide des Commandes Slash (\`/\`) & Mentions (\`@\`)

#### ⚡ Commandes rapides (\`/\`) :
- **\`/rules [consigne]\`** : Affiche les règles actives du projet ou en enregistre une nouvelle dans \`.agents/rules/\`.
- **\`/plan <objectif>\`** : Force l'Orchestrateur à générer une feuille de route détaillée sans modifier le code immédiatement.
- **\`/compact\`** : Allège et nettoie la mémoire LLM de la session active sans effacer l'historique visuel.
- **\`/diff\`** ou **\`/review\`** : Ouvre le tiroir de revue des modifications Git en cours.
- **\`/test [cmd]\`** : Lance les tests de validation du projet (\`npm run build\`, \`cargo check\`).
- **\`/mcp\`** : Ouvre la modale de configuration des serveurs MCP.
- **\`/clear\`** : Nettoie l'écran de la discussion active.
- **\`/help\`** : Affiche ce panneau d'aide.

#### 🎯 Mentions contextuelles (\`@\`) :
- **\`@rules\`** : Cible l'ajout ou la mise à jour des règles projet.
- **\`@file:<chemin>\`** : Injecte le contenu d'un fichier source.
- **\`@tool:<nom>\`** : Invoque un outil spécifique (\`run_command\`, \`search_code\`, etc.).
- **\`@mcp:<serveur>\`** : Invoque une requête ou un outil MCP.`;

      conv?.messages.push({
        id: "help-" + Date.now(),
        role: "assistant",
        content: helpContent,
        stageSummary: "Aide (/help)",
        createdAt: now,
        filesChanged: 0,
        linesAdded: 0,
        linesRemoved: 0,
      });
      return true;
    }

    default:
      return false;
  }
}

async function handleComposerSubmit(payload: { text: string; files: AttachedFile[] }) {
  const { text, files } = payload;
  if (!text && files.length === 0) return;

  // Interception des commandes slash (/)
  if (text.startsWith("/")) {
    const firstSpace = text.indexOf(" ");
    const cmd = firstSpace === -1 ? text : text.slice(0, firstSpace);
    const args = firstSpace === -1 ? "" : text.slice(firstSpace + 1);
    if (await executeSlashCommand(cmd, args)) {
      scrollToBottom(true);
      return;
    }
  }

  scrollToBottom(true);
  await store.sendPrompt(text, files);
  scrollToBottom(false);
}

async function copyCodeBlock(key: string, code: string) {
  try {
    await navigator.clipboard.writeText(code);
    copiedBlockKey.value = key;
    window.setTimeout(() => {
      if (copiedBlockKey.value === key) {
        copiedBlockKey.value = null;
      }
    }, 1600);
  } catch {
    // Ignore clipboard failure
  }
}
</script>

<template>
  <main
    class="la-discussion halo-surface"
    :class="{ 'is-wide': store.isWideScreen, 'is-drag-over': isDraggingOver }"
    @dragover="handleDragOver"
    @dragleave="handleDragLeave"
    @drop="handleDrop"
  >
    <!-- Top bar -->
    <header class="disc-header">
      <div class="disc-header-left">
        <template v-if="store.activeProject">
          <button
            type="button"
            class="breadcrumb-project"
            title="Paramètres et contexte global du projet"
            @click="store.openProjectSettings(store.activeProject)"
          >
            {{ store.activeProject.name }}
          </button>
          <span class="breadcrumb-sep">/</span>
          <input
            v-if="isRenamingTitle"
            ref="titleInputRef"
            v-model="titleDraft"
            type="text"
            class="breadcrumb-title-input"
            aria-label="Renommer la discussion"
            @keydown.enter.prevent="commitRenameTitle"
            @keydown.escape.prevent="cancelRenameTitle"
            @blur="commitRenameTitle"
          />
          <button
            v-else
            type="button"
            class="breadcrumb-title-btn"
            title="Cliquer pour renommer la discussion"
            @click="startRenameTitle"
          >
            <span class="disc-title">{{ conv?.title || "Nouvelle session" }}</span>
            <svg
              class="rename-pencil-icon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M12 20h9" />
              <path d="M16.5 3.5a2.121 2.121 0 1 1 3 3L7 19l-4 1 1-4Z" />
            </svg>
          </button>
        </template>
        <button
          v-else
          type="button"
          class="breadcrumb-project"
          title="Créer un nouveau projet"
          @click="store.newProjectModalOpen = true"
        >
          Aucun projet
        </button>
      </div>

      <button
        v-if="!store.hudVisible"
        class="restore-hud-btn"
        @click="store.hudVisible = true"
      >
        Afficher HUD
      </button>
    </header>

    <!-- Floating HUD -->
    <LeHud />

    <!-- Rail de navigation temporel (ronds centré verticalement à droite du menu) -->
    <nav
      v-if="assistantTimelineSteps.length > 0"
      class="disc-timeline-rail"
      aria-label="Timeline des réponses IA"
    >
      <div
        v-for="step in assistantTimelineSteps"
        :key="step.msgId"
        class="timeline-dot-wrapper"
        :class="{ 'is-current': step.isCurrent, 'is-compact': step.isCompaction }"
        @click="scrollToMessage(step.msgId)"
      >
        <span
          class="timeline-dot"
          :class="{ 'is-current': step.isCurrent, 'is-compact': step.isCompaction }"
        />
        <!-- Bulle d'aperçu au survol -->
        <div class="timeline-tooltip halo-hud">
          <div v-if="step.commitId || step.isCompaction" class="timeline-tooltip-header">
            <span v-if="step.commitId" class="tt-commit-tag">📦 {{ step.commitId }}</span>
            <span v-if="step.isCompaction" class="tt-compact-tag">🧹 Compacté</span>
          </div>
          <p class="timeline-tooltip-question">{{ step.question || "Étape d'initialisation" }}</p>
          <p v-if="step.answerSnippet" class="timeline-tooltip-answer">{{ step.answerSnippet }}</p>
          <div class="timeline-tooltip-footer">
            <span class="tt-time">{{ formatTimelineTime(step.timestamp) }}</span>
          </div>
        </div>
      </div>
    </nav>

    <!-- Scrollable stream or Empty welcome state -->
    <div ref="scrollRef" class="disc-stream custom-scroll">
      <!-- Empty state before first prompt -->
      <div v-if="!hasStarted && !store.isProcessing" class="welcome-state">
        <div class="welcome-logo halo-hud">
          <AppLogo :size="36" />
        </div>
        <h1 class="welcome-title">Qu'est-ce qu'on code patron?</h1>
        <div class="welcome-meta">
          <span v-if="store.activeProject" class="meta-chip">{{ store.activeProject.name }}</span>
          <span class="meta-chip">
            {{
              store.activeTargetLabel.isAgent
                ? "Mode Agent (Multi-modèles)"
                : `${store.activeTargetLabel.providerName} · ${store.activeTargetLabel.modelName}`
            }}
          </span>
          <span class="meta-chip">
            {{ store.stackStatus.nativeEngineReady ? "Moteur natif prêt" : "Moteur en attente" }}
          </span>
        </div>
        <div v-if="!store.activeProject" class="welcome-empty-project">
          <p class="welcome-empty-hint">
            Ajoutez un dossier de projet pour démarrer votre première session.
          </p>
          <button
            type="button"
            class="welcome-new-project-btn"
            @click="store.newProjectModalOpen = true"
          >
            + Nouveau projet
          </button>
        </div>
      </div>

      <!-- Conversation messages -->
      <div v-else class="messages-list">
        <div
          v-for="turn in conversationTurns"
          :key="turn.id"
          class="turn-group"
          :data-turn-assistant-id="turn.assistantMsgIds.join(' ')"
        >
          <div
            v-for="{ msg, globalIndex: msgIdx } in turn.items"
            :key="msg.id"
            v-memo="[
              msg.id,
              msg.content,
              msg.thinking,
              msg.agentTrace?.length,
              msg.turnDiffs?.length,
              msg.filesSummary?.length,
              msg.processSteps?.length,
              openProcessTimelineIds.has(msg.id),
              openStepDetailIds.size,
              openThinkingIds.has(msg.id),
              openFilesDropdown[msg.id],
              expandedQuestionIds.has(msg.id),
              copiedPromptId === msg.id,
              copiedBlockKey,
            ]"
            class="msg-wrapper"
            :class="msg.role === 'user' ? 'is-user' : 'is-assistant'"
          >
          <!-- USER MESSAGE -->
          <div v-if="msg.role === 'user'" class="user-msg-col">
            <div class="user-bubble-box">
              <div
                class="user-bubble"
                :class="{
                  'is-collapsible': isQuestionCollapsible(msg.id, msg.content),
                  'is-expanded': expandedQuestionIds.has(msg.id),
                  'is-clamped': isQuestionCollapsible(msg.id, msg.content) && !expandedQuestionIds.has(msg.id),
                }"
                :data-user-bubble-id="msg.id"
                :title="isQuestionCollapsible(msg.id, msg.content) ? (expandedQuestionIds.has(msg.id) ? 'Cliquer pour replier' : 'Cliquer pour afficher la question complète') : undefined"
                @click="handleBubbleClick(msg.id, isQuestionCollapsible(msg.id, msg.content), $event)"
              >
                <!-- Attached images in user message -->
                <div
                  v-if="msg.attachments && msg.attachments.some((a) => a.isImage)"
                  class="msg-gallery"
                  @click.stop
                >
                  <img
                    v-for="att in msg.attachments.filter((a) => a.isImage)"
                    :key="att.id"
                    :src="att.dataUrl"
                    :alt="att.name"
                    class="msg-img-thumb"
                    title="Cliquer pour agrandir"
                    @click="previewModalImage = att.dataUrl || null"
                  />
                </div>

                <!-- Attached files chips in user message -->
                <div
                  v-if="msg.attachments && msg.attachments.some((a) => !a.isImage)"
                  class="msg-files"
                  @click.stop
                >
                  <span
                    v-for="att in msg.attachments.filter((a) => !a.isImage)"
                    :key="att.id"
                    class="file-badge"
                  >
                    <svg class="file-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                      <polyline points="14 2 14 8 20 8" />
                    </svg>
                    <span class="file-name">{{ att.name }}</span>
                    <span class="file-size">({{ formatFileSize(att.size) }})</span>
                  </span>
                </div>

                <div
                  class="msg-text-wrapper"
                  :class="{
                    'is-clamped': isQuestionCollapsible(msg.id, msg.content) && !expandedQuestionIds.has(msg.id),
                  }"
                >
                  <p class="msg-text">{{ msg.content }}</p>
                  <div
                    v-if="isQuestionCollapsible(msg.id, msg.content) && !expandedQuestionIds.has(msg.id)"
                    class="bubble-fade-gradient"
                  />
                </div>
              </div>

              <!-- User prompt hover action buttons (copier, relancer) -->
              <div class="user-hover-actions">
                <button
                  class="user-action-pill"
                  :title="copiedPromptId === msg.id ? 'Prompt copié !' : 'Copier le prompt'"
                  @click.stop="copyPrompt(msg.content, msg.id)"
                >
                  <svg
                    v-if="copiedPromptId !== msg.id"
                    class="u-act-icon"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                  >
                    <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
                    <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
                  </svg>
                  <svg
                    v-else
                    class="u-act-icon text-success"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.2"
                  >
                    <polyline points="20 6 9 17 4 12" />
                  </svg>
                </button>

                <button
                  class="user-action-pill"
                  title="Relancer ce prompt"
                  @click.stop="rerunPrompt(msg.content)"
                >
                  <svg class="u-act-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M21 2v6h-6" />
                    <path d="M3 12a9 9 0 0 1 15.5-6.4L21 8" />
                    <path d="M3 22v-6h6" />
                    <path d="M21 12a9 9 0 0 1-15.5 6.4L3 16" />
                  </svg>
                </button>
              </div>
            </div>

            <div class="msg-timestamp is-right">
              Vous · {{ formatRelativeTime(msg.createdAt) }}
            </div>
          </div>

          <!-- ASSISTANT MESSAGE -->
          <article v-else :id="'msg-' + msg.id" :data-msg-id="msg.id" class="assistant-article">
            <!-- AI Thinking Section (presented right under user prompt) -->
            <div
              v-if="msg.thinking || (msg.agentTrace && msg.agentTrace.length > 0)"
              :id="'thinking-block-' + msg.id"
              class="thinking-block"
            >
              <button
                type="button"
                class="thinking-header"
                :title="openThinkingIds.has(msg.id) ? 'Replier la réflexion' : 'Déplier la réflexion'"
                @click="toggleThinking(msg.id)"
              >
                <div class="thinking-title">
                  <svg class="brain-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
                    <path d="M12 2a5 5 0 0 1 5 5c0 .7-.1 1.4-.4 2a5 5 0 0 1 3.4 5.8 4.5 4.5 0 0 1-2 7.2A4.5 4.5 0 0 1 12 22a4.5 4.5 0 0 1-6 0 4.5 4.5 0 0 1-2-7.2A5 5 0 0 1 7.4 9 5 5 0 0 1 12 2z" />
                  </svg>
                  <span>Réflexion de l'IA</span>
                </div>
                <div class="thinking-meta">
                  <div
                    v-if="msg.agentTrace && msg.agentTrace.length > 0"
                    class="trace-dots-preview"
                    :title="msg.agentTrace.map((s) => `${s.roleLabel}: ${providerShortName(s.provider)} · ${s.model}`).join('\n')"
                  >
                    <span
                      v-for="(st, sIdx) in msg.agentTrace"
                      :key="sIdx"
                      class="trace-preview-dot"
                      :style="{ backgroundColor: providerDotColor(st.provider) }"
                    />
                  </div>
                  <span
                    v-if="msg.agentTrace && msg.agentTrace.length > 0"
                    class="trace-steps-count"
                  >
                    {{ msg.agentTrace.length }} étape{{ msg.agentTrace.length > 1 ? "s" : "" }}
                  </span>
                  <span class="thinking-badge done">Terminée</span>
                  <svg
                    class="chevron-icon"
                    :class="{ rotated: openThinkingIds.has(msg.id) }"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                </div>
              </button>
              <div v-if="openThinkingIds.has(msg.id)" class="thinking-body custom-scroll">
                <div
                  v-if="msg.agentTrace && msg.agentTrace.length > 0"
                  class="trace-tree-container"
                >
                  <div
                    v-for="grp in groupTraceSteps(msg.agentTrace)"
                    :key="grp.id"
                    class="trace-group-node"
                    :class="{
                      'is-wave': grp.kind === 'wave',
                      'is-parallel-wave': grp.isParallel,
                      'is-phase': grp.kind === 'phase'
                    }"
                  >
                    <!-- EN-TÊTE DE VAGUE (DAG Wave) -->
                    <div v-if="grp.kind === 'wave'" class="dag-wave-header">
                      <span
                        class="dag-wave-badge"
                        :class="grp.isParallel ? 'badge-parallel' : 'badge-sequential'"
                      >
                        <span class="wave-icon">{{ grp.isParallel ? '⚡' : '▶' }}</span>
                        <span class="wave-text">{{ grp.label }}</span>
                      </span>
                      <span v-if="grp.isParallel" class="wave-tag-parallel">Simultané sans conflit</span>
                    </div>

                    <!-- ARBORESCENCE DES BRANCHES -->
                    <div :class="grp.kind === 'wave' ? 'dag-branch-tree' : 'dag-phase-wrap'">
                      <div
                        v-for="(step, sIdx) in grp.steps"
                        :key="`${step.role}-${sIdx}`"
                        class="dag-tree-item"
                        :class="{ 'is-last-branch': sIdx === grp.steps.length - 1 }"
                      >
                        <!-- CARTE DE LA TÂCHE -->
                        <div
                          class="trace-step-card"
                          :class="{ 'in-parallel': grp.isParallel }"
                          :style="{ '--step-role-color': roleTraceDotColor(step.role) }"
                        >
                          <div class="trace-step-top">
                            <div class="trace-role-wrap">
                              <span class="trace-role-emoji">{{ roleIcon(step.role) }}</span>
                              <span
                                class="trace-role-dot"
                                :style="{ backgroundColor: roleTraceDotColor(step.role) }"
                              />
                              <strong class="trace-role-label">{{ step.roleLabel }}</strong>
                              <span
                                v-for="facet in getCoderTraceFacets(step, msg)"
                                :key="facet.key"
                                class="trace-facet-badge"
                                :class="`is-${facet.key}`"
                              >
                                <span>{{ facet.icon }}</span>
                                <span>{{ facet.label }}</span>
                              </span>
                            </div>
                            <span class="trace-model-pill" :title="`${providerShortName(step.provider)} : ${step.model}`">
                              <span
                                class="trace-prov-dot"
                                :style="{ backgroundColor: providerDotColor(step.provider) }"
                              />
                              <span class="trace-prov-name">{{ providerShortName(step.provider) }}</span>
                              <span class="trace-model-name">{{ step.model }}</span>
                            </span>
                            <div class="trace-step-right">
                              <span v-if="step.tokens && step.tokens > 0" class="trace-tok-pill">
                                {{ formatCompactTok(step.tokens) }} tok
                              </span>
                              <span
                                class="trace-status"
                                :class="`is-${step.status}`"
                                :title="step.status"
                              >
                                <span v-if="step.status === 'running'" class="pulse-dot" />
                                <template v-else-if="step.status === 'done'">✓</template>
                                <template v-else-if="step.status === 'fallback'">⚠</template>
                                <template v-else>✕</template>
                              </span>
                            </div>
                          </div>
                          <div class="trace-step-summary">
                            <MarkdownInline
                              :tokens="tokenizeInline(stripFacetTags(step.summary))"
                              :available-paths="getAvailableDiffPaths(msg)"
                              :project-root="store.activeProject?.rootPath"
                              variant="thinking"
                              @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                            />
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>

                <div v-if="msg.thinking" class="thinking-md">
                  <template
                    v-for="(block, tbIdx) in parseMarkdownMessage(msg.thinking)"
                    :key="tbIdx"
                  >
                    <h1 v-if="block.kind === 'heading' && block.level === 1" class="tmd-h1">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                      />
                    </h1>
                    <h2 v-else-if="block.kind === 'heading' && block.level === 2" class="tmd-h2">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                      />
                    </h2>
                    <h3 v-else-if="block.kind === 'heading' && block.level === 3" class="tmd-h3">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                      />
                    </h3>
                    <blockquote v-else-if="block.kind === 'quote'" class="tmd-quote">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                      />
                    </blockquote>
                    <ul v-else-if="block.kind === 'ul'" class="tmd-ul">
                      <li
                        v-for="(item, liIdx) in block.items"
                        :key="liIdx"
                        :style="{ marginLeft: `${item.indent * 14}px` }"
                      >
                        <MarkdownInline
                          :tokens="item.tokens"
                          :available-paths="getAvailableDiffPaths(msg)"
                          :project-root="store.activeProject?.rootPath"
                          variant="thinking"
                          @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                        />
                      </li>
                    </ul>
                    <ol v-else-if="block.kind === 'ol'" class="tmd-ol">
                      <li v-for="(item, liIdx) in block.items" :key="liIdx">
                        <MarkdownInline
                          :tokens="item"
                          :available-paths="getAvailableDiffPaths(msg)"
                          :project-root="store.activeProject?.rootPath"
                          variant="thinking"
                          @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                        />
                      </li>
                    </ol>
                    <pre v-else-if="block.kind === 'code'" class="tmd-code-block custom-scroll"><code class="hljs" v-html="renderCodeHtml(block.body, block.lang, Boolean(block.streaming))" /></pre>
                    <div v-else-if="block.kind === 'table'" class="tmd-table-wrapper custom-scroll">
                      <table class="tmd-table">
                        <thead>
                          <tr>
                            <th
                              v-for="(h, thIdx) in block.headers"
                              :key="thIdx"
                              :style="{ textAlign: block.alignments?.[thIdx] || 'left' }"
                            >
                              <MarkdownInline
                                :tokens="h"
                                :available-paths="getAvailableDiffPaths()"
                                :project-root="store.activeProject?.rootPath"
                                variant="thinking"
                                @open-file="handleOpenFileInReview"
                              />
                            </th>
                          </tr>
                        </thead>
                        <tbody>
                          <tr v-for="(row, trIdx) in block.rows" :key="trIdx">
                            <td
                              v-for="(cell, tdIdx) in row"
                              :key="tdIdx"
                              :style="{ textAlign: block.alignments?.[tdIdx] || 'left' }"
                            >
                              <MarkdownInline
                                :tokens="cell"
                                :available-paths="getAvailableDiffPaths()"
                                :project-root="store.activeProject?.rootPath"
                                variant="thinking"
                                @open-file="handleOpenFileInReview"
                              />
                            </td>
                          </tr>
                        </tbody>
                      </table>
                    </div>
                    <p v-else-if="block.kind === 'paragraph'" class="tmd-p">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                      />
                    </p>
                  </template>
                </div>
              </div>
            </div>

            <!-- Process Timeline Dropdown -->
            <div
              v-if="getMessageProcessSteps(msg).length > 0"
              :id="'process-timeline-' + msg.id"
              class="process-timeline-block halo-hud"
              :class="{ 'is-collapsed': !isProcessTimelineOpen(msg.id) }"
            >
              <button
                type="button"
                class="process-timeline-header"
                :title="isProcessTimelineOpen(msg.id) ? 'Replier la timeline des actions' : 'Déplier la timeline des actions'"
                @click="toggleProcessTimeline(msg.id)"
              >
                <div class="timeline-header-left">
                  <div
                    class="timeline-status-icon-wrap"
                    :class="hasProcessError(getMessageProcessSteps(msg)) ? 'has-error' : 'is-success'"
                  >
                    <svg class="timeline-bolt-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
                    </svg>
                  </div>
                  <div class="timeline-header-info">
                    <div class="timeline-title-row">
                      <span class="timeline-main-title">Process Timeline</span>
                      <span class="timeline-steps-badge">
                        {{ getMessageProcessSteps(msg).length }} action{{ getMessageProcessSteps(msg).length > 1 ? 's' : '' }}
                      </span>
                    </div>
                    <div class="timeline-sub-title">
                      {{ formatProcessSubtitle(getMessageProcessSteps(msg)) }}
                    </div>
                  </div>
                </div>

                <div class="timeline-header-right">
                  <span class="timeline-stat-tag is-success" title="Actions réussies">
                    <svg class="stat-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>
                    {{ countProcessSuccess(getMessageProcessSteps(msg)) }}
                  </span>
                  <span
                    v-if="countProcessError(getMessageProcessSteps(msg)) > 0"
                    class="timeline-stat-tag is-error"
                    title="Actions en échec"
                  >
                    <svg class="stat-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                    {{ countProcessError(getMessageProcessSteps(msg)) }}
                  </span>
                  <span v-if="getProcessTotalDuration(getMessageProcessSteps(msg))" class="timeline-stat-duration">
                    {{ getProcessTotalDuration(getMessageProcessSteps(msg)) }}
                  </span>
                  <svg
                    class="timeline-chevron-icon"
                    :class="{ rotated: isProcessTimelineOpen(msg.id) }"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                </div>
              </button>

              <!-- Corps déroulant de la timeline -->
              <div v-if="isProcessTimelineOpen(msg.id)" class="process-timeline-body custom-scroll">
                <div class="timeline-track-rail" />
                <div
                  v-for="(step, sIdx) in getMessageProcessSteps(msg)"
                  :key="step.id || sIdx"
                  class="timeline-step-row"
                >
                  <!-- Puce de statut -->
                  <div class="timeline-step-bullet" :class="`is-${step.status}`">
                    <span v-if="step.status === 'running'" class="pulse-dot" />
                    <svg v-else-if="step.status === 'success'" class="bullet-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>
                    <svg v-else class="bullet-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                  </div>

                  <!-- Carte de l'étape -->
                  <div class="timeline-step-card" :class="`is-${step.status}`">
                    <div class="step-card-header">
                      <div class="step-card-left">
                        <span class="step-tool-badge" :class="`is-${getProcessToolCategory(step.tool)}`">
                          <svg v-if="getProcessToolCategory(step.tool) === 'terminal'" class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>
                          <svg v-else-if="getProcessToolCategory(step.tool) === 'create' || getProcessToolCategory(step.tool) === 'file'" class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
                          <svg v-else-if="getProcessToolCategory(step.tool) === 'edit'" class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>
                          <svg v-else-if="getProcessToolCategory(step.tool) === 'search'" class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
                          <svg v-else class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/></svg>
                          {{ step.tool }}
                        </span>
                        <span class="step-card-title font-code" :title="step.title">{{ step.title }}</span>
                        <span
                          v-for="facet in getProcessStepFacets(step)"
                          :key="facet.key"
                          class="trace-facet-badge is-mini"
                          :class="`is-${facet.key}`"
                        >
                          {{ facet.label }}
                        </span>
                        <span v-if="step.status === 'error'" class="step-error-tag">Échec</span>
                      </div>
                      <div class="step-card-right">
                        <span v-if="step.durationMs" class="step-duration font-code">+{{ formatStepDuration(step.durationMs) }}</span>
                        <button
                          v-if="step.detail"
                          type="button"
                          class="step-log-btn"
                          @click="toggleStepDetail(msg.id, step.id)"
                        >
                          <span>{{ isStepDetailOpen(msg.id, step.id) ? 'Masquer log' : 'Détail log' }}</span>
                          <svg class="log-chevron" :class="{ rotated: isStepDetailOpen(msg.id, step.id) }" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="6 9 12 15 18 9"/></svg>
                        </button>
                      </div>
                    </div>
                    <!-- Détail / Log repliable de l'étape -->
                    <div v-if="step.detail && isStepDetailOpen(msg.id, step.id)" class="step-log-panel custom-scroll">
                      <pre class="step-log-pre font-code">{{ step.detail }}</pre>
                    </div>
                  </div>
                </div>
              </div>
            </div>

            <!-- Markdown Content Result -->
            <div class="md-content">
              <template
                v-for="(block, bIdx) in parseMarkdownMessage(msg.content)"
                :key="bIdx"
              >
                <h1 v-if="block.kind === 'heading' && block.level === 1" class="md-h1">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                  />
                </h1>

                <h2 v-else-if="block.kind === 'heading' && block.level === 2" class="md-h2">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                  />
                </h2>

                <h3 v-else-if="block.kind === 'heading' && block.level === 3" class="md-h3">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                  />
                </h3>

                <blockquote v-else-if="block.kind === 'quote'" class="md-quote">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                  />
                </blockquote>

                <ul v-else-if="block.kind === 'ul'" class="md-ul">
                  <li
                    v-for="(item, liIdx) in block.items"
                    :key="liIdx"
                    :style="{ marginLeft: `${item.indent * 16}px` }"
                  >
                    <MarkdownInline
                      :tokens="item.tokens"
                      :available-paths="getAvailableDiffPaths(msg)"
                      :project-root="store.activeProject?.rootPath"
                      @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                    />
                  </li>
                </ul>

                <ol v-else-if="block.kind === 'ol'" class="md-ol">
                  <li v-for="(item, liIdx) in block.items" :key="liIdx">
                    <MarkdownInline
                      :tokens="item"
                      :available-paths="getAvailableDiffPaths(msg)"
                      :project-root="store.activeProject?.rootPath"
                      @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                    />
                  </li>
                </ol>

                <div
                  v-else-if="block.kind === 'code'"
                  class="msg-code-block halo-hud"
                >
                  <div class="code-toolbar">
                    <div class="code-toolbar-info">
                      <span class="code-lang">{{ block.lang }}</span>
                      <span v-if="block.filename" class="code-filename-pill">{{ block.filename }}</span>
                    </div>
                    <div class="code-toolbar-actions">
                      <!-- Bouton Télécharger le code (icône Remix download) -->
                      <button
                        class="code-btn-icon"
                        title="Télécharger le code"
                        @click="downloadCodeBlock(block.body, block.filename || guessFilename(block.lang, block.body))"
                      >
                        <svg class="code-icon" viewBox="0 0 24 24" fill="currentColor">
                          <path d="M3 19H21V21H3V19ZM13 9H18L12 16L6 9H11V3H13V9Z" />
                        </svg>
                      </button>

                      <!-- Bouton Copier le code (icône seule agrandie) -->
                      <button
                        class="code-btn-icon"
                        :class="{ copied: copiedBlockKey === `${msg.id}-${bIdx}` }"
                        title="Copier le code"
                        @click="copyCodeBlock(`${msg.id}-${bIdx}`, block.body)"
                      >
                        <svg
                          v-if="copiedBlockKey !== `${msg.id}-${bIdx}`"
                          class="code-icon"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                        >
                          <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
                          <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
                        </svg>
                        <svg
                          v-else
                          class="code-icon text-success"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2.3"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                        >
                          <polyline points="20 6 9 17 4 12" />
                        </svg>
                      </button>
                    </div>
                  </div>
                  <pre class="code-pre custom-scroll"><code class="hljs" v-html="renderCodeHtml(block.body, block.lang)" /></pre>
                </div>

                <div v-else-if="block.kind === 'table'" class="md-table-wrapper custom-scroll">
                  <table class="md-table">
                    <thead>
                      <tr>
                        <th
                          v-for="(h, thIdx) in block.headers"
                          :key="thIdx"
                          :style="{ textAlign: block.alignments?.[thIdx] || 'left' }"
                        >
                          <MarkdownInline
                            :tokens="h"
                            :available-paths="getAvailableDiffPaths(msg)"
                            :project-root="store.activeProject?.rootPath"
                            @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                          />
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      <tr v-for="(row, trIdx) in block.rows" :key="trIdx">
                        <td
                          v-for="(cell, tdIdx) in row"
                          :key="tdIdx"
                          :style="{ textAlign: block.alignments?.[tdIdx] || 'left' }"
                        >
                          <MarkdownInline
                            :tokens="cell"
                            :available-paths="getAvailableDiffPaths(msg)"
                            :project-root="store.activeProject?.rootPath"
                            @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                          />
                        </td>
                      </tr>
                    </tbody>
                  </table>
                </div>

                <p v-else-if="block.kind === 'paragraph'" class="md-p">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="(p) => handleOpenFileInReview(p, msg.id)"
                  />
                </p>
              </template>
            </div>

            <!-- Dropdown Sélecteur des fichiers modifiés avec diff -->
            <div v-if="messageDiffStats(msg, msgIdx)" class="msg-footer-actions-bar">
              <div
                :id="'diff-dropdown-' + msg.id"
                class="diff-dropdown-wrap"
                :class="{ 'is-open': openFilesDropdown[msg.id] }"
              >
              <button
                type="button"
                class="diff-dropdown-trigger"
                :title="openFilesDropdown[msg.id] ? 'Masquer la liste des fichiers modifiés' : 'Afficher les fichiers modifiés et leurs modifications'"
                @click="toggleFilesDropdown(msg.id)"
              >
                <div class="diff-trigger-left">
                  <svg class="diff-git-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <circle cx="18" cy="18" r="3" />
                    <circle cx="6" cy="6" r="3" />
                    <path d="M6 9v12" />
                    <path d="M18 9a9 9 0 0 1-9 9" />
                  </svg>
                  <span class="diff-trigger-title">
                    {{ messageDiffStats(msg, msgIdx)!.filesChanged }}
                    fichier{{ messageDiffStats(msg, msgIdx)!.filesChanged > 1 ? "s" : "" }} modifié{{ messageDiffStats(msg, msgIdx)!.filesChanged > 1 ? "s" : "" }}
                  </span>
                </div>

                <div class="diff-trigger-right">
                  <span class="diff-badge-add">+{{ messageDiffStats(msg, msgIdx)!.linesAdded }}</span>
                  <span class="diff-badge-del">-{{ messageDiffStats(msg, msgIdx)!.linesRemoved }}</span>
                  <svg
                    class="diff-chevron-arrow"
                    :class="{ rotated: openFilesDropdown[msg.id] }"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.2"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                </div>
              </button>

              <!-- Menu déroulant Dropdown -->
              <div v-if="openFilesDropdown[msg.id]" class="diff-dropdown-menu halo-hud">
                <div class="diff-menu-header">
                  <div class="diff-menu-header-left">
                    <span class="diff-menu-header-text">Fichiers modifiés</span>
                    <div class="diff-menu-stats-row">
                      <span v-if="getFilesBreakdown(msg, msgIdx).created > 0" class="badge-mini-status tag-created">
                        +{{ getFilesBreakdown(msg, msgIdx).created }} créé{{ getFilesBreakdown(msg, msgIdx).created > 1 ? "s" : "" }}
                      </span>
                      <span v-if="getFilesBreakdown(msg, msgIdx).modified > 0" class="badge-mini-status tag-modified">
                        ~{{ getFilesBreakdown(msg, msgIdx).modified }} modifié{{ getFilesBreakdown(msg, msgIdx).modified > 1 ? "s" : "" }}
                      </span>
                      <span v-if="getFilesBreakdown(msg, msgIdx).deleted > 0" class="badge-mini-status tag-deleted">
                        -{{ getFilesBreakdown(msg, msgIdx).deleted }} supprimé{{ getFilesBreakdown(msg, msgIdx).deleted > 1 ? "s" : "" }}
                      </span>
                    </div>
                  </div>
                  <button
                    v-if="canUndoMessageFiles(msgIdx)"
                    type="button"
                    class="diff-undo-all-action-btn"
                    title="Annuler toutes les modifications apportées aux fichiers sur cette étape"
                    @click.stop="confirmUndoAll(msg.id)"
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M5.82843 7.00005L8.36396 9.53558L6.94975 10.9498L2 6.00005L6.94975 1.05031L8.36396 2.46452L5.82843 5.00005H13C16.866 5.00005 20 8.13406 20 12C20 15.866 16.866 19 13 19H4V17H13C15.7614 17 18 14.7615 18 12C18 9.23862 15.7614 7.00005 13 7.00005H5.82843Z"/>
                    </svg>
                    <span>Tout défaire</span>
                  </button>
                </div>

                <div v-if="getFileListForMessage(msg, msgIdx).length === 0" class="diff-menu-empty">
                  Aucun détail de fichier spécifique pour ce message.
                </div>
                <div v-else class="diff-menu-list custom-scroll">
                  <div
                    v-for="file in getFileListForMessage(msg, msgIdx)"
                    :key="file.path"
                    class="diff-menu-row"
                    title="Cliquer pour afficher le diff de cette étape"
                    @click="store.openReview(file.path, null, msg.id)"
                  >
                    <span class="menu-state-badge" :class="getFileStatus(file)">
                      {{ getFileStatusLabel(file) }}
                    </span>
                    <span class="menu-file-path">{{ file.path }}</span>
                    <div class="menu-file-actions">
                      <div class="menu-file-delta">
                        <span v-if="file.modified > 0" class="c-mod">~{{ file.modified }}</span>
                        <span v-if="file.added > 0" class="c-add">+{{ file.added }}</span>
                        <span v-if="file.removed > 0" class="c-del">-{{ file.removed }}</span>
                      </div>
                      <button
                        v-if="canUndoMessageFiles(msgIdx)"
                        type="button"
                        class="menu-file-undo-btn"
                        :title="`Annuler les modifications de ${file.path} pour cette étape`"
                        @click.stop="confirmUndoFile(file.path, msg.id)"
                      >
                        <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor">
                          <path d="M5.82843 7.00005L8.36396 9.53558L6.94975 10.9498L2 6.00005L6.94975 1.05031L8.36396 2.46452L5.82843 5.00005H13C16.866 5.00005 20 8.13406 20 12C20 15.866 16.866 19 13 19H4V17H13C15.7614 17 18 14.7615 18 12C18 9.23862 15.7614 7.00005 13 7.00005H5.82843Z"/>
                        </svg>
                      </button>
                    </div>
                  </div>
                </div>

                <div class="diff-menu-footer">
                  <button
                    type="button"
                    class="diff-full-review-cta"
                    @click="store.openReview(getFileListForMessage(msg, msgIdx)[0]?.path, null, msg.id)"
                  >
                    <span>Ouvrir l'éditeur de review complet</span>
                    <svg class="cta-arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <line x1="5" y1="12" x2="19" y2="12" />
                      <polyline points="12 5 19 12 12 19" />
                    </svg>
                  </button>
                </div>
              </div>
            </div>
          </div>

            <!-- Relative Timestamp -->
            <div class="msg-timestamp">
              Studio ({{ msg.stageSummary || store.settings.ollamaModel }}) · {{ formatRelativeTime(msg.createdAt) }}
            </div>
          </article>
        </div>
      </div>

        <!-- LIVE STREAMING CARD (Displayed while IA is processing) -->
        <div v-if="store.isProcessing" class="msg-wrapper is-assistant is-live">
          <article class="assistant-article live-article">
            <!-- Live Thinking block avec Flux Moteur intégré -->
            <div class="thinking-block live-thinking">
              <div class="thinking-header is-live">
                <div class="thinking-title">
                  <svg class="brain-icon pulsing" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
                    <path d="M12 2a5 5 0 0 1 5 5c0 .7-.1 1.4-.4 2a5 5 0 0 1 3.4 5.8 4.5 4.5 0 0 1-2 7.2A4.5 4.5 0 0 1 12 22a4.5 4.5 0 0 1-6 0 4.5 4.5 0 0 1-2-7.2A5 5 0 0 1 7.4 9 5 5 0 0 1 12 2z" />
                  </svg>
                  <span>Activité de l'IA</span>
                </div>

                <!-- Onglets Réflexion / Flux Moteur intégrés sous le prompt -->
                <div class="thinking-tabs-group">
                  <button
                    type="button"
                    class="live-tab-pill"
                    :class="{ active: liveTab === 'thinking' }"
                    @click="liveTab = 'thinking'"
                  >
                    Réflexion
                  </button>
                  <button
                    type="button"
                    class="live-tab-pill"
                    :class="{ active: liveTab === 'stream' }"
                    @click="liveTab = 'stream'"
                  >
                    Flux Moteur
                    <span v-if="store.liveStreamLines.length > 0" class="stream-count-dot">
                      {{ store.liveStreamLines.length }}
                    </span>
                  </button>
                  <span class="thinking-badge running">
                    <span class="pulse-dot" />
                    {{ store.currentStage?.stageLabel || "En cours..." }}
                  </span>
                  <button
                    type="button"
                    class="live-stop-pill"
                    :disabled="store.isCancelling"
                    title="Arrêter le traitement en cours"
                    @click="store.cancelPrompt()"
                  >
                    ■ {{ store.isCancelling ? "Arrêt..." : "Stop" }}
                  </button>
                </div>
              </div>

              <div class="thinking-body live-body custom-scroll">
                <!-- Vue Réflexion : Trace live + Markdown -->
                <template v-if="liveTab === 'thinking'">
                  <div
                    v-if="store.liveAgentTrace.length > 0"
                    class="trace-tree-container"
                  >
                    <div
                      v-for="grp in groupTraceSteps(store.liveAgentTrace)"
                      :key="grp.id"
                      class="trace-group-node"
                      :class="{
                        'is-wave': grp.kind === 'wave',
                        'is-parallel-wave': grp.isParallel,
                        'is-phase': grp.kind === 'phase'
                      }"
                    >
                      <!-- EN-TÊTE DE VAGUE (DAG Wave) -->
                      <div v-if="grp.kind === 'wave'" class="dag-wave-header">
                        <span
                          class="dag-wave-badge"
                          :class="grp.isParallel ? 'badge-parallel' : 'badge-sequential'"
                        >
                          <span class="wave-icon">{{ grp.isParallel ? '⚡' : '▶' }}</span>
                          <span class="wave-text">{{ grp.label }}</span>
                        </span>
                        <span v-if="grp.isParallel" class="wave-tag-parallel">Simultané sans conflit</span>
                      </div>

                      <!-- ARBORESCENCE DES BRANCHES -->
                      <div :class="grp.kind === 'wave' ? 'dag-branch-tree' : 'dag-phase-wrap'">
                        <div
                          v-for="(step, sIdx) in grp.steps"
                          :key="`live-${step.role}-${sIdx}`"
                          class="dag-tree-item"
                          :class="{ 'is-last-branch': sIdx === grp.steps.length - 1 }"
                        >
                          <!-- CARTE DE LA TÂCHE -->
                          <div
                            class="trace-step-card"
                            :class="{ 'in-parallel': grp.isParallel }"
                            :style="{ '--step-role-color': roleTraceDotColor(step.role) }"
                          >
                            <div class="trace-step-top">
                              <div class="trace-role-wrap">
                                <span class="trace-role-emoji">{{ roleIcon(step.role) }}</span>
                                <span
                                  class="trace-role-dot"
                                  :style="{ backgroundColor: roleTraceDotColor(step.role) }"
                                />
                                <strong class="trace-role-label">{{ step.roleLabel }}</strong>
                                <span
                                  v-for="facet in getCoderTraceFacets(step)"
                                  :key="facet.key"
                                  class="trace-facet-badge"
                                  :class="`is-${facet.key}`"
                                >
                                  <span>{{ facet.icon }}</span>
                                  <span>{{ facet.label }}</span>
                                </span>
                              </div>
                              <span class="trace-model-pill" :title="`${providerShortName(step.provider)} : ${step.model}`">
                                <span
                                  class="trace-prov-dot"
                                  :style="{ backgroundColor: providerDotColor(step.provider) }"
                                />
                                <span class="trace-prov-name">{{ providerShortName(step.provider) }}</span>
                                <span class="trace-model-name">{{ step.model }}</span>
                              </span>
                              <div class="trace-step-right">
                                <span v-if="step.tokens && step.tokens > 0" class="trace-tok-pill">
                                  {{ formatCompactTok(step.tokens) }} tok
                                </span>
                                <span
                                  class="trace-status"
                                  :class="`is-${step.status}`"
                                  :title="step.status"
                                >
                                  <span v-if="step.status === 'running'" class="pulse-dot" />
                                  <template v-else-if="step.status === 'done'">✓</template>
                                  <template v-else-if="step.status === 'fallback'">⚠</template>
                                  <template v-else>✕</template>
                                </span>
                              </div>
                            </div>
                            <div class="trace-step-summary">
                              <MarkdownInline
                                :tokens="tokenizeInline(stripFacetTags(step.summary))"
                                :available-paths="getAvailableDiffPaths()"
                                :project-root="store.activeProject?.rootPath"
                                variant="thinking"
                                @open-file="handleOpenFileInReview"
                              />
                            </div>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div
                    v-if="store.liveThinking || store.liveAgentTrace.length === 0"
                    class="thinking-md"
                  >
                    <template
                      v-for="(block, lbIdx) in parseMarkdownMessage(
                        store.liveThinking ||
                          'Analyse du prompt, sélection des outils et préparation du code...',
                        Boolean(store.liveThinking)
                      )"
                      :key="lbIdx"
                    >
                    <h1 v-if="block.kind === 'heading' && block.level === 1" class="tmd-h1">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths()"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
                    </h1>
                    <h2 v-else-if="block.kind === 'heading' && block.level === 2" class="tmd-h2">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths()"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
                    </h2>
                    <h3 v-else-if="block.kind === 'heading' && block.level === 3" class="tmd-h3">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths()"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
                    </h3>
                    <blockquote v-else-if="block.kind === 'quote'" class="tmd-quote">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths()"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
                    </blockquote>
                    <ul v-else-if="block.kind === 'ul'" class="tmd-ul">
                      <li
                        v-for="(item, liIdx) in block.items"
                        :key="liIdx"
                        :style="{ marginLeft: `${item.indent * 14}px` }"
                      >
                        <MarkdownInline
                          :tokens="item.tokens"
                          :available-paths="getAvailableDiffPaths()"
                          :project-root="store.activeProject?.rootPath"
                          variant="thinking"
                          @open-file="handleOpenFileInReview"
                        />
                      </li>
                    </ul>
                    <ol v-else-if="block.kind === 'ol'" class="tmd-ol">
                      <li v-for="(item, liIdx) in block.items" :key="liIdx">
                        <MarkdownInline
                          :tokens="item"
                          :available-paths="getAvailableDiffPaths()"
                          :project-root="store.activeProject?.rootPath"
                          variant="thinking"
                          @open-file="handleOpenFileInReview"
                        />
                      </li>
                    </ol>
                    <pre v-else-if="block.kind === 'code'" class="tmd-code-block custom-scroll"><code class="hljs" v-html="renderCodeHtml(block.body, block.lang, Boolean(block.streaming))" /></pre>
                    <div v-else-if="block.kind === 'table'" class="tmd-table-wrapper custom-scroll">
                      <table class="tmd-table">
                        <thead>
                          <tr>
                            <th
                              v-for="(h, thIdx) in block.headers"
                              :key="thIdx"
                              :style="{ textAlign: block.alignments?.[thIdx] || 'left' }"
                            >
                              <MarkdownInline
                                :tokens="h"
                                :available-paths="getAvailableDiffPaths()"
                                :project-root="store.activeProject?.rootPath"
                                variant="thinking"
                                @open-file="handleOpenFileInReview"
                              />
                            </th>
                          </tr>
                        </thead>
                        <tbody>
                          <tr v-for="(row, trIdx) in block.rows" :key="trIdx">
                            <td
                              v-for="(cell, tdIdx) in row"
                              :key="tdIdx"
                              :style="{ textAlign: block.alignments?.[tdIdx] || 'left' }"
                            >
                              <MarkdownInline
                                :tokens="cell"
                                :available-paths="getAvailableDiffPaths()"
                                :project-root="store.activeProject?.rootPath"
                                variant="thinking"
                                @open-file="handleOpenFileInReview"
                              />
                            </td>
                          </tr>
                        </tbody>
                      </table>
                    </div>
                    <p v-else-if="block.kind === 'paragraph'" class="tmd-p">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths()"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
                    </p>
                  </template>
                  </div>
                </template>

                <!-- Vue Flux Moteur intégrée -->
                <pre
                  v-else
                  class="stream-log-live custom-scroll"
                >{{ store.liveStreamLines.length > 0 ? store.liveStreamLines.join("\n") : "En attente du flux moteur..." }}</pre>
              </div>
            </div>

            <!-- Live Process Timeline Dropdown -->
            <div
              v-if="store.liveProcessSteps.length > 0"
              class="process-timeline-block is-live halo-hud"
              :class="{ 'is-collapsed': !isLiveProcessTimelineOpen }"
            >
              <button
                type="button"
                class="process-timeline-header"
                :title="isLiveProcessTimelineOpen ? 'Replier la timeline' : 'Déplier la timeline'"
                @click="toggleLiveProcessTimeline()"
              >
                <div class="timeline-header-left">
                  <div
                    class="timeline-status-icon-wrap"
                    :class="hasProcessError(store.liveProcessSteps) ? 'has-error' : 'is-running'"
                  >
                    <svg class="timeline-bolt-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
                    </svg>
                  </div>
                  <div class="timeline-header-info">
                    <div class="timeline-title-row">
                      <span class="timeline-main-title">Process Timeline</span>
                      <span class="timeline-steps-badge">
                        {{ store.liveProcessSteps.length }} action{{ store.liveProcessSteps.length > 1 ? 's' : '' }}
                      </span>
                      <span class="timeline-live-tag">
                        <span class="pulse-dot" />
                        {{ getLiveTimelineBadgeLabel(store.liveProcessSteps) }}
                      </span>
                    </div>
                    <div class="timeline-sub-title">
                      {{ formatProcessSubtitle(store.liveProcessSteps) }}
                    </div>
                  </div>
                </div>

                <div class="timeline-header-right">
                  <span class="timeline-stat-tag is-success" title="Actions réussies">
                    <svg class="stat-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>
                    {{ countProcessSuccess(store.liveProcessSteps) }}
                  </span>
                  <span
                    v-if="countProcessError(store.liveProcessSteps) > 0"
                    class="timeline-stat-tag is-error"
                    title="Actions en échec"
                  >
                    <svg class="stat-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                    {{ countProcessError(store.liveProcessSteps) }}
                  </span>
                  <span v-if="getProcessTotalDuration(store.liveProcessSteps)" class="timeline-stat-duration">
                    {{ getProcessTotalDuration(store.liveProcessSteps) }}
                  </span>
                  <svg
                    class="timeline-chevron-icon"
                    :class="{ rotated: isLiveProcessTimelineOpen }"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                </div>
              </button>

              <!-- Corps déroulant de la timeline en direct -->
              <div v-if="isLiveProcessTimelineOpen" class="process-timeline-body custom-scroll">
                <div class="timeline-track-rail" />
                <div
                  v-for="(step, sIdx) in store.liveProcessSteps"
                  :key="'live-' + (step.id || sIdx)"
                  class="timeline-step-row"
                >
                  <!-- Puce de statut -->
                  <div class="timeline-step-bullet" :class="`is-${step.status}`">
                    <span v-if="step.status === 'running'" class="pulse-dot" />
                    <svg v-else-if="step.status === 'success'" class="bullet-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="20 6 9 17 4 12"/></svg>
                    <svg v-else class="bullet-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                  </div>

                  <!-- Carte de l'étape -->
                  <div class="timeline-step-card" :class="`is-${step.status}`">
                    <div class="step-card-header">
                      <div class="step-card-left">
                        <span class="step-tool-badge" :class="`is-${getProcessToolCategory(step.tool)}`">
                          <svg v-if="getProcessToolCategory(step.tool) === 'terminal'" class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>
                          <svg v-else-if="getProcessToolCategory(step.tool) === 'create' || getProcessToolCategory(step.tool) === 'file'" class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
                          <svg v-else-if="getProcessToolCategory(step.tool) === 'edit'" class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>
                          <svg v-else-if="getProcessToolCategory(step.tool) === 'search'" class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
                          <svg v-else class="tool-mini-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/></svg>
                          {{ step.tool }}
                        </span>
                        <span class="step-card-title font-code" :title="step.title">{{ step.title }}</span>
                        <span
                          v-for="facet in getProcessStepFacets(step)"
                          :key="facet.key"
                          class="trace-facet-badge is-mini"
                          :class="`is-${facet.key}`"
                        >
                          {{ facet.label }}
                        </span>
                        <span v-if="step.status === 'error'" class="step-error-tag">Échec</span>
                        <span v-else-if="step.status === 'running'" class="step-running-tag">En cours...</span>
                      </div>
                      <div class="step-card-right">
                        <span v-if="step.durationMs" class="step-duration font-code">+{{ formatStepDuration(step.durationMs) }}</span>
                        <button
                          v-if="step.detail"
                          type="button"
                          class="step-log-btn"
                          @click="toggleStepDetail('live', step.id)"
                        >
                          <span>{{ isStepDetailOpen('live', step.id) ? 'Masquer log' : 'Détail log' }}</span>
                          <svg class="log-chevron" :class="{ rotated: isStepDetailOpen('live', step.id) }" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="6 9 12 15 18 9"/></svg>
                        </button>
                      </div>
                    </div>
                    <!-- Détail / Log repliable de l'étape -->
                    <div v-if="step.detail && isStepDetailOpen('live', step.id)" class="step-log-panel custom-scroll">
                      <pre class="step-log-pre font-code">{{ step.detail }}</pre>
                    </div>
                  </div>
                </div>
              </div>
            </div>

            <!-- Live Content streaming right after thinking -->
            <div v-if="liveContentBlocks.length > 0" class="md-content live-content">
              <template
                v-for="(block, bIdx) in liveContentBlocks"
                :key="bIdx"
              >
                <h1 v-if="block.kind === 'heading' && block.level === 1" class="md-h1">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths()"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="handleOpenFileInReview"
                  />
                </h1>

                <h2 v-else-if="block.kind === 'heading' && block.level === 2" class="md-h2">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths()"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="handleOpenFileInReview"
                  />
                </h2>

                <h3 v-else-if="block.kind === 'heading' && block.level === 3" class="md-h3">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths()"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="handleOpenFileInReview"
                  />
                </h3>

                <blockquote v-else-if="block.kind === 'quote'" class="md-quote">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths()"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="handleOpenFileInReview"
                  />
                </blockquote>

                <ul v-else-if="block.kind === 'ul'" class="md-ul">
                  <li
                    v-for="(item, liIdx) in block.items"
                    :key="liIdx"
                    :style="{ marginLeft: `${item.indent * 16}px` }"
                  >
                    <MarkdownInline
                      :tokens="item.tokens"
                      :available-paths="getAvailableDiffPaths()"
                      :project-root="store.activeProject?.rootPath"
                      @open-file="handleOpenFileInReview"
                    />
                  </li>
                </ul>

                <ol v-else-if="block.kind === 'ol'" class="md-ol">
                  <li v-for="(item, liIdx) in block.items" :key="liIdx">
                    <MarkdownInline
                      :tokens="item"
                      :available-paths="getAvailableDiffPaths()"
                      :project-root="store.activeProject?.rootPath"
                      @open-file="handleOpenFileInReview"
                    />
                  </li>
                </ol>

                <div v-else-if="block.kind === 'code'" class="msg-code-block halo-hud">
                  <div class="code-toolbar">
                    <div class="code-toolbar-info">
                      <span class="code-lang">{{ block.lang }}</span>
                      <span v-if="block.filename" class="code-filename-pill">{{ block.filename }}</span>
                      <span v-if="block.streaming" class="code-streaming-pill">En cours…</span>
                    </div>
                  </div>
                  <pre class="code-pre custom-scroll"><code class="hljs" v-html="renderCodeHtml(block.body, block.lang, Boolean(block.streaming))" /></pre>
                </div>

                <div v-else-if="block.kind === 'table'" class="md-table-wrapper custom-scroll">
                  <table class="md-table">
                    <thead>
                      <tr>
                        <th
                          v-for="(h, thIdx) in block.headers"
                          :key="thIdx"
                          :style="{ textAlign: block.alignments?.[thIdx] || 'left' }"
                        >
                          <MarkdownInline
                            :tokens="h"
                            :available-paths="getAvailableDiffPaths()"
                            :project-root="store.activeProject?.rootPath"
                            @open-file="handleOpenFileInReview"
                          />
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      <tr v-for="(row, trIdx) in block.rows" :key="trIdx">
                        <td
                          v-for="(cell, tdIdx) in row"
                          :key="tdIdx"
                          :style="{ textAlign: block.alignments?.[tdIdx] || 'left' }"
                        >
                          <MarkdownInline
                            :tokens="cell"
                            :available-paths="getAvailableDiffPaths()"
                            :project-root="store.activeProject?.rootPath"
                            @open-file="handleOpenFileInReview"
                          />
                        </td>
                      </tr>
                    </tbody>
                  </table>
                </div>

                <p v-else-if="block.kind === 'paragraph'" class="md-p">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths()"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="handleOpenFileInReview"
                  />
                </p>
              </template>
              <span class="live-blinking-cursor" />
            </div>
          </article>
        </div>
      </div>
    </div>

    <!-- Image preview light-box modal -->
    <div
      v-if="previewModalImage"
      class="image-modal-overlay"
      @click="previewModalImage = null"
    >
      <div class="image-modal-card halo-hud" @click.stop>
        <button class="modal-close-btn" @click="previewModalImage = null">✕</button>
        <img :src="previewModalImage" class="modal-full-img" alt="Prévisualisation" />
      </div>
    </div>

    <!-- Bottom: Prompt Input -->
    <footer class="disc-footer">
      <div class="footer-inner">
        <!-- Tool Approval Guardrail Banner -->
        <div v-if="activeToolApprovals.length > 0" class="tool-approval-stack">
          <div
            v-for="req in activeToolApprovals"
            :key="req.requestId"
            class="tool-approval-card"
          >
            <div class="tool-approval-info">
              <div class="tool-approval-head">
                <span class="tool-approval-badge">{{ req.tool }}</span>
                <strong class="tool-approval-title">{{ req.title }}</strong>
              </div>
              <code class="tool-approval-target">{{ req.target }}</code>
              <p v-if="req.reason" class="tool-approval-reason">{{ req.reason }}</p>
            </div>
            <div class="tool-approval-actions">
              <button
                type="button"
                class="tool-approval-btn is-deny"
                @click="store.resolveToolApproval(req.requestId, false)"
              >
                Refuser
              </button>
              <button
                type="button"
                class="tool-approval-btn is-approve"
                @click="store.resolveToolApproval(req.requestId, true)"
              >
                {{ req.tool === "delete_file" ? "Confirmer la suppression" : "Autoriser" }}
              </button>
            </div>
          </div>
        </div>

        <!-- Prompt composer autonome et isolé (0 re-render de la discussion lors de la frappe) -->
        <PromptComposer
          ref="composerRef"
          @submit="handleComposerSubmit"
          @preview-image="(url) => (previewModalImage = url)"
          @slash-command="(cmd, args) => void executeSlashCommand(cmd, args)"
        />
      </div>
    </footer>
  </main>
</template>

<style scoped>
@property --rainbow-angle {
  syntax: "<angle>";
  inherits: false;
  initial-value: 0deg;
}

@keyframes rotateRainbow {
  0% {
    --rainbow-angle: 0deg;
  }
  100% {
    --rainbow-angle: 360deg;
  }
}

@keyframes pulseGlow {
  0%,
  100% {
    opacity: 0.55;
    transform: scale(0.96);
  }
  50% {
    opacity: 1;
    transform: scale(1.04);
  }
}

@keyframes blinkCursor {
  0%,
  100% {
    opacity: 0;
  }
  50% {
    opacity: 1;
  }
}

/* Timeline rail (ronds verticaux à droite du menu) */
.disc-timeline-rail {
  position: absolute;
  left: 8px;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 3px;
  z-index: 60;
  padding: 4px 0;
  background: transparent;
  border: none;
  box-shadow: none;
  backdrop-filter: none;
  overflow: visible;
}

.timeline-dot-wrapper {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 1px 3px;
}

.timeline-dot {
  display: block;
  width: 5px;
  height: 5px;
  border-radius: 9999px;
  background: rgba(255, 255, 255, 0.35);
  opacity: 0.6;
  transition: all 0.15s cubic-bezier(0.16, 1, 0.3, 1);
}

.timeline-dot.is-current {
  width: 6px;
  height: 6px;
  background: rgba(255, 255, 255, 0.95);
  opacity: 1;
  box-shadow: 0 0 4px rgba(255, 255, 255, 0.4);
}

.timeline-dot.is-compact {
  width: 5px;
  height: 12px;
  border-radius: 9999px;
}

.timeline-dot-wrapper:hover .timeline-dot {
  opacity: 1;
  background: rgba(255, 255, 255, 0.9);
  transform: scale(1.15);
}

.timeline-tooltip {
  position: absolute;
  left: 18px;
  top: 50%;
  transform: translateY(-50%);
  width: clamp(235px, calc((100% - 840px) / 2 - 20px), 340px);
  min-width: 220px;
  max-width: calc(100vw - 36px);
  padding: 9px 12px;
  background: rgba(15, 23, 42, 0.96);
  backdrop-filter: blur(14px);
  border: 1px solid rgba(255, 255, 255, 0.16);
  border-radius: 9px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.65), 0 0 1px rgba(255, 255, 255, 0.2);
  pointer-events: none;
  opacity: 0;
  visibility: hidden;
  transition: opacity 0.16s ease, visibility 0.16s ease, transform 0.16s ease;
  z-index: 100;
  display: flex;
  flex-direction: column;
  gap: 6px;
  word-break: break-word;
}

.timeline-dot-wrapper:hover .timeline-tooltip {
  opacity: 1;
  visibility: visible;
  transform: translateY(-50%) translateX(2px);
}

.timeline-tooltip-header {
  display: flex;
  align-items: center;
  gap: 5px;
}

.tt-commit-tag {
  font-size: 9.5px;
  font-weight: 600;
  padding: 1px 4px;
  border-radius: 4px;
  background: rgba(99, 102, 241, 0.2);
  color: #a5b4fc;
  border: 1px solid rgba(99, 102, 241, 0.3);
}

.tt-compact-tag {
  font-size: 9.5px;
  font-weight: 600;
  padding: 1px 4px;
  border-radius: 4px;
  background: rgba(245, 158, 11, 0.2);
  color: #fcd34d;
  border: 1px solid rgba(245, 158, 11, 0.3);
}

.timeline-tooltip-question {
  margin: 0;
  font-size: 12px;
  font-weight: 700;
  color: #ffffff;
  line-height: 1.4;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
  word-break: break-word;
}

.timeline-tooltip-answer {
  margin: 0;
  font-size: 11.5px;
  font-weight: 400;
  color: #94a3b8;
  line-height: 1.45;
  display: -webkit-box;
  -webkit-line-clamp: 4;
  -webkit-box-orient: vertical;
  overflow: hidden;
  word-break: break-word;
}

.timeline-tooltip-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  font-size: 9.5px;
  color: #94a3b8;
  margin-top: 1px;
  border-top: 1px solid rgba(255, 255, 255, 0.07);
  padding-top: 3px;
}

/* Commit badge pill & Pulse highlight */
.msg-footer-actions-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
}

.commit-pill-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.12);
  color: #f3f4f6;
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
}

.commit-pill-btn:hover {
  background: rgba(255, 255, 255, 0.12);
  border-color: rgba(99, 102, 241, 0.5);
  color: #ffffff;
  transform: translateY(-1px);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
}

.commit-pill-tag {
  font-weight: 700;
  color: #818cf8;
}

.commit-pill-summary {
  color: #94a3b8;
  max-width: 260px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.pulse-highlight {
  position: relative;
  isolation: isolate;
}

.pulse-highlight::before {
  content: "";
  position: absolute;
  inset: -14px -18px;
  border-radius: 14px;
  pointer-events: none;
  z-index: -1;
  border: 1px solid transparent;
  background:
    linear-gradient(rgba(18, 16, 26, 0.4), rgba(18, 16, 26, 0.4)) padding-box,
    linear-gradient(135deg, rgba(236, 64, 84, 0.7), rgba(251, 140, 26, 0.65)) border-box;
  box-shadow:
    0 0 16px rgba(236, 64, 84, 0.22),
    0 0 30px rgba(251, 140, 26, 0.14),
    inset 0 0 14px rgba(236, 64, 84, 0.06);
  animation: highlightPulseGlow 1.8s cubic-bezier(0.16, 1, 0.3, 1) forwards;
}

@keyframes highlightPulseGlow {
  0% {
    opacity: 0;
    transform: scale(0.995);
  }
  20% {
    opacity: 1;
    transform: scale(1);
  }
  75% {
    opacity: 0.9;
    transform: scale(1);
  }
  100% {
    opacity: 0;
    transform: scale(1);
  }
}

.la-discussion {
  position: relative;
  flex: 1;
  min-width: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.la-discussion.is-drag-over {
  border-color: var(--accent) !important;
  box-shadow: 0 0 24px 2px var(--halo-bright) !important;
}

.disc-header {
  height: 48px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 18px;
  border-bottom: 1px solid var(--border-subtle);
  flex-shrink: 0;
}

.disc-header-left {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
}

.breadcrumb-project {
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 13px;
  font-weight: 500;
  padding: 3px 6px;
  border-radius: 6px;
  cursor: pointer;
  white-space: nowrap;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  transition: background-color 0.14s ease, color 0.14s ease;
}

.breadcrumb-project:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.breadcrumb-sep {
  color: var(--text-dim);
  font-size: 13px;
  user-select: none;
  flex-shrink: 0;
}

.breadcrumb-title-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: 100%;
  border: 1px solid transparent;
  background: transparent;
  color: var(--text-main);
  padding: 3px 7px;
  border-radius: 7px;
  cursor: text;
  transition: background-color 0.14s ease, border-color 0.14s ease;
}

.breadcrumb-title-btn:hover {
  background: var(--hover-bg);
  border-color: var(--border-subtle);
}

.disc-title {
  font-weight: 600;
  font-size: 13.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rename-pencil-icon {
  width: 12px;
  height: 12px;
  color: var(--text-dim);
  opacity: 0;
  flex-shrink: 0;
  transition: opacity 0.14s ease, color 0.14s ease;
}

.breadcrumb-title-btn:hover .rename-pencil-icon {
  opacity: 1;
  color: var(--accent);
}

.breadcrumb-title-input {
  min-width: 180px;
  max-width: 380px;
  flex: 1;
  border: 1px solid var(--accent);
  background: var(--table-bg);
  color: var(--text-main);
  border-radius: 7px;
  padding: 3px 8px;
  font-size: 13.5px;
  font-weight: 600;
  outline: none;
  box-shadow: 0 0 0 2px rgba(168, 85, 247, 0.18);
}

.restore-hud-btn {
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  color: var(--text-muted);
  border-radius: 999px;
  padding: 4px 10px;
  font-size: 12px;
  cursor: pointer;
}

.disc-stream {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  padding: 16px 36px 42px;
}

.welcome-state {
  margin: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: 14px;
  padding: 24px 12px;
}

.welcome-logo {
  width: 68px;
  height: 68px;
  border-radius: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.welcome-title {
  margin: 0;
  font-size: 22px;
  font-weight: 600;
  letter-spacing: -0.02em;
}

.welcome-meta {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px;
}

.meta-chip {
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  color: var(--text-muted);
  border-radius: 999px;
  padding: 4px 10px;
  font-size: 12px;
}

.welcome-empty-project {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  margin-top: 6px;
}

.welcome-empty-hint {
  margin: 0;
  font-size: 13px;
  color: var(--text-muted);
}

.welcome-new-project-btn {
  border: none;
  background: var(--accent);
  color: #ffffff;
  border-radius: 9px;
  padding: 8px 16px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.14s ease, transform 0.14s ease;
}

.welcome-new-project-btn:hover {
  opacity: 0.92;
  transform: translateY(-1px);
}

.messages-list {
  width: 100%;
  max-width: 820px;
  margin-inline: auto;
  display: flex;
  flex-direction: column;
  gap: 28px;
  padding-bottom: 18px;
}

.turn-group {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 16px;
  position: relative;
  border-radius: 16px;
}

.msg-wrapper {
  display: flex;
}

.msg-wrapper.is-user {
  justify-content: flex-end;
}

.user-msg-col {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  max-width: 82%;
  gap: 4px;
}

.user-bubble-box {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
}

.user-bubble {
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  border-radius: 14px 14px 4px 14px;
  padding: 10px 14px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-family: var(--font-response);
  position: relative;
  transition: border-color 0.18s ease, box-shadow 0.18s ease;
}

.user-bubble.is-collapsible {
  cursor: pointer;
}

.user-bubble.is-collapsible:hover {
  border-color: rgba(255, 255, 255, 0.24);
}

.user-hover-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-top: 4px;
  opacity: 0;
  transform: translateY(-2px);
  transition: opacity 0.15s ease, transform 0.15s ease;
  pointer-events: none;
}

.user-msg-col:hover .user-hover-actions {
  opacity: 1;
  transform: translateY(0);
  pointer-events: auto;
}

.user-action-pill {
  width: 26px;
  height: 26px;
  border-radius: 6px;
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  color: var(--text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  transition: all 0.15s ease;
}

.user-action-pill:hover {
  background: var(--hover-bg);
  color: var(--text-main);
  border-color: var(--accent);
}

.u-act-icon {
  width: 14px;
  height: 14px;
}

.u-act-icon.text-success {
  color: #10b981;
}

.msg-gallery {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.msg-img-thumb {
  max-width: 180px;
  max-height: 140px;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  cursor: pointer;
  object-fit: cover;
  transition: transform 0.15s ease;
}

.msg-img-thumb:hover {
  transform: scale(1.03);
}

.msg-files {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.file-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  border-radius: 6px;
  background: var(--table-bg);
  border: 1px solid var(--border-subtle);
  font-size: 11.5px;
  color: var(--text-muted);
}

.file-icon {
  width: 12px;
  height: 12px;
}

.file-name {
  font-weight: 500;
  color: var(--text-main);
}

.file-size {
  opacity: 0.75;
}

.msg-text-wrapper {
  position: relative;
  width: 100%;
}

.msg-text-wrapper.is-clamped {
  max-height: 126px; /* 5-6 lignes max */
  overflow: hidden;
}

.bubble-fade-gradient {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 48px;
  background: linear-gradient(to bottom, transparent, var(--surface-elevated));
  pointer-events: none;
  border-bottom-left-radius: 12px;
  border-bottom-right-radius: 4px;
}

.msg-text {
  margin: 0;
  font-size: 13.5px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
}

.assistant-article {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 12px;
  line-height: 1.6;
  font-family: var(--font-response);
}

/* Thinking block (Réflexion de l'IA) */
.thinking-block {
  border: 1px solid var(--border-subtle);
  background: rgba(129, 140, 248, 0.05);
  border-radius: 10px;
  overflow: hidden;
  margin-bottom: 4px;
}

.thinking-header {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 7px 11px;
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--text-muted);
  font-size: 12px;
}

.thinking-header:hover {
  background: var(--hover-bg);
}

.thinking-title {
  display: flex;
  align-items: center;
  gap: 7px;
  font-weight: 600;
  color: var(--accent);
}

.brain-icon {
  width: 15px;
  height: 15px;
}

.brain-icon.pulsing {
  animation: pulseGlow 1.6s ease-in-out infinite;
}

.thinking-meta {
  display: flex;
  align-items: center;
  gap: 8px;
}

.thinking-tabs-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.live-tab-pill {
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  color: var(--text-muted);
  border-radius: 999px;
  padding: 2px 9px;
  font-size: 11px;
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  cursor: pointer;
  transition: all 0.15s ease;
}

.live-tab-pill:hover {
  background: var(--hover-bg);
  color: var(--text-main);
  border-color: var(--accent);
}

.live-tab-pill.active {
  background: var(--active-bg);
  color: var(--accent);
  border-color: var(--accent);
  font-weight: 600;
}

.stream-count-dot {
  background: var(--accent);
  color: #fff;
  border-radius: 999px;
  padding: 0 5px;
  font-size: 9.5px;
  font-weight: 700;
}

.stream-log-live {
  margin: 0;
  font-family: var(--font-code);
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-muted);
  white-space: pre-wrap;
  word-break: break-word;
}

.thinking-badge {
  font-size: 10.5px;
  font-weight: 500;
  border-radius: 999px;
  padding: 1px 7px;
}

.thinking-badge.done {
  background: var(--surface-elevated);
  color: var(--text-dim);
  border: 1px solid var(--border-subtle);
}

.thinking-badge.running {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  background: var(--active-bg);
  color: var(--accent);
}

.pulse-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  animation: pulseGlow 1.2s infinite;
}

.chevron-icon {
  width: 13px;
  height: 13px;
  transition: transform 0.16s ease;
}

.chevron-icon.rotated {
  transform: rotate(180deg);
}

.thinking-body {
  border-top: 1px solid var(--border-subtle);
  padding: 10px 12px;
  max-height: 290px;
  overflow-y: auto;
  background: var(--table-bg);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.trace-dots-preview {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.trace-preview-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}

.trace-steps-count {
  font-size: 10px;
  color: var(--text-dim);
  padding: 0 5px;
}

.trace-tree-container {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.trace-group-node {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.dag-wave-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 0 2px 2px;
}

.dag-wave-badge {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-weight: 600;
  padding: 2.5px 9px;
  border-radius: 9999px;
  letter-spacing: 0.01em;
}

.dag-wave-badge.badge-parallel {
  background: rgba(245, 158, 11, 0.12);
  color: #f59e0b;
  border: 1px solid rgba(245, 158, 11, 0.35);
  box-shadow: 0 0 10px rgba(245, 158, 11, 0.12);
}

.dag-wave-badge.badge-sequential {
  background: rgba(99, 102, 241, 0.12);
  color: #818cf8;
  border: 1px solid rgba(99, 102, 241, 0.3);
}

.wave-tag-parallel {
  font-size: 10px;
  color: #fbbf24;
  opacity: 0.85;
  font-style: italic;
}

.dag-branch-tree {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-left: 20px;
  margin-top: 4px;
}

.dag-phase-wrap {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.dag-tree-item {
  position: relative;
  display: flex;
  align-items: stretch;
  width: 100%;
}

/* Ligne verticale de liaison d'arbre pour les étapes intermédiaires */
.dag-branch-tree .dag-tree-item::before {
  content: '';
  position: absolute;
  left: -12px;
  top: 0;
  bottom: -6px; /* Franchit le gap entre tâches */
  width: 1.5px;
  background: rgba(255, 255, 255, 0.15);
  pointer-events: none;
}

/* Le dernier enfant de la vague ne continue pas sous son coude */
.dag-branch-tree .dag-tree-item:last-child::before {
  display: none;
}

/* Coude d'arborescence reliant le tronc vertical à la carte de tâche */
.dag-branch-tree .dag-tree-item::after {
  content: '';
  position: absolute;
  left: -12px;
  top: 0;
  width: 12px;
  height: 16px;
  border-left: 1.5px solid rgba(255, 255, 255, 0.15);
  border-bottom: 1.5px solid rgba(255, 255, 255, 0.15);
  border-bottom-left-radius: 6px;
  pointer-events: none;
}

/* Coloration distinctive ambrée pour les vagues parallèles */
.trace-group-node.is-parallel-wave .dag-tree-item::before {
  background: rgba(245, 158, 11, 0.45);
}

.trace-group-node.is-parallel-wave .dag-tree-item::after {
  border-color: rgba(245, 158, 11, 0.45);
}

.trace-step-card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px 9px;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  border-left: 2.5px solid var(--step-role-color, var(--accent));
  background: var(--hover-bg);
  width: 100%;
}

.trace-step-card.in-parallel {
  border-left-width: 3px;
  background: rgba(255, 255, 255, 0.025);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.15);
}

.trace-role-emoji {
  font-size: 13px;
  line-height: 1;
}

.trace-step-top {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}

.trace-role-wrap {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  font-size: 11.5px;
  color: var(--text-main);
}

.trace-facet-badge {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 6px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
  border: 1px solid transparent;
  line-height: 1.3;
  flex-shrink: 0;
}

.trace-facet-badge.is-mini {
  font-size: 9.5px;
  padding: 0 5px;
}

.trace-facet-badge.is-code {
  color: #60a5fa;
  background: rgba(59, 130, 246, 0.14);
  border-color: rgba(59, 130, 246, 0.35);
}

.trace-facet-badge.is-ui {
  color: #f472b6;
  background: rgba(236, 72, 153, 0.14);
  border-color: rgba(236, 72, 153, 0.35);
}

.trace-facet-badge.is-refacto {
  color: #fbbf24;
  background: rgba(245, 158, 11, 0.14);
  border-color: rgba(245, 158, 11, 0.35);
}

.trace-role-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
}

.trace-role-label {
  font-weight: 600;
}

.trace-model-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 1px 7px 1px 6px;
  border-radius: 999px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  font-size: 10.5px;
  max-width: 240px;
}

.trace-prov-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.trace-prov-name {
  font-weight: 600;
  color: var(--text-main);
  flex-shrink: 0;
}

.trace-model-name {
  font-family: var(--font-code);
  font-size: 10px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.trace-step-right {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.trace-tok-pill {
  font-family: var(--font-code);
  font-size: 10px;
  color: var(--text-dim);
}

.trace-status {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 700;
}

.trace-status.is-done {
  background: rgba(16, 185, 129, 0.16);
  color: #10b981;
}

.trace-status.is-fallback {
  background: rgba(245, 158, 11, 0.18);
  color: #f59e0b;
}

.trace-status.is-error {
  background: rgba(239, 68, 68, 0.18);
  color: #ef4444;
}

.trace-status.is-running {
  background: var(--active-bg);
}

.trace-step-summary {
  font-size: 11.5px;
  line-height: 1.4;
  color: var(--text-muted);
  word-break: break-word;
  padding-left: 13px;
}

.thinking-md {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-muted);
}

.tmd-h1,
.tmd-h2,
.tmd-h3 {
  margin: 4px 0 1px;
  font-weight: 600;
  color: var(--text-main);
  line-height: 1.3;
}

.tmd-h1 {
  font-size: 13.5px;
}

.tmd-h2 {
  font-size: 12.5px;
}

.tmd-h3 {
  font-size: 12px;
  color: var(--accent);
}

.tmd-p {
  margin: 0;
  word-break: break-word;
}

.tmd-quote {
  margin: 2px 0;
  padding: 3px 8px;
  border-left: 2px solid var(--accent);
  background: var(--hover-bg);
  border-radius: 0 5px 5px 0;
  color: var(--text-dim);
}

.tmd-ul,
.tmd-ol {
  margin: 0;
  padding-left: 16px;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.tmd-ul li,
.tmd-ol li {
  line-height: 1.45;
  word-break: break-word;
}

.tmd-inline-code {
  font-family: var(--font-code);
  font-size: 11px;
  padding: 1px 5px;
  border-radius: 4px;
  background: var(--hover-bg);
  border: 1px solid var(--border-subtle);
  color: var(--accent-2);
}

.tmd-code-block {
  margin: 2px 0;
  padding: 7px 9px;
  border-radius: 7px;
  border: 1px solid var(--border-subtle);
  background: var(--discussion-bg);
  font-family: var(--font-code);
  font-size: 11px;
  line-height: 1.45;
  overflow-x: auto;
}

/* Live stream styling */
.live-article {
  opacity: 0.95;
}

.live-blinking-cursor {
  display: inline-block;
  width: 7px;
  height: 14px;
  margin-left: 4px;
  background: var(--accent);
  vertical-align: middle;
  animation: blinkCursor 0.9s infinite;
}

.md-content {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.md-h1 {
  margin: 4px 0 0;
  font-size: 19px;
  font-weight: 600;
  letter-spacing: -0.015em;
}

.md-h2 {
  margin: 2px 0 0;
  font-size: 16px;
  font-weight: 600;
  letter-spacing: -0.01em;
}

.md-h3 {
  margin: 0;
  font-size: 14.5px;
  font-weight: 600;
}

.md-p {
  margin: 0;
  font-size: 13.5px;
  line-height: 1.6;
}

.md-quote {
  margin: 0;
  padding: 4px 12px;
  border-left: 3px solid var(--accent);
  color: var(--text-muted);
  background: var(--hover-bg);
  border-radius: 0 6px 6px 0;
}

.md-ul,
.md-ol {
  margin: 0;
  padding-left: 20px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 13.5px;
}

.md-inline-code {
  background: var(--surface-elevated);
  border: 1px solid var(--border-subtle);
  border-radius: 5px;
  padding: 1px 5px;
  font-family: var(--font-code);
  font-size: 12px;
  color: var(--accent-2);
}

.md-table-wrapper,
.tmd-table-wrapper {
  margin: 10px 0;
  max-width: 100%;
  overflow-x: auto;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  background: var(--surface-card);
}

.md-table,
.tmd-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
  line-height: 1.5;
  text-align: left;
}

.md-table th,
.tmd-table th {
  padding: 8px 12px;
  background: var(--surface-elevated);
  color: var(--text-muted);
  font-weight: 600;
  font-size: 11.5px;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  border-bottom: 1px solid var(--border-subtle);
  white-space: nowrap;
}

.md-table td,
.tmd-table td {
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-subtle);
  color: var(--text-main);
  vertical-align: top;
  word-break: break-word;
}

.md-table tbody tr:last-child td,
.tmd-table tbody tr:last-child td {
  border-bottom: none;
}

.md-table tbody tr:hover,
.tmd-table tbody tr:hover {
  background: var(--hover-bg);
}

/* Code Blocks with strong contrast in dark mode */
.msg-code-block {
  border-radius: 10px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  background: #151922;
  border: 1px solid rgba(148, 163, 184, 0.22);
  box-shadow: 0 4px 16px -2px rgba(0, 0, 0, 0.4);
}

:root[data-theme="light"] .msg-code-block {
  background: #f8fafc;
  border: 1px solid rgba(15, 23, 42, 0.15);
  box-shadow: 0 4px 14px -2px rgba(15, 23, 42, 0.08);
}

.code-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 7px 12px;
  border-bottom: 1px solid rgba(148, 163, 184, 0.15);
  background: rgba(255, 255, 255, 0.04);
}

:root[data-theme="light"] .code-toolbar {
  background: rgba(0, 0, 0, 0.03);
  border-bottom: 1px solid rgba(15, 23, 42, 0.1);
}

.code-toolbar-info {
  display: flex;
  align-items: center;
  gap: 8px;
}

.code-lang {
  font-family: var(--font-code);
  font-size: 11px;
  font-weight: 600;
  color: var(--accent-2);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.code-filename-pill {
  font-family: var(--font-code);
  font-size: 11px;
  background: var(--surface-elevated);
  border: 1px solid var(--border-subtle);
  border-radius: 4px;
  padding: 1px 6px;
  color: var(--text-main);
}

.code-streaming-pill {
  font-size: 10px;
  font-weight: 600;
  color: var(--accent);
  background: var(--active-bg);
  border: 1px solid rgba(129, 140, 248, 0.35);
  border-radius: 999px;
  padding: 1px 7px;
}

.code-toolbar-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.code-btn-icon {
  width: 26px;
  height: 26px;
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  color: var(--text-muted);
  border-radius: 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  transition: all 0.15s ease;
}

.code-btn-icon:hover {
  background: var(--hover-bg);
  color: var(--text-main);
  border-color: var(--accent);
}

.code-btn-icon.copied {
  color: #10b981;
  border-color: #10b981;
}

.code-icon {
  width: 14.5px;
  height: 14.5px;
}

.code-icon.text-success {
  color: #10b981;
}

.code-pre {
  margin: 0;
  padding: 12px 14px;
  overflow-x: auto;
  font-family: var(--font-code);
  font-size: 12.5px;
  line-height: 1.55;
  background: transparent !important;
}

.code-pre code {
  font-family: var(--font-code) !important;
  background: transparent !important;
  padding: 0 !important;
}

/* Accordéon dropdown des fichiers impactés */
.files-impact-accordion {
  align-self: flex-start;
  width: 100%;
  max-width: 680px;
  margin-top: 4px;
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  border-radius: 9px;
  overflow: hidden;
  transition: border-color 0.15s ease;
}

.files-impact-accordion:hover {
  border-color: rgba(129, 140, 248, 0.4);
}

.files-impact-header {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  background: transparent;
  border: none;
  cursor: pointer;
  font-size: 12px;
  color: var(--text-main);
  gap: 10px;
}

.files-impact-header:hover {
  background: var(--hover-bg);
}

.impact-header-left {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

/* Vrai Dropdown Sélecteur des fichiers modifiés */
.diff-dropdown-wrap {
  position: relative;
  align-self: flex-start;
  margin-top: 4px;
  display: inline-flex;
  flex-direction: column;
}

.diff-dropdown-trigger {
  align-self: flex-start;
  display: inline-flex;
  align-items: center;
  gap: 12px;
  padding: 6px 12px;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  color: var(--text-main);
  cursor: pointer;
  font-size: 12px;
  font-weight: 500;
  transition: all 0.15s ease;
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.12);
}

.diff-dropdown-trigger:hover {
  background: var(--hover-bg);
  border-color: var(--accent);
  box-shadow: 0 0 10px var(--halo-soft);
}

.diff-trigger-left {
  display: flex;
  align-items: center;
  gap: 7px;
}

.diff-git-icon {
  width: 14px;
  height: 14px;
  color: var(--accent);
}

.diff-trigger-title {
  font-weight: 600;
}

.diff-trigger-right {
  display: flex;
  align-items: center;
  gap: 6px;
  font-family: var(--font-code);
  font-size: 11px;
}

.diff-badge-add {
  color: var(--diff-add-bar);
  font-weight: 700;
}

.diff-badge-del {
  color: var(--diff-del-bar);
  font-weight: 700;
}

.diff-chevron-arrow {
  width: 13px;
  height: 13px;
  color: var(--text-muted);
  transition: transform 0.18s ease;
}

.diff-chevron-arrow.rotated {
  transform: rotate(180deg);
}

/* Menu Dropdown déroulant */
.diff-dropdown-menu {
  margin-top: 6px;
  min-width: 380px;
  max-width: 540px;
  overflow: hidden;
  border-radius: 10px;
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  box-shadow: 0 12px 28px -6px rgba(0, 0, 0, 0.45);
  display: flex;
  flex-direction: column;
}

.diff-menu-header {
  padding: 8px 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  border-bottom: 1px solid var(--border-subtle);
  background: rgba(255, 255, 255, 0.02);
}

.diff-menu-header-left {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.diff-undo-all-action-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: 5px;
  border: 1px solid rgba(239, 68, 68, 0.3);
  background: rgba(239, 68, 68, 0.08);
  color: #f87171;
  font-size: 10.5px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.diff-undo-all-action-btn:hover {
  background: rgba(239, 68, 68, 0.22);
  border-color: #ef4444;
  color: #ffffff;
  box-shadow: 0 0 8px rgba(239, 68, 68, 0.35);
}

.diff-menu-header-text {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-dim);
}

.diff-menu-stats-row {
  display: flex;
  align-items: center;
  gap: 4px;
}

.badge-mini-status {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 5px;
  border-radius: 4px;
}

.badge-mini-status.tag-created {
  background: rgba(16, 185, 129, 0.18);
  color: #10b981;
}

.badge-mini-status.tag-modified {
  background: rgba(59, 130, 246, 0.18);
  color: #3b82f6;
}

.badge-mini-status.tag-deleted {
  background: rgba(239, 68, 68, 0.18);
  color: #ef4444;
}

.diff-menu-list {
  display: flex;
  flex-direction: column;
  max-height: 240px;
  overflow-y: auto;
}

.diff-menu-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border-subtle);
  cursor: pointer;
  transition: background 0.12s ease;
  font-size: 11.5px;
}

.diff-menu-row:last-child {
  border-bottom: none;
}

.diff-menu-row:hover {
  background: var(--hover-bg);
}

.menu-state-badge {
  font-family: var(--font-code);
  font-size: 9px;
  font-weight: 700;
  padding: 1px 4px;
  border-radius: 3px;
}

.menu-state-badge.created {
  background: rgba(16, 185, 129, 0.2);
  color: #10b981;
}

.menu-state-badge.modified {
  background: rgba(59, 130, 246, 0.2);
  color: #3b82f6;
}

.menu-state-badge.deleted {
  background: rgba(239, 68, 68, 0.2);
  color: #ef4444;
}

.menu-file-path {
  flex: 1;
  font-family: var(--font-code);
  font-size: 11px;
  color: var(--text-main);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.menu-file-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.menu-file-delta {
  font-family: var(--font-code);
  font-size: 11px;
  display: inline-flex;
  gap: 5px;
}

.menu-file-undo-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 4px;
  border: 1px solid var(--border-subtle);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  transition: all 0.15s ease;
  opacity: 0.65;
}

.diff-menu-row:hover .menu-file-undo-btn {
  opacity: 1;
}

.menu-file-undo-btn:hover {
  background: rgba(239, 68, 68, 0.2);
  border-color: #ef4444;
  color: #ef4444;
  box-shadow: 0 0 6px rgba(239, 68, 68, 0.3);
}

.diff-menu-empty {
  padding: 12px;
  font-size: 11.5px;
  color: var(--text-muted);
}

.diff-menu-footer {
  padding: 6px 12px;
  border-top: 1px solid var(--border-subtle);
  display: flex;
  justify-content: flex-end;
  background: rgba(0, 0, 0, 0.04);
}

.diff-full-review-cta {
  border: none;
  background: transparent;
  color: var(--accent);
  font-size: 11px;
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  cursor: pointer;
  padding: 3px 6px;
  border-radius: 4px;
}

.diff-full-review-cta:hover {
  background: var(--active-bg);
  text-decoration: underline;
}

.cta-arrow {
  width: 12px;
  height: 12px;
}

.c-mod {
  color: var(--diff-mod-bar);
  font-weight: 600;
}

.c-add {
  color: var(--diff-add-bar);
  font-weight: 600;
}

.c-del {
  color: var(--diff-del-bar);
  font-weight: 600;
}

.review-cta {
  color: var(--accent);
  font-weight: 500;
}

.msg-timestamp {
  font-size: 11px;
  color: var(--text-dim);
}

.msg-timestamp.is-right {
  text-align: right;
}

.disc-footer {
  padding: 8px 36px 16px;
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.footer-inner {
  width: 100%;
  max-width: 820px;
  margin-inline: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.stage-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 7px 12px;
  border-radius: 10px;
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  font-size: 12px;
}

.stage-bar-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.stage-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.stage-dot.running {
  background: var(--accent);
  box-shadow: 0 0 6px var(--accent);
}

.stage-dot.done {
  background: var(--diff-add-bar);
}

.stage-dot.error {
  background: var(--diff-del-bar);
}

.stage-title {
  font-weight: 600;
  white-space: nowrap;
}

.stage-detail {
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.logs-toggle {
  border: none;
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  font-size: 11px;
}

.logs-toggle:hover {
  color: var(--text-main);
}

.stream-log {
  margin: 0;
  max-height: 120px;
  overflow-y: auto;
  border-radius: 8px;
  padding: 8px 10px;
  border: 1px solid var(--border-subtle);
  background: #07090e;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  line-height: 1.45;
  color: var(--text-muted);
  white-space: pre-wrap;
}

.composer-wrapper {
  position: relative;
  width: 100%;
}

/* Mention Popover (@) */
.mention-popover {
  position: absolute;
  bottom: calc(100% + 8px);
  left: 0;
  right: 0;
  max-height: 280px;
  background: var(--table-bg);
  border: 1px solid var(--border-subtle);
  border-radius: 12px;
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  z-index: 35;
}

.mention-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-subtle);
  background: rgba(255, 255, 255, 0.02);
}

.mention-hint {
  font-size: 11.5px;
  color: var(--text-dim);
}

.mention-tabs {
  display: flex;
  gap: 4px;
}

.mention-tab {
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 5px;
  padding: 2px 7px;
  font-size: 11px;
  cursor: pointer;
}

.mention-tab.active,
.mention-tab:hover {
  background: var(--active-bg);
  color: var(--accent);
}

.mention-list {
  max-height: 220px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  padding: 4px;
}

.mention-empty {
  padding: 12px;
  text-align: center;
  font-size: 12px;
  color: var(--text-dim);
}

.mention-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 9px;
  border-radius: 7px;
  border: none;
  background: transparent;
  color: var(--text-main);
  text-align: left;
  cursor: pointer;
}

.mention-item:hover,
.mention-item.active {
  background: var(--hover-bg);
}

.mention-badge {
  font-size: 13px;
}

.mention-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.mention-name {
  font-size: 12px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mention-desc {
  font-size: 10.5px;
  color: var(--text-dim);
}

.mention-tag {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  color: var(--accent);
  background: rgba(129, 140, 248, 0.1);
  padding: 1px 5px;
  border-radius: 4px;
}

/* Prompt composer container */
.composer {
  display: flex;
  flex-direction: column;
  padding: 8px 10px;
  gap: 6px;
  transition: border-color 0.25s ease, box-shadow 0.25s ease;
}

/* ROTATING RAINBOW BORDER DURING THINKING */
.composer.is-thinking {
  border: 1.5px solid transparent !important;
  background:
    linear-gradient(var(--surface-elevated), var(--surface-elevated)) padding-box,
    conic-gradient(
      from var(--rainbow-angle, 0deg),
      #ff0055 0%,
      #ff7700 14%,
      #ffee00 28%,
      #00ff66 42%,
      #00e5ff 57%,
      #2f00ff 71%,
      #c400ff 85%,
      #ff0055 100%
    )
      border-box !important;
  animation: rotateRainbow 2.2s linear infinite;
  box-shadow: 0 0 24px -2px rgba(168, 85, 247, 0.4), 0 0 12px 0 rgba(56, 189, 248, 0.25) !important;
}

/* Attachments preview chips floating above the input frame */
.attachments-bar.floating-bar {
  position: absolute;
  top: -14px;
  left: 14px;
  z-index: 25;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  max-width: calc(100% - 28px);
  pointer-events: auto;
}

.att-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  background: var(--surface-elevated);
  border: 1px solid var(--border-subtle);
  border-radius: 7px;
  font-size: 11.5px;
  color: var(--text-muted);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(8px);
}

.att-thumb {
  width: 22px;
  height: 22px;
  border-radius: 4px;
  object-fit: cover;
  cursor: pointer;
}

.att-file-icon {
  width: 13px;
  height: 13px;
}

.att-name {
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.att-remove-btn {
  border: none;
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  padding: 0 2px;
  font-size: 11px;
}

.att-remove-btn:hover {
  color: var(--diff-del-bar);
}

/* Ligne 1: Input de texte du prompt (extensible) */
.composer-line-1 {
  width: 100%;
  display: flex;
  align-items: flex-start;
  position: relative;
}

.composer-line-1 textarea {
  flex: 1;
  min-width: 0;
  min-height: 48px;
  max-height: 220px;
  border: none;
  background: transparent;
  color: var(--text-main);
  resize: vertical;
  outline: none;
  font-size: 13.5px;
  line-height: 1.5;
  padding: 4px 6px;
}

.prompt-clear-btn {
  margin-top: 6px;
  margin-right: 4px;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  border: none;
  background: transparent;
  color: #94a3b8;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.prompt-clear-btn:hover {
  background: rgba(255, 255, 255, 0.12);
  color: #f1f5f9;
}

.prompt-clear-btn svg {
  width: 13px;
  height: 13px;
}

/* Ligne 2: Bouton PJ + choix modèle à gauche, permissions tools et bouton envoi à droite */
.composer-line-2 {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  width: 100%;
  padding-top: 4px;
  border-top: 1px solid rgba(255, 255, 255, 0.04);
}

.composer-line-2-left {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.composer-line-2-right {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* Attach button (hauteur = 32px) */
.attach-btn {
  height: 32px;
  width: 32px;
  box-sizing: border-box;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  color: var(--text-muted);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.attach-btn:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--hover-bg);
}

.attach-btn svg {
  width: 16px;
  height: 16px;
}

/* Model & Agent Target Cascading Dropdown (hauteur = 32px, identique au bouton PJ) */
.model-picker-wrapper {
  position: relative;
  display: inline-flex;
  align-items: center;
  height: 32px;
  box-sizing: border-box;
}

.target-trigger-btn {
  height: 32px;
  box-sizing: border-box;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0 9px;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  color: var(--text-main);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  max-width: 195px;
  transition: all 0.15s ease;
}

.target-trigger-btn:hover:not(:disabled),
.target-trigger-btn.is-open {
  border-color: var(--accent);
  background: var(--hover-bg);
}

.target-trigger-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.target-model-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11.5px;
}

.provider-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
}

.target-chevron {
  width: 12px;
  height: 12px;
  color: var(--text-dim);
  flex-shrink: 0;
  transition: transform 0.15s ease;
}

.target-chevron.rotated {
  transform: rotate(180deg);
}

.target-popover {
  position: absolute;
  bottom: calc(100% + 8px);
  left: 0;
  z-index: 60;
  min-width: 230px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.38);
}

.target-agent-item {
  width: 100%;
  border: 1px solid transparent;
  background: transparent;
  color: var(--text-main);
  border-radius: 8px;
  padding: 7px 8px;
  display: flex;
  align-items: center;
  gap: 9px;
  cursor: pointer;
  text-align: left;
  transition: all 0.12s ease;
}

.target-agent-item:hover {
  background: var(--hover-bg);
}

.target-agent-item.active {
  background: var(--active-bg);
  border-color: var(--accent);
}

.target-item-copy {
  display: flex;
  flex-direction: column;
  gap: 1px;
  flex: 1;
  min-width: 0;
}

.target-item-copy strong {
  font-size: 12.5px;
  font-weight: 600;
  line-height: 1.2;
}

.target-item-copy span {
  font-size: 10.5px;
  color: var(--text-dim);
}

.target-divider {
  height: 1px;
  background: var(--border-subtle);
  margin: 2px 4px;
}

.target-provider-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 7px;
  cursor: pointer;
  font-size: 12px;
  color: var(--text-main);
  transition: background-color 0.12s ease;
}

.target-provider-row:hover,
.target-provider-row.is-active-sub {
  background: var(--hover-bg);
}

.target-provider-row.is-selected-prov {
  color: var(--accent);
  font-weight: 600;
}

.target-prov-name {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.target-prov-count {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 10px;
  color: var(--text-dim);
  padding: 0 5px;
  border-radius: 999px;
  background: var(--table-bg);
}

.target-arrow {
  font-size: 13px;
  color: var(--text-dim);
  line-height: 1;
}

.target-submenu-bridge {
  position: absolute;
  left: 100%;
  bottom: -6px;
  padding-left: 6px;
  z-index: 65;
}

.target-submenu {
  min-width: 225px;
  max-width: 280px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.42);
}

.target-submenu-header {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 4px 6px 5px;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border-subtle);
}

.target-submenu-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  max-height: 240px;
  overflow-y: auto;
}

.target-submenu-empty {
  padding: 8px;
  font-size: 11.5px;
  color: var(--text-dim);
  text-align: center;
}

.target-model-item {
  width: 100%;
  border: none;
  background: transparent;
  color: var(--text-main);
  border-radius: 6px;
  padding: 6px 8px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  cursor: pointer;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  text-align: left;
}

.target-model-item:hover {
  background: var(--hover-bg);
}

.target-model-item.active {
  background: var(--active-bg);
  color: var(--accent);
  font-weight: 600;
}

.target-model-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.target-check {
  color: var(--accent);
  font-size: 12px;
  font-weight: 700;
  flex-shrink: 0;
}

.target-settings-btn {
  width: 100%;
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 7px;
  padding: 6px 8px;
  font-size: 11.5px;
  text-align: left;
  cursor: pointer;
  transition: all 0.12s ease;
}

.target-settings-btn:hover {
  background: var(--hover-bg);
  color: var(--accent);
}

/* Permissions des tools (hauteur = 32px) */
.tool-perms-btn {
  height: 32px;
  box-sizing: border-box;
  border-radius: 8px;
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  color: var(--text-muted);
  padding: 0 10px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
}

.tool-perms-btn:hover {
  background: var(--hover-bg);
  color: var(--text-main);
  border-color: var(--accent);
}

.tool-perms-btn.active {
  color: #10b981;
  border-color: rgba(16, 185, 129, 0.35);
  background: rgba(16, 185, 129, 0.08);
}

.tool-perms-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #10b981;
  flex-shrink: 0;
}

.tool-perms-btn:not(.active) .tool-perms-dot {
  background: #f59e0b;
}

/* Bouton d'envoi avec flèche vers la droite (hauteur = 32px) */
.send-btn {
  height: 32px;
  width: 34px;
  box-sizing: border-box;
  border: none;
  background: var(--accent);
  color: #fff;
  border-radius: 8px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.send-btn:hover:not(:disabled) {
  opacity: 0.92;
  transform: translateX(1px);
}

.send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.stop-btn {
  height: 32px;
  width: 32px;
  box-sizing: border-box;
  border: 1px solid var(--border-subtle);
  background: rgba(148, 163, 184, 0.22);
  color: #ef4444;
  border-radius: 50%;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.stop-btn:hover:not(:disabled) {
  background: rgba(148, 163, 184, 0.34);
  border-color: rgba(239, 68, 68, 0.55);
  transform: scale(1.04);
}

.stop-btn:disabled {
  opacity: 0.55;
  cursor: wait;
}

.stop-square-icon {
  width: 14px;
  height: 14px;
}

.live-stop-pill {
  border: 1px solid rgba(239, 68, 68, 0.4);
  background: rgba(239, 68, 68, 0.14);
  color: #ef4444;
  border-radius: 999px;
  padding: 2px 8px;
  font-size: 10.5px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.14s ease;
}

.live-stop-pill:hover:not(:disabled) {
  background: rgba(239, 68, 68, 0.24);
  border-color: #ef4444;
}

.live-stop-pill:disabled {
  opacity: 0.55;
  cursor: wait;
}

.send-arrow-icon {
  width: 16px;
  height: 16px;
}

.mention-badge.mcp {
  background: rgba(16, 185, 129, 0.15);
  color: #10b981;
}

/* ULTRA-WIDE MODE (Largeur > 1 écran standard: discussion positionnée à gauche) */
.la-discussion.is-wide .messages-list {
  margin-inline: 0 !important;
  max-width: 100% !important;
  padding-left: 28px !important;
  padding-right: 28px !important;
}

.la-discussion.is-wide .footer-inner {
  margin-inline: 0 !important;
  max-width: 100% !important;
}

.la-discussion.is-wide .disc-footer {
  padding-left: 28px !important;
  padding-right: 28px !important;
}

.la-discussion.is-wide .welcome-card {
  align-items: flex-start !important;
  text-align: left !important;
  padding-left: 28px !important;
}

.la-discussion.is-wide .welcome-meta {
  justify-content: flex-start !important;
}

.hidden-file-input {
  display: none;
}

/* Modal light-box image preview */
.image-modal-overlay {
  position: fixed;
  inset: 0;
  z-index: 999;
  background: rgba(0, 0, 0, 0.78);
  backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
}

.image-modal-card {
  position: relative;
  max-width: 90vw;
  max-height: 85vh;
  padding: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.modal-close-btn {
  position: absolute;
  top: 14px;
  right: 14px;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  color: var(--text-main);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
}

.modal-full-img {
  max-width: 100%;
  max-height: 80vh;
  border-radius: 10px;
  object-fit: contain;
}

/* Tool approval guardrails */
.tool-approval-stack {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.tool-approval-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 14px;
  border-radius: 12px;
  border: 1px solid rgba(239, 68, 68, 0.35);
  background: var(--surface-elevated);
  box-shadow: 0 8px 24px -10px rgba(0, 0, 0, 0.35);
}

.tool-approval-info {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  flex: 1;
}

.tool-approval-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.tool-approval-badge {
  font-family: var(--font-code);
  font-size: 11px;
  font-weight: 600;
  padding: 2px 7px;
  border-radius: 6px;
  background: var(--diff-del-bg);
  color: var(--diff-del-bar);
  flex-shrink: 0;
}

.tool-approval-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
}

.tool-approval-target {
  font-family: var(--font-code);
  font-size: 12px;
  color: var(--text-main);
  background: var(--table-bg);
  border: 1px solid var(--border-subtle);
  border-radius: 6px;
  padding: 4px 8px;
  overflow-x: auto;
  white-space: nowrap;
}

.tool-approval-reason {
  margin: 0;
  font-size: 12px;
  color: var(--text-muted);
}

.tool-approval-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.tool-approval-btn {
  border-radius: 8px;
  padding: 6px 12px;
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  transition: transform 0.12s ease, opacity 0.12s ease, background 0.15s ease;
}

.tool-approval-btn.is-deny {
  border: 1px solid var(--border-subtle);
  background: var(--table-bg);
  color: var(--text-main);
}

.tool-approval-btn.is-deny:hover {
  background: var(--hover-bg);
}

.tool-approval-btn.is-approve {
  border: 1px solid rgba(239, 68, 68, 0.5);
  background: var(--diff-del-bar);
  color: #ffffff;
}

.tool-approval-btn.is-approve:hover {
  opacity: 0.92;
  transform: translateY(-1px);
}

/* --- PROCESS TIMELINE DROPDOWN --- */
.process-timeline-block {
  margin: 10px 0 14px 0;
  border-radius: 10px;
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  overflow: hidden;
  transition: all 0.2s ease;
}

.process-timeline-block.is-live {
  border-color: rgba(245, 158, 11, 0.4);
  box-shadow: 0 0 16px rgba(245, 158, 11, 0.1);
}

.process-timeline-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 8px 12px;
  background: transparent;
  border: none;
  cursor: pointer;
  text-align: left;
  transition: background 0.15s ease;
}

.process-timeline-header:hover {
  background: var(--hover-bg);
}

.timeline-header-left {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.timeline-status-icon-wrap {
  width: 26px;
  height: 26px;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.timeline-status-icon-wrap.is-success {
  background: rgba(16, 185, 129, 0.15);
  color: #34d399;
  border: 1px solid rgba(16, 185, 129, 0.35);
}

.timeline-status-icon-wrap.has-error {
  background: rgba(244, 63, 94, 0.15);
  color: #fb7185;
  border: 1px solid rgba(244, 63, 94, 0.35);
}

.timeline-status-icon-wrap.is-running {
  background: rgba(245, 158, 11, 0.15);
  color: #fbbf24;
  border: 1px solid rgba(245, 158, 11, 0.35);
}

.timeline-bolt-icon {
  width: 14px;
  height: 14px;
}

.timeline-header-info {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.timeline-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.timeline-main-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-main);
  letter-spacing: 0.01em;
}

.timeline-steps-badge {
  font-size: 10px;
  font-family: var(--font-code);
  font-weight: 500;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--hover-bg);
  color: var(--text-muted);
  border: 1px solid var(--border-subtle);
}

.timeline-live-tag {
  font-size: 10px;
  font-weight: 500;
  color: #fbbf24;
  background: rgba(245, 158, 11, 0.12);
  border: 1px solid rgba(245, 158, 11, 0.3);
  border-radius: 9999px;
  padding: 1px 7px;
  display: inline-flex;
  align-items: center;
  gap: 5px;
}

.timeline-sub-title {
  font-size: 11px;
  color: var(--text-muted);
  margin-top: 1px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.timeline-header-right {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}

.timeline-stat-tag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: 11px;
  font-family: var(--font-code);
}

.timeline-stat-tag.is-success {
  color: #34d399;
}

.timeline-stat-tag.is-error {
  color: #fb7185;
}

.timeline-stat-duration {
  font-size: 11px;
  font-family: var(--font-code);
  color: var(--text-muted);
  border-left: 1px solid var(--border-subtle);
  padding-left: 8px;
}

.timeline-chevron-icon {
  width: 15px;
  height: 15px;
  color: var(--text-muted);
  transition: transform 0.2s ease;
}

.timeline-chevron-icon.rotated {
  transform: rotate(180deg);
}

.process-timeline-body {
  padding: 10px 14px 12px 14px;
  position: relative;
  max-height: 380px;
  overflow-y: auto;
  border-top: 1px solid var(--border-subtle);
}

.timeline-track-rail {
  position: absolute;
  left: 25px;
  top: 18px;
  bottom: 18px;
  width: 2px;
  background: var(--border-subtle);
  pointer-events: none;
}

.timeline-step-row {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: 12px;
  margin-bottom: 8px;
  z-index: 1;
}

.timeline-step-row:last-child {
  margin-bottom: 0;
}

.timeline-step-bullet {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.2);
}

.timeline-step-bullet.is-success {
  background: rgba(16, 185, 129, 0.15);
  border: 1px solid rgba(16, 185, 129, 0.45);
  color: #34d399;
}

.timeline-step-bullet.is-error {
  background: rgba(244, 63, 94, 0.15);
  border: 1px solid rgba(244, 63, 94, 0.45);
  color: #fb7185;
}

.timeline-step-bullet.is-running {
  background: rgba(245, 158, 11, 0.15);
  border: 1px solid rgba(245, 158, 11, 0.45);
  color: #fbbf24;
}

.bullet-icon {
  width: 12px;
  height: 12px;
}

.timeline-step-card {
  flex: 1;
  min-width: 0;
  border: 1px solid var(--border-subtle);
  border-radius: 7px;
  background: var(--hover-bg);
  padding: 6px 9px;
  transition: all 0.15s ease;
}

.timeline-step-card.is-error {
  border-color: rgba(244, 63, 94, 0.35);
  background: rgba(244, 63, 94, 0.05);
}

.timeline-step-card.is-running {
  border-color: rgba(245, 158, 11, 0.4);
  background: rgba(245, 158, 11, 0.05);
}

.step-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.step-card-left {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  flex-wrap: wrap;
}

.step-tool-badge {
  padding: 1px 6px;
  border-radius: 4px;
  font-size: 10px;
  font-family: var(--font-code);
  font-weight: 600;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.step-tool-badge.is-terminal {
  background: rgba(245, 158, 11, 0.15);
  color: #fbbf24;
  border: 1px solid rgba(245, 158, 11, 0.3);
}

.step-tool-badge.is-create {
  background: rgba(56, 189, 248, 0.15);
  color: #38bdf8;
  border: 1px solid rgba(56, 189, 248, 0.3);
}

.step-tool-badge.is-edit {
  background: rgba(168, 85, 247, 0.15);
  color: #c084fc;
  border: 1px solid rgba(168, 85, 247, 0.3);
}

.step-tool-badge.is-search {
  background: rgba(16, 185, 129, 0.15);
  color: #34d399;
  border: 1px solid rgba(16, 185, 129, 0.3);
}

.step-tool-badge.is-mcp {
  background: rgba(236, 72, 153, 0.15);
  color: #f472b6;
  border: 1px solid rgba(236, 72, 153, 0.3);
}

.step-tool-badge.is-file {
  background: rgba(148, 163, 184, 0.15);
  color: #cbd5e1;
  border: 1px solid rgba(148, 163, 184, 0.3);
}

.tool-mini-icon {
  width: 11px;
  height: 11px;
}

.step-card-title {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-main);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.step-error-tag {
  font-size: 9px;
  font-family: var(--font-code);
  font-weight: 600;
  padding: 1px 5px;
  border-radius: 3px;
  background: rgba(244, 63, 94, 0.2);
  color: #fb7185;
  border: 1px solid rgba(244, 63, 94, 0.35);
}

.step-running-tag {
  font-size: 10px;
  color: #fbbf24;
}

.step-card-right {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.step-duration {
  font-size: 10px;
  color: var(--text-muted);
}

.step-log-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 10px;
  font-weight: 500;
  padding: 2px 6px;
  border-radius: 4px;
  background: transparent;
  border: 1px solid var(--border-subtle);
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.12s ease;
}

.step-log-btn:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.log-chevron {
  width: 10px;
  height: 10px;
  transition: transform 0.2s ease;
}

.log-chevron.rotated {
  transform: rotate(180deg);
}

.step-log-panel {
  margin-top: 6px;
  padding: 6px 8px;
  border-radius: 5px;
  background: rgba(0, 0, 0, 0.35);
  border: 1px solid var(--border-subtle);
  max-height: 160px;
  overflow: auto;
}

.step-log-pre {
  margin: 0;
  font-size: 11px;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text-muted);
  line-height: 1.4;
}
</style>
