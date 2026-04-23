/**
 * Transport abstraction layer — routes communication through Tauri IPC Commands
 * in desktop mode and falls back to WebSocket/HTTP in browser mode.
 */

import * as wsClient from "./ws-client";

export const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

let _invoke: typeof import("@tauri-apps/api/core").invoke | null = null;
let _Channel: typeof import("@tauri-apps/api/core").Channel | null = null;
let _listen: typeof import("@tauri-apps/api/event").listen | null = null;

async function ensureTauriApi() {
  if (!_invoke) {
    const core = await import("@tauri-apps/api/core");
    _invoke = core.invoke;
    _Channel = core.Channel;
  }
}

async function ensureTauriEvents() {
  if (!_listen) {
    const events = await import("@tauri-apps/api/event");
    _listen = events.listen;
  }
}

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  await ensureTauriApi();
  return _invoke!<T>(cmd, args);
}

export async function invokeWithRetry<T>(
  cmd: string,
  maxRetries = 15,
  intervalMs = 500,
): Promise<T> {
  await ensureTauriApi();
  for (let i = 0; i < maxRetries; i++) {
    try {
      return await _invoke!<T>(cmd);
    } catch {
      if (i < maxRetries - 1)
        await new Promise((r) => setTimeout(r, intervalMs));
    }
  }
  throw new Error(`IPC ${cmd} failed after ${maxRetries} retries`);
}

// ─── Model connection test ───

export async function testModelConnection(baseUrl: string, apiKey: string, model?: string): Promise<void> {
  await tauriInvoke<{ ok: boolean }>("test_model_connection", {
    baseUrl,
    apiKey,
    model: model || null,
  });
}

// ─── Agents ───

export interface AgentSummary {
  agentId: string;
  name: string;
  model: string;
}

export async function listAgents(): Promise<AgentSummary[]> {
  if (isTauri) {
    const resp = await tauriInvoke<{ agents: AgentSummary[] }>("list_agents");
    return resp.agents;
  }
  const resp = (await wsClient.send("agents")) as {
    data?: { agents?: AgentSummary[] };
  };
  return resp?.data?.agents ?? [];
}

// ─── Sessions ───

export interface SessionSummary {
  id: string;
  agentId: string;
  title: string | null;
  workDir?: string | null;
  messageCount: number;
  createdAt: string;
  updatedAt: string;
  totalPromptTokens?: number;
  totalCompletionTokens?: number;
  totalElapsedMs?: number;
}

export async function listSessions(
  limit = 50,
  offset = 0,
): Promise<SessionSummary[]> {
  if (isTauri) {
    const resp = await tauriInvoke<{
      sessions: SessionSummary[];
      count: number;
    }>("list_sessions", { limit, offset });
    return resp.sessions;
  }
  const resp = (await wsClient.send("sessions.list", { limit, offset })) as {
    data?: { sessions?: SessionSummary[] };
  };
  return resp?.data?.sessions ?? [];
}

