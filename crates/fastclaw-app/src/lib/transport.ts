/**
 * Transport abstraction layer — unified WebSocket-only architecture.
 *
 * In the new architecture:
 * - All business logic (chat, sessions, agents, etc.) goes through WebSocket
 * - Only local file operations use Tauri IPC (upload, export, etc.)
 * - Tauri app connects to a Gateway daemon process via WebSocket
 */

import * as wsClient from "./ws-client";

export const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

let _invoke: typeof import("@tauri-apps/api/core").invoke | null = null;

async function ensureTauriApi() {
  if (!_invoke) {
    const core = await import("@tauri-apps/api/core");
    _invoke = core.invoke;
  }
}

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  await ensureTauriApi();
  return _invoke!<T>(cmd, args);
}

// ─── Gateway Info (IPC) ───

export interface GatewayInfo {
  port: number;
  wsUrl: string;
  httpUrl: string;
  version: string;
}

export async function getGatewayInfo(): Promise<GatewayInfo | null> {
  if (!isTauri) return null;
  try {
    return await tauriInvoke<GatewayInfo>("get_gateway_info");
  } catch {
    return null;
  }
}

// ─── Local File Operations (IPC only) ───

export async function uploadAgentAvatar(
  agentId: string,
  sourcePath: string,
): Promise<{ ok: boolean; path?: string }> {
  if (!isTauri) return { ok: false };
  return tauriInvoke("upload_agent_avatar", { agentId, sourcePath });
}

export async function readIdentityFiles(
  agentId: string,
): Promise<{ soul: string | null; user: string | null; agents: string | null; tools: string | null }> {
  if (!isTauri) return { soul: null, user: null, agents: null, tools: null };
  return tauriInvoke("read_identity_files", { agentId });
}

export async function uploadSkill(
  sourcePath: string,
): Promise<{ installed?: string }> {
  if (!isTauri) return {};
  return tauriInvoke("upload_skill", { sourcePath });
}

export interface ExportOptions {
  includeSessions: boolean;
  includeSkills: boolean;
  includeAgentWorkspaces: boolean;
}

export interface ImportOptions {
  merge: boolean;
  overwriteConfig: boolean;
  overwriteAgents: boolean;
  overwriteSessions: boolean;
  overwriteSkills: boolean;
}

export async function exportData(options: ExportOptions): Promise<Uint8Array> {
  if (!isTauri) throw new Error("export only available in desktop mode");
  const result = await tauriInvoke<number[]>("export_data", { options });
  return new Uint8Array(result);
}

export async function importData(data: Uint8Array, options: ImportOptions): Promise<void> {
  if (!isTauri) throw new Error("import only available in desktop mode");
  await tauriInvoke("import_data", {
    data: Array.from(data),
    options,
  });
}

// ─── Session Export (IPC only) ───
// Frontend fetches session content via WebSocket, then passes to IPC for local file save.

export type ExportFormat = "markdown" | "json";

export interface SessionExportResult {
  success: boolean;
  path?: string;
  filename: string;
  content: string;
  mimeType: string;
}

export async function exportSessionContent(
  sessionId: string,
  format: ExportFormat,
): Promise<SessionExportResult> {
  const resp = (await wsClient.send("sessions.export", { sessionId, format })) as {
    data?: { filename?: string; content?: string; mimeType?: string };
  };
  const filename = resp?.data?.filename ?? `session.${format === "json" ? "json" : "md"}`;
  const content = resp?.data?.content ?? "";
  const mimeType = resp?.data?.mimeType ?? (format === "json" ? "application/json" : "text/markdown");
  return { success: true, filename, content, mimeType };
}

export async function saveSessionFile(
  content: string,
  filename: string,
  mimeType: string,
): Promise<{ success: boolean; path?: string }> {
  if (!isTauri) throw new Error("save only available in desktop mode");
  return tauriInvoke("export_session_content", { content, filename, mimeType });
}

// ─── WebSocket Connection ───

export function connectWs(url: string, token?: string): Promise<void> {
  return wsClient.connect(url, token).then(() => {
    wsClient.send("subscribe", {
      events: [
        "sessions.changed",
        "channels.changed",
        "cron.job.complete",
        "cron.job.failed",
        "notification.new",
        "notification.read",
      ],
    }).catch(() => {});
  });
}

