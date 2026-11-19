<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { useStudioStore } from "../stores/studio";
import type { DiffLine } from "../types/studio";

const props = defineProps<{
  isResizing: boolean;
}>();

const store = useStudioStore();
const copied = ref(false);
const diffViewerRef = ref<HTMLElement | null>(null);

const conv = computed(() => store.activeConversation);
const modifiedFiles = computed(() => conv.value?.modifiedFiles ?? []);
const artifacts = computed(() => conv.value?.artifacts ?? []);

const selectedFile = computed(() => {
  if (!modifiedFiles.value.length) return null;
  return (
    modifiedFiles.value.find((f) => f.path === store.activeDiffFilePath) ??
    modifiedFiles.value[0]
  );
});

function fullPathFor(relPath: string): string {
  const root = store.activeProject?.rootPath || "";
  const sep = root.endsWith("/") || root.endsWith("\\") ? "" : "/";
  return `${root}${sep}${relPath}`;
}

const selectedArtifact = computed(() => {
  if (!artifacts.value.length) return null;
  return (
    artifacts.value.find((a) => a.id === store.activeArtifactId) ??
    artifacts.value[artifacts.value.length - 1]
  );
});

const isPreviewRenderable = computed(() => {
  if (!selectedArtifact.value) return false;
  const t = selectedArtifact.value.title.toLowerCase();
  const l = selectedArtifact.value.language.toLowerCase();
  const c = selectedArtifact.value.content.trim().toLowerCase();
  return (
    t.endsWith(".html") ||
    t.endsWith(".htm") ||
    t.endsWith(".svg") ||
    l === "html" ||
    l === "svg" ||
    c.startsWith("<!doctype html") ||
    c.startsWith("<html") ||
    c.startsWith("<svg")
  );
});

const isSvg = computed(() => {
  if (!selectedArtifact.value) return false;
  const t = selectedArtifact.value.title.toLowerCase();
  const l = selectedArtifact.value.language.toLowerCase();
  const c = selectedArtifact.value.content.trim().toLowerCase();
  return t.endsWith(".svg") || l === "svg" || c.startsWith("<svg");
});

const artifactRenderMode = ref<"code" | "preview">("preview");

const drawerStyle = computed(() => {
  if (store.isWideScreen) {
    return {
      width: "50%",
      flex: "0 0 50%",
      maxWidth: "50%",
      opacity: 1,
      transform: "translateX(0px)",
      pointerEvents: "auto" as const,
      transition: "none",
    };
  }
  if (store.codeOpen) {
    return {
      width: `${store.codeWidth}px`,
      opacity: 1,
      transform: "translateX(0px)",
      pointerEvents: "auto" as const,
      transition: props.isResizing
        ? "none"
        : "width 300ms cubic-bezier(0.22, 1, 0.36, 1), opacity 220ms ease, transform 300ms cubic-bezier(0.22, 1, 0.36, 1)",
    };
  }
  return {
    width: "0px",
    opacity: 0,
    transform: "translateX(32px)",
    pointerEvents: "none" as const,
    borderWidth: "0px",
    transition:
      "width 300ms cubic-bezier(0.22, 1, 0.36, 1), opacity 200ms ease, transform 300ms cubic-bezier(0.22, 1, 0.36, 1)",
  };
});

async function copyArtifact() {
  if (!selectedArtifact.value) return;
  await navigator.clipboard.writeText(selectedArtifact.value.content);
  copied.value = true;
  setTimeout(() => (copied.value = false), 1500);
}

async function confirmUndoFile(filePath: string) {
  const ok = window.confirm(
    `Voulez-vous annuler toutes les modifications apportées au fichier "${filePath}" et le restaurer dans son état d'origine ?`
  );
  if (!ok) return;
  await store.undoFile(filePath);
}

async function confirmUndoAll() {
  const count = modifiedFiles.value.length;
  const ok = window.confirm(
    `Voulez-vous annuler TOUTES les modifications (${count} fichier${count > 1 ? "s" : ""}) et restaurer le projet dans son état d'origine ?\nCette action est irréversible.`
  );
  if (!ok) return;
  await store.undoAll();
}

