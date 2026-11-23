<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useStudioStore } from "../stores/studio";
import type { AttachedFile, ProjectRuleItem } from "../types/studio";
import AppLogo from "./AppLogo.vue";

const emit = defineEmits<{
  (e: "submit", payload: { text: string; files: AttachedFile[] }): void;
  (e: "previewImage", url: string): void;
  (e: "slashCommand", command: string, args: string): void;
}>();

const store = useStudioStore();
const hasActiveConversation = computed(() => Boolean(store.activeConversation));
const textareaRef = ref<HTMLTextAreaElement | null>(null);
const fileInputRef = ref<HTMLInputElement | null>(null);
const targetPickerRef = ref<HTMLElement | null>(null);

const promptDraft = ref("");
const attachedFiles = ref<AttachedFile[]>([]);
const targetMenuOpen = ref(false);
const hoveredProvider = ref<string | null>(null);

// Système de commandes slash (/)
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

// Système de mentions (@)
export interface MentionItem {
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
  if (!store.activeProjectId) {
    projectFiles.value = [];
    projectRules.value = [];
    return;
  }
  projectFiles.value = await store.listProjectFiles();
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

  return [...builtInTools, ...builtInPlugins, ...mcpMentions.value, ...ruleItems, ...fileItems];
});

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

// Choix de modèle et fournisseurs
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
  if (targetPickerRef.value && !targetPickerRef.value.contains(ev.target as Node)) {
    targetMenuOpen.value = false;
    hoveredProvider.value = null;
  }
}

// Redimensionnement fluide asynchrone du textarea sans blocage synchrone
let resizeRafId: number | null = null;

function autoResizeTextarea() {
  const el = textareaRef.value;
  if (!el) return;

  if (resizeRafId !== null) cancelAnimationFrame(resizeRafId);
  resizeRafId = requestAnimationFrame(() => {
    resizeRafId = null;
    if (!el.value.includes("\n") && el.value.length < 80) {
      if (el.style.height && el.style.height !== "48px") {
        el.style.height = "48px";
      }
      return;
    }
    el.style.height = "auto";
    const nextHeight = Math.min(220, Math.max(48, el.scrollHeight));
    el.style.height = `${nextHeight}px`;
  });
}

function handleTextareaInput(ev: Event) {
  autoResizeTextarea();
  const el = ev.target as HTMLTextAreaElement;
  const cursor = el.selectionStart;
  const textBefore = promptDraft.value.slice(0, cursor);

  // Commandes slash (/)
  const slashMatch = /(?:^|\n)\/([a-zA-Z0-9_-]*)$/.exec(textBefore);
  if (slashMatch) {
    slashOpen.value = true;
    slashQuery.value = slashMatch[1];
    slashActiveIndex.value = 0;
    mentionOpen.value = false;
    return;
  } else {
    slashOpen.value = false;
  }

  // Mentions (@)
  const match = /(?:^|\s)@([a-zA-Z0-9_\-./:]*)$/.exec(textBefore);
  if (match) {
    mentionOpen.value = true;
    mentionQuery.value = match[1];
    mentionActiveIndex.value = 0;
  } else {
    mentionOpen.value = false;
  }
}

