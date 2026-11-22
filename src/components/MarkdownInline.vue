<script setup lang="ts">
import { resolveFileLinkTarget, type InlineToken, type ResolvedFileTarget } from "../utils/markdownInline";
import { useStudioStore } from "../stores/studio";

defineOptions({ name: "MarkdownInline" });

const store = useStudioStore();

const props = withDefaults(
  defineProps<{
    tokens: InlineToken[];
    availablePaths?: string[];
    projectRoot?: string;
    variant?: "default" | "thinking";
  }>(),
  {
    availablePaths: () => [],
    projectRoot: "",
    variant: "default",
  }
);

const emit = defineEmits<{
  (e: "open-file", payload: ResolvedFileTarget): void;
}>();

function isExternalHref(href: string): boolean {
  return /^https?:\/\//i.test(href.trim());
}

function isMessageLink(href: string): boolean {
  return /^#msg-|^msg:|^#commit-/i.test(href.trim());
}

function handleMessageLinkClick(href: string) {
  const msgId = href.replace(/^#msg-|^msg:|^#commit-/i, "").trim();
  if (msgId) {
    store.scrollToMessage(msgId);
  }
}

function handleRerunWriteClick() {
  store.sendPrompt("Écris et applique directement le code complet des fichiers nécessaires sur le disque maintenant.");
}

function resolveLink(href: string) {
  return resolveFileLinkTarget(href, props.availablePaths, props.projectRoot, true);
}

function resolveCodeFile(codeText: string) {
  return resolveFileLinkTarget(codeText, props.availablePaths, props.projectRoot, false);
}

function formatCodeFileLabel(codeText: string, resolvedPath: string): string {
  const base = resolvedPath.split("/").pop() || resolvedPath;
  const dupCount = props.availablePaths.filter(
    (p) => (p.split("/").pop() || p) === base
  ).length;
  if (dupCount > 1 && codeText.includes("/")) {
    return resolvedPath;
  }
  return base;
}

function getToolCategory(toolName: string): "terminal" | "search" | "edit" | "delete" | "mcp" | "file" {
  const t = (toolName || "").toLowerCase();
  if (t === "run_command" || t === "terminal" || t === "bash" || t === "shell") return "terminal";
  if (t.includes("search") || t.includes("fetch") || t.includes("web")) return "search";
  if (t === "edit_file" || t === "write_file" || t.includes("patch")) return "edit";
  if (t === "delete_file" || t.includes("remove")) return "delete";
  if (t.includes("mcp") || t.includes("server")) return "mcp";
  return "file";
}
</script>

<template>
  <template v-for="(tok, tIdx) in tokens" :key="tIdx">
    <strong v-if="tok.kind === 'bold'">
      <MarkdownInline
        :tokens="tok.children"
        :available-paths="availablePaths"
        :project-root="projectRoot"
        :variant="variant"
        @open-file="emit('open-file', $event)"
      />
    </strong>
    <em v-else-if="tok.kind === 'italic'">{{ tok.text }}</em>
    <span v-else-if="tok.kind === 'tool-call'" class="md-tool-call-group">
      <span
        v-for="tool in tok.tools"
        :key="tool"
        class="md-tool-call-pill"
        :class="[`is-${getToolCategory(tool)}`, { 'is-thinking': variant === 'thinking' }]"
        :title="`Appel d'outil natif exécuté : ${tool}`"
      >
        <span class="tool-icon">
          <svg v-if="getToolCategory(tool) === 'terminal'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
            <polyline points="4 17 10 11 4 5" />
            <line x1="12" y1="19" x2="20" y2="19" />
          </svg>
          <svg v-else-if="getToolCategory(tool) === 'search'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <svg v-else-if="getToolCategory(tool) === 'edit'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
            <path d="M12 20h9" />
            <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z" />
          </svg>
          <svg v-else-if="getToolCategory(tool) === 'delete'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
            <polyline points="3 6 5 6 21 6" />
            <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
          </svg>
          <svg v-else-if="getToolCategory(tool) === 'mcp'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
            <path d="M12 2v6m0 8v6M4.93 4.93l4.24 4.24m5.66 5.66l4.24 4.24M2 12h6m8 0h6M4.93 19.07l4.24-4.24m5.66-5.66l4.24-4.24" />
          </svg>
          <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
            <polyline points="14 2 14 8 20 8" />
          </svg>
        </span>
        <span class="tool-tag">Outil</span>
        <code class="tool-name">{{ tool }}</code>
        <span class="tool-pulse-dot" />
      </span>
    </span>
    <template v-else-if="tok.kind === 'link'">
      <a
        v-if="isExternalHref(tok.href)"
        :href="tok.href"
        target="_blank"
        rel="noopener noreferrer"
        class="md-ext-link"
      >
        {{ tok.label }}
      </a>
      <button
        v-else-if="tok.href === '#rerun-write' || tok.href === 'action:rerun-write'"
        type="button"
        class="md-rerun-write-pill"
        title="Relancer l'écriture des fichiers sur le disque"
        @click.stop="handleRerunWriteClick"
      >
        <span class="rerun-icon">⚡</span>
        <span>{{ tok.label }}</span>
      </button>
      <button
        v-else-if="isMessageLink(tok.href)"
        type="button"
        class="md-commit-pill-btn"
        :title="`Aller à la réponse associée (${tok.label})`"
        @click.stop="handleMessageLinkClick(tok.href)"
      >
        <span class="commit-pill-tag">{{ tok.label }}</span>
      </button>
      <button
        v-else-if="resolveLink(tok.href)"
        type="button"
        class="md-file-link"
        :class="{ 'is-thinking': variant === 'thinking' }"
        :title="`Ouvrir le diff de ${resolveLink(tok.href)!.path} à droite`"
        @click.stop="emit('open-file', resolveLink(tok.href)!)"
      >
        <svg class="md-file-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
          <polyline points="14 2 14 8 20 8" />
        </svg>
        <span>{{ tok.label }}</span>
      </button>
      <span v-else>{{ tok.label }}</span>
    </template>
    <template v-else-if="tok.kind === 'code'">
      <button
        v-if="resolveCodeFile(tok.text)"
        type="button"
        class="md-file-link"
        :class="{ 'is-thinking': variant === 'thinking' }"
        :title="`Ouvrir le diff de ${resolveCodeFile(tok.text)!.path} à droite`"
        @click.stop="emit('open-file', resolveCodeFile(tok.text)!)"
      >
        <svg class="md-file-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
          <polyline points="14 2 14 8 20 8" />
        </svg>
        <span>{{ formatCodeFileLabel(tok.text, resolveCodeFile(tok.text)!.path) }}</span>
      </button>
      <code v-else :class="variant === 'thinking' ? 'tmd-inline-code' : 'md-inline-code'">{{ tok.text }}</code>
    </template>
    <template v-else>{{ tok.text }}</template>
  </template>
</template>

<style scoped>
.md-file-link {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-code);
  font-size: 0.9em;
  line-height: 1.3;
  padding: 1px 6px;
  margin: 0 1px;
  border-radius: 5px;
  border: 1px solid rgba(56, 189, 248, 0.32);
  background: rgba(56, 189, 248, 0.1);
  color: var(--accent-2);
  cursor: pointer;
  vertical-align: baseline;
  transition: all 0.15s ease;
}