function selectDiffFile(filePath: string) {
  store.activeDiffTargetLine = null;
  store.activeDiffFilePath = filePath;
}

function isTargetDiffLine(line: DiffLine): boolean {
  if (!store.activeDiffTargetLine) return false;
  return (line.newLine ?? line.oldLine ?? null) === store.activeDiffTargetLine;
}

watch(
  [
    () => selectedFile.value?.path,
    () => store.activeDiffTargetLine,
    () => store.codeOpen,
    () => store.codeMode,
  ],
  async ([nextPath, targetLine, isOpen, mode], [prevPath]) => {
    if (!isOpen || mode !== "review" || !nextPath) return;
    await nextTick();
    const viewer = diffViewerRef.value;
    if (!viewer) return;

    if (targetLine && targetLine > 0) {
      const lineNodes = Array.from(
        viewer.querySelectorAll<HTMLElement>(".diff-line[data-line]")
      );
      let bestEl: HTMLElement | null = null;
      let bestDelta = Number.POSITIVE_INFINITY;
      for (const el of lineNodes) {
        const ln = Number(el.dataset.line);
        if (!Number.isFinite(ln)) continue;
        const delta = Math.abs(ln - targetLine);
        if (delta < bestDelta) {
          bestDelta = delta;
          bestEl = el;
          if (delta === 0) break;
        }
      }
      if (bestEl) {
        const viewerRect = viewer.getBoundingClientRect();
        const lineRect = bestEl.getBoundingClientRect();
        const relTop = lineRect.top - viewerRect.top + viewer.scrollTop;
        viewer.scrollTop = Math.max(0, relTop - viewer.clientHeight / 3);
        return;
      }
    }

    if (nextPath !== prevPath) {
      viewer.scrollTop = 0;
    }
  }
);
</script>

