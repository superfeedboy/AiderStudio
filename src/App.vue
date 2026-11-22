<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { useStudioStore } from "./stores/studio";
import LeMenu from "./components/LeMenu.vue";
import LaDiscussion from "./components/LaDiscussion.vue";
import LeCode from "./components/LeCode.vue";
import SettingsModal from "./components/SettingsModal.vue";
import ContextModal from "./components/ContextModal.vue";
import ProjectSettingsModal from "./components/ProjectSettingsModal.vue";
import NewProjectModal from "./components/NewProjectModal.vue";
import McpModal from "./components/McpModal.vue";
import DeleteConvModal from "./components/DeleteConvModal.vue";
import AppSplashLoader from "./components/AppSplashLoader.vue";

const store = useStudioStore();
const isResizingCode = ref(false);
const isInitializing = ref(true);

onMounted(async () => {
  try {
    await store.init();
  } finally {
    isInitializing.value = false;
  }
});

onBeforeUnmount(() => {
  store.dispose();
});

function startResizeMenu(ev: MouseEvent) {
  ev.preventDefault();
  const startX = ev.clientX;
  const startW = store.menuWidth;

  const onMove = (moveEv: MouseEvent) => {
    const next = startW + (moveEv.clientX - startX);
    store.menuWidth = Math.max(195, Math.min(380, next));
  };

  const onUp = () => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
  };

  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}

function startResizeCode(ev: MouseEvent) {
  ev.preventDefault();
  isResizingCode.value = true;
  const startX = ev.clientX;
  const startW = store.codeWidth;

  const onMove = (moveEv: MouseEvent) => {
    const next = startW - (moveEv.clientX - startX);
    store.codeWidth = Math.max(280, Math.min(680, next));
  };

  const onUp = () => {
    isResizingCode.value = false;
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
  };

  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}
</script>

<template>
  <div class="app-table">
    <!-- 1. LE MENU -->
    <LeMenu />

    <div
      class="resizer-bar"
      title="Redimensionner le menu"
      @mousedown="startResizeMenu"
    />

    <!-- WORKSPACE STAGE: 2. LA DISCUSSION (+ 3. LE HUD) + 4. LE CODE -->
    <div class="workspace-stage" :class="{ 'is-wide-split': store.isWideScreen }">
      <LaDiscussion />

      <div
        v-if="store.codeOpen && !store.isWideScreen"
        class="resizer-bar"
        title="Redimensionner le code"
        @mousedown="startResizeCode"
      />

      <LeCode :is-resizing="isResizingCode" />
    </div>

    <!-- Modals -->
    <SettingsModal />
    <ContextModal />
    <ProjectSettingsModal />
    <NewProjectModal />
    <McpModal />
    <DeleteConvModal />

    <!-- Splash Loader initial -->
    <AppSplashLoader :loading="isInitializing" />
  </div>
</template>

<style scoped>
.app-table {
  width: 100vw;
  height: 100vh;
  display: flex;
  background: var(--table-bg);
  padding: 8px 10px 8px 0;
  overflow: hidden;
}

.workspace-stage {
  flex: 1;
  min-width: 0;
  height: 100%;
  display: flex;
  overflow: hidden;
}

.workspace-stage.is-wide-split > :deep(.la-discussion) {
  flex: 0 0 50% !important;
  max-width: 50% !important;
  width: 50% !important;
}

.workspace-stage.is-wide-split > :deep(.le-code) {
  flex: 0 0 50% !important;
  max-width: 50% !important;
  width: 50% !important;
}
</style>
