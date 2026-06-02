export interface Agent {
  id: string;
  name: string;
  initial: string;
  color: string;
  tagline: string;
  online: boolean;
  model: string;
  avatar?: string;
}

export interface ChatMessageToolCall {
  id: string;
  name: string;
  status: "running" | "success" | "error";
  args?: string;
  result?: string;
  duration?: number;
}

export interface ChatMessageImage {
  url: string;
  alt?: string;
}

export interface QueuedMention {
  type: "file" | "dir" | "skill";
  id: string;
  label: string;
}

export interface QueuedMessage {
  id: string;
  content: string;
  mentions: QueuedMention[];
  images: ChatMessageImage[];
  createdAt: Date;
  status: "pending" | "sending" | "failed";
  error?: string;
}

export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
  id: number;
  timestamp: Date;
  chatId: string;
  toolCalls?: ChatMessageToolCall[];
  images?: ChatMessageImage[];
  usage?: ChatUsage;
}

export interface SubAgentToolCall {
  id: string;
  name: string;
  status: "running" | "success" | "error";
  args?: string;
  result?: string;
}

export interface SubAgentRunUI {
  runId: string;
  agentId: string;
  subagentType: string;
  task: string;
  depth: number;
  status: "pending" | "running" | "completed" | "failed" | "cancelled";
  content: string;
  toolCalls: SubAgentToolCall[];
  result?: string;
  toolCallsMade: number;
  iterations: number;
  elapsedMs?: number;
}

export interface ChatStreamSegment {
  id: string;
  type: "text" | "tool";
  content?: string;
  toolCall?: ChatMessageToolCall;
}

export type StreamItem = { type: "message"; data: ChatMessage };

export interface ChatUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  elapsedMs: number;
  contextTokens?: number;
  contextWindow?: number;
}

export type ExecutionMode = "agent" | "plan";

export interface Chat {
  id: string;
  localKey: string;
  title: string;
  workDir: string | null;
  source: string;
  stream: StreamItem[];
  createdAt: Date;
  messageCount: number;
  open: boolean;
  usage?: ChatUsage;
  subAgentRuns: Record<string, SubAgentRunUI>;
  executionMode: ExecutionMode;
  planFilePath?: string;
  planFileExists?: boolean;
  lastSegments?: ChatStreamSegment[];
}

export interface AgentChats {
  chatList: Chat[];
  activeChatId: string;
  unread: number;
  lastMsg: string | null;
  lastTime: string | null;
  messageQueue: QueuedMessage[];
}

import type { NavItem } from "./ui-store";

export interface AgentState {
  agents: Agent[];
  activeAgentId: string;
  agentChats: Record<string, AgentChats>;
  detailOpen: boolean;
  sidebarCollapsed: boolean;
  activeNav: NavItem;

  setActiveAgent: (id: string) => void;
  toggleDetail: () => void;
  closeDetail: () => void;
  toggleSidebar: () => void;
  setActiveNav: (nav: NavItem) => void;
  addMessage: (agentId: string, msg: Omit<ChatMessage, "id" | "chatId">, targetChatId?: string) => void;
  newChat: (agentId: string, workDir?: string) => void;
  setActiveChat: (agentId: string, chatId: string) => void;
  closeChat: (agentId: string, chatId: string) => void;
  reopenChat: (agentId: string, chatId: string) => void;
  setWorkDir: (agentId: string, chatId: string, workDir: string | null) => void;
  renameChat: (agentId: string, chatId: string, title: string) => void;
  reorderChats: (agentId: string, fromIdx: number, toIdx: number) => void;
  clearUnread: (agentId: string) => void;
  updateAgentProps: (agentId: string, props: Partial<Pick<Agent, "name" | "model" | "avatar">>) => void;
  syncAgentsFromBackend: (backendAgents: Array<{ agentId: string; name: string; model: string; avatar?: string | null }>) => void;
  syncSessionsForAgent: (agentId: string, sessions: BackendSession[]) => void;
  loadChatStream: (agentId: string, chatId: string, messages: BackendMessage[]) => void;
  updateChatBackendId: (agentId: string, localChatId: string, backendSessionId: string) => void;
  appendStreamDelta: (agentId: string, chatId: string, delta: string) => void;
  updateChatUsage: (agentId: string, chatId: string, usage: ChatUsage) => void;
  removeAgent: (agentId: string) => void;

  enqueueMessage: (agentId: string, chatId: string, message: Omit<QueuedMessage, "id">) => void;
  dequeueMessage: (agentId: string, chatId: string) => QueuedMessage | undefined;
  updateQueuedMessage: (agentId: string, chatId: string, messageId: string, updates: Partial<QueuedMessage>) => void;
  removeQueuedMessage: (agentId: string, chatId: string, messageId: string) => void;
  clearQueue: (agentId: string, chatId: string) => void;
  reorderQueue: (agentId: string, chatId: string, fromIndex: number, toIndex: number) => void;

  setChatExecutionMode: (agentId: string, chatId: string, mode: ExecutionMode) => void;
  setChatPlanFile: (agentId: string, chatId: string, path: string, exists: boolean) => void;
  setChatLastSegments: (agentId: string, chatId: string, segments: ChatStreamSegment[]) => void;

  subAgentStart: (agentId: string, chatId: string, run: SubAgentRunUI) => void;
  subAgentDelta: (agentId: string, chatId: string, runId: string, content: string) => void;
  subAgentToolStart: (agentId: string, chatId: string, runId: string, toolCall: SubAgentToolCall) => void;
  subAgentToolDone: (agentId: string, chatId: string, runId: string, callId: string, output: string, success: boolean) => void;
  subAgentComplete: (agentId: string, chatId: string, runId: string, status: string, result?: string, toolCallsMade?: number, iterations?: number, elapsedMs?: number) => void;
}

export interface BackendSession {
  id: string;
  agentId: string;
  title: string | null;
  workDir?: string | null;
  source?: string;
  messageCount: number;
  createdAt: string;
  updatedAt: string;
  totalPromptTokens?: number;
  totalCompletionTokens?: number;
  totalElapsedMs?: number;
}

export interface BackendMessage {
  id: number;
  role: string;
  content: unknown;
  name: string | null;
  toolCallId: string | null;
  toolCallsJson?: Array<{ id: string; type: string; function: { name: string; arguments: string } }> | null;
  createdAt: string;
  promptTokens?: number;
  completionTokens?: number;
  totalTokens?: number;
  elapsedMs?: number;
}