.md-file-link.is-thinking {
  font-size: 11px;
  padding: 0 5px;
}

.md-file-link:hover {
  background: rgba(56, 189, 248, 0.2);
  border-color: var(--accent-2);
  text-decoration: underline;
  box-shadow: 0 0 8px rgba(56, 189, 248, 0.22);
}

.md-file-icon {
  width: 11px;
  height: 11px;
  flex-shrink: 0;
  opacity: 0.85;
}

.md-ext-link {
  color: var(--accent-2);
  text-decoration: underline;
  text-underline-offset: 2px;
}

.md-ext-link:hover {
  opacity: 0.85;
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

.tmd-inline-code {
  font-family: var(--font-code);
  font-size: 11px;
  padding: 1px 5px;
  border-radius: 4px;
  background: var(--hover-bg);
  border: 1px solid var(--border-subtle);
  color: var(--accent-2);
}
.md-commit-pill-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  margin: 0 3px;
  border-radius: 9999px;
  background: rgba(99, 102, 241, 0.18);
  border: 1px solid rgba(99, 102, 241, 0.4);
  color: #a5b4fc;
  font-size: 0.88em;
  font-weight: 600;
  font-family: var(--font-code);
  cursor: pointer;
  transition: all 0.15s ease;
  vertical-align: middle;
}

.md-commit-pill-btn:hover {
  background: rgba(99, 102, 241, 0.32);
  border-color: rgba(99, 102, 241, 0.7);
  color: #ffffff;
  transform: translateY(-0.5px);
  box-shadow: 0 0 8px rgba(99, 102, 241, 0.35);
}

.md-commit-pill-btn:active {
  transform: translateY(0);
}