async function selectSlashCommand(cmd?: SlashCommandItem) {
  if (!cmd) return;
  slashOpen.value = false;
  const el = textareaRef.value;

  if (cmd.needsArg) {
    promptDraft.value = `${cmd.command} `;
    nextTick(() => {
      if (el) {
        el.focus();
        el.setSelectionRange(promptDraft.value.length, promptDraft.value.length);
        autoResizeTextarea();
      }
    });
  } else {
    promptDraft.value = "";
    if (el) el.style.height = "";
    emit("slashCommand", cmd.command, "");
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

function bulletForLevel(level: number): string {
  return level <= 0 ? "•" : level === 1 ? "◦" : "▪";
}

function handleKeydown(ev: KeyboardEvent) {
  // Navigation clavier pour le menu Slash (/)
  if (slashOpen.value && filteredSlashCommands.value.length > 0) {
    if (ev.key === "ArrowDown") {
      ev.preventDefault();
      slashActiveIndex.value = (slashActiveIndex.value + 1) % filteredSlashCommands.value.length;
      return;
    }
    if (ev.key === "ArrowUp") {
      ev.preventDefault();
      slashActiveIndex.value =
        (slashActiveIndex.value - 1 + filteredSlashCommands.value.length) % filteredSlashCommands.value.length;
      return;
    }
    if (ev.key === "Enter" || ev.key === "Tab") {
      ev.preventDefault();
      void selectSlashCommand(filteredSlashCommands.value[slashActiveIndex.value]);
      return;
    }
    if (ev.key === "Escape") {
      ev.preventDefault();
      slashOpen.value = false;
      return;
    }
  }

  // Navigation clavier pour le menu Mention (@)
  if (mentionOpen.value && filteredMentions.value.length > 0) {
    if (ev.key === "ArrowDown") {
      ev.preventDefault();
      mentionActiveIndex.value = (mentionActiveIndex.value + 1) % filteredMentions.value.length;
      return;
    }
    if (ev.key === "ArrowUp") {
      ev.preventDefault();
      mentionActiveIndex.value =
        (mentionActiveIndex.value - 1 + filteredMentions.value.length) % filteredMentions.value.length;
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
        const nextCaret = Math.max(blockStart, Math.min(blockStart + newBlock.length, selStart + delta));
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
          promptDraft.value = value.slice(0, lineStart) + updatedLine + value.slice(lineEnd);
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
        value.slice(0, lineStart) + updatedCurrentLine + nextBulletPrefix + afterRest + value.slice(lineEnd);

      const nextPos = lineStart + updatedCurrentLine.length + nextBulletPrefix.length;
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

// Gestion des pièces jointes (Fichiers & Images)
function openFilePicker() {
  fileInputRef.value?.click();
}

function processFileList(files: FileList | File[]) {
  const arr = Array.from(files);
  for (const file of arr) {
    const id = `att-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    if (file.type.startsWith("image/")) {
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
    processFileList(target.files);
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
    processFileList(imageFiles);
  }
}

function clearPromptInput() {
  promptDraft.value = "";
  mentionOpen.value = false;
  slashOpen.value = false;
  nextTick(() => {
    if (textareaRef.value) {
      textareaRef.value.style.height = "";
      textareaRef.value.focus();
    }
  });
}

async function handleSubmit() {
  const text = promptDraft.value.trim();
  const files = [...attachedFiles.value];
  if (!hasActiveConversation.value || (!text && files.length === 0) || store.isProcessing) {
    return;
  }

  promptDraft.value = "";
  attachedFiles.value = [];
  mentionOpen.value = false;
  slashOpen.value = false;
  await nextTick();
  if (textareaRef.value) {
    textareaRef.value.style.height = "";
    textareaRef.value.focus();
  }

  emit("submit", { text, files });
}

// Expose public API
defineExpose({
  setPrompt(content: string) {
    promptDraft.value = content;
    nextTick(() => {
      if (textareaRef.value) {
        textareaRef.value.focus();
        textareaRef.value.setSelectionRange(content.length, content.length);
        autoResizeTextarea();
      }
    });
  },
  addFiles(files: FileList | File[]) {
    processFileList(files);
  },
  focus() {
    textareaRef.value?.focus();
  },
});

watch(
  () => store.activeProjectId,
  () => {
    void refreshProjectFiles();
  }
);

onMounted(() => {
  void refreshProjectFiles();
  document.addEventListener("click", handleTargetOutsideClick);
});

onBeforeUnmount(() => {
  document.removeEventListener("click", handleTargetOutsideClick);
  if (resizeRafId !== null) cancelAnimationFrame(resizeRafId);
});
</script>

<template>
  <div class="composer-wrapper">
    <!-- Input file caché pour les pièces jointes -->
    <input
      ref="fileInputRef"
      type="file"
      multiple
      class="hidden-file-input"
      @change="onFileInputChange"
    />

    <!-- Slash Commands (/) Autocompletion Popover -->
    <div v-if="slashOpen" class="mention-popover halo-hud custom-scroll">
      <div class="mention-header">
        <span class="mention-hint">Commandes rapides de l'IDE (/)</span>
      </div>
      <div class="mention-list">
        <div v-if="filteredSlashCommands.length === 0" class="mention-empty">
          Aucune commande pour "{{ slashQuery }}"
        </div>
        <button
          v-for="(item, sIdx) in filteredSlashCommands"
          :key="item.id"
          class="mention-item"
          :class="{ active: sIdx === slashActiveIndex }"
          @mouseenter="slashActiveIndex = sIdx"
          @click="selectSlashCommand(item)"
        >
          <span class="mention-badge">{{ item.icon }}</span>
          <div class="mention-info">
            <span class="mention-name">{{ item.command }}</span>
            <span class="mention-desc">{{ item.description }}</span>
          </div>
          <code v-if="item.argPlaceholder" class="mention-tag">{{ item.argPlaceholder }}</code>
        </button>
      </div>
    </div>

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
          <button
            class="mention-tab"
            :class="{ active: mentionCategory === 'rule' }"
            @click="mentionCategory = 'rule'"
          >
            Règles
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
            {{
              item.category === 'file'
                ? '📁'
                : item.category === 'tool'
                  ? '⚙️'
                  : item.category === 'plugin'
                    ? '🧩'
                    : item.category === 'rule'
                      ? '📜'
                      : '🔌'
            }}
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
          @click="emit('previewImage', att.dataUrl || '')"
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
    <div class="composer halo-input" :class="{ 'is-thinking': store.isProcessing }">
      <!-- Ligne 1: input de texte du prompt -->
      <div class="composer-line-1">
        <textarea
          ref="textareaRef"
          v-model="promptDraft"
          rows="2"
          :placeholder="
            hasActiveConversation
              ? 'Décrivez ce que vous voulez coder... (Tapez @ pour outils/fichiers/MCP, collez des images)'
              : 'Créez ou sélectionnez un projet pour démarrer une discussion...'
          "
          :disabled="store.isProcessing || !hasActiveConversation"
          @input="handleTextareaInput"
          @keydown="handleKeydown"
          @paste="handlePaste"
        />
        <button
          v-if="promptDraft.length > 0 && !store.isProcessing"
          type="button"
          class="prompt-clear-btn"
          title="Vider le texte"
          aria-label="Vider le champ"
          @click="clearPromptInput"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <!-- Ligne 2: Bouton PJ + choix modèle à gauche, permissions tools et bouton envoi à droite -->
      <div class="composer-line-2">
        <div class="composer-line-2-left">
          <button
            type="button"
            class="attach-btn"
            title="Ajouter des images ou des fichiers à interpréter"
            :disabled="store.isProcessing || !hasActiveConversation"
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
              <AppLogo v-if="store.activeTargetLabel.isAgent" :size="15" />
              <span
                v-else
                class="provider-dot"
                :style="{ backgroundColor: providerDotColor(store.activeTargetLabel.providerId) }"
              />
              <span class="target-model-label">
                {{ store.activeTargetLabel.isAgent ? "Agent" : store.activeTargetLabel.modelName }}
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
                <span v-if="store.activeTargetLabel.isAgent" class="target-check">✓</span>
              </button>

              <div class="target-divider" />

              <div
                v-for="grp in providerGroups"
                :key="grp.id"
                class="target-provider-row"
                :class="{
                  'is-active-sub': hoveredProvider === grp.id,
                  'is-selected-prov':
                    !store.activeTargetLabel.isAgent && store.activeTargetLabel.providerId === grp.id,
                }"
                @mouseenter="hoveredProvider = grp.id"
                @click.stop="hoveredProvider = hoveredProvider === grp.id ? null : grp.id"
              >
                <span class="provider-dot" :style="{ backgroundColor: grp.color }" />
                <span class="target-prov-name">{{ grp.name }}</span>
                <span class="target-prov-count">{{ grp.models.length }}</span>
                <span class="target-arrow">›</span>

                <div v-if="hoveredProvider === grp.id" class="target-submenu-bridge" @click.stop>
                  <div class="target-submenu halo-hud">
                    <div class="target-submenu-header">
                      <span class="provider-dot" :style="{ backgroundColor: grp.color }" />
                      <span>{{ grp.name }}</span>
                    </div>
                    <div class="target-submenu-list custom-scroll">
                      <div v-if="grp.models.length === 0" class="target-submenu-empty">
                        Aucun modèle détecté
                      </div>
                      <button
                        v-for="mdl in grp.models"
                        :key="mdl"
                        type="button"
                        class="target-model-item"
                        :class="{ active: isTargetSelected(grp.id, mdl) }"
                        @click="selectTarget(`${grp.id}:${mdl}`)"
                      >
                        <span class="target-model-name">{{ mdl }}</span>
                        <span v-if="isTargetSelected(grp.id, mdl)" class="target-check">✓</span>
                      </button>
                    </div>
                  </div>
                </div>
              </div>

              <div class="target-divider" />

              <button type="button" class="target-settings-btn" @click="openTargetSettings">
                <span>⚙ Configurer fournisseurs & agents</span>
              </button>
            </div>
          </div>
        </div>

        <div class="composer-line-2-right">
          <button
            type="button"
            class="tool-perms-btn"
            :class="{ active: store.autoToolsEnabled }"
            :title="
              store.autoToolsEnabled
                ? 'Permissions tools: Exécution automatique active'
                : 'Permissions tools: Confirmation requise'
            "
            @click="store.autoToolsEnabled = !store.autoToolsEnabled"
          >
            <span class="tool-perms-dot" />
            <span class="tool-perms-text">{{ store.autoToolsEnabled ? "Tools: Auto" : "Tools: Confirm" }}</span>
          </button>

          <button
            v-if="store.isProcessing"
            type="button"
            class="stop-btn"
            :disabled="store.isCancelling"
            title="Arrêter le traitement en cours"
            aria-label="Arrêter le traitement en cours"
            @click="store.cancelPrompt()"
          >
            <svg class="stop-square-icon" viewBox="0 0 24 24" fill="currentColor">
              <rect x="6" y="6" width="12" height="12" rx="2.5" />
            </svg>
          </button>
          <button
            v-else
            class="send-btn"
            :disabled="!hasActiveConversation || (!promptDraft.trim() && attachedFiles.length === 0)"
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
</template>

<style scoped>
.hidden-file-input {
  display: none;
}

.composer-wrapper {
  position: relative;
  width: 100%;
}

.composer {
  display: flex;
  flex-direction: column;
  padding: 8px 10px;
  gap: 6px;
  transition: border-color 0.25s ease, box-shadow 0.25s ease;
}

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
  animation: vaixRotateRainbow 2.2s linear infinite;
  transform: translateZ(0);
  will-change: background, box-shadow;
  box-shadow: 0 0 24px -2px rgba(168, 85, 247, 0.4), 0 0 12px 0 rgba(56, 189, 248, 0.25) !important;
}

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
  field-sizing: content;
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

.attach-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.attach-btn svg {
  width: 16px;
  height: 16px;
}

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

.send-arrow-icon {
  width: 16px;
  height: 16px;
}

/* Mention (@) et Slash (/) Popovers */
.mention-popover {
  position: absolute;
  bottom: calc(100% + 8px);
  left: 0;
  right: 0;
  max-height: 280px;
  padding: 6px;
  z-index: 50;
  display: flex;
  flex-direction: column;
  gap: 4px;
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
}

.mention-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 6px;
  border-bottom: 1px solid var(--border-subtle);
}

.mention-hint {
  font-size: 11px;
  color: var(--text-dim);
}

.mention-tabs {
  display: flex;
  gap: 4px;
}

.mention-tab {
  font-size: 10.5px;
  padding: 2px 6px;
  border-radius: 4px;
  border: 1px solid transparent;
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}

.mention-tab.active {
  background: var(--active-bg);
  border-color: var(--accent);
  color: var(--accent);
}

.mention-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow-y: auto;
}

.mention-empty {
  padding: 8px;
  font-size: 11.5px;
  color: var(--text-dim);
  text-align: center;
}

.mention-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: 6px;
  border: none;
  background: transparent;
  color: var(--text-main);
  cursor: pointer;
  text-align: left;
  transition: all 0.12s ease;
}

.mention-item:hover,
.mention-item.active {
  background: var(--hover-bg);
}

.mention-badge {
  font-size: 13px;
  padding: 2px 4px;
  border-radius: 4px;
}

.mention-badge.file {
  background: rgba(59, 130, 246, 0.15);
  color: #3b82f6;
}

.mention-badge.tool {
  background: rgba(245, 158, 11, 0.15);
  color: #f59e0b;
}

.mention-badge.plugin {
  background: rgba(168, 85, 247, 0.15);
  color: #a855f7;
}

.mention-badge.mcp {
  background: rgba(16, 185, 129, 0.15);
  color: #10b981;
}

.mention-badge.rule {
  background: rgba(244, 63, 94, 0.15);
  color: #f43f5e;
}

.mention-info {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}

.mention-name {
  font-size: 12px;
  font-weight: 500;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.mention-desc {
  font-size: 10.5px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mention-tag {
  font-size: 10.5px;
  color: var(--accent);
  background: rgba(129, 140, 248, 0.1);
  padding: 1px 5px;
  border-radius: 4px;
}
</style>
