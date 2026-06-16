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
        "projects.changed",
        "channels.changed",
        "git.status_changed",
        "cron.job.complete",
        "cron.job.failed",
        "notification.new",
        "notification.read",
        "permissions.changed",
        "automations.changed",
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
  projectId?: string | null;
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

export async function cancelSubAgentRun(runId: string): Promise<void> {
  await wsClient.send("subagents.cancel", { runId });
}

export interface SubAgentRunWs {
  runId: string;
  parentSessionId: string;
  agentId: string;
  subagentType: string;
  task: string;
  status: string;
  result?: string | null;
  elapsedMs?: number | null;
  toolCallsMade: number;
  iterations: number;
}

export async function listSubAgentRunsWs(sessionId?: string): Promise<SubAgentRunWs[]> {
  const resp = (await wsClient.send("sub_agents.runs", sessionId ? { sessionId } : {})) as {
    data?: { runs?: SubAgentRunWs[] };
  };
  return resp?.data?.runs ?? [];
}

export async function sendSteeringMessage(
  runId: string,
  message: string,
  priority?: "normal" | "high",
): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("subagent.steer", {
    run_id: runId,
    message,
    ...(priority && { priority }),
  })) as { data?: { ok?: boolean } };
  return { ok: resp?.data?.ok ?? false };
}

export async function setSessionWorkDir(sessionId: string, workDir: string | null): Promise<void> {
  await wsClient.send("sessions.set_work_dir", { sessionId, workDir });
}

// ─── Projects ───

export interface ProjectSummary {
  id: string;
  name: string;
  rootPath: string;
  color: string;
  pinned: boolean;
  archived: boolean;
  reachable: boolean;
  lastOpenedAt: string;
  sessionCount: number;
}

export async function listProjects(includeArchived = false): Promise<ProjectSummary[]> {
  const resp = (await wsClient.send("projects.list", { includeArchived })) as {
    data?: { projects?: ProjectSummary[] };
  };
  return resp?.data?.projects ?? [];
}

export async function createProject(rootPath: string, name?: string, color?: string): Promise<ProjectSummary | null> {
  const resp = (await wsClient.send("projects.create", { rootPath, name, color })) as {
    data?: ProjectSummary;
  };
  return resp?.data ?? null;
}

export async function updateProject(
  id: string,
  patch: { name?: string; color?: string; pinned?: boolean; archived?: boolean }
): Promise<void> {
  await wsClient.send("projects.update", { id, ...patch });
}

export async function deleteProject(id: string): Promise<void> {
  await wsClient.send("projects.delete", { id });
}

export async function detectProject(path: string): Promise<ProjectSummary | null> {
  const resp = (await wsClient.send("projects.detect", { path })) as {
    data?: { project?: ProjectSummary };
  };
  return resp?.data?.project ?? null;
}