<template>
  <aside
    class="le-code halo-surface"
    :style="drawerStyle"
    :aria-hidden="!store.codeOpen"
  >
    <div class="code-header">
      <div class="code-tabs">
        <button
          class="code-tab"
          :class="{ active: store.codeMode === 'review' }"
          @click="store.codeMode = 'review'"
        >
          Review ({{ modifiedFiles.length }})
        </button>
        <button
          class="code-tab"
          :class="{ active: store.codeMode === 'artifact' }"
          @click="store.codeMode = 'artifact'"
        >
          Artefacts ({{ artifacts.length }})
        </button>
      </div>

      <div class="code-header-actions">
        <button
          v-if="store.codeMode === 'review' && modifiedFiles.length > 0"
          class="undo-all-header-btn"
          title="Tout défaire : annuler toutes les modifications apportées aux fichiers"
          @click="confirmUndoAll"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor">
            <path d="M5.82843 7.00005L8.36396 9.53558L6.94975 10.9498L2 6.00005L6.94975 1.05031L8.36396 2.46452L5.82843 5.00005H13C16.866 5.00005 20 8.13406 20 12C20 15.866 16.866 19 13 19H4V17H13C15.7614 17 18 14.7615 18 12C18 9.23862 15.7614 7.00005 13 7.00005H5.82843Z"/>
          </svg>
          <span class="undo-label">Tout défaire</span>
        </button>
        <button
          v-if="store.codeMode === 'review'"
          class="refresh-btn"
          title="Rafraîchir le git diff du projet"
          @click="store.refreshDiff()"
        >
          ↻
        </button>
        <button
          class="close-btn"
          title="Fermer le code"
          @click="store.closeCode()"
        >
          ✕
        </button>
      </div>
    </div>

    <!-- MODE 1: MODIFIED FILES REVIEW (Blue = modified, Green = added, Red = deleted) -->
    <div v-if="store.codeMode === 'review'" class="code-body">
      <div v-if="modifiedFiles.length === 0" class="empty-code">
        <p>Aucun fichier modifié détecté dans le dépôt Git pour le moment.</p>
        <button class="refresh-pill" @click="store.refreshDiff()">
          Scanner Git diff
        </button>
      </div>

      <template v-else>
        <div class="files-list custom-scroll">
          <button
            v-for="file in modifiedFiles"
            :key="file.path"
            class="file-row has-tooltip"
            :class="{ selected: selectedFile?.path === file.path }"
            :title="fullPathFor(file.path)"
            @click="selectDiffFile(file.path)"
          >
            <span class="file-path">{{ file.path }}</span>
            <div class="file-row-actions">
              <span class="file-counts">
                <span v-if="file.modified" class="c-mod">~{{ file.modified }}</span>
                <span class="c-add">+{{ file.added }}</span>
                <span class="c-del">-{{ file.removed }}</span>
              </span>
              <button
                class="file-row-undo-btn"
                :title="`Annuler les modifications de ${file.path}`"
                @click.stop="confirmUndoFile(file.path)"
              >
                <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M5.82843 7.00005L8.36396 9.53558L6.94975 10.9498L2 6.00005L6.94975 1.05031L8.36396 2.46452L5.82843 5.00005H13C16.866 5.00005 20 8.13406 20 12C20 15.866 16.866 19 13 19H4V17H13C15.7614 17 18 14.7615 18 12C18 9.23862 15.7614 7.00005 13 7.00005H5.82843Z"/>
                </svg>
              </button>
            </div>

            <!-- Tooltip au hover affichant le chemin complet absolu -->
            <div class="custom-file-tooltip">
              <span class="tooltip-label">Chemin complet :</span>
              <span class="tooltip-path">{{ fullPathFor(file.path) }}</span>
            </div>
          </button>
        </div>

        <div class="diff-legend">
          <div class="legend-tags">
            <span class="leg mod">Modifié</span>
            <span class="leg add">Ajouté</span>
            <span class="leg del">Supprimé</span>
          </div>
          <button
            v-if="selectedFile"
            class="diff-undo-selected-btn"
            :title="`Annuler les modifications de ${selectedFile.path}`"
            @click="confirmUndoFile(selectedFile.path)"
          >
            <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor">
              <path d="M5.82843 7.00005L8.36396 9.53558L6.94975 10.9498L2 6.00005L6.94975 1.05031L8.36396 2.46452L5.82843 5.00005H13C16.866 5.00005 20 8.13406 20 12C20 15.866 16.866 19 13 19H4V17H13C15.7614 17 18 14.7615 18 12C18 9.23862 15.7614 7.00005 13 7.00005H5.82843Z"/>
            </svg>
            <span>Annuler ce fichier</span>
          </button>
        </div>

        <div v-if="selectedFile" ref="diffViewerRef" class="diff-viewer custom-scroll">
          <div
            v-for="(line, idx) in selectedFile.lines"
            :key="idx"
            class="diff-line"
            :data-line="line.newLine ?? line.oldLine ?? undefined"
            :class="[
              `diff-line--${line.kind}`,
              { 'diff-line--target': isTargetDiffLine(line) },
            ]"
          >
            <span class="ln-num">{{ line.newLine ?? line.oldLine ?? "" }}</span>
            <span class="ln-prefix">
              {{
                line.kind === "mod"
                  ? "~"
                  : line.kind === "add"
                    ? "+"
                    : line.kind === "del"
                      ? "-"
                      : " "
              }}
            </span>
            <span class="ln-code">{{ line.content }}</span>
          </div>
        </div>
      </template>
    </div>

    <!-- MODE 2: ARTIFACTS VIEWER -->
    <div v-else class="code-body">
      <div v-if="artifacts.length === 0" class="empty-code">
        Aucun artefact généré dans cette conversation.
      </div>

      <template v-else>
        <div class="files-list custom-scroll">
          <button
            v-for="art in artifacts"
            :key="art.id"
            class="file-row"
            :class="{ selected: selectedArtifact?.id === art.id }"
            @click="store.activeArtifactId = art.id"
          >
            <span class="file-path">{{ art.title }}</span>
            <span class="art-lang">{{ art.language }}</span>
          </button>
        </div>

        <div v-if="selectedArtifact" class="artifact-viewer">
          <div class="artifact-toolbar">
            <span class="artifact-name">{{ selectedArtifact.title }}</span>

            <div class="artifact-toolbar-right">
              <div v-if="isPreviewRenderable" class="view-toggle">
                <button
                  class="toggle-btn"
                  :class="{ active: artifactRenderMode === 'preview' }"
                  title="Aperçu interprété"
                  @click="artifactRenderMode = 'preview'"
                >
                  👁️ Aperçu
                </button>
                <button
                  class="toggle-btn"
                  :class="{ active: artifactRenderMode === 'code' }"
                  title="Afficher le code source"
                  @click="artifactRenderMode = 'code'"
                >
                  💻 Code
                </button>
              </div>

              <button class="copy-btn" @click="copyArtifact">
                {{ copied ? "Copié ✓" : "Copier" }}
              </button>
            </div>
          </div>

          <!-- Rendu interprété (ex: HTML, SVG) -->
          <div
            v-if="isPreviewRenderable && artifactRenderMode === 'preview'"
            class="artifact-preview-container"
          >
            <div v-if="isSvg" class="rendered-svg-wrap" v-html="selectedArtifact.content" />
            <iframe
              v-else
              class="rendered-html-iframe"
              sandbox="allow-scripts"
              :srcdoc="selectedArtifact.content"
              title="Aperçu HTML"
            />
          </div>

          <!-- Code source brut (Turret Road) -->
          <pre
            v-else
            class="artifact-code custom-scroll"
          ><code>{{ selectedArtifact.content }}</code></pre>
        </div>
      </template>
    </div>
  </aside>
