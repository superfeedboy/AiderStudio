export type ThemePreference = "system" | "dark" | "light";
export type CodeDrawerMode = "review" | "artifact";
export type AiderEditFormat = "auto" | "whole" | "diff";
export type ProviderId =
  | "ollama"
  | "gemini"
  | "openai"
  | "anthropic"
  | "deepseek"
  | "openrouter"
  | "groq"
  | "cerebras"
  | "mistral";
export type ExternalProviderId = Exclude<ProviderId, "ollama">;
export type AgentRoleKey =
  | "architect"
  | "cartographer"
  | "designer"
  | "coder"
  | "refactoring"
  | "tester"
  | "synthesizer";

export interface ProviderConfig {
  id: ExternalProviderId | string;
  name: string;
  apiKey: string;
  baseUrl?: string | null;
  validated: boolean;
  availableModels: string[];
  dailyTokenLimit: number;
}

export interface AgentModelSlot {
  provider: ProviderId | string;
  model: string;
}

export interface AgentRoutingConfig {
  architect: AgentModelSlot;
  cartographer: AgentModelSlot;
  designer: AgentModelSlot;
  coder: AgentModelSlot;
  refactoring: AgentModelSlot;
  tester: AgentModelSlot;
  synthesizer: AgentModelSlot;
}

export interface ProviderUsageEntry {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  calls: number;
}

export interface ApiUsageState {
  date: string;
  byProvider: Record<string, ProviderUsageEntry>;
}

export interface AppSettings {
  theme: ThemePreference;
  ollamaUrl: string;
  ollamaModel: string;
  autoStartOllama: boolean;
  contextWindowTokens?: number;
  aiderEditFormat?: AiderEditFormat;
  activeTarget?: string;
  providers?: ProviderConfig[];
  agentRouting?: AgentRoutingConfig;
}

export interface Project {
  id: string;
  name: string;
  rootPath: string;
  globalContext: string;
  sharedDirectories?: string[];
  updatedAt: string;
}

export interface ArtifactItem {
  id: string;
  title: string;
  language: string;
  filePath: string | null;
  content: string;
  createdAt: string;
}

export interface DiffLine {
  kind: "mod" | "add" | "del" | "ctx";
  oldLine: number | null;
  newLine: number | null;
  content: string;
}

export interface ModifiedFileDiff {
  path: string;
  added: number;
  removed: number;
  modified: number;
  lines: DiffLine[];
}

export interface FileChangeSummary {
  path: string;
  added: number;
  removed: number;
  modified: number;
}

export interface AttachedFile {
  id: string;
  name: string;
  size: number;
  mimeType: string;
  isImage: boolean;
  dataUrl?: string;
  content?: string;
}

export interface ToolExecutionResult {
  tool: string;
  success: boolean;
  message: string;
  path?: string | null;
}

export interface AgentTraceStep {
  role: AgentRoleKey | string;
  roleLabel: string;
  provider: string;
  model: string;
  summary: string;
  status: "running" | "done" | "fallback" | "error";
  tokens?: number;
}

export interface AgentTraceEvent {
  conversationId: string;
  step: AgentTraceStep;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  thinking?: string | null;
  attachments?: AttachedFile[];
  createdAt: string;
  stageSummary: string | null;
  filesChanged?: number;
  linesAdded?: number;
  linesRemoved?: number;
  filesSummary?: FileChangeSummary[];
  agentTrace?: AgentTraceStep[];
  commitId?: string;
  commitSummary?: string;
}

export interface Conversation {
  id: string;
  projectId: string;
  title: string;
  conversationContext: string;
  messages: ChatMessage[];
  artifacts: ArtifactItem[];
  modifiedFiles: ModifiedFileDiff[];
  linesAdded: number;
  linesRemoved: number;
  createdAt: string;
  updatedAt: string;
}

export interface McpServerConfig {
  id: string;
  name: string;
  serverType?: "sqlite" | "postgres" | "custom" | string;
  command: string;
  args: string[];
  env?: Record<string, string>;
  enabled: boolean;
  projectId?: string | null;
}

export interface McpToolInfo {
  serverId: string;
  serverName: string;
  name: string;
  description: string;
  inputSchema: any;
}

export interface AppStateData {
  settings: AppSettings;
  projects: Project[];
  conversations: Conversation[];
  activeProjectId: string | null;
  activeConversationId: string | null;
  mcpServers?: McpServerConfig[];
  apiUsage?: ApiUsageState;
}

export interface HardwareTelemetry {
  cpuUsage: number;
  gpuUsage: number | null;
  memUsedGb: number;
  memTotalGb: number;
  memPercent: number;
  tempCelsius: number | null;
  platform: string;
}

export interface TokenUsageEvent {
  conversationId: string;
  subagent: string;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  role?: AgentRoleKey | string;
  provider?: string;
  model?: string;
  contextLimit?: number;
}

export interface PromptStageEvent {
  conversationId: string;
  stageIndex: number;
  stageTotal: number;
  stageLabel: string;
  detail: string;
  status: "running" | "done" | "error";
}

export interface ToolApprovalRequest {
  requestId: string;
  conversationId: string;
  tool: string;
  title: string;
  target: string;
  reason?: string | null;
}

export interface StackStatus {
  nativeEngineReady: boolean;
  ollamaInstalled: boolean;
  ollamaRunning: boolean;
  modelAvailable: boolean;
  availableModels: string[];
}

export interface SendPromptResult {
  conversation: Conversation;
  project: Project;
}

export interface ProjectRuleItem {
  name: string;
  relativePath: string;
  category: "root" | "frontend" | "backend" | "git" | "validation" | "custom" | string;
  description: string;
  content: string;
}