export async function workspaceInit(workDir?: string): Promise<{
  alreadyExists: boolean;
  root: string;
  message: string;
  created?: string[];
}> {
  const resp = await wsClient.send("workspace.init", workDir ? { workDir } : {}) as { data?: Record<string, unknown> };
  return (resp?.data ?? resp) as { alreadyExists: boolean; root: string; message: string; created?: string[] };
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

// ─── Permissions ───

export interface PermissionPreset {
  id: string;
  name: string;
  description: string;
  behaviorOverride: {
    approvalStrategy?: string | null;
    fileAccess?: string | null;
    toolsAsk?: string[] | null;
    toolsDeny?: string[] | null;
  };
}

export async function getPermissionPresets(): Promise<PermissionPreset[]> {
  const resp = (await wsClient.send("permissions.get_presets")) as {
    data?: { presets?: PermissionPreset[] };
  };
  return resp?.data?.presets ?? [];
}

export async function getSessionPermission(
  sessionId: string,
): Promise<{ sessionId: string; hasOverride: boolean; presetId: string }> {
  const resp = (await wsClient.send("permissions.get_session", { sessionId })) as {
    data?: { sessionId: string; hasOverride: boolean; presetId: string };
  };
  return resp?.data ?? { sessionId, hasOverride: false, presetId: "" };
}

export async function setSessionPermission(
  sessionId: string,
  presetId: string,
): Promise<{ sessionId: string; presetId: string }> {
  const resp = (await wsClient.send("permissions.set_session", {
    sessionId,
    presetId,
  })) as {
    data?: { sessionId: string; presetId: string };
  };
  return resp?.data ?? { sessionId, presetId };
}

// ─── Chat Streaming (WebSocket) ───
// Protocol types are generated from Rust: see xiaolin-protocol/generated/protocol.ts
export type { AgentEvent, TurnSummary, TokenUsage, ToolCallData, ClientOp, AbortReason, ErrorCode, WarningCategory, TurnContextItem } from "../../../xiaolin-protocol/generated/protocol";

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
  responseLanguage?: string | null;
  goalMode?: boolean;
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
  "sub_agent_notification",
  "approval_required",
  "approval_resolved",
  "error",
  "stream_error",
  "warning",
  "memory_stored",
  "memory_recalled",
  "goal_updated",
  "goal_cleared",
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
      sessionId: params.sessionId,
      stream: true,
      ...(params.agentId ? { agentId: params.agentId } : {}),
      ...(params.model ? { model: params.model } : {}),
      ...(params.temperature != null ? { temperature: params.temperature } : {}),
      ...(params.maxTokens != null ? { maxTokens: params.maxTokens } : {}),
      ...(params.workDir ? { workDir: params.workDir } : {}),
      ...(params.responseLanguage ? { responseLanguage: params.responseLanguage } : {}),
      ...(params.goalMode ? { goalMode: true } : {}),
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

export function onProjectsChanged(handler: () => void): UnsubscribeFn {
  return wsClient.on("projects.changed", () => {
    handler();
  });
}

// ── Git API ──────────────────────────────────────────────────────────────

export async function gitStatus(projectId: string) {
  const resp = (await wsClient.send("git.status", { projectId })) as { data?: unknown };
  return resp?.data ?? null;
}

export async function gitDiff(projectId: string, path: string, staged = false) {
  const resp = (await wsClient.send("git.diff", { projectId, path, staged })) as {
    data?: { hunks?: unknown[] };
  };
  return resp?.data?.hunks ?? [];
}

export async function gitBranches(projectId: string) {
  const resp = (await wsClient.send("git.branches", { projectId })) as {
    data?: { branches?: unknown[]; current?: string };
  };
  return resp?.data ?? { branches: [], current: "" };
}

export async function gitLog(projectId: string, limit = 20) {
  const resp = (await wsClient.send("git.log", { projectId, limit })) as {
    data?: { commits?: unknown[] };
  };
  return resp?.data?.commits ?? [];
}

export async function gitStage(projectId: string, files: string[] = []) {
  await wsClient.send("git.stage", { projectId, files });
}

export async function gitUnstage(projectId: string, files: string[] = []) {
  await wsClient.send("git.unstage", { projectId, files });
}

export async function gitCommit(projectId: string, message: string) {
  const resp = (await wsClient.send("git.commit", { projectId, message })) as {
    data?: { sha?: string; message?: string };
  };
  return resp?.data ?? null;
}

export async function gitRevert(projectId: string, files: string[]) {
  await wsClient.send("git.revert", { projectId, files });
}

export async function gitInit(projectId: string) {
  return await wsClient.send("git.init", { projectId });
}

export function onGitStatusChanged(handler: (projectId: string, status: unknown) => void): UnsubscribeFn {
  return wsClient.on("git.status_changed", (msg: unknown) => {
    const data = (msg as { data?: { projectId?: string; status?: unknown } })?.data;
    if (data?.projectId) handler(data.projectId, data.status);
  });
}

export function onChannelsChanged(handler: (channelId: string, action: string) => void): UnsubscribeFn {
  return wsClient.on("channels.changed", (msg: unknown) => {
    const data = (msg as { data?: { channelId?: string; action?: string } })?.data;
    if (data?.channelId) handler(data.channelId, data.action ?? "updated");
  });
}

export function onPermissionsChanged(handler: (sessionId: string, presetId: string) => void): UnsubscribeFn {
  return wsClient.on("permissions.changed", (msg: unknown) => {
    const data = (msg as { data?: { sessionId?: string; presetId?: string } })?.data;
    if (data?.sessionId) handler(data.sessionId, data.presetId ?? "");
  });
}

export function onWsEvent(event: string, handler: (data: unknown) => void): UnsubscribeFn {
  return wsClient.on(event, handler);
}

// ─── MCP Server Management ───

export interface McpServerStatus {
  id: string;
  status: "connecting" | "connected" | "failed" | "disabled" | "needs_auth";
  error?: string | null;
  toolCount: number;
  connectedAt?: string | null;
}

export async function getMcpStatus(): Promise<McpServerStatus[]> {
  const resp = (await wsClient.send("mcp.status")) as { data?: { servers?: McpServerStatus[] } };
  return resp?.data?.servers ?? [];
}

export async function reloadMcpServers(): Promise<McpServerStatus[]> {
  const resp = (await wsClient.send("mcp.reload")) as { data?: { servers?: McpServerStatus[] } };
  return resp?.data?.servers ?? [];
}

export interface AddMcpServerParams {
  id: string;
  command?: string;
  args?: string[];
  transport?: "stdio" | "sse" | "streamable_http" | "http";
  url?: string;
  env?: Record<string, string>;
  bearer_token_env_var?: string;
  http_headers?: Record<string, string>;
}

export async function addMcpServer(
  params: AddMcpServerParams,
): Promise<{ ok: boolean; id: string; status?: McpServerStatus }> {
  const payload: Record<string, unknown> = { id: params.id };
  if (params.command) payload.command = params.command;
  if (params.args?.length) payload.args = params.args;
  if (params.transport) payload.transport = params.transport;
  if (params.url) payload.url = params.url;
  if (params.env && Object.keys(params.env).length > 0) payload.env = params.env;
  const resp = (await wsClient.send("mcp.add", payload)) as {
    data?: { ok?: boolean; id?: string; status?: McpServerStatus };
  };
  return { ok: resp?.data?.ok ?? false, id: resp?.data?.id ?? params.id, status: resp?.data?.status };
}

export async function removeMcpServer(id: string): Promise<{ ok: boolean; id: string }> {
  const resp = (await wsClient.send("mcp.remove", { id })) as {
    data?: { ok?: boolean; id?: string };
  };
  return { ok: resp?.data?.ok ?? false, id: resp?.data?.id ?? id };
}

export interface McpDetailResult {
  id: string;
  status: string;
  error?: string | null;
  toolCount: number;
  connectedAt?: string | null;
  config: {
    command: string;
    args: string[];
    transport: string;
    url?: string | null;
    env: Record<string, string>;
    source?: "user" | "project" | "unknown";
  };
  tools: Array<{ name: string; description: string }>;
}

export async function mcpDetail(id: string): Promise<McpDetailResult | null> {
  const resp = (await wsClient.send("mcp.detail", { id })) as { data?: McpDetailResult };
  return resp?.data ?? null;
}

// ─── MCP Prompts ───

export interface McpPromptInfo {
  server: string;
  name: string;
  description?: string;
  arguments?: Array<{ name: string; description?: string; required?: boolean }>;
}

export interface McpPromptMessageContent {
  type: "text" | "image" | "resource";
  text?: string;
  data?: string;
  mime_type?: string;
  resource?: { uri: string; mimeType?: string; text?: string };
}

export interface McpPromptMessage {
  role: string;
  content: McpPromptMessageContent;
}

export interface McpResourceInfo {
  uri: string;
  name: string;
  description?: string | null;
  mimeType?: string | null;
}

export async function mcpResources(serverName: string): Promise<McpResourceInfo[]> {
  const resp = (await wsClient.send("plugins.resources", { server_name: serverName })) as {
    data?: { resources: McpResourceInfo[] };
  };
  return resp?.data?.resources ?? [];
}

export async function mcpPrompts(): Promise<McpPromptInfo[]> {
  const resp = (await wsClient.send("plugins.prompts", {})) as { data?: { prompts: McpPromptInfo[] } };
  return resp?.data?.prompts ?? [];
}

export async function mcpGetPrompt(
  serverName: string,
  promptName: string,
  args?: Record<string, string>,
): Promise<McpPromptMessage[]> {
  const resp = (await wsClient.send("plugins.get_prompt", {
    server_name: serverName,
    prompt_name: promptName,
    arguments: args,
  })) as { data?: { messages: McpPromptMessage[] } };
  return resp?.data?.messages ?? [];
}

export async function mcpElicitationReply(
  elicitationId: string,
  action: "accept" | "decline",
  content?: Record<string, unknown>,
): Promise<void> {
  await wsClient.send("plugins.elicitation_reply", {
    elicitation_id: elicitationId,
    action,
    content,
  });
}

// ─── Channel Management ───

export interface ChannelStatus {
  id: string;
  name: string;
  description: string;
  aliases: string[];
  status: "connected" | "disconnected" | "configured" | "available";
  connectionMode: string;
  capabilities: {
    directMessage?: boolean;
    groupChat?: boolean;
    media?: boolean;
    streaming?: boolean;
    reactions?: boolean;
    threads?: boolean;
  };
}

export interface ChannelDetailResult extends ChannelStatus {
  tools: Array<{ name: string; description: string }>;
  config: Record<string, unknown>;
  hasBackup?: boolean;
}

export async function channelsDetail(id: string): Promise<ChannelDetailResult | null> {
  const resp = (await wsClient.send("channels.detail", { id })) as { data?: ChannelDetailResult };
  return resp?.data ?? null;
}

export async function channelsList(): Promise<ChannelStatus[]> {
  const resp = (await wsClient.send("channels.list")) as {
    data?: { channels?: ChannelStatus[] };
  };
  return resp?.data?.channels ?? [];
}

export async function channelsWechatLogin(): Promise<{
  sessionKey: string;
  qrUrl: string;
  status: string;
}> {
  const resp = (await wsClient.send("channels.wechat_login")) as {
    data?: { sessionKey?: string; qrUrl?: string; status?: string };
  };
  return {
    sessionKey: resp?.data?.sessionKey ?? "",
    qrUrl: resp?.data?.qrUrl ?? "",
    status: resp?.data?.status ?? "error",
  };
}

export async function channelsWechatPoll(sessionKey: string): Promise<{
  status: string;
  qrUrl?: string;
  accountId?: string;
  message?: string;
}> {
  const resp = (await wsClient.send("channels.wechat_poll", { sessionKey })) as {
    data?: { status?: string; qrUrl?: string; accountId?: string; message?: string };
  };
  return {
    status: resp?.data?.status ?? "error",
    qrUrl: resp?.data?.qrUrl,
    accountId: resp?.data?.accountId,
    message: resp?.data?.message,
  };
}

export async function channelsWechatVerify(
  sessionKey: string,
  code: string,
): Promise<{ ok: boolean; message?: string }> {
  const resp = (await wsClient.send("channels.wechat_verify", { sessionKey, code })) as {
    data?: { ok?: boolean; message?: string };
  };
  return { ok: resp?.data?.ok ?? false, message: resp?.data?.message };
}

export async function channelsConnect(id: string): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("channels.connect", { id })) as {
    data?: { ok?: boolean };
  };
  return { ok: resp?.data?.ok ?? false };
}

