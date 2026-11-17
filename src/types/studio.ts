export type ThemePreference = "system" | "dark" | "light";
export type CodeDrawerMode = "review" | "artifact";

export interface AppSettings {
  theme: ThemePreference;
  ollamaUrl: string;
  ollamaModel: string;
  aiderPath: string;
  autoStartOllama: boolean;
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

export interface PromptStageEvent {
  conversationId: string;
  stageIndex: number;
  stageTotal: number;
  stageLabel: string;
  detail: string;
  status: "running" | "done" | "error";
}

export interface StackStatus {
  aiderInstalled: boolean;
  aiderBin: string | null;
  ollamaInstalled: boolean;
  ollamaRunning: boolean;
  modelAvailable: boolean;
  availableModels: string[];
}

export interface SendPromptResult {
  conversation: Conversation;
  project: Project;
}
