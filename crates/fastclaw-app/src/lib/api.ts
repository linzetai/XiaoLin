import { useGatewayStore } from "./store";
import * as transport from "./transport";

function getHttpBase(): string {
  return useGatewayStore.getState().info?.httpUrl ?? "";
}

function ensureBase(): string {
  const base = getHttpBase();
  if (!base) throw new Error("gateway not ready (httpUrl is empty)");
  return base;
}

async function httpGet<T>(path: string): Promise<T> {
  const base = ensureBase();
  const resp = await fetch(`${base}${path}`);
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

async function httpPost<T>(path: string, body?: unknown): Promise<T> {
  const base = ensureBase();
  const resp = await fetch(`${base}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

async function httpPut<T>(path: string, body?: unknown): Promise<T> {
  const base = ensureBase();
  const resp = await fetch(`${base}${path}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

async function httpDelete(path: string): Promise<void> {
  const base = ensureBase();
  const resp = await fetch(`${base}${path}`, { method: "DELETE" });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
}

// ─── Re-exports from transport (preferred for Tauri-aware callers) ───

export type { ModelInfo } from "./transport";

export const listModels = transport.listModels;
export const getConfig = transport.getConfig;
export const setConfig = transport.setConfig;
export const updateSessionTitle = transport.updateSessionTitle;
export const deleteSession = transport.deleteSession;
export const createSession = transport.createSession;
export const setSessionWorkDir = transport.setSessionWorkDir;

// ─── REST API wrappers (Tauri mode uses IPC, browser mode falls back to HTTP) ───

export interface SkillInfo {
  id: string;
  name: string;
  description: string;
  version?: string;
  scope?: string;
  tags?: string[];
}

export async function listSkills(agentId?: string): Promise<SkillInfo[]> {
  try {
    if (transport.isTauri) {
      console.info("[api] listSkills via IPC, agent:", agentId);
      const skills = await transport.listSkillsIpc(agentId);
      console.info("[api] listSkills IPC result:", skills.length, "skills");
      return skills.map((s) => ({
        id: s.id,
        name: s.name,
        description: s.description ?? "",
        tags: s.tags,
      }));
    }
    const url = agentId ? `/api/v1/skills?agentId=${encodeURIComponent(agentId)}` : "/api/v1/skills";
    const resp = await httpGet<{ skills?: SkillInfo[] }>(url);
    return resp?.skills ?? [];
  } catch (e) {
    console.warn("[api] listSkills error:", e);
    return [];
  }
}

export async function refreshSkills(): Promise<number> {
  try {
    if (transport.isTauri) {
      const resp = await transport.refreshSkillsIpc();
      return resp.count;
    }
    const resp = await httpPost<{ count?: number }>("/api/v1/skills/refresh");
    return resp?.count ?? 0;
  } catch (e) {
    console.warn("[api] refreshSkills error:", e);
    return 0;
  }
}

export async function uploadSkill(sourcePath: string): Promise<string | null> {
  try {
    if (transport.isTauri) {
      const resp = await transport.uploadSkillIpc(sourcePath);
      return resp.installed ?? null;
    }
    return null;
  } catch (e) {
    console.warn("[api] uploadSkill error:", e);
    return null;
  }
}

export interface ToolInfo {
  id: string;
  name: string;
  description?: string;
}

interface RawToolDef {
  type?: string;
  function?: { name?: string; description?: string };
  name?: string;
  description?: string;
}

export async function listTools(): Promise<ToolInfo[]> {
  try {
    if (transport.isTauri) {
      const raw = await transport.listToolsIpc();
      return raw.map((t) => ({
        id: t.function?.name ?? "unknown",
        name: t.function?.name ?? "unknown",
        description: t.function?.description,
      }));
    }
    const resp = await httpGet<{ tools?: RawToolDef[] }>("/api/v1/tools");
    return (resp?.tools ?? []).map((t) => ({
      id: t.function?.name ?? t.name ?? "unknown",
      name: t.function?.name ?? t.name ?? "unknown",
      description: t.function?.description ?? t.description,
    }));
  } catch (e) {
    console.warn("[api] listTools error:", e);
    return [];
  }
}

export interface AgentToolInfo {
  id: string;
  name: string;
  enabled: boolean;
  description?: string;
}

export async function listAgentTools(agentId: string): Promise<AgentToolInfo[]> {
  try {
    if (transport.isTauri) {
      console.info("[api] listAgentTools via IPC, agent:", agentId);
      const tools = await transport.listAgentToolsIpc(agentId);
      console.info("[api] listAgentTools IPC result:", tools.length, "tools");
      return tools.map((t) => ({ id: t.id, name: t.id, enabled: t.enabled, description: t.description }));
    }
    const resp = await httpGet<{ tools?: Omit<AgentToolInfo, "name">[] }>(`/api/v1/agents/${agentId}/tools`);
    return (resp?.tools ?? []).map((t) => ({ ...t, name: t.id }));
  } catch (e) {
    console.warn("[api] listAgentTools error:", e);
    return [];
  }
}

export async function updateAgentTools(
  agentId: string,
  tools: Array<{ id: string; enabled: boolean }>,
): Promise<boolean> {
  try {
    if (transport.isTauri) {
      console.info("[api] updateAgentTools via IPC, agent:", agentId, tools);
      return await transport.updateAgentToolsIpc(agentId, tools);
    }
    await httpPut(`/api/v1/agents/${agentId}/tools`, { tools });
    return true;
  } catch (e) {
    console.warn("[api] updateAgentTools error:", e);
    return false;
  }
}

export async function getSkillsDenyList(): Promise<string[]> {
  try {
    console.info("[api] getSkillsDenyList via config key 'skills.deny'");
    const resp = await transport.getConfig("skills.deny") as { key?: string; value?: string[] } | null;
    console.info("[api] getSkillsDenyList raw response:", resp);
    if (Array.isArray(resp?.value)) return resp!.value!;
    if (Array.isArray(resp)) return resp as unknown as string[];
    return [];
  } catch (e) {
    console.warn("[api] getSkillsDenyList error:", e);
    return [];
  }
}

export async function updateSkillsDenyList(deny: string[]): Promise<boolean> {
  try {
    console.info("[api] updateSkillsDenyList:", deny);
    const result = await transport.setConfig("skills.deny", deny);
    console.info("[api] updateSkillsDenyList result:", result);
    return result.persisted;
  } catch (e) {
    console.warn("[api] updateSkillsDenyList error:", e);
    return false;
  }
}

export interface AgentModelConfig {
  provider: string;
  model: string;
  temperature: number;
  maxTokens?: number;
  contextWindow?: number;
  costPer1kInput?: number;
  costPer1kOutput?: number;
  supportsReasoning?: boolean;
  fallbacks?: Array<{ provider: string; model: string }>;
  maxConcurrentRequests?: number;
}

export type FileAccessMode = "none" | "workspace" | "full";

export interface AgentBehaviorConfig {
  maxToolCallsPerTurn?: number;
  maxConsecutiveErrors?: number;
  requireConfirmationFor?: string[];
  toolsAllow?: string[];
  toolsDeny?: string[];
  fileAccess?: FileAccessMode;
}

export interface AgentChannelConfig {
  enabled?: boolean;
  appId?: string;
  appSecret?: string;
  verificationToken?: string;
  encryptKey?: string;
  connectionMode?: string;
  domain?: string;
  replyMode?: string;
  userAccessToken?: string;
}

export interface BackendAgent {
  agentId: string;
  name: string;
  model: AgentModelConfig | string;
  systemPrompt?: string;
  tools?: Array<{ id: string }>;
  behavior?: AgentBehaviorConfig;
  channels?: Record<string, AgentChannelConfig>;
}

export async function getAgent(agentId: string): Promise<BackendAgent | null> {
  try {
    if (transport.isTauri) {
      const raw = await transport.getAgentIpc(agentId);
      return raw as BackendAgent | null;
    }
    return await httpGet<BackendAgent>(`/api/v1/agents/${agentId}`);
  } catch (e) {
    console.warn("[api] getAgent error:", e);
    return null;
  }
}

export async function updateAgent(agentId: string, updates: Partial<BackendAgent>): Promise<boolean> {
  try {
    if (transport.isTauri) {
      return await transport.updateAgentIpc(agentId, updates as Record<string, unknown>);
    }
    await httpPut(`/api/v1/agents/${agentId}`, updates);
    return true;
  } catch (e) {
    console.warn("[api] updateAgent error:", e);
    return false;
  }
}

export async function createAgent(agent: { name: string; agentId?: string; model?: string; provider?: string; systemPrompt?: string }): Promise<BackendAgent | null> {
  try {
    const agentId = agent.agentId?.trim() || agent.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || `agent-${Date.now()}`;
    const config: Record<string, unknown> = {
      agentId,
      name: agent.name,
      model: {
        provider: agent.provider || "openai_compatible",
        model: agent.model || "gpt-4o",
      },
      systemPrompt: agent.systemPrompt || null,
      tools: [],
      behavior: {},
      mcpServers: [],
      channels: {},
    };
    if (transport.isTauri) {
      const resp = await transport.createAgentIpc(config);
      if (resp.ok && resp.agentId) {
        return await getAgent(resp.agentId);
      }
      return null;
    }
    return await httpPost<BackendAgent>("/api/v1/agents", config);
  } catch (e) {
    console.warn("[api] createAgent error:", e);
    return null;
  }
}

export async function deleteAgent(agentId: string): Promise<boolean> {
  try {
    if (transport.isTauri) {
      return await transport.deleteAgentIpc(agentId);
    }
    await httpDelete(`/api/v1/agents/${agentId}`);
    return true;
  } catch (e) {
    console.warn("[api] deleteAgent error:", e);
    return false;
  }
}

// ─── Avatar upload ───

export async function uploadAgentAvatar(agentId: string, sourcePath: string): Promise<string | null> {
  try {
    const resp = await transport.uploadAgentAvatarIpc(agentId, sourcePath);
    return resp.ok ? (resp.path ?? null) : null;
  } catch (e) {
    console.warn("[api] uploadAgentAvatar error:", e);
    return null;
  }
}

// ─── Identity files ───

export interface IdentityFiles {
  soul: string | null;
  user: string | null;
  agents: string | null;
}

export async function getIdentityFiles(agentId: string): Promise<IdentityFiles> {
  try {
    return await transport.readIdentityFilesIpc(agentId);
  } catch (e) {
    console.warn("[api] getIdentityFiles error:", e);
    return { soul: null, user: null, agents: null };
  }
}

// ─── Cron jobs ───

export type { CronJob, CronJobAction, CronJobRun } from "./transport";

export async function listCronJobs(agentId?: string) {
  try {
    const resp = await transport.cronListJobs(agentId);
    return resp.jobs;
  } catch (e) {
    console.warn("[api] listCronJobs error:", e);
    return [];
  }
}

export async function getCronJob(jobId: string) {
  return transport.cronGetJob(jobId);
}

export async function upsertCronJob(
  job: Parameters<typeof transport.cronUpsertJob>[0],
) {
  return transport.cronUpsertJob(job);
}

export async function deleteCronJob(jobId: string) {
  return transport.cronDeleteJob(jobId);
}

export async function listCronRuns(jobId: string, limit?: number) {
  try {
    const resp = await transport.cronListRuns(jobId, limit);
    return resp.runs;
  } catch (e) {
    console.warn("[api] listCronRuns error:", e);
    return [];
  }
}

// ─── File listing (via Tauri or fallback) ───

export async function listFiles(dirPath: string): Promise<{ files: string[]; dirs: string[] }> {
  if (transport.isTauri) {
    try {
      const { readDir } = await import("@tauri-apps/plugin-fs");
      const entries = await readDir(dirPath);
      const files: string[] = [];
      const dirs: string[] = [];
      for (const entry of entries) {
        if (entry.name?.startsWith(".")) continue;
        if (entry.isDirectory) dirs.push(entry.name!);
        else if (entry.isFile) files.push(entry.name!);
      }
      files.sort();
      dirs.sort();
      return { files, dirs };
    } catch (e) {
      console.warn("[api] listFiles error:", e);
      return { files: [], dirs: [] };
    }
  }
  return { files: [], dirs: [] };
}
