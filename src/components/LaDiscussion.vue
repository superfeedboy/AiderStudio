<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import hljs from "highlight.js";
import "highlight.js/styles/github-dark.css";
import { useStudioStore } from "../stores/studio";
import type { AttachedFile, ChatMessage, FileChangeSummary } from "../types/studio";
import AppLogo from "./AppLogo.vue";
import LeHud from "./LeHud.vue";
import MarkdownInline from "./MarkdownInline.vue";
import { tokenizeInline, type InlineToken, type ResolvedFileTarget } from "../utils/markdownInline";

const store = useStudioStore();
const scrollRef = ref<HTMLElement | null>(null);
const textareaRef = ref<HTMLTextAreaElement | null>(null);
const fileInputRef = ref<HTMLInputElement | null>(null);
const promptDraft = ref("");
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

async function toggleFilesDropdown(msgId: string) {
  const willOpen = !openFilesDropdown.value[msgId];
  openFilesDropdown.value[msgId] = willOpen;

  if (!willOpen) return;

  await nextTick();

  const container = scrollRef.value;
  const wrapEl = document.getElementById(`diff-dropdown-${msgId}`);
  if (!container || !wrapEl) return;

  const containerRect = container.getBoundingClientRect();
  const wrapRect = wrapEl.getBoundingClientRect();
  const bottomMargin = 20;
  const topMargin = 16;

  if (wrapRect.bottom > containerRect.bottom - bottomMargin) {
    const needed = wrapRect.bottom - (containerRect.bottom - bottomMargin);
    const maxShift = Math.max(0, wrapRect.top - (containerRect.top + topMargin));
    container.scrollTop += Math.min(needed, maxShift);
  } else if (wrapRect.top < containerRect.top + topMargin) {
    container.scrollTop -= containerRect.top + topMargin - wrapRect.top;
  }
}

// Thinking state per message
const openThinkingIds = ref<Set<string>>(new Set());

function toggleThinking(msgId: string) {
  if (openThinkingIds.value.has(msgId)) {
    openThinkingIds.value.delete(msgId);
  } else {
    openThinkingIds.value.add(msgId);
  }
}

// File and image attachments
const attachedFiles = ref<AttachedFile[]>([]);

// Mention system (@)
interface MentionItem {
  id: string;
  name: string;
  description: string;
  category: "file" | "tool" | "plugin" | "mcp";
  tag: string;
}

const mentionOpen = ref(false);
const mentionQuery = ref("");
const mentionActiveIndex = ref(0);
const mentionCategory = ref<"all" | "file" | "tool" | "plugin" | "mcp">("all");
const projectFiles = ref<string[]>([]);

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
}