</template>

<style scoped>
.le-code {
  flex-shrink: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-width: 0;
}

.code-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-subtle);
  flex-shrink: 0;
}

.code-tabs {
  display: flex;
  align-items: center;
  gap: 6px;
}

.code-tab {
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 7px;
  padding: 4px 10px;
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
}

.code-tab.active {
  background: var(--active-bg);
  color: var(--accent);
}

.code-header-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.undo-all-header-btn {
  border: 1px solid rgba(239, 68, 68, 0.3);
  background: rgba(239, 68, 68, 0.08);
  color: #f87171;
  border-radius: 6px;
  padding: 3px 8px;
  cursor: pointer;
  font-size: 11px;
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  transition: all 0.15s ease;
}

.undo-all-header-btn:hover {
  background: rgba(239, 68, 68, 0.22);
  border-color: #ef4444;
  color: #ffffff;
  box-shadow: 0 0 8px rgba(239, 68, 68, 0.35);
}

.undo-label {
  font-size: 11px;
}

.refresh-btn,
.close-btn {
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 6px;
  padding: 4px 7px;
  cursor: pointer;
  font-size: 13px;
}

.refresh-btn:hover,
.close-btn:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.code-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.empty-code {
  margin: auto;
  padding: 24px;
  text-align: center;
  color: var(--text-muted);
  font-size: 13px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.refresh-pill {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  border-radius: 8px;
  padding: 6px 12px;
  font-size: 12px;
  cursor: pointer;
}

.files-list {
  max-height: 155px;
  overflow-y: auto;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-subtle);
  display: flex;
  flex-direction: column;
  gap: 3px;
  flex-shrink: 0;
}

.file-row {
  border: none;
  background: transparent;
  border-radius: 8px;
  padding: 6px 8px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  cursor: pointer;
  font-size: 12px;
  color: var(--text-muted);
  text-align: left;
}

.file-row:hover {
  background: var(--hover-bg);
  color: var(--text-main);
}

.file-row.has-tooltip {
  position: relative;
}

.custom-file-tooltip {
  position: absolute;
  top: calc(100% + 4px);
  left: 8px;
  z-index: 100;
  background: #090c12;
  border: 1px solid var(--border-subtle);
  border-radius: 6px;
  padding: 5px 8px;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.55);
  display: none;
  flex-direction: column;
  gap: 2px;
  max-width: 380px;
  pointer-events: none;
  word-break: break-all;
  text-align: left;
}

.file-row:hover .custom-file-tooltip {
  display: flex;
}

.tooltip-label {
  font-size: 9.5px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--accent);
}

.tooltip-path {
  font-family: var(--font-code), ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  color: #e2e8f0;
  line-height: 1.35;
}

.file-row.selected {
  background: var(--active-bg);
  color: var(--text-main);
}