export async function channelsDisconnect(
  channelId: string,
  accountId?: string,
): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("channels.disconnect", { channelId, accountId })) as {
    data?: { ok?: boolean };
  };
  return { ok: resp?.data?.ok ?? false };
}

export interface ChannelsUpdateResult {
  ok: boolean;
  channelId: string;
  reloadError?: string | null;
  hasBackup: boolean;
}

export async function channelsUpdate(
  id: string,
  config: Record<string, unknown>,
): Promise<ChannelsUpdateResult> {
  const resp = (await wsClient.send("channels.update", { id, config })) as {
    data?: ChannelsUpdateResult;
  };
  return resp?.data ?? { ok: false, channelId: id, hasBackup: false };
}

export async function channelsRestore(id: string): Promise<{ ok: boolean; reloadError?: string | null }> {
  const resp = (await wsClient.send("channels.restore", { id })) as {
    data?: { ok?: boolean; reloadError?: string | null };
  };
  return { ok: resp?.data?.ok ?? false, reloadError: resp?.data?.reloadError };
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

export async function approvePlan(
  sessionId: string,
  mode: "agent" | "plan" = "agent",
): Promise<{ ok: boolean; from: string; to: string }> {
  const resp = (await wsClient.send("execution.approve_plan", { sessionId, mode })) as {
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

export async function chatCancel(sessionId: string): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("cancel", { sessionId })) as {
    data?: { cancelled?: boolean };
  };
  return { ok: resp?.data?.cancelled ?? false };
}

export async function pauseGoal(sessionId: string): Promise<void> {
  await wsClient.send("goal.pause", { sessionId });
}

export async function resumeGoal(sessionId: string): Promise<void> {
  await wsClient.send("goal.resume", { sessionId });
}

export async function clearGoal(sessionId: string): Promise<void> {
  await wsClient.send("goal.clear", { sessionId });
}

export async function editGoal(
  sessionId: string,
  description: string,
): Promise<void> {
  await wsClient.send("goal.edit", { sessionId, description });
}

export async function addGoalBudget(
  sessionId: string,
  amount: number,
): Promise<void> {
  await wsClient.send("goal.add_budget", { sessionId, amount });
}

export async function chatSteer(
  sessionId: string,
  messages: Array<{ role: string; content: string }>,
): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("chat.steer", { sessionId, messages })) as {
    data?: { ok?: boolean };
  };
  return { ok: resp?.data?.ok ?? false };
}