export function disconnectWs(): void {
  wsClient.disconnect();
}

export function isWsConnected(): boolean {
  return wsClient.isConnected();
}

// ─── WebSocket Operations (all business logic) ───

export interface AgentSummary {
  agentId: string;
  name: string;
  model: string;
  avatar?: string | null;
}

export async function listAgents(): Promise<AgentSummary[]> {
  const resp = (await wsClient.send("agents")) as {
    data?: { agents?: AgentSummary[] };
  };
  return resp?.data?.agents ?? [];
}

export interface SessionSummary {
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

export async function listSessions(limit = 50, offset = 0): Promise<SessionSummary[]> {
  const resp = (await wsClient.send("sessions.list", { limit, offset })) as {
    data?: { sessions?: SessionSummary[] };
  };
  return resp?.data?.sessions ?? [];
}

export async function getSession(sessionId: string): Promise<SessionSummary | null> {
  const resp = (await wsClient.send("sessions.get", { sessionId })) as {
    data?: SessionSummary;
  };
  return resp?.data ?? null;
}

export interface SessionMessage {
  id: number;
  role: string;
  content: unknown;
  name: string | null;
  toolCallId: string | null;
  createdAt: string;
}

export async function getSessionMessages(sessionId: string): Promise<SessionMessage[]> {
  const resp = (await wsClient.send("sessions.messages", { sessionId })) as {
    data?: { messages?: SessionMessage[] };
  };
  return resp?.data?.messages ?? [];
}

export async function createSession(agentId?: string): Promise<string> {
  const resp = (await wsClient.send("sessions.new", agentId ? { agentId } : {})) as {
    data?: { sessionId?: string };
  };
  return resp?.data?.sessionId ?? "";
}

export async function updateSessionTitle(sessionId: string, title: string): Promise<void> {
  await wsClient.send("sessions.update_title", { sessionId, title });
}

export async function deleteSession(sessionId: string): Promise<void> {
  await wsClient.send("sessions.delete", { sessionId });
}

export async function setSessionWorkDir(sessionId: string, workDir: string | null): Promise<void> {
  await wsClient.send("sessions.set_work_dir", { sessionId, workDir });
}

export interface ModelInfo {
  agentId: string;
  model: string;
  provider: string;
  contextWindow: number;
  costPer1kInput: number;
  costPer1kOutput: number;
  supportsReasoning: boolean;
  capabilities?: import("./model-registry").ModelCapabilities;
}

export async function listModels(): Promise<ModelInfo[]> {
  const resp = (await wsClient.send("models.list")) as {
    data?: { models?: ModelInfo[] };
  };
  return resp?.data?.models ?? [];
}

export interface SkillInfo {
  id: string;
  name: string;
  description: string | null;
  tags?: string[];
}

export async function listSkills(agentId?: string): Promise<SkillInfo[]> {
  const resp = (await wsClient.send("skills.list", agentId ? { agentId } : {})) as {
    data?: { skills?: SkillInfo[] };
  };
  return resp?.data?.skills ?? [];
}

export async function refreshSkills(): Promise<{ refreshed: boolean; count: number }> {
  const resp = (await wsClient.send("skills.refresh")) as {
    data?: { refreshed?: boolean; count?: number };
  };
  return { refreshed: resp?.data?.refreshed ?? false, count: resp?.data?.count ?? 0 };
}

export async function getAgent(agentId: string): Promise<unknown> {
  const resp = (await wsClient.send("agents.get", { agentId })) as {
    data?: unknown;
  };
  return resp?.data ?? null;
}

export async function createAgent(config: Record<string, unknown>): Promise<{ ok: boolean; agentId?: string }> {
  const resp = (await wsClient.send("agents.create", { config })) as {
    data?: { ok?: boolean; agentId?: string };
  };
  return { ok: resp?.data?.ok ?? false, agentId: resp?.data?.agentId };
}

export async function updateAgent(agentId: string, config: Record<string, unknown>): Promise<boolean> {
  const resp = (await wsClient.send("agents.update", { agentId, config })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function deleteAgent(agentId: string): Promise<boolean> {
  const resp = (await wsClient.send("agents.delete", { agentId })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function getConfig(key?: string): Promise<unknown> {
  const resp = (await wsClient.send("config.get", key ? { key } : {})) as {
    data?: unknown;
  };
  return resp?.data;
}

export async function setConfig(
  key: string,
  value: unknown,
): Promise<{ persisted: boolean; pendingRestart: boolean }> {
  const resp = (await wsClient.send("config.set", { key, value })) as {
    data?: { persisted?: boolean; pendingRestart?: boolean };
  };
  return {
    persisted: resp?.data?.persisted ?? false,
    pendingRestart: resp?.data?.pendingRestart ?? false,
  };
}

// ─── Chat Streaming (WebSocket) ───
// Protocol types are generated from Rust: see fastclaw-protocol/generated/protocol.ts
export type { AgentEvent, TurnSummary, TokenUsage, ToolCallData, ClientOp, AbortReason, ErrorCode, WarningCategory, TurnContextItem } from "../../../fastclaw-protocol/generated/protocol";

export interface ChatStreamEvent {
  type: string;
  data?: Record<string, unknown>;
  error?: { message: string };
}

type ChatEventHandler = (event: ChatStreamEvent) => void;

export interface ChatStreamParams {
  messages: Array<{ role: string; content: string | unknown[] }>;
  agentId?: string;
  sessionId?: string;
  model?: string;
  temperature?: number;
  maxTokens?: number;
  workDir?: string;
}

const CHAT_EVENT_TYPES = [
  "turn_start",
  "content_delta",
  "reasoning_delta",
  "turn_end",
  "turn_aborted",
  "tool_executing",
  "tool_result",
  "tool_progress",
  "ask_question",
  "brief_message",
  "suggestions",
  "context_warning",
  "context_usage_update",
  "compact_boundary",
  "mode_change",
  "plan_file_update",
  "sub_agent_start",
  "sub_agent_delta",
  "sub_agent_tool_executing",
  "sub_agent_tool_result",
  "sub_agent_complete",
  "approval_required",
  "approval_resolved",
  "error",
  "stream_error",
  "warning",
  "memory_stored",
  "memory_recalled",
] as const;

export function chatStream(
  params: ChatStreamParams,
  onEvent: ChatEventHandler,
): { promise: Promise<void>; cleanup: () => void } {
  const handlers: Array<(() => void) | undefined> = [];
  let done = false;

  for (const type of CHAT_EVENT_TYPES) {
    const unsub = wsClient.on(type, (m: unknown) => {
      if (!done) onEvent(m as ChatStreamEvent);
    });
    handlers.push(unsub);
  }

  const cleanup = () => {
    done = true;
    handlers.forEach((h) => {
      if (typeof h === "function") h();
    });
  };

  const promise = wsClient
    .send("chat", {
      messages: params.messages,
      agentId: params.agentId,
      sessionId: params.sessionId,
      stream: true,
      ...(params.workDir ? { workDir: params.workDir } : {}),
    })
    .then(() => {})
    .catch(() => {
      cleanup();
    });

  return { promise, cleanup };
}

// ─── Event Subscriptions ───

export type UnsubscribeFn = () => void;

export function onSessionChanged(handler: (sessionId: string) => void): UnsubscribeFn {
  return wsClient.on("sessions.changed", (msg: unknown) => {
    const sid = (msg as { data?: { sessionId?: string } })?.data?.sessionId;
    if (sid) handler(sid);
  });
}

export function onChannelsChanged(handler: (channelId: string, action: string) => void): UnsubscribeFn {
  return wsClient.on("channels.changed", (msg: unknown) => {
    const data = (msg as { data?: { channelId?: string; action?: string } })?.data;
    if (data?.channelId) handler(data.channelId, data.action ?? "updated");
  });
}

export function onWsEvent(event: string, handler: (data: unknown) => void): UnsubscribeFn {
  return wsClient.on(event, handler);
}

// ─── MCP Server Management ───

export interface McpServerStatus {
  id: string;
  status: "connecting" | "connected" | "failed" | "disabled";
  error?: string | null;
  toolCount: number;
  connectedAt?: string | null;
}

export async function getMcpStatus(): Promise<McpServerStatus[]> {
  const resp = await wsClient.send("mcp.status");
  return (resp as { servers?: McpServerStatus[] }).servers ?? [];
}

export async function reloadMcpServers(): Promise<McpServerStatus[]> {
  const resp = await wsClient.send("mcp.reload");
  return (resp as { servers?: McpServerStatus[] }).servers ?? [];
}

export async function addMcpServer(
  id: string,
  command: string,
  args?: string[],
): Promise<{ ok: boolean; id: string; status?: McpServerStatus }> {
  return wsClient.send("mcp.add", { id, command, args: args ?? [] }) as Promise<{
    ok: boolean;
    id: string;
    status?: McpServerStatus;
  }>;
}

export async function removeMcpServer(id: string): Promise<{ ok: boolean; id: string }> {
  return wsClient.send("mcp.remove", { id }) as Promise<{
    ok: boolean;
    id: string;
  }>;
}

// ─── Tools ───

export interface AgentToolInfo {
id: string;
  enabled: boolean;
  description?: string;
}

export async function listAgentTools(agentId: string): Promise<AgentToolInfo[]> {
  const resp = (await wsClient.send("tools.list", { agentId })) as {
    data?: { tools?: AgentToolInfo[] };
  };
  return resp?.data?.tools ?? [];
}

export async function updateAgentTools(
  agentId: string,
  tools: Array<{ id: string; enabled: boolean }>,
): Promise<boolean> {
  const resp = (await wsClient.send("tools.update", { agentId, tools })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

// ─── Execution Mode ───

export async function setExecutionMode(
  mode: "agent" | "plan",
  sessionId?: string,
): Promise<{ ok: boolean; from: string; to: string }> {
  const resp = (await wsClient.send("execution.set_mode", { mode, sessionId })) as {
    data?: { ok?: boolean; from?: string; to?: string };
  };
  return {
    ok: resp?.data?.ok ?? false,
    from: resp?.data?.from ?? "",
    to: resp?.data?.to ?? "",
  };
}

export async function getPlanFile(sessionId?: string): Promise<{ path: string; content: string | null; exists: boolean }> {
  const resp = (await wsClient.send("execution.get_plan", { sessionId })) as {
    data?: { path?: string; content?: string | null; exists?: boolean };
  };
  return {
    path: resp?.data?.path ?? "",
    content: resp?.data?.content ?? null,
    exists: resp?.data?.exists ?? false,
  };
}

export async function submitToolAnswer(requestId: string, answer: string): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("tools.submit_answer", { requestId, answer })) as {
    data?: { ok?: boolean };
  };
  return { ok: resp?.data?.ok ?? false };
}

export async function resolveApproval(
  approvalId: string,
  decision: string,
): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("resolve_approval", {
    approvalId,
    decision: { decision },
  })) as { data?: { resolved?: boolean } };
  return { ok: resp?.data?.resolved ?? false };
}

// ─── Tools (raw IPC) ───

export async function listToolsIpc(): Promise<Array<{ type?: string; function?: { name?: string; description?: string } }>> {
  const resp = (await wsClient.send("tools.raw_list")) as {
    data?: { tools?: Array<{ type?: string; function?: { name?: string; description?: string } }> };
  };
  return resp?.data?.tools ?? [];
}

// ─── Cron Jobs ───

export interface CronJobAction {
  type: "agent_chat" | "webhook";
  agent_id?: string;
  message?: string;
  url?: string;
  method?: string;
  headers?: Record<string, string>;
  body?: string;
}

export interface NotifyChannel {
  channel_id: string;
  target_id: string;
  target_type: "p2p" | "group";
}

export interface CronJob {
  id: string;
  name: string;
  schedule: string;
  enabled: boolean;
  agentId?: string;
  action: CronJobAction;
  status?: string;
  run_count?: number;
  error_count?: number;
  notify_channels?: NotifyChannel[];
  last_run?: string | null;
  next_run?: string | null;
  last_error?: string | null;
  created_at?: string;
  createdAt?: string;
  updatedAt?: string;
}

export interface CronJobRun {
  id: string;
  jobId: string;
  status: "ok" | "running" | "completed" | "failed";
  started_at: string;
  ended_at?: string;
  output?: string;
  error?: string;
}

export async function cronListJobs(agentId?: string): Promise<{ jobs: CronJob[] }> {
  const resp = (await wsClient.send("cron.list_jobs", agentId ? { agentId } : {})) as {
    data?: { jobs?: CronJob[] };
  };
  return { jobs: resp?.data?.jobs ?? [] };
}

export async function cronGetJob(jobId: string): Promise<CronJob | null> {
  const resp = (await wsClient.send("cron.get_job", { jobId })) as {
    data?: CronJob;
  };
  return resp?.data ?? null;
}

export async function cronUpsertJob(job: Partial<CronJob> & { name: string; schedule: string; action: CronJobAction }): Promise<{ ok: boolean; jobId?: string }> {
  const resp = (await wsClient.send("cron.upsert_job", { job })) as {
    data?: { ok?: boolean; jobId?: string };
  };
  return { ok: resp?.data?.ok ?? false, jobId: resp?.data?.jobId };
}

export async function cronDeleteJob(jobId: string): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("cron.delete_job", { jobId })) as {
    data?: { ok?: boolean };
  };
  return { ok: resp?.data?.ok ?? false };
}

export async function cronListRuns(jobId: string, limit?: number): Promise<{ runs: CronJobRun[] }> {
  const resp = (await wsClient.send("cron.list_runs", { jobId, limit: limit ?? 20 })) as {
    data?: { runs?: CronJobRun[] };
  };
  return { runs: resp?.data?.runs ?? [] };
}

// ─── Notifications ───

export interface AppNotification {
  id: string;
  type: "info" | "warning" | "error" | "success";
  category?: string;
  title: string;
  message: string;
  body?: string;
  detail?: string;
  isRead: boolean;
  createdAt: string;
  metadata?: Record<string, unknown>;
}

export async function notificationUnreadCount(): Promise<{ count: number }> {
  const resp = (await wsClient.send("notifications.unread_count")) as {
    data?: { count?: number };
  };
  return { count: resp?.data?.count ?? 0 };
}

export async function notificationList(limit = 50): Promise<{ notifications: AppNotification[]; unreadCount: number }> {
  const resp = (await wsClient.send("notifications.list", { limit })) as {
    data?: { notifications?: AppNotification[]; unreadCount?: number };
  };
  return {
    notifications: resp?.data?.notifications ?? [],
    unreadCount: resp?.data?.unreadCount ?? 0,
  };
}

export async function notificationMarkRead(notificationId: string): Promise<{ unreadCount: number }> {
  const resp = (await wsClient.send("notifications.mark_read", { notificationId })) as {
    data?: { unreadCount?: number };
  };
  return { unreadCount: resp?.data?.unreadCount ?? 0 };
}

export async function notificationMarkAllRead(): Promise<{ unreadCount: number }> {
  const resp = (await wsClient.send("notifications.mark_all_read")) as {
    data?: { unreadCount?: number };
  };
  return { unreadCount: resp?.data?.unreadCount ?? 0 };
}

export async function notificationDelete(notificationId: string): Promise<void> {
  await wsClient.send("notifications.delete", { notificationId });
}

// ─── Model Connection Test ───

export async function testModelConnection(
  url: string,
  apiKey: string,
  model?: string,
): Promise<{ ok: boolean; error?: string }> {
  if (!isTauri) throw new Error("testModelConnection only available in desktop mode");
  return tauriInvoke("test_model_connection", { url, apiKey, model });
}

// ─── Backward Compatibility Aliases ───
// These are kept for gradual migration of the frontend code.

export const listSkillsIpc = listSkills;
export const uploadSkillIpc = uploadSkill;
export const listAgentToolsIpc = listAgentTools;
export const updateAgentToolsIpc = updateAgentTools;
export const refreshSkillsIpc = refreshSkills;
export const submitToolAnswerIpc = submitToolAnswer;
export const setExecutionModeIpc = setExecutionMode;
export const getPlanFileIpc = getPlanFile;
export const getAgentIpc = getAgent;
export const updateAgentIpc = async (agentId: string, config: Record<string, unknown>) => updateAgent(agentId, config);
export const createAgentIpc = createAgent;
export const deleteAgentIpc = async (agentId: string) => deleteAgent(agentId);
export const uploadAgentAvatarIpc = uploadAgentAvatar;
export const readIdentityFilesIpc = readIdentityFiles;
export const reloadChannelIpc = async (_channelId: string) => false; // deprecated