.file-path {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.file-row-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.file-row-undo-btn {
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
  opacity: 0.45;
}

.file-row:hover .file-row-undo-btn {
  opacity: 1;
}

.file-row-undo-btn:hover {
  background: rgba(239, 68, 68, 0.2);
  border-color: #ef4444;
  color: #ef4444;
  box-shadow: 0 0 6px rgba(239, 68, 68, 0.3);
}

.file-counts {
  display: flex;
  gap: 6px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11.5px;
  flex-shrink: 0;
}

.c-mod {
  color: var(--diff-mod-bar);
}

.c-add {
  color: var(--diff-add-bar);
}

.c-del {
  color: var(--diff-del-bar);
}

.diff-legend {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--border-subtle);
  font-size: 11.5px;
  color: var(--text-muted);
  flex-shrink: 0;
}

.legend-tags {
  display: flex;
  align-items: center;
  gap: 6px;
}

.diff-undo-selected-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 7px;
  border-radius: 4px;
  border: 1px solid rgba(239, 68, 68, 0.25);
  background: rgba(239, 68, 68, 0.08);
  color: #f87171;
  font-size: 10.5px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
}

.diff-undo-selected-btn:hover {
  background: rgba(239, 68, 68, 0.2);
  border-color: #ef4444;
  color: #ffffff;
  box-shadow: 0 0 6px rgba(239, 68, 68, 0.3);
}

.leg {
  padding: 2px 7px;
  border-radius: 5px;
}

.leg.mod {
  background: var(--diff-mod-bg);
  border-left: 3px solid var(--diff-mod-bar);
}

.leg.add {
  background: var(--diff-add-bg);
  border-left: 3px solid var(--diff-add-bar);
}

.leg.del {
  background: var(--diff-del-bg);
  border-left: 3px solid var(--diff-del-bar);
}

.diff-viewer {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 8px 0;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12px;
}

.diff-line {
  display: flex;
  align-items: baseline;
  padding: 2px 10px;
  border-left: 3px solid transparent;
  white-space: pre;
}

.diff-line--mod {
  background: var(--diff-mod-bg);
  border-left-color: var(--diff-mod-bar);
}

.diff-line--add {
  background: var(--diff-add-bg);
  border-left-color: var(--diff-add-bar);
}

.diff-line--del {
  background: var(--diff-del-bg);
  border-left-color: var(--diff-del-bar);
}

.diff-line--target {
  box-shadow: inset 0 0 0 1px rgba(129, 140, 248, 0.65);
  background: rgba(129, 140, 248, 0.16);
}

.ln-num {
  width: 34px;
  flex-shrink: 0;
  color: var(--text-dim);
  user-select: none;
}

.ln-prefix {
  width: 16px;
  flex-shrink: 0;
  user-select: none;
  font-weight: 600;
}

.ln-code {
  flex: 1;
}

.art-lang {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11.5px;
  color: var(--accent-2);
}

.artifact-viewer {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 10px;
  gap: 8px;
}

.artifact-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.artifact-toolbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.view-toggle {
  display: flex;
  align-items: center;
  background: var(--surface-elevated);
  border: 1px solid var(--border-subtle);
  border-radius: 7px;
  padding: 2px;
  gap: 2px;
}

.toggle-btn {
  border: none;
  background: transparent;
  color: var(--text-muted);
  border-radius: 5px;
  padding: 2px 7px;
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;
}

.toggle-btn.active {
  background: var(--active-bg);
  color: var(--accent);
}

.artifact-preview-container {
  flex: 1;
  min-height: 0;
  border-radius: 10px;
  border: 1px solid var(--border-subtle);
  background: #ffffff;
  overflow: hidden;
  display: flex;
}

.rendered-html-iframe {
  width: 100%;
  height: 100%;
  border: none;
  background: #ffffff;
}

.rendered-svg-wrap {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: #ffffff;
  overflow: auto;
}

.artifact-name {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12.5px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.copy-btn {
  border: 1px solid var(--border-subtle);
  background: var(--hover-bg);
  border-radius: 7px;
  padding: 3px 9px;
  font-size: 12px;
  cursor: pointer;
}

.artifact-code {
  margin: 0;
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 10px;
  border-radius: 10px;
  border: 1px solid var(--border-subtle);
  background: var(--surface-elevated);
  font-family: var(--font-code), ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 12.5px;
  line-height: 1.5;
}
</style>