export async function submitFeedback(
  sessionId: string,
  turnId: string,
  rating: "positive" | "negative",
): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("evolution.feedback", { sessionId, turnId, rating })) as {
    data?: { ok?: boolean };
  };
  return { ok: resp?.data?.ok ?? false };
}

export async function retryTurn(
  sessionId: string,
  turnId: string,
): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("chat.retry_turn", { sessionId, turnId })) as {
    data?: { ok?: boolean };
  };
  return { ok: resp?.data?.ok ?? false };
}

export async function chatSend(content: string, agentId?: string): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("chat", {
    messages: [{ role: "user", content }],
    agentId,
    stream: true,
  })) as { data?: { ok?: boolean } };
  return { ok: resp?.data?.ok ?? false };
}

export async function submitToolAnswer(requestId: string, answer: string, sessionId?: string): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("tools.submit_answer", { requestId, answer, sessionId })) as {
    data?: { ok?: boolean };
  };
  return { ok: resp?.data?.ok ?? false };
}

export async function resolveApproval(
  approvalId: string,
  decision: string,
  sessionId?: string,
  extra?: Record<string, unknown>,
): Promise<{ ok: boolean }> {
  const resp = (await wsClient.send("resolve_approval", {
    approvalId,
    decision: { decision, ...extra },
    ...(sessionId && { sessionId }),
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
  work_dir?: string | null;
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

// ─── Automations (user-facing wrapper over cron) ───

export async function automationsList(): Promise<CronJob[]> {
  const resp = (await wsClient.send("automations.list")) as {
    data?: { jobs?: CronJob[] };
  };
  return resp?.data?.jobs ?? [];
}

export async function automationsCreate(
  job: Partial<CronJob> & { name: string; schedule: string; action: CronJobAction },
): Promise<CronJob | null> {
  const resp = (await wsClient.send("automations.create", job as Record<string, unknown>)) as {
    data?: { job?: CronJob };
  };
  return resp?.data?.job ?? null;
}

export async function automationsUpdate(
  jobId: string,
  patch: Partial<CronJob>,
): Promise<CronJob | null> {
  const resp = (await wsClient.send("automations.update", { jobId, ...patch })) as {
    data?: { job?: CronJob };
  };
  return resp?.data?.job ?? null;
}

export async function automationsDelete(jobId: string): Promise<boolean> {
  const resp = (await wsClient.send("automations.delete", { jobId })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function automationsRunNow(jobId: string): Promise<boolean> {
  const resp = (await wsClient.send("automations.run_now", { jobId })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function automationsRuns(
  jobId: string,
  limit?: number,
): Promise<CronJobRun[]> {
  const resp = (await wsClient.send("automations.runs", { jobId, limit: limit ?? 20 })) as {
    data?: { runs?: CronJobRun[] };
  };
  return resp?.data?.runs ?? [];
}

export type AutomationChangedEvent = {
  event: "created" | "updated" | "deleted" | "run_completed";
  jobId: string;
  job: CronJob | null;
};

export function onAutomationsChanged(
  handler: (data: AutomationChangedEvent) => void,
): () => void {
  return wsClient.on("automations.changed", (raw) => {
    const msg = raw as { data?: AutomationChangedEvent };
    if (msg?.data) handler(msg.data);
  });
}

// ─── Plugins ───

export interface PluginSummary {
  id: string;
  name: string;
  scope: "user" | "project" | "global";
  enabled: boolean;
  status: "connected" | "connecting" | "failed" | "disabled" | "pending_approval" | "needs_auth";
  toolCount: number;
  lastError?: string | null;
  connectedAt?: string | null;
  commandPreview?: string | null;
  transport?: "stdio" | "sse" | "streamable_http" | null;
  capabilities?: {
    tools: boolean;
    resources: boolean;
    prompts: boolean;
  };
}

export interface PluginTool {
  name: string;
  description: string;
}

export async function listPlugins(): Promise<PluginSummary[]> {
  const resp = (await wsClient.send("plugins.list")) as {
    data?: { plugins?: PluginSummary[] };
  };
  return resp?.data?.plugins ?? [];
}

export async function enablePlugin(id: string): Promise<boolean> {
  const resp = (await wsClient.send("plugins.enable", { id })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function disablePlugin(id: string): Promise<boolean> {
  const resp = (await wsClient.send("plugins.disable", { id })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function restartPlugin(id: string): Promise<boolean> {
  const resp = (await wsClient.send("plugins.restart", { id })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function getPluginTools(id: string): Promise<PluginTool[]> {
  const resp = (await wsClient.send("plugins.tools", { id })) as {
    data?: { tools?: PluginTool[] };
  };
  return resp?.data?.tools ?? [];
}

export async function approvePlugin(id: string): Promise<boolean> {
  const resp = (await wsClient.send("plugins.approve", { id })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function rejectPlugin(id: string): Promise<boolean> {
  const resp = (await wsClient.send("plugins.reject", { id })) as {
    data?: { ok?: boolean };
  };
  return resp?.data?.ok ?? false;
}

export async function oauthLoginPlugin(id: string): Promise<{ ok: boolean; auth_url?: string }> {
  const resp = (await wsClient.send("plugins.oauth_login", { id })) as {
    data?: { ok?: boolean; auth_url?: string };
  };
  return { ok: resp?.data?.ok ?? false, auth_url: resp?.data?.auth_url };
}

export function onPluginsStatusChanged(
  handler: (plugins: PluginSummary[]) => void,
): () => void {
  return wsClient.on("plugins.status_changed", (raw) => {
    const msg = raw as { data?: { plugins?: PluginSummary[] } };
    if (msg?.data?.plugins) handler(msg.data.plugins);
  });
}

// ─── Global Search ───

export interface SearchResult {
  session_id: string;
  turn_id: string;
  role: string;
  message_id: string | null;
  session_title: string;
  work_dir: string | null;
  snippet: string;
  timestamp: string;
  rank: number;
}

export interface SearchIndexStatus {
  indexed_count: number;
  total_count: number;
  is_indexing: boolean;
}

export async function searchQuery(params: {
  q: string;
  filters?: { work_dir?: string; date_from?: string; date_to?: string };
  page?: number;
  limit?: number;
}): Promise<{ results: SearchResult[]; total_estimate: number; page: number }> {
  const resp = (await wsClient.send("search.query", params)) as {
    data?: { results?: SearchResult[]; total_estimate?: number; page?: number };
  };
  return {
    results: resp?.data?.results ?? [],
    total_estimate: resp?.data?.total_estimate ?? 0,
    page: resp?.data?.page ?? 0,
  };
}

export async function searchIndexStatus(): Promise<SearchIndexStatus> {
  const resp = (await wsClient.send("search.index_status")) as {
    data?: SearchIndexStatus;
  };
  return (
    resp?.data ?? { indexed_count: 0, total_count: 0, is_indexing: false }
  );
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

// ─── Cost ───

export interface CostSummaryData {
  total_cost_usd: number;
  today_cost_usd: number;
  budget_limit: number | null;
  budget_used_pct: number | null;
}

export interface TokenUsageDailyData {
  date: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  cost_usd: number;
  call_count: number;
}

export interface ToolCallDailyData {
  date: string;
  tool_name: string;
  success_count: number;
  failure_count: number;
  total_duration_ms: number;
}

export async function costSummary(): Promise<CostSummaryData> {
  const resp = (await wsClient.send("cost.summary")) as { data?: CostSummaryData };
  return resp?.data ?? { total_cost_usd: 0, today_cost_usd: 0, budget_limit: null, budget_used_pct: null };
}

export async function costDaily(start?: string, end?: string): Promise<TokenUsageDailyData[]> {
  const params: Record<string, unknown> = {};
  if (start) params.start = start;
  if (end) params.end = end;
  const resp = (await wsClient.send("cost.daily", params)) as { data?: { items?: TokenUsageDailyData[] } };
  return resp?.data?.items ?? [];
}

export async function costTools(start?: string, end?: string): Promise<ToolCallDailyData[]> {
  const params: Record<string, unknown> = {};
  if (start) params.start = start;
  if (end) params.end = end;
  const resp = (await wsClient.send("cost.tools", params)) as { data?: { items?: ToolCallDailyData[] } };
  return resp?.data?.items ?? [];
}

export interface SessionCostData {
  session_id: string;
  started_at: string;
  ended_at: string | null;
  total_cost_usd: number;
  total_input_tokens: number;
  total_output_tokens: number;
  turn_count: number;
  model_breakdown: string | null;
}

export async function costSessions(limit?: number): Promise<SessionCostData[]> {
  const params: Record<string, unknown> = {};
  if (limit) params.limit = limit;
  const resp = (await wsClient.send("cost.sessions", params)) as { data?: { items?: SessionCostData[] } };
  return resp?.data?.items ?? [];
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