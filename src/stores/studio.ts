import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppSettings,
  AppStateData,
  AttachedFile,
  CodeDrawerMode,
  Conversation,
  HardwareTelemetry,
  McpServerConfig,
  McpToolInfo,
  Project,
  PromptStageEvent,
  SendPromptResult,
  StackStatus,
  ThemePreference,
  ToolExecutionResult,
} from "../types/studio";

const isTauri = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const defaultSettings: AppSettings = {
  theme: "system",
  ollamaUrl: "http://127.0.0.1:11434",
  ollamaModel: "qwen2.5-coder:3b",
  aiderPath: "aider",
  autoStartOllama: true,
};

export const useStudioStore = defineStore("studio", () => {
  const settings = ref<AppSettings>({ ...defaultSettings });
  const projects = ref<Project[]>([]);
  const conversations = ref<Conversation[]>([]);
  const activeProjectId = ref<string | null>(null);
  const activeConversationId = ref<string | null>(null);

  const telemetry = ref<HardwareTelemetry>({
    cpuUsage: 0,
    gpuUsage: null,
    memUsedGb: 0,
    memTotalGb: 0,
    memPercent: 0,
    tempCelsius: null,
    platform: "macOS",
  });

  const stackStatus = ref<StackStatus>({
    aiderInstalled: false,
    aiderBin: null,
    ollamaInstalled: false,
    ollamaRunning: false,
    modelAvailable: false,
    availableModels: [],
  });

  // 4-zone layout state
  const menuWidth = ref<number>(250);
  const hudVisible = ref<boolean>(true);
  const codeOpen = ref<boolean>(false); // Hidden by default on the right
  const codeMode = ref<CodeDrawerMode>("review");
  const codeWidth = ref<number>(420);
  const activeDiffFilePath = ref<string | null>(null);
  const activeArtifactId = ref<string | null>(null);

  // Screen aspect ratio & ultra-wide 50/50 detection (ratio > standard 16:9, e.g. ratio >= 2.05 & width >= 1550)
  const isWideScreen = ref<boolean>(false);
  // Auto-tools execution permission toggle (Ligne 2 du composer)
  const autoToolsEnabled = ref<boolean>(true);

  // Modals
  const settingsOpen = ref<boolean>(false);
  const contextModalOpen = ref<boolean>(false);
  const projectSettingsModalOpen = ref<boolean>(false);
  const projectToEdit = ref<Project | null>(null);
  const newProjectModalOpen = ref<boolean>(false);
  const mcpModalOpen = ref<boolean>(false);
  const deleteConvModalOpen = ref<boolean>(false);
  const convToDelete = ref<Conversation | null>(null);

  // MCP Servers
  const mcpServers = ref<McpServerConfig[]>([]);

  // Prompt processing state
  const isProcessing = ref<boolean>(false);
  const currentStage = ref<PromptStageEvent | null>(null);
  const liveStreamLines = ref<string[]>([]);
  const liveThinking = ref<string>("");
  const liveContent = ref<string>("");

  // System dark/light tracking
  const systemPrefersLight = ref<boolean>(
    typeof window !== "undefined" &&
      window.matchMedia("(prefers-color-scheme: light)").matches
  );

  const effectiveTheme = computed<"dark" | "light">(() => {
    if (settings.value.theme === "system") {
      return systemPrefersLight.value ? "light" : "dark";
    }
    return settings.value.theme;
  });

  const activeProject = computed<Project | null>(() => {
    if (!activeProjectId.value) return projects.value[0] ?? null;
    return (
      projects.value.find((p) => p.id === activeProjectId.value) ??
      projects.value[0] ??
      null
    );
  });

  const activeConversation = computed<Conversation | null>(() => {
    if (!activeConversationId.value) return conversations.value[0] ?? null;
    return (
      conversations.value.find((c) => c.id === activeConversationId.value) ??
      conversations.value[0] ??
      null
    );
  });

  const contextTokensMax = ref<number>(8192);

  const contextTokensUsed = computed<number>(() => {
    let totalChars = 0;
    if (activeProject.value?.globalContext) {
      totalChars += activeProject.value.globalContext.length;
    }
    if (activeConversation.value) {
      totalChars += (activeConversation.value.conversationContext || "").length;
      for (const msg of activeConversation.value.messages) {
        totalChars += (msg.content || "").length;
      }
    }
    return Math.ceil(totalChars / 3.8);
  });

  const contextPercent = computed<number>(() => {
    return Math.min(100, Math.round((contextTokensUsed.value / contextTokensMax.value) * 100));
  });

  const conversationsForProject = (projectId: string) =>
    conversations.value.filter((c) => c.projectId === projectId);

  function applyThemeToDom() {
    if (typeof document !== "undefined") {
      document.documentElement.setAttribute("data-theme", effectiveTheme.value);
    }
  }

  function applyStateData(data: AppStateData) {
    settings.value = data.settings;
    projects.value = data.projects;
    conversations.value = data.conversations;
    activeProjectId.value = data.activeProjectId;
    activeConversationId.value = data.activeConversationId;
    mcpServers.value = data.mcpServers ?? [];
    applyThemeToDom();
  }

  let initialized = false;
  let telemetryTimer: number | null = null;

  async function init() {
    if (initialized) return;
    initialized = true;

    const mql = window.matchMedia("(prefers-color-scheme: light)");
    mql.addEventListener("change", (ev) => {
      systemPrefersLight.value = ev.matches;
      applyThemeToDom();
    });

    if (isTauri()) {
      const initial = await invoke<AppStateData>("get_initial_state");
      applyStateData(initial);

      await listen<PromptStageEvent>("aider:stage", (event) => {
        currentStage.value = event.payload;
      });

      await listen<[string, string]>("aider:stream", (event) => {
        const [convId, line] = event.payload;
        if (convId === activeConversationId.value) {
          liveStreamLines.value = [...liveStreamLines.value.slice(-18), line];
        }
      });

      await listen<[string, string]>("aider:thinking", (event) => {
        const [convId, text] = event.payload;
        if (convId === activeConversationId.value) {
          liveThinking.value = text;
        }
      });

      await listen<[string, string]>("aider:live_chunk", (event) => {
        const [convId, text] = event.payload;
        if (convId === activeConversationId.value) {
          liveContent.value = text;
        }
      });

      await Promise.all([refreshTelemetry(), refreshStackStatus()]);

      telemetryTimer = window.setInterval(() => {
        void refreshTelemetry();
      }, 2500);
    } else {
      // Browser preview fallback when inspecting via `npm run dev`
      const now = new Date().toISOString();
      const pid = "preview-project";
      const cid = "preview-conv";
      applyStateData({
        settings: { ...defaultSettings },
        projects: [
          {
            id: pid,
            name: "AiderStudio",
            rootPath: "/Users/gilles/Documents/Dev/AiderStudio",
            globalContext:
              "Application Tauri v2 + Vue 3 + Rust orchestrant Aider & Ollama.",
            updatedAt: now,
          },
        ],
        conversations: [
          {
            id: cid,
            projectId: pid,
            title: "Nouvelle session",
            conversationContext: "",
            messages: [],
            artifacts: [],
            modifiedFiles: [],
            linesAdded: 0,
            linesRemoved: 0,
            createdAt: now,
            updatedAt: now,
          },
        ],
        activeProjectId: pid,
        activeConversationId: cid,
      });
    }

    if (typeof window !== "undefined") {
      handleWindowResize();
      window.addEventListener("resize", handleWindowResize);
      if (isWideScreen.value) {
        codeOpen.value = true;
      }
    }
  }

  function handleWindowResize() {
    if (typeof window === "undefined") return;
    const w = window.innerWidth;
    const h = window.innerHeight;
    const ratio = h > 0 ? w / h : 0;
    // Ratio standard classique (16:9 = 1.78, 16:10 = 1.6).
    // Détection si l'application est vraiment au-dessus d'un ratio standard :
    // ratio >= 2.05 (ex: écrans 21:9 ultrawide, double écran, ou fenêtre très étirée horizontalement)
    const isUltraWide = ratio >= 2.05 && w >= 1550;
    if (isUltraWide !== isWideScreen.value) {
      isWideScreen.value = isUltraWide;
      if (isUltraWide) {
        codeOpen.value = true;
      }
    }
  }

  function dispose() {
    if (telemetryTimer !== null) {
      clearInterval(telemetryTimer);
      telemetryTimer = null;
    }
    if (typeof window !== "undefined") {
      window.removeEventListener("resize", handleWindowResize);
    }
  }

  async function refreshTelemetry() {
    if (!isTauri()) return;
    try {
      telemetry.value = await invoke<HardwareTelemetry>("get_hardware_telemetry");
    } catch {
      // Ignore transient telemetry errors
    }
  }

  async function refreshStackStatus() {
    if (!isTauri()) return;
    try {
      stackStatus.value = await invoke<StackStatus>("check_stack_status");
    } catch {
      // Ignore transient probe errors
    }
  }

  async function startOllama() {
    if (!isTauri()) return;
    stackStatus.value = await invoke<StackStatus>("start_ollama_service");
  }

  async function saveSettings(next: AppSettings) {
    settings.value = { ...next };
    applyThemeToDom();
    if (isTauri()) {
      await invoke<AppSettings>("save_settings", { settings: next });
      await refreshStackStatus();
    }
  }

  async function setTheme(theme: ThemePreference) {
    await saveSettings({ ...settings.value, theme });
  }

  async function pickProjectDirectory(): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<string | null>("pick_project_directory");
    } catch {
      return null;
    }
  }

  async function createProject(
    name: string,
    rootPath: string,
    globalContext: string
  ) {
    if (isTauri()) {
      const next = await invoke<AppStateData>("create_project", {
        name,
        rootPath,
        globalContext,
      });
      applyStateData(next);
    }
    newProjectModalOpen.value = false;
  }

  function openProjectSettings(project?: Project | null) {
    const target = project ?? activeProject.value;
    if (!target) return;
    projectToEdit.value = JSON.parse(JSON.stringify(target));
    projectSettingsModalOpen.value = true;
  }

  async function updateProject(
    projectId: string,
    name: string,
    globalContext: string,
    sharedDirectories: string[]
  ) {
    if (isTauri()) {
      const next = await invoke<AppStateData>("update_project", {
        projectId,
        name,
        globalContext,
        sharedDirectories,
      });
      applyStateData(next);
    } else {
      const idx = projects.value.findIndex((p) => p.id === projectId);
      if (idx !== -1) {
        projects.value[idx].name = name;
        projects.value[idx].globalContext = globalContext;
        projects.value[idx].sharedDirectories = [...sharedDirectories];
      }
    }
    projectSettingsModalOpen.value = false;
  }

  async function updateProjectContext(projectId: string, globalContext: string) {
    if (isTauri()) {
      const updated = await invoke<Project>("update_project_context", {
        projectId,
        globalContext,
      });
      const idx = projects.value.findIndex((p) => p.id === projectId);
      if (idx !== -1) projects.value[idx] = updated;
    } else {
      const idx = projects.value.findIndex((p) => p.id === projectId);
      if (idx !== -1) projects.value[idx].globalContext = globalContext;
    }
  }

  async function createConversation(projectId: string, title?: string) {
    if (isTauri()) {
      const next = await invoke<AppStateData>("create_conversation", {
        projectId,
        title: title ?? null,
      });
      applyStateData(next);
    }
  }

  async function selectConversation(projectId: string, conversationId: string) {
    activeProjectId.value = projectId;
    activeConversationId.value = conversationId;
    activeDiffFilePath.value = null;
    activeArtifactId.value = null;
    if (isTauri()) {
      await invoke("select_conversation", { projectId, conversationId });
    }
  }

  async function deleteProject(projectId: string) {
    if (isTauri()) {
      const next = await invoke<AppStateData>("delete_project", { projectId });
      applyStateData(next);
    } else {
      projects.value = projects.value.filter((p) => p.id !== projectId);
      conversations.value = conversations.value.filter((c) => c.projectId !== projectId);
      if (activeProjectId.value === projectId) {
        activeProjectId.value = projects.value[0]?.id ?? null;
        activeConversationId.value = conversations.value[0]?.id ?? null;
      }
    }
  }

  function promptDeleteConversation(conv: Conversation) {
    convToDelete.value = conv;
    deleteConvModalOpen.value = true;
  }

  async function confirmDeleteConversation(synthesize: boolean) {
    if (!convToDelete.value) return;
    const conv = convToDelete.value;
    const pid = conv.projectId;

    if (synthesize) {
      const userPrompts = conv.messages
        .filter((m) => m.role === "user")
        .map((m) => m.content.slice(0, 140).replace(/\n/g, " "))
        .filter(Boolean);

      const touchedFiles = (conv.modifiedFiles || []).map((f) => f.path);
      const convCtx = conv.conversationContext ? `\n  - Résumé session : ${conv.conversationContext.replace(/\n/g, " ")}` : "";

      const synthEntry = `• [Acquis de la session "${conv.title}"] :\n  - Requêtes : ${userPrompts.slice(0, 4).join(" | ")}${touchedFiles.length > 0 ? `\n  - Fichiers créés/modifiés : ${touchedFiles.join(", ")}` : ""}${convCtx}`;

      const targetProj = projects.value.find((p) => p.id === pid);
      if (targetProj) {
        const newGlobalCtx = targetProj.globalContext.trim()
          ? `${targetProj.globalContext.trim()}\n\n${synthEntry}`
          : synthEntry;
        await updateProjectContext(pid, newGlobalCtx);
      }
    }

    await deleteConversation(conv.id);
    deleteConvModalOpen.value = false;
    convToDelete.value = null;
  }

  async function deleteConversation(conversationId: string) {
    if (isTauri()) {
      const next = await invoke<AppStateData>("delete_conversation", {
        conversationId,
      });
      applyStateData(next);
    }
  }

  async function listProjectFiles(projectId?: string): Promise<string[]> {
    if (!isTauri()) {
      return [
        "src/App.vue",
        "src/main.ts",
        "src/components/LaDiscussion.vue",
        "src/components/LeHud.vue",
        "src/components/LeCode.vue",
        "src/stores/studio.ts",
        "package.json",
      ];
    }
    try {
      return await invoke<string[]>("list_project_files", {
        projectId: projectId ?? activeProjectId.value,
      });
    } catch {
      return [];
    }
  }

  async function executeTool(
    toolName: string,
    args: Record<string, any>,
    projectId?: string
  ): Promise<ToolExecutionResult | null> {
    if (!isTauri()) return null;
    try {
      return await invoke<ToolExecutionResult>("execute_local_tool", {
        projectId: projectId ?? activeProjectId.value,
        toolName,
        arguments: args,
      });
    } catch {
      return null;
    }
  }

  async function compactConversation(conversationId?: string) {
    const cid = conversationId || activeConversationId.value;
    if (!cid) return;
    const convIdx = conversations.value.findIndex((c) => c.id === cid);
    if (convIdx === -1) return;

    const targetConv = conversations.value[convIdx];
    if (targetConv.messages.length <= 1) return;

    const olderMessages = targetConv.messages.slice(0, Math.max(1, targetConv.messages.length - 2));
    const recentMessages = targetConv.messages.slice(Math.max(1, targetConv.messages.length - 2));

    const summaryBulletPoints = olderMessages
      .filter((m) => m.role === "user")
      .map((m) => `• ${m.content.slice(0, 100).replace(/\n/g, " ")}`)
      .join("\n");

    const compactNotice = `[Session compactée - Mémoire libérée]\nHistorique résumé:\n${summaryBulletPoints}`;

    targetConv.conversationContext = targetConv.conversationContext
      ? `${targetConv.conversationContext}\n\n${compactNotice}`
      : compactNotice;

    const now = new Date().toISOString();
    targetConv.messages = [
      {
        id: "compact-" + Date.now(),
        role: "assistant",
        content: `🧹 **Contexte compacté (/compact)** : La mémoire de conversation a été résumée et compactée. L'espace de la fenêtre de contexte a été libéré avec succès tout en conservant les acquis du projet.`,
        stageSummary: "Nettoyage contexte (/compact)",
        createdAt: now,
        filesChanged: 0,
        linesAdded: 0,
        linesRemoved: 0,
      },
      ...recentMessages,
    ];
    targetConv.updatedAt = now;

    if (isTauri()) {
      await invoke("save_settings", { settings: settings.value });
    }
  }

  async function sendPrompt(prompt: string, attachments?: AttachedFile[]) {
    const conv = activeConversation.value;
    const hasAttachments = Boolean(attachments && attachments.length > 0);
    if (!conv || (!prompt.trim() && !hasAttachments) || isProcessing.value) return;

    // Handle slash command /compact
    if (prompt.trim() === "/compact") {
      await compactConversation(conv.id);
      return;
    }

    // Force /compact automatically at 95% threshold to prevent LLM overflow
    if (contextPercent.value >= 95) {
      await compactConversation(conv.id);
    }

    isProcessing.value = true;
    liveStreamLines.value = [];
    liveThinking.value = "";
    liveContent.value = "";
    currentStage.value = {
      conversationId: conv.id,
      stageIndex: 1,
      stageTotal: 4,
      stageLabel: "Démarrage du traitement",
      detail: "Préparation du contexte...",
      status: "running",
    };

    try {
      if (isTauri()) {
        const res = await invoke<SendPromptResult>("send_prompt", {
          conversationId: conv.id,
          prompt: prompt.trim(),
          attachments: hasAttachments ? attachments : null,
        });

        const cIdx = conversations.value.findIndex(
          (c) => c.id === res.conversation.id
        );
        if (cIdx !== -1) conversations.value[cIdx] = res.conversation;

        const pIdx = projects.value.findIndex((p) => p.id === res.project.id);
        if (pIdx !== -1) projects.value[pIdx] = res.project;

        if (res.conversation.modifiedFiles.length > 0 && !activeDiffFilePath.value) {
          activeDiffFilePath.value = res.conversation.modifiedFiles[0].path;
        }
        if (res.conversation.artifacts.length > 0) {
          activeArtifactId.value =
            res.conversation.artifacts[res.conversation.artifacts.length - 1].id;
        }
      }
    } catch (err) {
      currentStage.value = {
        conversationId: conv.id,
        stageIndex: 2,
        stageTotal: 4,
        stageLabel: "Erreur d'exécution",
        detail: String(err),
        status: "error",
      };
    } finally {
      isProcessing.value = false;
      void refreshStackStatus();
    }
  }

  async function refreshDiff() {
    const conv = activeConversation.value;
    if (!conv || !isTauri()) return;
    const updated = await invoke<Conversation>("refresh_project_diff", {
      conversationId: conv.id,
    });
    const idx = conversations.value.findIndex((c) => c.id === updated.id);
    if (idx !== -1) conversations.value[idx] = updated;
    if (updated.modifiedFiles.length > 0 && !activeDiffFilePath.value) {
      activeDiffFilePath.value = updated.modifiedFiles[0].path;
    }
  }

  function openReview(filePath?: string) {
    codeMode.value = "review";
    if (filePath) {
      activeDiffFilePath.value = filePath;
    } else if (
      !activeDiffFilePath.value &&
      activeConversation.value?.modifiedFiles.length
    ) {
      activeDiffFilePath.value = activeConversation.value.modifiedFiles[0].path;
    }
    codeOpen.value = true;
  }

  function openArtifact(artifactId?: string) {
    codeMode.value = "artifact";
    if (artifactId) {
      activeArtifactId.value = artifactId;
    } else if (
      !activeArtifactId.value &&
      activeConversation.value?.artifacts.length
    ) {
      activeArtifactId.value = activeConversation.value.artifacts[0].id;
    }
    codeOpen.value = true;
  }

  async function clearGlobalContext(projectId: string) {
    await updateProjectContext(projectId, "");
  }

  async function clearConversationContext(conversationId: string) {
    const idx = conversations.value.findIndex((c) => c.id === conversationId);
    if (idx !== -1) {
      conversations.value[idx].conversationContext = "";
      if (isTauri()) {
        await invoke("clear_conversation_context", { conversationId });
      }
    }
  }

  async function saveMcpServer(server: McpServerConfig): Promise<McpServerConfig[]> {
    if (isTauri()) {
      mcpServers.value = await invoke<McpServerConfig[]>("save_mcp_server", { server });
    } else {
      const idx = mcpServers.value.findIndex((s) => s.id === server.id);
      if (idx !== -1) mcpServers.value[idx] = server;
      else mcpServers.value.push(server);
    }
    return mcpServers.value;
  }

  async function deleteMcpServer(serverId: string): Promise<McpServerConfig[]> {
    if (isTauri()) {
      mcpServers.value = await invoke<McpServerConfig[]>("delete_mcp_server", { serverId });
    } else {
      mcpServers.value = mcpServers.value.filter((s) => s.id !== serverId);
    }
    return mcpServers.value;
  }

  async function testMcpServer(server: McpServerConfig): Promise<McpToolInfo[]> {
    if (isTauri()) {
      return await invoke<McpToolInfo[]>("test_mcp_server", {
        server,
        projectId: activeProjectId.value,
      });
    }
    return [
      {
        serverId: server.id,
        serverName: server.name,
        name: "read_query",
        description: "Execute a SELECT query on SQLite",
        inputSchema: { type: "object", properties: { query: { type: "string" } } },
      },
      {
        serverId: server.id,
        serverName: server.name,
        name: "list_tables",
        description: "List all tables in the database",
        inputSchema: { type: "object", properties: {} },
      },
    ];
  }

  async function executeMcpTool(
    serverId: string,
    toolName: string,
    args: Record<string, any>
  ): Promise<string> {
    if (isTauri()) {
      return await invoke<string>("execute_mcp_tool", {
        serverId,
        toolName,
        arguments: args,
        projectId: activeProjectId.value,
      });
    }
    return "Résultat simulé outil MCP";
  }

  function closeCode() {
    codeOpen.value = false;
  }

  return {
    settings,
    projects,
    conversations,
    activeProjectId,
    activeConversationId,
    activeProject,
    activeConversation,
    conversationsForProject,
    telemetry,
    stackStatus,
    menuWidth,
    hudVisible,
    codeOpen,
    codeMode,
    codeWidth,
    isWideScreen,
    autoToolsEnabled,
    activeDiffFilePath,
    activeArtifactId,
    settingsOpen,
    contextModalOpen,
    projectSettingsModalOpen,
    projectToEdit,
    openProjectSettings,
    updateProject,
    newProjectModalOpen,
    mcpModalOpen,
    mcpServers,
    isProcessing,
    currentStage,
    liveStreamLines,
    liveThinking,
    liveContent,
    listProjectFiles,
    executeTool,
    effectiveTheme,
    init,
    dispose,
    refreshTelemetry,
    refreshStackStatus,
    startOllama,
    saveSettings,
    setTheme,
    pickProjectDirectory,
    createProject,
    deleteProject,
    updateProjectContext,
    createConversation,
    selectConversation,
    deleteConversation,
    deleteConvModalOpen,
    convToDelete,
    promptDeleteConversation,
    confirmDeleteConversation,
    sendPrompt,
    refreshDiff,
    openReview,
    openArtifact,
    closeCode,
    saveMcpServer,
    deleteMcpServer,
    testMcpServer,
    executeMcpTool,
    clearGlobalContext,
    clearConversationContext,
    contextTokensMax,
    contextTokensUsed,
    contextPercent,
    compactConversation,
  };
});