export async function getSession(
  sessionId: string,
): Promise<SessionSummary | null> {
  if (isTauri) {
    try {
      return await tauriInvoke<SessionSummary>("get_session", { sessionId });
    } catch {
      return null;
    }
  }
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

export async function getSessionMessages(
  sessionId: string,
): Promise<SessionMessage[]> {
  if (isTauri) {
    const resp = await tauriInvoke<{ messages: SessionMessage[] }>(
      "get_session_messages",
      { sessionId },
    );
    return resp.messages;
  }
  const resp = (await wsClient.send("sessions.messages", { sessionId })) as {
    data?: { messages?: SessionMessage[] };
  };
  return resp?.data?.messages ?? [];
}

export async function createSession(agentId?: string): Promise<string> {
  if (isTauri) {
    const resp = await tauriInvoke<{ sessionId: string }>("create_session", {
      agentId: agentId ?? null,
    });
    return resp.sessionId;
  }
  const resp = (await wsClient.send(
    "sessions.new",
    agentId ? { agentId } : {},
  )) as {
    data?: { sessionId?: string };
  };
  return resp?.data?.sessionId ?? "";
}

export async function updateSessionTitle(
  sessionId: string,
  title: string,
): Promise<void> {
  if (isTauri) {
    await tauriInvoke("update_session_title", { sessionId, title });
    return;
  }
  await wsClient.send("sessions.update_title", { sessionId, title });
}

export async function deleteSession(sessionId: string): Promise<void> {
  if (isTauri) {
    await tauriInvoke("delete_session", { sessionId });
    return;
  }
  await wsClient.send("sessions.delete", { sessionId });
}

export async function setSessionWorkDir(
  sessionId: string,
  workDir: string | null,
): Promise<void> {
  if (isTauri) {
    await tauriInvoke("set_session_work_dir", { sessionId, workDir });
    return;
  }
  await wsClient.send("sessions.set_work_dir", { sessionId, workDir });
}

// ─── Models ───

export interface ModelInfo {
  agentId: string;
  model: string;
  provider: string;
  contextWindow: number;
  costPer1kInput: number;
  costPer1kOutput: number;
  supportsReasoning: boolean;
}

export async function listModels(): Promise<ModelInfo[]> {
  if (isTauri) {
    const resp = await tauriInvoke<{ models: ModelInfo[] }>("list_models");
    return resp.models;
  }
  const resp = (await wsClient.send("models.list")) as {
    data?: { models?: ModelInfo[] };
  };
  return resp?.data?.models ?? [];
}

// ─── Skills & Tools (Tauri IPC commands) ───

export interface SkillInfo {
  id: string;
  name: string;
  description: string | null;
  tags?: string[];
}

export interface AgentToolInfo {
  id: string;
  enabled: boolean;
  description?: string;
}

export async function listSkillsIpc(agentId?: string): Promise<SkillInfo[]> {
  if (isTauri) {
    const resp = await tauriInvoke<{ skills: SkillInfo[] }>("list_skills", {
      agentId: agentId ?? null,
    });
    return resp.skills;
  }
  return [];
}

export async function listAgentToolsIpc(agentId: string): Promise<AgentToolInfo[]> {
  if (isTauri) {
    const resp = await tauriInvoke<{ tools: AgentToolInfo[] }>("list_agent_tools", {
      agentId,
    });
    return resp.tools;
  }
  return [];
}

export async function refreshSkillsIpc(): Promise<{ refreshed: boolean; count: number }> {
  if (isTauri) {
    return tauriInvoke("refresh_skills");
  }
  return { refreshed: false, count: 0 };
}

export async function uploadSkillIpc(sourcePath: string): Promise<{ installed?: string; count?: number }> {
  if (isTauri) {
    return tauriInvoke("upload_skill", { sourcePath });
  }
  return {};
}

export async function submitToolAnswerIpc(
  requestId: string,
  answer: string,
): Promise<{ ok: boolean }> {
  if (isTauri) {
    return tauriInvoke("submit_tool_answer", { requestId, answer });
  }
  return { ok: false };
}

export async function getAgentIpc(agentId: string): Promise<unknown> {
  if (isTauri) {
    return tauriInvoke("get_agent", { agentId });
  }
  return null;
}

export async function updateAgentToolsIpc(
  agentId: string,
  tools: Array<{ id: string; enabled: boolean }>,
): Promise<boolean> {
  if (isTauri) {
    const resp = await tauriInvoke<{ ok: boolean }>("update_agent_tools", {
      agentId,
      tools,
    });
    return resp.ok;
  }
  return false;
}

export async function listToolsIpc(): Promise<Array<{ type?: string; function?: { name?: string; description?: string } }>> {
  if (isTauri) {
    const resp = await tauriInvoke<{ tools: Array<{ type?: string; function?: { name?: string; description?: string } }> }>("list_tools");
    return resp.tools;
  }
  return [];
}

export async function updateAgentIpc(
  agentId: string,
  config: Record<string, unknown>,
): Promise<boolean> {
  if (isTauri) {
    const resp = await tauriInvoke<{ ok: boolean }>("update_agent", {
      agentId,
      config,
    });
    return resp.ok;
  }
  return false;
}

export async function createAgentIpc(
  config: Record<string, unknown>,
): Promise<{ ok: boolean; agentId?: string }> {
  if (isTauri) {
    return tauriInvoke<{ ok: boolean; agentId?: string }>("create_agent", { config });
  }
  return { ok: false };
}

export async function deleteAgentIpc(agentId: string): Promise<boolean> {
  if (isTauri) {
    const resp = await tauriInvoke<{ ok: boolean }>("delete_agent", { agentId });
    return resp.ok;
  }
  return false;
}

// ─── Avatar upload ───

export async function uploadAgentAvatarIpc(
  agentId: string,
  sourcePath: string,
): Promise<{ ok: boolean; path?: string }> {
  if (isTauri) {
    return tauriInvoke("upload_agent_avatar", { agentId, sourcePath });
  }
  return { ok: false };
}

// ─── Identity files ───

export async function readIdentityFilesIpc(
  agentId: string,
): Promise<{ soul: string | null; user: string | null; agents: string | null }> {
  if (isTauri) {
    return tauriInvoke("read_identity_files", { agentId });
  }
  return { soul: null, user: null, agents: null };
}

// ─── Config ───

export async function getConfig(key?: string): Promise<unknown> {
  if (isTauri) {
    return tauriInvoke("get_config", { key: key ?? null });
  }
  const resp = (await wsClient.send(
    "config.get",
    key ? { key } : {},
  )) as {
    data?: unknown;
  };
  return resp?.data;
}

export async function setConfig(
  key: string,
  value: unknown,
): Promise<{ persisted: boolean; pendingRestart: boolean }> {
  if (isTauri) {
    return tauriInvoke("set_config", { key, value });
  }
  const resp = (await wsClient.send("config.set", { key, value })) as {
    data?: { persisted?: boolean; pendingRestart?: boolean };
  };
  return {
    persisted: resp?.data?.persisted ?? false,
    pendingRestart: resp?.data?.pendingRestart ?? false,
  };
}

// ─── Chat streaming ───

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

/**
 * Start a streaming chat. In Tauri mode uses Channel (in-process),
 * in browser mode uses the existing WebSocket path.
 *
 * Returns an unsubscribe/cleanup function.
 */
export function chatStream(
  params: ChatStreamParams,
  onEvent: ChatEventHandler,
): { promise: Promise<void>; cleanup: () => void } {
  if (isTauri) {
    return chatStreamTauri(params, onEvent);
  }
  return chatStreamWs(params, onEvent);
}

function chatStreamTauri(
  params: ChatStreamParams,
  onEvent: ChatEventHandler,
): { promise: Promise<void>; cleanup: () => void } {
  let cancelled = false;
  const requestId = `stream-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
  const cleanup = () => {
    if (cancelled) return;
    cancelled = true;
    void tauriInvoke("cancel_chat_stream", { requestId }).catch(() => {});
  };

  const promise = (async () => {
    await ensureTauriApi();
    const channel = new _Channel!<ChatStreamEvent>();
    channel.onmessage = (event) => {
      if (!cancelled) onEvent(event);
    };
    try {
      await _invoke!("chat_stream", {
        requestId,
        channel,
        messages: params.messages,
        agentId: params.agentId ?? null,
        sessionId: params.sessionId ?? null,
        model: params.model ?? null,
        temperature: params.temperature ?? null,
        maxTokens: params.maxTokens ?? null,
        workDir: params.workDir ?? null,
      });
    } catch (err) {
      const message =
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : "chat stream failed to start";
      if (!cancelled) {
        onEvent({
          type: "chat.error",
          error: { message },
        });
      }
      throw err;
    }
  })();

  return { promise, cleanup };
}

function chatStreamWs(
  params: ChatStreamParams,
  onEvent: ChatEventHandler,
): { promise: Promise<void>; cleanup: () => void } {
  const handlers: Array<(() => void) | undefined> = [];
  let done = false;

  const wrap = (type: string) => {
    const unsub = wsClient.on(type, (m: unknown) => {
      if (!done) onEvent(m as ChatStreamEvent);
    });
    handlers.push(unsub);
  };

  wrap("chat.start");
  wrap("chat.delta");
  wrap("chat.complete");
  wrap("chat.tool.start");
  wrap("chat.tool.done");
  wrap("chat.ask_question");
  wrap("chat.error");

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

// ─── Event subscriptions (Tauri Events vs WS subscribe) ───

export type UnsubscribeFn = () => void;

export async function onSessionChanged(
  handler: (sessionId: string) => void,
): Promise<UnsubscribeFn> {
  if (isTauri) {
    await ensureTauriEvents();
    const unlisten = await _listen!<{ sessionId: string }>(
      "sessions-changed",
      (event) => {
        handler(event.payload.sessionId);
      },
    );
    return unlisten;
  }
  wsClient.send("subscribe", { events: ["sessions.changed"] }).catch(() => {});
  return wsClient.on("sessions.changed", (msg: unknown) => {
    const sid = (msg as { data?: { sessionId?: string } })?.data?.sessionId;
    if (sid) handler(sid);
  });
}

// ─── MCP server management ───

export interface McpServerStatus {
  id: string;
  status: "connecting" | "connected" | "failed" | "disabled";
  error?: string | null;
  toolCount: number;
  connectedAt?: string | null;
}

export async function getMcpStatus(): Promise<McpServerStatus[]> {
  if (isTauri) {
    return tauriInvoke<McpServerStatus[]>("get_mcp_status");
  }
  const resp = await wsClient.send("mcp.status");
  return (resp as { servers?: McpServerStatus[] }).servers ?? [];
}

export async function reloadMcpServers(): Promise<McpServerStatus[]> {
  if (isTauri) {
    return tauriInvoke<McpServerStatus[]>("reload_mcp_servers");
  }
  const resp = await wsClient.send("mcp.reload");
  return (resp as { servers?: McpServerStatus[] }).servers ?? [];
}

export async function addMcpServer(
  id: string,
  command: string,
  args?: string[],
): Promise<{ ok: boolean; id: string; status?: McpServerStatus }> {
  if (isTauri) {
    return tauriInvoke("add_mcp_server", { id, command, args: args ?? [] });
  }
  return wsClient.send("mcp.add", { id, command, args: args ?? [] }) as Promise<{
    ok: boolean;
    id: string;
    status?: McpServerStatus;
  }>;
}

export async function removeMcpServer(
  id: string,
): Promise<{ ok: boolean; id: string }> {
  if (isTauri) {
    return tauriInvoke("remove_mcp_server", { id });
  }
  return wsClient.send("mcp.remove", { id }) as Promise<{
    ok: boolean;
    id: string;
  }>;
}

// ─── Cron jobs ───

export interface CronJobAction {
  type: "agent_chat" | "dag_execute" | "webhook";
  agent_id?: string;
  message?: string;
  session_id?: string;
  url?: string;
  method?: string;
  body?: unknown;
  dag?: unknown;
  input?: unknown;
}

export interface CronJob {
  id: string;
  name: string;
  schedule: string;
  action: CronJobAction;
  enabled: boolean;
  last_run: string | null;
  next_run: string | null;
  status: "idle" | "running" | "failed" | "disabled";
  created_at: string;
  run_count: number;
  error_count: number;
  last_error: string | null;
}

export async function cronListJobs(
  agentId?: string,
): Promise<{ jobs: CronJob[]; count: number }> {
  if (isTauri) {
    return tauriInvoke("cron_list_jobs", { agentId: agentId ?? null });
  }
  throw new Error("cron IPC only available in Tauri mode");
}

export async function cronGetJob(jobId: string): Promise<CronJob> {
  if (isTauri) {
    return tauriInvoke("cron_get_job", { jobId });
  }
  throw new Error("cron IPC only available in Tauri mode");
}

export async function cronUpsertJob(
  job: Partial<CronJob> & { schedule: string; action: CronJobAction },
): Promise<{ id: string; ok: boolean }> {
  if (isTauri) {
    return tauriInvoke("cron_upsert_job", { job });
  }
  throw new Error("cron IPC only available in Tauri mode");
}

export async function cronDeleteJob(
  jobId: string,
): Promise<{ deleted: boolean }> {
  if (isTauri) {
    return tauriInvoke("cron_delete_job", { jobId });
  }
  throw new Error("cron IPC only available in Tauri mode");
}

export interface CronJobRun {
  id: number;
  job_id: string;
  started_at: string;
  ended_at: string | null;
  status: string;
  output: string | null;
  error: string | null;
}

export async function cronListRuns(
  jobId: string,
  limit?: number,
): Promise<{ runs: CronJobRun[]; count: number }> {
  if (isTauri) {
    return tauriInvoke("cron_list_runs", { jobId, limit: limit ?? null });
  }
  throw new Error("cron IPC only available in Tauri mode");
}

// ─── WebSocket passthrough (browser mode only) ───

export function connectWs(url: string, token?: string): Promise<void> {
  return wsClient.connect(url, token);
}

export function disconnectWs(): void {
  wsClient.disconnect();
}

export function onWsEvent(event: string, handler: (data: unknown) => void): UnsubscribeFn {
  return wsClient.on(event, handler);
}

export function isWsConnected(): boolean {
  return wsClient.isConnected();
}