const allMentions = computed<MentionItem[]>(() => {
  const fileItems: MentionItem[] = projectFiles.value.map((f) => ({
    id: `file-${f}`,
    name: f,
    description: "Fichier du projet",
    category: "file",
    tag: `@file:${f}`,
  }));
  return [...fileItems, ...builtInTools, ...builtInPlugins, ...mcpMentions.value];
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
  synthesizer: "#94a3b8",
};

const providerShortLabels: Record<string, string> = {
  ollama: "Ollama",
  gemini: "Gemini",
  openai: "OpenAI",
  anthropic: "Claude",
  deepseek: "DeepSeek",
  openrouter: "OpenRouter",
  mistral: "Mistral",
};

function roleTraceDotColor(role: string): string {
  return roleTraceColors[role] || "#3b82f6";
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

function scrollToBottom(force = false) {
  nextTick(() => {
    if (scrollRef.value) {
      scrollRef.value.scrollTop = scrollRef.value.scrollHeight;
      requestAnimationFrame(() => {
        if (scrollRef.value) {
          scrollRef.value.scrollTop = scrollRef.value.scrollHeight;
        }
      });
    }
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
    streamObserver = new MutationObserver(() => {
      if (store.isProcessing) {
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
  document.removeEventListener("click", handleTargetOutsideClick);
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

const isRenamingTitle = ref(false);
const titleDraft = ref("");
const titleInputRef = ref<HTMLInputElement | null>(null);

watch(
  () => store.activeConversationId,
  () => {
    isRenamingTitle.value = false;
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
      !/^\s*\d+\.\s+/.test(lines[i])
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
  const extMap: Record<string, string> = {
    vue: "Component.vue",
    ts: "script.ts",
    typescript: "script.ts",
    js: "script.js",
    javascript: "script.js",
    rs: "main.rs",
    rust: "main.rs",
    py: "script.py",
    python: "script.py",
    html: "index.html",
    css: "styles.css",
    scss: "styles.scss",
    sql: "query.sql",
    json: "data.json",
    yaml: "config.yaml",
    yml: "config.yaml",
    toml: "config.toml",
    sh: "script.sh",
    bash: "script.sh",
    zsh: "script.sh",
    md: "document.md",
    markdown: "document.md",
  };

  return extMap[cleanLang] || `code.${cleanLang || "txt"}`;
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

function renderCodeHtml(code: string, lang: string, isStreaming = false): string {
  if (!code) return "";
  const cleanLang = (lang || "").trim().toLowerCase();
  const mappedLang = HLJS_LANG_ALIASES[cleanLang] || cleanLang;
  try {
    if (
      mappedLang &&
      mappedLang !== "code" &&
      mappedLang !== "txt" &&
      mappedLang !== "text" &&
      hljs.getLanguage(mappedLang)
    ) {
      return hljs.highlight(code, { language: mappedLang, ignoreIllegals: true }).value;
    }
    if (
      isStreaming ||
      !cleanLang ||
      cleanLang === "code" ||
      cleanLang === "txt" ||
      cleanLang === "text" ||
      code.length > 20_000
    ) {
      return escapeHtml(code);
    }
    return hljs.highlightAuto(code).value;
  } catch {
    return escapeHtml(code);
  }
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

function sanitizeLiveContent(raw: string): string {
  if (!raw) return "";
  return raw
    .replace(/<tool_call>[\s\S]*?<\/tool_call>/g, "")
    .replace(/<tool_call(?:\s[^>]*)?>[\s\S]*$/g, "")
    .replace(/<tool(?:_c(?:a(?:l(?:l)?)?)?)?$/g, "")
    .replace(/^\s*```(?:json:)?tool_call\b[^\n]*\n[\s\S]*?^\s*```\s*$/gm, "")
    .replace(/^\s*```(?:json:)?tool_call\b[^\n]*(?:\n[\s\S]*)?$/m, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function parseMarkdownMessage(content: string, isStreaming = false): MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  const normalized = (content || "").replace(/\r\n/g, "\n");
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
  return blocks;
}

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
  idx: number
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
  if (
    idx === lastAssistantIndex.value &&
    (conv.value?.modifiedFiles.length ?? 0) > 0
  ) {
    return {
      filesChanged: conv.value?.modifiedFiles.length ?? 0,
      linesAdded: conv.value?.linesAdded ?? 0,
      linesRemoved: conv.value?.linesRemoved ?? 0,
    };
  }
  return null;
}

function getFileListForMessage(msg: ChatMessage, idx: number): FileChangeSummary[] {
  if (idx === lastAssistantIndex.value && (conv.value?.modifiedFiles.length ?? 0) > 0) {
    return conv.value!.modifiedFiles;
  }
  if (msg.filesSummary && msg.filesSummary.length > 0) {
    return msg.filesSummary;
  }
  return [];
}

function canUndoMessageFiles(idx: number): boolean {
  return idx === lastAssistantIndex.value && (conv.value?.modifiedFiles.length ?? 0) > 0;
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
  for (const f of conv.value?.modifiedFiles ?? []) {
    addPath(f.path);
  }
  if (msg?.filesSummary) {
    for (const f of msg.filesSummary) {
      addPath(f.path);
    }
  }
  return paths;
}

function handleOpenFileInReview(payload: ResolvedFileTarget) {
  store.openReview(payload.path, payload.line);
}

function getFileStatus(file: FileChangeSummary): "created" | "modified" | "deleted" {
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
  promptDraft.value = content;
  scrollToBottom(true);
  nextTick(() => {
    if (textareaRef.value) {
      textareaRef.value.focus();
      textareaRef.value.setSelectionRange(content.length, content.length);
    }
  });
}

async function confirmUndoFile(filePath: string) {
  const ok = window.confirm(
    `Voulez-vous annuler toutes les modifications apportées au fichier "${filePath}" et le restaurer dans son état d'origine ?`
  );
  if (!ok) return;
  await store.undoFile(filePath);
}

async function confirmUndoAll() {
  const count = conv.value?.modifiedFiles.length ?? 0;
  const ok = window.confirm(
    `Voulez-vous annuler TOUTES les modifications (${count} fichier${count > 1 ? "s" : ""}) et restaurer le projet dans son état d'origine ?\nCette action est irréversible.`
  );
  if (!ok) return;
  await store.undoAll();
}

function openFilePicker() {
  fileInputRef.value?.click();
}

async function processFileList(files: FileList | File[]) {
  for (const file of Array.from(files)) {
    const isImage = file.type.startsWith("image/");
    const id = `att-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

    if (isImage) {
      const reader = new FileReader();
      reader.onload = (e) => {
        attachedFiles.value.push({
          id,
          name: file.name,
          size: file.size,
          mimeType: file.type,
          isImage: true,
          dataUrl: e.target?.result as string,
        });
      };
      reader.readAsDataURL(file);
    } else {
      const reader = new FileReader();
      reader.onload = (e) => {
        attachedFiles.value.push({
          id,
          name: file.name,
          size: file.size,
          mimeType: file.type || "text/plain",
          isImage: false,
          content: e.target?.result as string,
        });
      };
      reader.readAsText(file);
    }
  }
}

function onFileInputChange(ev: Event) {
  const target = ev.target as HTMLInputElement;
  if (target.files && target.files.length > 0) {
    void processFileList(target.files);
    target.value = "";
  }
}

function removeAttachment(id: string) {
  attachedFiles.value = attachedFiles.value.filter((a) => a.id !== id);
}

function handlePaste(ev: ClipboardEvent) {
  const items = ev.clipboardData?.items;
  if (!items) return;
  const imageFiles: File[] = [];
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.type.startsWith("image/")) {
      const file = item.getAsFile();
      if (file) imageFiles.push(file);
    }
  }
  if (imageFiles.length > 0) {
    void processFileList(imageFiles);
  }
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
    void processFileList(ev.dataTransfer.files);
  }
}

function bulletForLevel(level: number): string {
  return level <= 0 ? "•" : level === 1 ? "◦" : "▪";
}

function autoResizeTextarea() {
  const el = textareaRef.value;
  if (!el) return;
  el.style.height = "auto";
  const nextHeight = Math.min(220, Math.max(48, el.scrollHeight));
  el.style.height = `${nextHeight}px`;
}

function handleTextareaInput(ev: Event) {
  autoResizeTextarea();
  const el = ev.target as HTMLTextAreaElement;
  const cursor = el.selectionStart;
  const textBefore = promptDraft.value.slice(0, cursor);
  const match = /(?:^|\s)@([a-zA-Z0-9_\-./:]*)$/.exec(textBefore);
  if (match) {
    mentionOpen.value = true;
    mentionQuery.value = match[1];
    mentionActiveIndex.value = 0;
  } else {
    mentionOpen.value = false;
  }
}

function selectMention(item?: MentionItem) {
  if (!item) return;
  const el = textareaRef.value;
  const cursor = el?.selectionStart ?? promptDraft.value.length;
  const textBefore = promptDraft.value.slice(0, cursor);
  const textAfter = promptDraft.value.slice(cursor);
  const match = /(?:^|\s)@([a-zA-Z0-9_\-./:]*)$/.exec(textBefore);
  if (!match) return;

  const atIndex = textBefore.lastIndexOf("@");
  const newTextBefore = textBefore.slice(0, atIndex) + item.tag + " ";
  promptDraft.value = newTextBefore + textAfter;
  mentionOpen.value = false;

  nextTick(() => {
    if (el) {
      el.focus();
      const nextPos = newTextBefore.length;
      el.setSelectionRange(nextPos, nextPos);
      autoResizeTextarea();
    }
  });
}

async function handleSubmit() {
  const text = promptDraft.value.trim();
  const files = [...attachedFiles.value];
  if ((!text && files.length === 0) || store.isProcessing) return;

  promptDraft.value = "";
  attachedFiles.value = [];
  mentionOpen.value = false;
  await nextTick();
  if (textareaRef.value) {
    textareaRef.value.style.height = "";
  }
  scrollToBottom(true);
  await store.sendPrompt(text, files);
  scrollToBottom(true);
}

function handleKeydown(ev: KeyboardEvent) {
  if (mentionOpen.value && filteredMentions.value.length > 0) {
    if (ev.key === "ArrowDown") {
      ev.preventDefault();
      mentionActiveIndex.value =
        (mentionActiveIndex.value + 1) % filteredMentions.value.length;
      return;
    }
    if (ev.key === "ArrowUp") {
      ev.preventDefault();
      mentionActiveIndex.value =
        (mentionActiveIndex.value - 1 + filteredMentions.value.length) %
        filteredMentions.value.length;
      return;
    }
    if (ev.key === "Enter" || ev.key === "Tab") {
      ev.preventDefault();
      selectMention(filteredMentions.value[mentionActiveIndex.value]);
      return;
    }
    if (ev.key === "Escape") {
      ev.preventDefault();
      mentionOpen.value = false;
      return;
    }
  }

  const el = textareaRef.value;

  if (ev.key === "Tab" && el) {
    ev.preventDefault();
    const value = promptDraft.value;
    const selStart = el.selectionStart;
    const selEnd = el.selectionEnd;
    const blockStart = value.lastIndexOf("\n", selStart - 1) + 1;
    const rawBlockEnd = value.indexOf("\n", selEnd);
    const blockEnd = rawBlockEnd === -1 ? value.length : rawBlockEnd;
    const blockLines = value.slice(blockStart, blockEnd).split("\n");

    const transformed = blockLines.map((line) => {
      const m = /^(\s*)(?:-(?=\s|$)|[•◦▪])(\s*)(.*)$/.exec(line);
      if (m) {
        const curIndent = m[1];
        const rest = m[3];
        const nextIndent = ev.shiftKey
          ? curIndent.slice(0, Math.max(0, curIndent.length - 2))
          : curIndent.length < 8
            ? curIndent + "  "
            : curIndent;
        const glyph = bulletForLevel(Math.floor(nextIndent.length / 2));
        return `${nextIndent}${glyph} ${rest}`;
      }
      if (ev.shiftKey) {
        return line.replace(/^ {1,2}/, "");
      }
      return `  ${line}`;
    });

    const newBlock = transformed.join("\n");
    promptDraft.value = value.slice(0, blockStart) + newBlock + value.slice(blockEnd);

    const delta = newBlock.length - (blockEnd - blockStart);
    nextTick(() => {
      el.focus();
      if (blockLines.length === 1 && selStart === selEnd) {
        const nextCaret = Math.max(
          blockStart,
          Math.min(blockStart + newBlock.length, selStart + delta)
        );
        el.setSelectionRange(nextCaret, nextCaret);
      } else {
        el.setSelectionRange(blockStart, blockStart + newBlock.length);
      }
      autoResizeTextarea();
    });
    return;
  }

  if (ev.key === "Enter" && ev.shiftKey && el) {
    const value = promptDraft.value;
    const selStart = el.selectionStart;
    const selEnd = el.selectionEnd;
    const lineStart = value.lastIndexOf("\n", selStart - 1) + 1;
    const rawLineEnd = value.indexOf("\n", selEnd);
    const lineEnd = rawLineEnd === -1 ? value.length : rawLineEnd;
    const lineText = value.slice(lineStart, lineEnd);
    const match = /^(\s*)(?:-(?=\s|$)|[•◦▪])(\s*)(.*)$/.exec(lineText);

    if (match) {
      ev.preventDefault();
      const indent = match[1];
      const rest = match[3];
      const level = Math.floor(indent.length / 2);

      if (rest.trim() === "") {
        if (indent.length >= 2) {
          const nextIndent = indent.slice(2);
          const nextGlyph = bulletForLevel(Math.floor(nextIndent.length / 2));
          const updatedLine = `${nextIndent}${nextGlyph} `;
          promptDraft.value =
            value.slice(0, lineStart) + updatedLine + value.slice(lineEnd);
          const nextPos = lineStart + updatedLine.length;
          nextTick(() => {
            el.focus();
            el.setSelectionRange(nextPos, nextPos);
            autoResizeTextarea();
          });
        } else {
          promptDraft.value = value.slice(0, lineStart) + value.slice(lineEnd);
          nextTick(() => {
            el.focus();
            el.setSelectionRange(lineStart, lineStart);
            autoResizeTextarea();
          });
        }
        return;
      }

      const glyph = bulletForLevel(level);
      const newPrefix = `${indent}${glyph} `;
      const markerLen = lineText.length - rest.length;
      const relStartInRest = Math.max(0, selStart - lineStart - markerLen);
      const relEndInRest = Math.max(relStartInRest, selEnd - lineStart - markerLen);
      const beforeRest = rest.slice(0, relStartInRest);
      const afterRest = rest.slice(relEndInRest);
      const updatedCurrentLine = `${newPrefix}${beforeRest}`;
      const nextBulletPrefix = `\n${newPrefix}`;

      promptDraft.value =
        value.slice(0, lineStart) +
        updatedCurrentLine +
        nextBulletPrefix +
        afterRest +
        value.slice(lineEnd);

      const nextPos =
        lineStart + updatedCurrentLine.length + nextBulletPrefix.length;
      nextTick(() => {
        el.focus();
        el.setSelectionRange(nextPos, nextPos);
        autoResizeTextarea();
      });
      return;
    }
  }

  if (ev.key === "Enter" && !ev.shiftKey) {
    ev.preventDefault();
    void handleSubmit();
  }
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
        <button
          type="button"
          class="breadcrumb-project"
          title="Paramètres et contexte global du projet"
          @click="store.openProjectSettings(store.activeProject)"
        >
          {{ store.activeProject?.name || "Projet" }}
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

    <!-- Scrollable stream or Empty welcome state -->
    <div ref="scrollRef" class="disc-stream custom-scroll">
      <!-- Empty state before first prompt -->
      <div v-if="!hasStarted && !store.isProcessing" class="welcome-state">
        <div class="welcome-logo halo-hud">
          <AppLogo :size="36" />
        </div>
        <h1 class="welcome-title">Qu'est-ce qu'on code patron?</h1>
        <div class="welcome-meta">
          <span class="meta-chip">{{ store.activeProject?.name }}</span>
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
      </div>

      <!-- Conversation messages -->
      <div v-else class="messages-list">
        <div
          v-for="(msg, msgIdx) in messages"
          :key="msg.id"
          class="msg-wrapper"
          :class="msg.role === 'user' ? 'is-user' : 'is-assistant'"
        >
          <!-- USER MESSAGE -->
          <div v-if="msg.role === 'user'" class="user-msg-col">
            <div class="user-bubble-box">
              <div class="user-bubble">
                <!-- Attached images in user message -->
                <div
                  v-if="msg.attachments && msg.attachments.some((a) => a.isImage)"
                  class="msg-gallery"
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

                <p class="msg-text">{{ msg.content }}</p>
              </div>

              <!-- User prompt hover action buttons (copier, relancer) -->
              <div class="user-hover-actions">
                <button
                  class="user-action-pill"
                  :title="copiedPromptId === msg.id ? 'Prompt copié !' : 'Copier le prompt'"
                  @click="copyPrompt(msg.content, msg.id)"
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
                  @click="rerunPrompt(msg.content)"
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
          <article v-else class="assistant-article">
            <!-- AI Thinking Section (presented right under user prompt) -->
            <div v-if="msg.thinking || (msg.agentTrace && msg.agentTrace.length > 0)" class="thinking-block">
              <button
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
                  class="trace-timeline"
                >
                  <div
                    v-for="(step, sIdx) in msg.agentTrace"
                    :key="`${step.role}-${sIdx}`"
                    class="trace-step-card"
                    :style="{ '--step-role-color': roleTraceDotColor(step.role) }"
                  >
                    <div class="trace-step-top">
                      <div class="trace-role-wrap">
                        <span
                          class="trace-role-dot"
                          :style="{ backgroundColor: roleTraceDotColor(step.role) }"
                        />
                        <strong class="trace-role-label">{{ step.roleLabel }}</strong>
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
                        :tokens="tokenizeInline(step.summary)"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
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
                        @open-file="handleOpenFileInReview"
                      />
                    </h1>
                    <h2 v-else-if="block.kind === 'heading' && block.level === 2" class="tmd-h2">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
                    </h2>
                    <h3 v-else-if="block.kind === 'heading' && block.level === 3" class="tmd-h3">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
                    </h3>
                    <blockquote v-else-if="block.kind === 'quote'" class="tmd-quote">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
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
                          :available-paths="getAvailableDiffPaths(msg)"
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
                          :available-paths="getAvailableDiffPaths(msg)"
                          :project-root="store.activeProject?.rootPath"
                          variant="thinking"
                          @open-file="handleOpenFileInReview"
                        />
                      </li>
                    </ol>
                    <pre v-else-if="block.kind === 'code'" class="tmd-code-block custom-scroll"><code class="hljs" v-html="renderCodeHtml(block.body, block.lang, Boolean(block.streaming))" /></pre>
                    <p v-else-if="block.kind === 'paragraph'" class="tmd-p">
                      <MarkdownInline
                        :tokens="block.tokens"
                        :available-paths="getAvailableDiffPaths(msg)"
                        :project-root="store.activeProject?.rootPath"
                        variant="thinking"
                        @open-file="handleOpenFileInReview"
                      />
                    </p>
                  </template>
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
                    @open-file="handleOpenFileInReview"
                  />
                </h1>

                <h2 v-else-if="block.kind === 'heading' && block.level === 2" class="md-h2">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="handleOpenFileInReview"
                  />
                </h2>

                <h3 v-else-if="block.kind === 'heading' && block.level === 3" class="md-h3">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="handleOpenFileInReview"
                  />
                </h3>

                <blockquote v-else-if="block.kind === 'quote'" class="md-quote">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
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
                      :available-paths="getAvailableDiffPaths(msg)"
                      :project-root="store.activeProject?.rootPath"
                      @open-file="handleOpenFileInReview"
                    />
                  </li>
                </ul>

                <ol v-else-if="block.kind === 'ol'" class="md-ol">
                  <li v-for="(item, liIdx) in block.items" :key="liIdx">
                    <MarkdownInline
                      :tokens="item"
                      :available-paths="getAvailableDiffPaths(msg)"
                      :project-root="store.activeProject?.rootPath"
                      @open-file="handleOpenFileInReview"
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

                <p v-else-if="block.kind === 'paragraph'" class="md-p">
                  <MarkdownInline
                    :tokens="block.tokens"
                    :available-paths="getAvailableDiffPaths(msg)"
                    :project-root="store.activeProject?.rootPath"
                    @open-file="handleOpenFileInReview"
                  />
                </p>
              </template>
            </div>

            <!-- Dropdown Sélecteur des fichiers modifiés avec diff -->
            <div
              v-if="messageDiffStats(msg, msgIdx)"
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
                    title="Annuler toutes les modifications apportées aux fichiers du projet"
                    @click.stop="confirmUndoAll()"
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
                    title="Cliquer pour afficher la review"
                    @click="store.openReview(file.path)"
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
                        :title="`Annuler les modifications de ${file.path}`"
                        @click.stop="confirmUndoFile(file.path)"
                      >
                        <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor">
                          <path d="M5.82843 7.00005L8.36396 9.53558L6.94975 10.9498L2 6.00005L6.94975 1.05031L8.36396 2.46452L5.82843 5.00005H13C16.866 5.00005 20 8.13406 20 12C20 15.866 16.866 19 13 19H4V17H13C15.7614 17 18 14.7615 18 12C18 9.23862 15.7614 7.00005 13 7.00005H5.82843Z"/>
                        </svg>
                      </button>
                    </div>
                  </div>
                </div>

                <div class="diff-menu-footer">
                  <button type="button" class="diff-full-review-cta" @click="store.openReview()">
                    <span>Ouvrir l'éditeur de review complet</span>
                    <svg class="cta-arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <line x1="5" y1="12" x2="19" y2="12" />
                      <polyline points="12 5 19 12 12 19" />
                    </svg>
                  </button>
                </div>
              </div>
            </div>

            <!-- Relative Timestamp -->
            <div class="msg-timestamp">
              Studio ({{ msg.stageSummary || store.settings.ollamaModel }}) · {{ formatRelativeTime(msg.createdAt) }}
            </div>
          </article>
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
                    class="trace-timeline"
                  >
                    <div
                      v-for="(step, sIdx) in store.liveAgentTrace"
                      :key="`live-${step.role}-${sIdx}`"
                      class="trace-step-card"
                      :style="{ '--step-role-color': roleTraceDotColor(step.role) }"
                    >
                      <div class="trace-step-top">
                        <div class="trace-role-wrap">
                          <span
                            class="trace-role-dot"
                            :style="{ backgroundColor: roleTraceDotColor(step.role) }"
                          />
                          <strong class="trace-role-label">{{ step.roleLabel }}</strong>
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
                          :tokens="tokenizeInline(step.summary)"
                          :available-paths="getAvailableDiffPaths()"
                          :project-root="store.activeProject?.rootPath"
                          variant="thinking"
                          @open-file="handleOpenFileInReview"
                        />
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

            <!-- Live Content streaming right after thinking -->
            <div v-if="sanitizeLiveContent(store.liveContent)" class="md-content live-content">
              <template
                v-for="(block, bIdx) in parseMarkdownMessage(sanitizeLiveContent(store.liveContent), true)"
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

    <!-- Hidden file input for file/image attachments -->
    <input
      ref="fileInputRef"
      type="file"
      multiple
      class="hidden-file-input"
      accept="image/*,.txt,.md,.json,.js,.ts,.vue,.rs,.py,.html,.css,.yml,.yaml,.toml,.sh"
      @change="onFileInputChange"
    />

    <!-- Bottom: Prompt Input -->
    <footer class="disc-footer">
      <div class="footer-inner">
        <!-- Prompt composer with rotating rainbow border during thinking -->
        <div class="composer-wrapper">
          <!-- Mention (@) Autocompletion Popover -->
          <div v-if="mentionOpen" class="mention-popover halo-hud custom-scroll">
            <div class="mention-header">
              <span class="mention-hint">Spécifier des outils, fichiers ou plugins (@)</span>
              <div class="mention-tabs">
                <button
                  class="mention-tab"
                  :class="{ active: mentionCategory === 'all' }"
                  @click="mentionCategory = 'all'"
                >
                  Tous
                </button>
                <button
                  class="mention-tab"
                  :class="{ active: mentionCategory === 'file' }"
                  @click="mentionCategory = 'file'"
                >
                  Fichiers
                </button>
                <button
                  class="mention-tab"
                  :class="{ active: mentionCategory === 'tool' }"
                  @click="mentionCategory = 'tool'"
                >
                  Outils
                </button>
                <button
                  class="mention-tab"
                  :class="{ active: mentionCategory === 'plugin' }"
                  @click="mentionCategory = 'plugin'"
                >
                  Plugins
                </button>
                <button
                  class="mention-tab"
                  :class="{ active: mentionCategory === 'mcp' }"
                  @click="mentionCategory = 'mcp'"
                >
                  MCP (Bases/APIs)
                </button>
              </div>
            </div>

            <div class="mention-list">
              <div v-if="filteredMentions.length === 0" class="mention-empty">
                Aucun résultat pour "{{ mentionQuery }}"
              </div>
              <button
                v-for="(item, mIdx) in filteredMentions"
                :key="item.id"
                class="mention-item"
                :class="{ active: mIdx === mentionActiveIndex }"
                @mouseenter="mentionActiveIndex = mIdx"
                @click="selectMention(item)"
              >
                <span class="mention-badge" :class="item.category">
                  {{ item.category === 'file' ? '📁' : item.category === 'tool' ? '⚙️' : item.category === 'plugin' ? '🧩' : '🔌' }}
                </span>
                <div class="mention-info">
                  <span class="mention-name">{{ item.name }}</span>
                  <span class="mention-desc">{{ item.description }}</span>
                </div>
                <code class="mention-tag">{{ item.tag }}</code>
              </button>
            </div>
          </div>

          <!-- Attached files / images preview bar (chevauchant le cadre en z-index supérieur) -->
          <div v-if="attachedFiles.length > 0" class="attachments-bar floating-bar">
            <div
              v-for="att in attachedFiles"
              :key="att.id"
              class="att-chip"
              :class="{ 'is-image': att.isImage }"
            >
              <img
                v-if="att.isImage && att.dataUrl"
                :src="att.dataUrl"
                class="att-thumb"
                alt="thumbnail"
                @click="previewModalImage = att.dataUrl || null"
              />
              <svg
                v-else
                class="att-file-icon"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
              >
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                <polyline points="14 2 14 8 20 8" />
              </svg>
              <span class="att-name" :title="att.name">{{ att.name }}</span>
              <button
                class="att-remove-btn"
                title="Supprimer"
                @click.stop="removeAttachment(att.id)"
              >
                ✕
              </button>
            </div>
          </div>

          <!-- Composer Box with Rainbow Border when thinking -->
          <div
            class="composer halo-input"
            :class="{ 'is-thinking': store.isProcessing }"
          >
            <!-- Ligne 1 (extensible): input de texte du prompt -->
            <div class="composer-line-1">
              <textarea
                ref="textareaRef"
                v-model="promptDraft"
                rows="2"
                placeholder="Décrivez ce que vous voulez coder... (Tapez @ pour outils/fichiers/MCP, collez des images)"
                :disabled="store.isProcessing"
                @input="handleTextareaInput"
                @keydown="handleKeydown"
                @paste="handlePaste"
              />
            </div>

            <!-- Ligne 2: Bouton PJ + choix modèle à gauche, permissions tools et bouton envoi (flèche droite) à droite -->
            <div class="composer-line-2">
              <!-- À gauche: Bouton PJ suivi du choix de modèle (les 2 à la même hauteur) -->
              <div class="composer-line-2-left">
                <button
                  type="button"
                  class="attach-btn"
                  title="Ajouter des images ou des fichiers à interpréter"
                  :disabled="store.isProcessing"
                  @click="openFilePicker"
                >
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l7.88-7.88" />
                  </svg>
                </button>

                <div ref="targetPickerRef" class="model-picker-wrapper">
                  <button
                    type="button"
                    class="target-trigger-btn"
                    :class="{ 'is-open': targetMenuOpen }"
                    :disabled="store.isProcessing"
                    :title="
                      store.activeTargetLabel.isAgent
                        ? 'Agent (Multi-agents selon vos paramètres)'
                        : `${store.activeTargetLabel.providerName} · ${store.activeTargetLabel.modelName}`
                    "
                    @click.stop="toggleTargetMenu"
                  >
                    <AppLogo
                      v-if="store.activeTargetLabel.isAgent"
                      :size="15"
                    />
                    <span
                      v-else
                      class="provider-dot"
                      :style="{
                        backgroundColor: providerDotColor(
                          store.activeTargetLabel.providerId
                        ),
                      }"
                    />
                    <span class="target-model-label">
                      {{
                        store.activeTargetLabel.isAgent
                          ? "Agent"
                          : store.activeTargetLabel.modelName
                      }}
                    </span>
                    <svg
                      class="target-chevron"
                      :class="{ rotated: targetMenuOpen }"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    >
                      <path d="m18 15-6-6-6 6" />
                    </svg>
                  </button>

                  <!-- Upward cascading popover menu -->
                  <div v-if="targetMenuOpen" class="target-popover halo-hud">
                    <button
                      type="button"
                      class="target-agent-item"
                      :class="{ active: store.activeTargetLabel.isAgent }"
                      @click="selectTarget('agent')"
                    >
                      <AppLogo :size="16" />
                      <div class="target-item-copy">
                        <strong>Agent</strong>
                        <span>Multi-agents</span>
                      </div>
                      <span
                        v-if="store.activeTargetLabel.isAgent"
                        class="target-check"
                      >
                        ✓
                      </span>
                    </button>

                    <div class="target-divider" />

                    <div
                      v-for="grp in providerGroups"
                      :key="grp.id"
                      class="target-provider-row"
                      :class="{
                        'is-active-sub': hoveredProvider === grp.id,
                        'is-selected-prov':
                          !store.activeTargetLabel.isAgent &&
                          store.activeTargetLabel.providerId === grp.id,
                      }"
                      @mouseenter="hoveredProvider = grp.id"
                      @click.stop="
                        hoveredProvider =
                          hoveredProvider === grp.id ? null : grp.id
                      "
                    >
                      <span
                        class="provider-dot"
                        :style="{ backgroundColor: grp.color }"
                      />
                      <span class="target-prov-name">{{ grp.name }}</span>
                      <span class="target-prov-count">
                        {{ grp.models.length }}
                      </span>
                      <span class="target-arrow">›</span>

                      <!-- Cascading submenu to the right (bridge padding prevents hover drop) -->
                      <div
                        v-if="hoveredProvider === grp.id"
                        class="target-submenu-bridge"
                        @click.stop
                      >
                        <div class="target-submenu halo-hud">
                          <div class="target-submenu-header">
                            <span
                              class="provider-dot"
                              :style="{ backgroundColor: grp.color }"
                            />
                            <span>{{ grp.name }}</span>
                          </div>
                          <div class="target-submenu-list custom-scroll">
                            <div
                              v-if="grp.models.length === 0"
                              class="target-submenu-empty"
                            >
                              Aucun modèle détecté
                            </div>
                            <button
                              v-for="mdl in grp.models"
                              :key="mdl"
                              type="button"
                              class="target-model-item"
                              :class="{
                                active: isTargetSelected(grp.id, mdl),
                              }"
                              @click="selectTarget(`${grp.id}:${mdl}`)"
                            >
                              <span class="target-model-name">{{ mdl }}</span>
                              <span
                                v-if="isTargetSelected(grp.id, mdl)"
                                class="target-check"
                              >
                                ✓
                              </span>
                            </button>
                          </div>
                        </div>
                      </div>
                    </div>

                    <div class="target-divider" />

                    <button
                      type="button"
                      class="target-settings-btn"
                      @click="openTargetSettings"
                    >
                      <span>⚙ Configurer fournisseurs & agents</span>
                    </button>
                  </div>
                </div>
              </div>

              <!-- À droite: Permissions des tools + Bouton d'envoi (flèche vers la droite) -->
              <div class="composer-line-2-right">
                <!-- Permissions des tools -->
                <button
                  type="button"
                  class="tool-perms-btn"
                  :class="{ active: store.autoToolsEnabled }"
                  :title="store.autoToolsEnabled ? 'Permissions tools: Exécution automatique active' : 'Permissions tools: Confirmation requise'"
                  @click="store.autoToolsEnabled = !store.autoToolsEnabled"
                >
                  <span class="tool-perms-dot" />
                  <span class="tool-perms-text">{{ store.autoToolsEnabled ? "Tools: Auto" : "Tools: Confirm" }}</span>
                </button>

                <!-- Bouton Stop (pendant l'exécution) ou Bouton d'envoi -->
                <button
                  v-if="store.isProcessing"
                  type="button"
                  class="stop-btn"
                  :disabled="store.isCancelling"
                  title="Arrêter le traitement en cours"
                  aria-label="Arrêter le traitement en cours"
                  @click="store.cancelPrompt()"
                >
                  <svg
                    class="stop-square-icon"
                    viewBox="0 0 24 24"
                    fill="currentColor"
                  >
                    <rect x="6" y="6" width="12" height="12" rx="2.5" />
                  </svg>
                </button>
                <button
                  v-else
                  class="send-btn"
                  :disabled="!promptDraft.trim() && attachedFiles.length === 0"
                  title="Envoyer le prompt"
                  @click="handleSubmit"
                >
                  <svg
                    class="send-arrow-icon"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.4"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <line x1="5" y1="12" x2="19" y2="12" />
                    <polyline points="12 5 19 12 12 19" />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        </div>
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
  padding: 16px 32px 42px;
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

.messages-list {
  width: 100%;
  max-width: 820px;
  margin-inline: auto;
  display: flex;
  flex-direction: column;
  gap: 22px;
  padding-bottom: 18px;
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

.trace-timeline {
  display: flex;
  flex-direction: column;
  gap: 6px;
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
  gap: 6px;
  font-size: 11.5px;
  color: var(--text-main);
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
  padding: 8px 32px 16px;
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
}

.composer-line-1 textarea {
  width: 100%;
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
  width: 34px;
  box-sizing: border-box;
  border: none;
  background: #ef4444;
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

.stop-btn:hover:not(:disabled) {
  background: #dc2626;
  transform: scale(1.03);
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
</style>
