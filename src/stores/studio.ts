import { computed, ref } from "vue";
import { acceptHMRUpdate, defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AgentRoutingConfig,
  AgentTraceEvent,
  AgentTraceStep,
  ApiUsageState,
  AppSettings,
  AppStateData,
  AttachedFile,
  ChatMessage,
  CodeDrawerMode,
  Conversation,
  HardwareTelemetry,
  McpServerConfig,
  McpToolInfo,
  ProcessStep,
  ProcessStepEvent,
  Project,
  ProjectRuleItem,
  PromptStageEvent,
  ProviderConfig,
  SendPromptResult,
  StackStatus,
  ThemePreference,
  TokenUsageEvent,
  ToolApprovalRequest,
  ToolExecutionResult,
} from "../types/studio";

const isTauri = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function defaultProvidersList(): ProviderConfig[] {
  return [
    {
      id: "gemini",
      name: "Gemini",
      apiKey: "",
      baseUrl: null,
      validated: false,
      availableModels: [
        "gemini-2.5-flash",
        "gemini-2.5-pro",
        "gemini-2.0-flash",
      ],
      dailyTokenLimit: 500000,
    },
    {
      id: "openai",
      name: "OpenAI",
      apiKey: "",
      baseUrl: null,
      validated: false,
      availableModels: ["gpt-4.1", "gpt-4.1-mini", "o4-mini", "gpt-4o"],
      dailyTokenLimit: 500000,
    },
    {
      id: "anthropic",
      name: "Anthropic (Claude)",
      apiKey: "",
      baseUrl: null,
      validated: false,
      availableModels: [
        "claude-sonnet-4-20250514",
        "claude-3-7-sonnet-20250219",
        "claude-3-5-haiku-20241022",
      ],
      dailyTokenLimit: 500000,
    },
    {
      id: "deepseek",
      name: "DeepSeek",
      apiKey: "",
      baseUrl: null,
      validated: false,
      availableModels: ["deepseek-chat", "deepseek-reasoner"],
      dailyTokenLimit: 500000,
    },
    {
      id: "openrouter",
      name: "OpenRouter",
      apiKey: "",
      baseUrl: null,
      validated: false,
      availableModels: [
        "deepseek/deepseek-r1:free",
        "deepseek/deepseek-chat-v3-0324:free",
        "qwen/qwen3-coder:free",
        "google/gemini-2.0-flash-exp:free",
        "anthropic/claude-sonnet-4",
        "google/gemini-2.5-flash",
        "deepseek/deepseek-chat-v3-0324",
        "qwen/qwen-2.5-coder-32b-instruct",
      ],
      dailyTokenLimit: 500000,
    },
    {
      id: "groq",
      name: "Groq",
      apiKey: "",
      baseUrl: null,
      validated: false,
      availableModels: [
        "llama-3.3-70b-versatile",
        "qwen/qwen3-32b",
        "moonshotai/kimi-k2-instruct",
        "llama-3.1-8b-instant",
      ],
      dailyTokenLimit: 500000,
    },
    {
      id: "cerebras",
      name: "Cerebras",
      apiKey: "",
      baseUrl: null,
      validated: false,
      availableModels: [
        "qwen-3-235b-a22b",
        "llama-3.3-70b",
        "qwen-3-32b",
      ],
      dailyTokenLimit: 500000,
    },
    {
      id: "mistral",
      name: "Mistral",
      apiKey: "",
      baseUrl: null,
      validated: false,
      availableModels: [
        "codestral-latest",
        "devstral-small-2505",
        "mistral-large-latest",
      ],
      dailyTokenLimit: 500000,
    },
  ];
}

export function defaultAgentRouting(): AgentRoutingConfig {
  return {
    architect: { provider: "ollama", model: "", fallbacks: [] },
    coder: { provider: "ollama", model: "", fallbacks: [] },
    reviewer: { provider: "ollama", model: "", fallbacks: [] },
  };
}