/* --- Badges Graphiques d'Appels d'Outils --- */
.md-tool-call-group {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 5px;
  margin: 2px 3px;
  vertical-align: middle;
}

.md-tool-call-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 8px;
  border-radius: 7px;
  font-family: var(--font-code);
  font-size: 0.85em;
  line-height: 1.35;
  background: rgba(15, 23, 42, 0.7);
  border: 1px solid rgba(255, 255, 255, 0.12);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.25);
  backdrop-filter: blur(6px);
  transition: all 0.2s ease;
  user-select: none;
}

.md-tool-call-pill.is-thinking {
  font-size: 11px;
  padding: 1px 6px;
}

.md-tool-call-pill:hover {
  transform: translateY(-0.5px);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35);
}

.tool-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 13px;
  height: 13px;
}

.tool-icon svg {
  width: 100%;
  height: 100%;
}

.tool-tag {
  font-size: 9px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  font-weight: 700;
  opacity: 0.75;
}

.tool-name {
  font-size: 11px;
  font-weight: 600;
  background: transparent;
  padding: 0;
  border: none;
  color: inherit;
}

.tool-pulse-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
  opacity: 0.85;
  animation: toolPulse 2s infinite ease-in-out;
}

@keyframes toolPulse {
  0%, 100% {
    opacity: 0.4;
    transform: scale(0.85);
  }
  50% {
    opacity: 1;
    transform: scale(1.15);
  }
}

/* Couleurs par rôle/catégorie d'outil */
.md-tool-call-pill.is-terminal {
  background: rgba(245, 158, 11, 0.12);
  border-color: rgba(245, 158, 11, 0.35);
  color: #fbbf24;
}
.md-tool-call-pill.is-terminal:hover {
  border-color: #f59e0b;
  box-shadow: 0 0 10px rgba(245, 158, 11, 0.25);
}

.md-tool-call-pill.is-search {
  background: rgba(16, 185, 129, 0.12);
  border-color: rgba(16, 185, 129, 0.35);
  color: #34d399;
}
.md-tool-call-pill.is-search:hover {
  border-color: #10b981;
  box-shadow: 0 0 10px rgba(16, 185, 129, 0.25);
}

.md-tool-call-pill.is-edit {
  background: rgba(168, 85, 247, 0.12);
  border-color: rgba(168, 85, 247, 0.35);
  color: #c084fc;
}
.md-tool-call-pill.is-edit:hover {
  border-color: #a855f7;
  box-shadow: 0 0 10px rgba(168, 85, 247, 0.25);
}

.md-tool-call-pill.is-delete {
  background: rgba(244, 63, 94, 0.12);
  border-color: rgba(244, 63, 94, 0.35);
  color: #fb7185;
}
.md-tool-call-pill.is-delete:hover {
  border-color: #f43f5e;
  box-shadow: 0 0 10px rgba(244, 63, 94, 0.25);
}

.md-tool-call-pill.is-mcp {
  background: rgba(236, 72, 153, 0.12);
  border-color: rgba(236, 72, 153, 0.35);
  color: #f472b6;
}
.md-tool-call-pill.is-mcp:hover {
  border-color: #ec4899;
  box-shadow: 0 0 10px rgba(236, 72, 153, 0.25);
}

.md-tool-call-pill.is-file {
  background: rgba(56, 189, 248, 0.12);
  border-color: rgba(56, 189, 248, 0.35);
  color: #38bdf8;
}
.md-tool-call-pill.is-file:hover {
  border-color: #38bdf8;
  box-shadow: 0 0 10px rgba(56, 189, 248, 0.25);
}

.md-rerun-write-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 14px;
  margin: 6px 2px;
  border-radius: 8px;
  background: linear-gradient(135deg, rgba(245, 158, 11, 0.22), rgba(234, 88, 12, 0.3));
  border: 1px solid rgba(245, 158, 11, 0.55);
  color: #fef3c7;
  font-family: var(--font-ui, system-ui);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  box-shadow: 0 2px 10px rgba(245, 158, 11, 0.2);
  transition: all 0.2s ease;
  vertical-align: middle;
}

.md-rerun-write-pill:hover {
  background: linear-gradient(135deg, rgba(245, 158, 11, 0.35), rgba(234, 88, 12, 0.45));
  border-color: #f59e0b;
  transform: translateY(-1px);
  box-shadow: 0 4px 14px rgba(245, 158, 11, 0.35);
  color: #ffffff;
}

.md-rerun-write-pill:active {
  transform: translateY(0);
}
</style>
