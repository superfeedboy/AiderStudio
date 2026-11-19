<script setup lang="ts">
import { resolveFileLinkTarget, type InlineToken, type ResolvedFileTarget } from "../utils/markdownInline";

defineOptions({ name: "MarkdownInline" });

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
</style>