const defaultSettings: AppSettings = {
  theme: "system",
  ollamaUrl: "http://127.0.0.1:11434",
  ollamaModel: "qwen2.5-coder:3b",
  autoStartOllama: true,
  contextWindowTokens: 8192,
  aiderEditFormat: "auto",
  activeTarget: "agent",
  providers: defaultProvidersList(),
  agentRouting: defaultAgentRouting(),
  hudContextView: "agent",
  hudShowTokenUsage: false,
  hudShowLocalResources: true,
  hudShowGlobalDiff: true,
  hudAutoArtifactEvolutionReport: true,
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
    nativeEngineReady: true,
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
  const activeDiffMessageId = ref<string | null>(null);
  const activeDiffTargetLine = ref<number | null>(null);
  const activeArtifactId = ref<string | null>(null);
  const targetScrollMsgId = ref<string>("");

  // Screen aspect ratio & ultra-wide 50/50 detection (ratio > standard 16:9, e.g. ratio >= 2.05 & width >= 1550)
  const isWideScreen = ref<boolean>(false);
  // Auto-tools execution permission toggle (Ligne 2 du composer)
  const autoToolsEnabled = ref<boolean>(true);

  // Modals
  const settingsOpen = ref<boolean>(false);
  const settingsActiveTab = ref<"general" | "providers" | "agents">("general");
  const contextModalOpen = ref<boolean>(false);

  function openSettingsTab(tab: "general" | "providers" | "agents" = "general") {
    settingsActiveTab.value = tab;
    settingsOpen.value = true;
  }

  const apiUsage = ref<ApiUsageState>({
    date: new Date().toISOString().slice(0, 10),
    byProvider: {},
  });
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
  const isCancelling = ref<boolean>(false);
  const processingConversationId = ref<string | null>(null);
  const currentStage = ref<PromptStageEvent | null>(null);
  const liveStreamLines = ref<string[]>([]);
  const liveThinking = ref<string>("");
  const liveContent = ref<string>("");
  const liveAgentTrace = ref<AgentTraceStep[]>([]);
  const liveProcessSteps = ref<ProcessStep[]>([]);
  const pendingToolApprovals = ref<ToolApprovalRequest[]>([]);

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

  const contextTokensMax = computed<number>(() => {
    return settings.value.contextWindowTokens && settings.value.contextWindowTokens > 0
      ? settings.value.contextWindowTokens
      : 8192;
  });

  // Track live token usage, peak load & per-subagent breakdown per conversation
  const conversationTokenUsage = ref<Record<string, TokenUsageEvent>>({});
  const conversationPeakTokens = ref<Record<string, TokenUsageEvent>>({});
  const conversationTokenSteps = ref<Record<string, TokenUsageEvent[]>>({});

  const lastCallTokens = computed<TokenUsageEvent | null>(() => {
    if (!activeConversationId.value) return null;
    return conversationTokenUsage.value[activeConversationId.value] ?? null;
  });

  const peakCallTokens = computed<TokenUsageEvent | null>(() => {
    if (!activeConversationId.value) return null;
    return conversationPeakTokens.value[activeConversationId.value] ?? null;
  });

  const turnTokenSteps = computed<TokenUsageEvent[]>(() => {
    if (!activeConversationId.value) return [];
    return conversationTokenSteps.value[activeConversationId.value] ?? [];
  });

  const displayedCallTokenEvent = computed<TokenUsageEvent | null>(() => {
    if (isProcessing.value) {
      return lastCallTokens.value ?? peakCallTokens.value;
    }
    return peakCallTokens.value ?? lastCallTokens.value;
  });

  const contextTokensUsed = computed<number>(() => {
    let totalChars = 0;
    if (activeProject.value?.globalContext) {
      totalChars += activeProject.value.globalContext.length;
    }
    if (activeConversation.value) {
      totalChars += (activeConversation.value.conversationContext || "").length;

      const msgs = activeConversation.value.messages;
      const count = msgs.length;

      msgs.forEach((msg, idx) => {
        const isRecent = idx >= count - 2;
        let content = msg.content || "";

        if (msg.role === "assistant") {
          // Exclude bulky generated code blocks from past messages:
          // The code is already written on the hard drive and tracked by Git/Aider.
          // Including hundreds of lines of past code in session tokens is a pure duplicate.
          if (!isRecent) {
            content = content.replace(/```[^\n]*\n[\s\S]*?```/g, "[Code sur disque]");
          } else {
            content = content.replace(/```[^\n]*\n([\s\S]*?)```/g, (_, code) => {
              const lines = code.trim().split("\n").length;
              return `[Code: ${lines}l]`;
            });
          }
        }

        totalChars += content.length;
      });
    }
    return Math.ceil(totalChars / 3.8);
  });

  // While running, track live sub-agent tokens; once complete, keep peak call load
  // so small Stage 5 Synthesizer calls never hide the Codeur's actual peak VRAM usage.
  const activeCallTokens = computed<number>(() => {
    if (isProcessing.value) {
      if (lastCallTokens.value && lastCallTokens.value.totalTokens > 0) {
        return lastCallTokens.value.totalTokens;
      }
      return contextTokensUsed.value;
    }
    if (peakCallTokens.value && peakCallTokens.value.totalTokens > 0) {
      return peakCallTokens.value.totalTokens;
    }
    if (lastCallTokens.value && lastCallTokens.value.totalTokens > 0) {
      return lastCallTokens.value.totalTokens;
    }
    return contextTokensUsed.value;
  });

  const contextPercent = computed<number>(() => {
    return Math.min(100, Math.round((contextTokensUsed.value / contextTokensMax.value) * 100));
  });

  const activeContextPercent = computed<number>(() => {
    return Math.min(100, Math.round((activeCallTokens.value / contextTokensMax.value) * 100));
  });

  const validatedExternalProviders = computed<ProviderConfig[]>(() => {
    return (settings.value.providers ?? []).filter(
      (p) => p.validated && p.apiKey.trim().length > 0
    );
  });

  const externalApiTokensTotal = computed<number>(() => {
    const entries = Object.values(apiUsage.value.byProvider ?? {});
    return entries.reduce((sum, e) => sum + (e.totalTokens || 0), 0);
  });

  const externalApiTokenLimit = computed<number>(() => {
    const provs = settings.value.providers ?? [];
    const activeIds = new Set<string>([
      ...validatedExternalProviders.value.map((p) => p.id),
      ...Object.keys(apiUsage.value.byProvider ?? {}),
    ]);
    let limit = 0;
    for (const pid of activeIds) {
      const cfg = provs.find((p) => p.id === pid);
      limit += cfg?.dailyTokenLimit && cfg.dailyTokenLimit > 0 ? cfg.dailyTokenLimit : 500000;
    }
    return limit > 0 ? limit : 500000;
  });

  const externalApiPercent = computed<number>(() => {
    return Math.min(
      100,
      Math.round((externalApiTokensTotal.value / Math.max(1, externalApiTokenLimit.value)) * 100)
    );
  });

  const turnTotalTokens = computed<number>(() => {
    return turnTokenSteps.value.reduce((sum, s) => sum + (s.totalTokens || 0), 0);
  });

  const localPeakTokens = computed<number>(() => {
    let peak = 0;
    for (const s of turnTokenSteps.value) {
      if ((s.provider || "ollama") === "ollama") {
        peak = Math.max(peak, s.totalTokens || 0);
      }
    }
    return peak;
  });

  const localContextPercent = computed<number>(() => {
    const denom = Math.max(1, contextTokensMax.value);
    const localUsed = Math.max(contextTokensUsed.value, localPeakTokens.value);
    return Math.min(100, Math.round((localUsed / denom) * 100));
  });

  const hudShowTokenUsageEffective = computed<boolean>(() => {
    return settings.value.hudShowTokenUsage ?? false;
  });

  const hudShowLocalResourcesEffective = computed<boolean>(() => {
    return settings.value.hudShowLocalResources ?? false;
  });

  const hudShowGlobalDiffEffective = computed<boolean>(() => {
    return settings.value.hudShowGlobalDiff ?? true;
  });

  const hudContextViewEffective = computed<"agent" | "global">(() => {
    return settings.value.hudContextView === "global" ? "global" : "agent";
  });

  const hudAutoArtifactEvolutionReportEffective = computed<boolean>(() => {
    return settings.value.hudAutoArtifactEvolutionReport ?? true;
  });

  const activeTargetLabel = computed<{
    isAgent: boolean;
    providerId: string;
    providerName: string;
    modelName: string;
  }>(() => {
    const raw = (settings.value.activeTarget || "agent").trim();
    if (!raw || raw === "agent") {
      return {
        isAgent: true,
        providerId: "agent",
        providerName: "Agent",
        modelName: "Multi-agents",
      };
    }
    const colonIdx = raw.indexOf(":");
    const provId = colonIdx !== -1 ? raw.slice(0, colonIdx) : "ollama";
    const modelName = colonIdx !== -1 ? raw.slice(colonIdx + 1) : raw;
    const providerNames: Record<string, string> = {
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
    return {
      isAgent: false,
      providerId: provId,
      providerName: providerNames[provId] || provId,
      modelName,
    };
  });

  const conversationsForProject = (projectId: string) =>
    conversations.value.filter((c) => c.projectId === projectId);

  function applyThemeToDom() {
    if (typeof document !== "undefined") {
      document.documentElement.setAttribute("data-theme", effectiveTheme.value);
    }
  }

  function applyStateData(data: AppStateData) {
    const rawSettings = data.settings || ({} as Partial<AppSettings>);
    const providersList =
      rawSettings.providers && rawSettings.providers.length > 0
        ? rawSettings.providers
        : defaultProvidersList();

    const hasExternal = providersList.some(
      (p) => p.validated || (p.apiKey && p.apiKey.trim().length > 0)
    );
    const ollamaUrl = (rawSettings.ollamaUrl || "http://127.0.0.1:11434").toLowerCase();
    const isLocalOllama = ollamaUrl.includes("127.0.0.1") || ollamaUrl.includes("localhost");

    const resolvedHudShowTokenUsage: boolean =
      typeof rawSettings.hudShowTokenUsage === "boolean"
        ? rawSettings.hudShowTokenUsage
        : hasExternal;

    const resolvedHudShowLocalResources: boolean =
      typeof rawSettings.hudShowLocalResources === "boolean"
        ? rawSettings.hudShowLocalResources
        : isLocalOllama;

    const resolvedHudShowGlobalDiff: boolean =
      typeof rawSettings.hudShowGlobalDiff === "boolean"
        ? rawSettings.hudShowGlobalDiff
        : true;

    const resolvedHudAutoArtifactEvolutionReport: boolean =
      typeof rawSettings.hudAutoArtifactEvolutionReport === "boolean"
        ? rawSettings.hudAutoArtifactEvolutionReport
        : true;

    const resolvedHudContextView: "agent" | "global" =
      rawSettings.hudContextView === "global" ? "global" : "agent";

    const baseRouting = defaultAgentRouting();
    const rawRouting = rawSettings.agentRouting;

    settings.value = {
      ...defaultSettings,
      ...rawSettings,
      activeTarget: rawSettings.activeTarget || "agent",
      providers: providersList,
      agentRouting: {
        architect: rawRouting?.architect ?? rawRouting?.synthesizer ?? baseRouting.architect,
        coder:
          rawRouting?.coder ??
          rawRouting?.designer ??
          rawRouting?.refactoring ??
          baseRouting.coder,
        reviewer: rawRouting?.reviewer ?? rawRouting?.tester ?? baseRouting.reviewer,
      },
      hudContextView: resolvedHudContextView,
      hudShowTokenUsage: resolvedHudShowTokenUsage,
      hudShowLocalResources: resolvedHudShowLocalResources,
      hudShowGlobalDiff: resolvedHudShowGlobalDiff,
      hudAutoArtifactEvolutionReport: resolvedHudAutoArtifactEvolutionReport,
    };
    projects.value = data.projects;
    conversations.value = data.conversations;
    activeProjectId.value = data.activeProjectId;
    activeConversationId.value = data.activeConversationId;
    mcpServers.value = data.mcpServers ?? [];
    apiUsage.value = data.apiUsage ?? {
      date: new Date().toISOString().slice(0, 10),
      byProvider: {},
    };
    applyThemeToDom();
    void checkAiderignore();
  }

  let initialized = false;
  let telemetryTimer: number | null = null;
  let pendingLiveContent: string | null = null;
  let pendingLiveThinking: string | null = null;
  let pendingLiveStreamLines: string[] | null = null;
  let liveFlushRafId: number | null = null;

  function clearPendingLiveBuffers() {
    if (liveFlushRafId !== null && typeof window !== "undefined") {
      window.cancelAnimationFrame(liveFlushRafId);
      liveFlushRafId = null;
    }
    pendingLiveContent = null;
    pendingLiveThinking = null;
    pendingLiveStreamLines = null;
  }

  function flushLiveBuffers() {
    liveFlushRafId = null;
    if (pendingLiveStreamLines !== null) {
      liveStreamLines.value = pendingLiveStreamLines;
      pendingLiveStreamLines = null;
    }
    if (pendingLiveThinking !== null) {
      liveThinking.value = pendingLiveThinking;
      pendingLiveThinking = null;
    }
    if (pendingLiveContent !== null) {
      liveContent.value = pendingLiveContent;
      pendingLiveContent = null;
    }
  }

  function scheduleLiveFlush() {
    if (typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") {
      flushLiveBuffers();
      return;
    }
    if (liveFlushRafId === null) {
      liveFlushRafId = window.requestAnimationFrame(flushLiveBuffers);
    }
  }

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

      await listen<ApiUsageState>("aider:api_usage", (event) => {
        if (event.payload) {
          apiUsage.value = event.payload;
        }
      });

      await listen<AgentTraceEvent>("aider:trace", (event) => {
        const payload = event.payload;
        if (!payload || !payload.step) return;
        if (
          payload.conversationId === activeConversationId.value ||
          payload.conversationId === processingConversationId.value
        ) {
          const next = [...liveAgentTrace.value];
          const idx = next.findIndex((s) => s.role === payload.step.role);
          if (idx !== -1) {
            next[idx] = payload.step;
          } else {
            next.push(payload.step);
          }
          liveAgentTrace.value = next;
        }
      });

      await listen<ProcessStepEvent>("aider:process_step", (event) => {
        const payload = event.payload;
        if (!payload || !payload.step) return;
        if (
          payload.conversationId === activeConversationId.value ||
          payload.conversationId === processingConversationId.value
        ) {
          const next = [...liveProcessSteps.value];
          const idx = next.findIndex((s) => s.id === payload.step.id);
          if (idx !== -1) {
            next[idx] = payload.step;
          } else {
            next.push(payload.step);
          }
          liveProcessSteps.value = next;
        }
      });

      await listen<TokenUsageEvent>("aider:tokens", (event) => {
        const payload = event.payload;
        if (payload && payload.conversationId) {
          const cid = payload.conversationId;
          conversationTokenUsage.value = {
            ...conversationTokenUsage.value,
            [cid]: payload,
          };

          const existingSteps = conversationTokenSteps.value[cid] ?? [];
          const stepKey = payload.role || payload.subagent;
          const stepIdx = existingSteps.findIndex(
            (s) => (s.role || s.subagent) === stepKey
          );
          const nextSteps = [...existingSteps];
          if (stepIdx !== -1) {
            const prev = nextSteps[stepIdx];
            if (prev.provider === payload.provider && prev.model === payload.model) {
              nextSteps[stepIdx] = {
                ...payload,
                promptTokens: prev.promptTokens + payload.promptTokens,
                completionTokens: prev.completionTokens + payload.completionTokens,
                totalTokens: prev.totalTokens + payload.totalTokens,
              };
            } else {
              nextSteps[stepIdx] = payload;
            }
          } else {
            nextSteps.push(payload);
          }
          conversationTokenSteps.value = {
            ...conversationTokenSteps.value,
            [cid]: nextSteps,
          };

          const currentPeak = conversationPeakTokens.value[cid];
          if (!currentPeak || payload.totalTokens >= currentPeak.totalTokens) {
            conversationPeakTokens.value = {
              ...conversationPeakTokens.value,
              [cid]: payload,
            };
          }
        }
      });

      await listen<[string, string]>("aider:stream", (event) => {
        const [convId, line] = event.payload;
        if (convId === activeConversationId.value) {
          const base = pendingLiveStreamLines ?? liveStreamLines.value;
          pendingLiveStreamLines = [...base.slice(-18), line];
          scheduleLiveFlush();
        }
      });

      await listen<[string, string]>("aider:thinking", (event) => {
        const [convId, text] = event.payload;
        if (convId === activeConversationId.value) {
          pendingLiveThinking = text;
          scheduleLiveFlush();
        }
      });

      await listen<[string, string]>("aider:live_chunk", (event) => {
        const [convId, text] = event.payload;
        if (convId === activeConversationId.value) {
          pendingLiveContent = text;
          scheduleLiveFlush();
        }
      });

      await listen<[string, string]>("aider:conversation_titled", (event) => {
        const [convId, title] = event.payload;
        const idx = conversations.value.findIndex((c) => c.id === convId);
        if (idx !== -1 && title) {
          conversations.value[idx] = {
            ...conversations.value[idx],
            title,
          };
        }
      });

      await listen<ToolApprovalRequest>("aider:tool_approval_request", (event) => {
        const req = event.payload;
        if (!req || !req.requestId) return;
        if (!pendingToolApprovals.value.some((item) => item.requestId === req.requestId)) {
          pendingToolApprovals.value = [...pendingToolApprovals.value, req];
        }
      });

      await listen<[string, boolean]>("aider:tool_approval_resolved", (event) => {
        const [requestId] = event.payload || [];
        if (!requestId) return;
        pendingToolApprovals.value = pendingToolApprovals.value.filter(
          (item) => item.requestId !== requestId
        );
      });

      await Promise.all([refreshTelemetry(), refreshStackStatus()]);

      telemetryTimer = window.setInterval(() => {
        if (hudShowLocalResourcesEffective.value) {
          void refreshTelemetry();
        }
      }, 3000);
    } else {
      // Browser preview fallback when inspecting via `npm run dev`
      applyStateData({
        settings: { ...defaultSettings },
        projects: [],
        conversations: [],
        activeProjectId: null,
        activeConversationId: null,
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
    clearPendingLiveBuffers();
    if (telemetryTimer !== null) {
      clearInterval(telemetryTimer);
      telemetryTimer = null;
    }
    if (typeof window !== "undefined") {
      window.removeEventListener("resize", handleWindowResize);
    }
  }

  let refreshingTelemetry = false;
  async function refreshTelemetry() {
    if (!isTauri() || refreshingTelemetry || !hudShowLocalResourcesEffective.value) return;
    refreshingTelemetry = true;
    try {
      telemetry.value = await invoke<HardwareTelemetry>("get_hardware_telemetry");
    } catch {
      // Ignore transient telemetry errors
    } finally {
      refreshingTelemetry = false;
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

  async function testOllamaConnection(url?: string): Promise<StackStatus> {
    if (!isTauri()) return stackStatus.value;
    const status = await invoke<StackStatus>("check_stack_status", {
      url: url || settings.value.ollamaUrl,
    });
    stackStatus.value = status;
    return status;
  }

  async function startOllama() {
    if (!isTauri()) return;
    stackStatus.value = await invoke<StackStatus>("start_ollama_service");
  }

  async function saveSettings(next: AppSettings) {
    settings.value = { ...next };
    applyThemeToDom();
    if (isTauri()) {
      const saved = await invoke<AppSettings>("save_settings", { settings: next });
      settings.value = saved;
      await refreshStackStatus();
    }
  }

  async function validateProviderKey(
    providerId: string,
    apiKey: string,
    baseUrl?: string | null
  ): Promise<ProviderConfig> {
    if (isTauri()) {
      const updated = await invoke<ProviderConfig>("validate_provider_key", {
        providerId,
        apiKey,
        baseUrl: baseUrl || null,
      });
      const currentProviders = [...(settings.value.providers ?? defaultProvidersList())];
      const idx = currentProviders.findIndex((p) => p.id === updated.id);
      if (idx !== -1) {
        currentProviders[idx] = updated;
      } else {
        currentProviders.push(updated);
      }
      settings.value = {
        ...settings.value,
        providers: currentProviders,
      };
      return updated;
    }
    if (!apiKey.trim()) {
      throw new Error("Clé API vide.");
    }
    const currentProviders = [...(settings.value.providers ?? defaultProvidersList())];
    const idx = currentProviders.findIndex((p) => p.id === providerId);
    const existing = idx !== -1 ? currentProviders[idx] : defaultProvidersList()[0];
    const simulated: ProviderConfig = {
      ...existing,
      id: providerId,
      apiKey: apiKey.trim(),
      baseUrl: baseUrl || null,
      validated: true,
    };
    if (idx !== -1) currentProviders[idx] = simulated;
    settings.value = { ...settings.value, providers: currentProviders };
    return simulated;
  }

  async function resetApiUsage() {
    if (isTauri()) {
      apiUsage.value = await invoke<ApiUsageState>("reset_api_usage");
    } else {
      apiUsage.value = {
        date: new Date().toISOString().slice(0, 10),
        byProvider: {},
      };
    }
  }

  async function setActiveTarget(target: string) {
    const clean = target.trim() || "agent";
    const next: AppSettings = {
      ...settings.value,
      activeTarget: clean,
    };
    if (clean.startsWith("ollama:")) {
      const mdl = clean.slice("ollama:".length).trim();
      if (mdl) {
        next.ollamaModel = mdl;
      }
    }
    await saveSettings(next);
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
    globalContext: string,
    initRules: boolean = true,
    initialPrompt: string = ""
  ) {
    if (isTauri()) {
      const next = await invoke<AppStateData>("create_project", {
        name,
        rootPath,
        globalContext,
        initRules,
        initialPrompt,
      });
      applyStateData(next);
    } else {
      const now = new Date().toISOString();
      const projectId = `preview-proj-${Date.now()}`;
      const convId = `preview-conv-${Date.now()}`;
      const finalGlobalContext = globalContext.trim()
        ? globalContext
        : initialPrompt;
      projects.value = [
        {
          id: projectId,
          name: name.trim(),
          rootPath: rootPath.trim(),
          globalContext: finalGlobalContext,
          sharedDirectories: [],
          updatedAt: now,
        },
        ...projects.value,
      ];
      conversations.value = [
        {
          id: convId,
          projectId,
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
        ...conversations.value,
      ];
      activeProjectId.value = projectId;
      activeConversationId.value = convId;
    }
    newProjectModalOpen.value = false;
  }

  async function listProjectRules(projectId?: string): Promise<ProjectRuleItem[]> {
    const targetPid = projectId ?? activeProjectId.value;
    if (!isTauri() || !targetPid) return [];
    try {
      return await invoke<ProjectRuleItem[]>("list_project_rules", {
        projectId: targetPid,
      });
    } catch (e) {
      console.warn("Impossible de lister les règles projet:", e);
      return [];
    }
  }

  async function initProjectRules(
    projectId?: string,
    initialPrompt?: string
  ): Promise<string[]> {
    const targetPid = projectId ?? activeProjectId.value;
    if (!isTauri() || !targetPid) return [];
    try {
      return await invoke<string[]>("init_project_agent_rules", {
        projectId: targetPid,
        initialPrompt,
      });
    } catch (e) {
      console.warn("Impossible d'initialiser les règles projet:", e);
      return [];
    }
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
      activeDiffMessageId.value = null;
      applyStateData(next);
    }
  }

  async function selectConversation(projectId: string, conversationId: string) {
    activeProjectId.value = projectId;
    activeConversationId.value = conversationId;
    activeDiffFilePath.value = null;
    activeDiffMessageId.value = null;
    activeArtifactId.value = null;
    if (isTauri()) {
      await invoke("select_conversation", { projectId, conversationId });
    }
  }

  async function renameConversation(conversationId: string, title: string) {
    const clean = title.trim();
    if (!clean) return;
    const idx = conversations.value.findIndex((c) => c.id === conversationId);
    if (idx !== -1) {
      conversations.value[idx] = {
        ...conversations.value[idx],
        title: clean,
      };
    }
    if (isTauri()) {
      try {
        const updated = await invoke<Conversation>("rename_conversation", {
          conversationId,
          title: clean,
        });
        const uIdx = conversations.value.findIndex((c) => c.id === updated.id);
        if (uIdx !== -1) conversations.value[uIdx] = updated;
      } catch {
        // Ignore transient rename failure
      }
    }
  }

  async function cancelPrompt() {
    if (!isProcessing.value || isCancelling.value) return;
    isCancelling.value = true;
    const targetConvId =
      processingConversationId.value || activeConversationId.value || "";
    if (currentStage.value) {
      currentStage.value = {
        ...currentStage.value,
        detail: "Interruption du traitement en cours...",
      };
    }
    if (isTauri() && targetConvId) {
      try {
        await invoke<boolean>("cancel_prompt", { conversationId: targetConvId });
      } catch {
        // Ignore cancel invoke error
      }
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
        const nextProject = projects.value[0] ?? null;
        activeProjectId.value = nextProject?.id ?? null;
        activeConversationId.value = nextProject
          ? (conversations.value.find((c) => c.projectId === nextProject.id)?.id ?? null)
          : null;
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
    const targetPid = projectId ?? activeProjectId.value;
    if (!targetPid) return [];
    if (!isTauri()) {
      return ["README.md", "src/main.ts", "src/utils.ts"];
    }
    try {
      return await invoke<string[]>("list_project_files", {
        projectId: targetPid,
      });
    } catch {
      return [];
    }
  }

  const hasAiderignore = ref<boolean>(true);

  async function checkAiderignore(projectId?: string): Promise<boolean> {
    const targetPid = projectId ?? activeProjectId.value;
    if (!isTauri() || !targetPid) {
      hasAiderignore.value = true;
      return true;
    }
    try {
      const exists = await invoke<boolean>("check_project_aiderignore", {
        projectId: targetPid,
      });
      hasAiderignore.value = exists;
      return exists;
    } catch {
      hasAiderignore.value = true;
      return true;
    }
  }

  async function ensureAiderignore(projectId?: string): Promise<boolean> {
    const targetPid = projectId ?? activeProjectId.value;
    if (!targetPid) return false;
    if (!isTauri()) {
      hasAiderignore.value = true;
      return true;
    }
    try {
      const created = await invoke<boolean>("ensure_project_aiderignore", {
        projectId: targetPid,
      });
      hasAiderignore.value = true;
      return created;
    } catch {
      return false;
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

    const userPrompts = targetConv.messages
      .filter((m) => m.role === "user")
      .map((m) => `• ${m.content.slice(0, 100).replace(/\n/g, " ")}`);

    const summaryText =
      userPrompts.length > 0
        ? `[Session compactée (${userPrompts.length} échanges)]\n${userPrompts.slice(-5).join("\n")}`
        : "[Mémoire de session réinitialisée]";

    targetConv.conversationContext = summaryText;

    // Réinitialiser les compteurs de tokens pour le HUD
    delete conversationTokenSteps.value[cid];
    delete conversationTokenUsage.value[cid];
    delete conversationPeakTokens.value[cid];

    const now = new Date().toISOString();
    targetConv.messages.push({
      id: "compact-" + Date.now(),
      role: "assistant",
      content: `🧹 **Contexte compacté (/compact)** : La mémoire de travail LLM a été allégée avec succès. L'historique et les directives du projet restent intacts.`,
      stageSummary: "Nettoyage contexte (/compact)",
      createdAt: now,
      filesChanged: 0,
      linesAdded: 0,
      linesRemoved: 0,
    });
    targetConv.updatedAt = now;

    if (isTauri()) {
      try {
        await invoke("update_conversation_context", {
          conversationId: targetConv.id,
          conversationContext: targetConv.conversationContext,
        });
      } catch (e) {
        console.warn("Erreur sauvegarde compactage:", e);
      }
    }
  }

  async function clearConversation(conversationId?: string) {
    const cid = conversationId || activeConversationId.value;
    if (!cid) return;
    const convIdx = conversations.value.findIndex((c) => c.id === cid);
    if (convIdx === -1) return;

    const targetConv = conversations.value[convIdx];
    const now = new Date().toISOString();
    targetConv.messages = [];
    targetConv.conversationContext = "";
    delete conversationTokenSteps.value[cid];
    delete conversationTokenUsage.value[cid];
    delete conversationPeakTokens.value[cid];
    if (activeConversationId.value === cid) {
      activeDiffFilePath.value = null;
      activeDiffMessageId.value = null;
    }
    targetConv.updatedAt = now;

    if (isTauri()) {
      try {
        await invoke("update_conversation_context", {
          conversationId: targetConv.id,
          conversationContext: "",
        });
      } catch (e) {
        console.warn("Erreur réinitialisation conversation:", e);
      }
    }
  }

  async function resolveToolApproval(requestId: string, approved: boolean) {
    pendingToolApprovals.value = pendingToolApprovals.value.filter(
      (item) => item.requestId !== requestId
    );
    if (isTauri()) {
      try {
        await invoke("resolve_tool_approval", { requestId, approved });
      } catch {
        // Ignore if request already expired or pipeline cancelled
      }
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

    // Force /compact automatically when rolling session memory approaches saturation
    if (contextPercent.value >= 90) {
      await compactConversation(conv.id);
    }

    isProcessing.value = true;
    isCancelling.value = false;
    processingConversationId.value = conv.id;
    pendingToolApprovals.value = pendingToolApprovals.value.filter(
      (item) => item.conversationId !== conv.id
    );
    const nextPeak = { ...conversationPeakTokens.value };
    delete nextPeak[conv.id];
    conversationPeakTokens.value = nextPeak;
    conversationTokenSteps.value = {
      ...conversationTokenSteps.value,
      [conv.id]: [],
    };
    clearPendingLiveBuffers();
    liveStreamLines.value = [];
    liveThinking.value = "";
    liveContent.value = "";
    liveAgentTrace.value = [];
    liveProcessSteps.value = [];
    currentStage.value = {
      conversationId: conv.id,
      stageIndex: 1,
      stageTotal: 5,
      stageLabel: "1. Architecte & Orchestrateur",
      detail: "Initialisation du pipeline multi-agents...",
      status: "running",
    };

    // Affichage optimiste immédiat du prompt utilisateur pour qu'il reste visible pendant la réflexion
    const tempUserMsgId = `optimistic-user-${Date.now()}`;
    const optimisticMsg: ChatMessage = {
      id: tempUserMsgId,
      role: "user",
      content: prompt.trim(),
      thinking: null,
      attachments: hasAttachments ? attachments : undefined,
      createdAt: new Date().toISOString(),
      stageSummary: null,
    };
    conv.messages = [...conv.messages, optimisticMsg];

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
      flushLiveBuffers();
      clearPendingLiveBuffers();
      isProcessing.value = false;
      isCancelling.value = false;
      processingConversationId.value = null;
      liveAgentTrace.value = [];
      liveProcessSteps.value = [];
      pendingToolApprovals.value = pendingToolApprovals.value.filter(
        (item) => item.conversationId !== conv.id
      );
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

  async function undoFile(
    filePath: string,
    conversationId?: string,
    messageId?: string | null
  ): Promise<boolean> {
    const cid = conversationId || activeConversationId.value;
    if (!cid) return false;
    if (!isTauri()) {
      // Mock for browser dev mode
      const idx = conversations.value.findIndex((c) => c.id === cid);
      if (idx !== -1) {
        if (messageId) {
          const msg = conversations.value[idx].messages.find((m) => m.id === messageId);
          if (msg) {
            msg.turnDiffs = (msg.turnDiffs ?? []).filter((f) => f.path !== filePath);
            msg.filesSummary = (msg.filesSummary ?? []).filter((f) => f.path !== filePath);
            msg.filesChanged = msg.turnDiffs.length;
            msg.linesAdded = msg.turnDiffs.reduce((sum, f) => sum + f.added + f.modified, 0);
            msg.linesRemoved = msg.turnDiffs.reduce((sum, f) => sum + f.removed + f.modified, 0);
          }
        } else {
          conversations.value[idx].modifiedFiles = conversations.value[idx].modifiedFiles.filter(
            (f) => f.path !== filePath
          );
          conversations.value[idx].linesAdded = conversations.value[idx].modifiedFiles.reduce(
            (sum, f) => sum + f.added + f.modified,
            0
          );
          conversations.value[idx].linesRemoved = conversations.value[idx].modifiedFiles.reduce(
            (sum, f) => sum + f.removed + f.modified,
            0
          );
        }
      }
      if (activeDiffFilePath.value === filePath) {
        const scopedMsg = activeDiffMessageId.value
          ? activeConversation.value?.messages.find((m) => m.id === activeDiffMessageId.value)
          : undefined;
        if (scopedMsg?.turnDiffs && scopedMsg.turnDiffs.length > 0) {
          activeDiffFilePath.value = scopedMsg.turnDiffs[0].path;
        } else {
          activeDiffMessageId.value = null;
          activeDiffFilePath.value = activeConversation.value?.modifiedFiles[0]?.path ?? null;
        }
      }
      return true;
    }

    try {
      const updated = await invoke<Conversation>("undo_file_modification", {
        conversationId: cid,
        filePath,
        messageId: messageId ?? null,
      });
      const idx = conversations.value.findIndex((c) => c.id === updated.id);
      if (idx !== -1) conversations.value[idx] = updated;
      if (activeDiffFilePath.value === filePath) {
        const scopedMsg = activeDiffMessageId.value
          ? updated.messages.find((m) => m.id === activeDiffMessageId.value)
          : undefined;
        if (scopedMsg?.turnDiffs && scopedMsg.turnDiffs.length > 0) {
          activeDiffFilePath.value = scopedMsg.turnDiffs[0].path;
        } else {
          activeDiffMessageId.value = null;
          activeDiffFilePath.value = updated.modifiedFiles[0]?.path ?? null;
        }
      }
      return true;
    } catch (err) {
      console.error("Erreur lors de l'annulation du fichier:", err);
      return false;
    }
  }

  async function undoAll(
    conversationId?: string,
    messageId?: string | null
  ): Promise<boolean> {
    const cid = conversationId || activeConversationId.value;
    if (!cid) return false;
    if (!isTauri()) {
      // Mock for browser dev mode
      const idx = conversations.value.findIndex((c) => c.id === cid);
      if (idx !== -1) {
        if (messageId) {
          const msg = conversations.value[idx].messages.find((m) => m.id === messageId);
          if (msg) {
            msg.turnDiffs = [];
            msg.filesSummary = [];
            msg.filesChanged = 0;
            msg.linesAdded = 0;
            msg.linesRemoved = 0;
          }
        } else {
          conversations.value[idx].modifiedFiles = [];
          conversations.value[idx].linesAdded = 0;
          conversations.value[idx].linesRemoved = 0;
        }
      }
      activeDiffFilePath.value = null;
      if (!messageId || messageId === activeDiffMessageId.value) {
        activeDiffMessageId.value = null;
      }
      return true;
    }

    try {
      const updated = await invoke<Conversation>("undo_all_modifications", {
        conversationId: cid,
        messageId: messageId ?? null,
      });
      const idx = conversations.value.findIndex((c) => c.id === updated.id);
      if (idx !== -1) conversations.value[idx] = updated;
      activeDiffFilePath.value = null;
      if (!messageId || messageId === activeDiffMessageId.value) {
        activeDiffMessageId.value = null;
      }
      return true;
    } catch (err) {
      console.error("Erreur lors de l'annulation complète:", err);
      return false;
    }
  }

  function openReview(
    filePath?: string,
    targetLine?: number | null,
    messageId?: string | null
  ) {
    codeMode.value = "review";
    activeDiffTargetLine.value =
      typeof targetLine === "number" && Number.isFinite(targetLine) && targetLine > 0
        ? targetLine
        : null;

    const scopedMsg = messageId
      ? activeConversation.value?.messages.find((m) => m.id === messageId)
      : undefined;
    const hasStepDiffs = Boolean(scopedMsg?.turnDiffs && scopedMsg.turnDiffs.length > 0);
    activeDiffMessageId.value = hasStepDiffs && messageId ? messageId : null;

    const sourceFiles = hasStepDiffs
      ? (scopedMsg?.turnDiffs ?? [])
      : (activeConversation.value?.modifiedFiles ?? []);

    if (filePath) {
      activeDiffFilePath.value = filePath;
      if (!hasStepDiffs) {
        const alreadyLoaded = sourceFiles.some((f) => f.path === filePath);
        if (!alreadyLoaded) {
          void refreshDiff();
        }
      }
    } else if (sourceFiles.length > 0) {
      const currentValid =
        activeDiffFilePath.value &&
        sourceFiles.some((f) => f.path === activeDiffFilePath.value);
      if (!currentValid) {
        activeDiffFilePath.value = sourceFiles[0].path;
      }
    } else {
      activeDiffFilePath.value = null;
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

  function scrollToMessage(msgId: string) {
    targetScrollMsgId.value = msgId;
    if (!isWideScreen.value) {
      codeOpen.value = false;
    }
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

  async function updateConversationContext(
    conversationId: string,
    conversationContext: string
  ) {
    const idx = conversations.value.findIndex((c) => c.id === conversationId);
    if (idx !== -1) {
      conversations.value[idx].conversationContext = conversationContext;
      if (isTauri()) {
        const updated = await invoke<Conversation>(
          "update_conversation_context",
          {
            conversationId,
            conversationContext,
          }
        );
        conversations.value[idx] = updated;
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
    activeDiffMessageId,
    activeDiffTargetLine,
    activeArtifactId,
    settingsOpen,
    settingsActiveTab,
    openSettingsTab,
    apiUsage,
    contextModalOpen,
    projectSettingsModalOpen,
    projectToEdit,
    openProjectSettings,
    updateProject,
    newProjectModalOpen,
    mcpModalOpen,
    mcpServers,
    isProcessing,
    isCancelling,
    processingConversationId,
    currentStage,
    liveStreamLines,
    liveThinking,
    liveContent,
    liveAgentTrace,
    liveProcessSteps,
    pendingToolApprovals,
    resolveToolApproval,
    listProjectFiles,
    executeTool,
    effectiveTheme,
    init,
    dispose,
    refreshTelemetry,
    refreshStackStatus,
    testOllamaConnection,
    startOllama,
    saveSettings,
    validateProviderKey,
    resetApiUsage,
    setActiveTarget,
    setTheme,
    pickProjectDirectory,
    createProject,
    listProjectRules,
    initProjectRules,
    deleteProject,
    updateProjectContext,
    createConversation,
    selectConversation,
    renameConversation,
    cancelPrompt,
    deleteConversation,
    deleteConvModalOpen,
    convToDelete,
    promptDeleteConversation,
    confirmDeleteConversation,
    sendPrompt,
    hasAiderignore,
    checkAiderignore,
    ensureAiderignore,
    refreshDiff,
    undoFile,
    undoAll,
    openReview,
    openArtifact,
    scrollToMessage,
    targetScrollMsgId,
    closeCode,
    saveMcpServer,
    deleteMcpServer,
    testMcpServer,
    executeMcpTool,
    clearGlobalContext,
    clearConversationContext,
    updateConversationContext,
    conversationTokenUsage,
    lastCallTokens,
    peakCallTokens,
    turnTokenSteps,
    turnTotalTokens,
    localPeakTokens,
    localContextPercent,
    validatedExternalProviders,
    externalApiTokensTotal,
    externalApiTokenLimit,
    externalApiPercent,
    activeTargetLabel,
    displayedCallTokenEvent,
    activeCallTokens,
    contextTokensMax,
    contextTokensUsed,
    contextPercent,
    activeContextPercent,
    compactConversation,
    clearConversation,
    hudShowTokenUsageEffective,
    hudShowLocalResourcesEffective,
    hudShowGlobalDiffEffective,
    hudContextViewEffective,
    hudAutoArtifactEvolutionReportEffective,
  };
});

if (import.meta.hot) {
  import.meta.hot.accept(acceptHMRUpdate(useStudioStore, import.meta.hot));
}
