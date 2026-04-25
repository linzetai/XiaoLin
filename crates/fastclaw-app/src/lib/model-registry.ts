/**
 * Model Registry — central source of truth for model provider configurations.
 *
 * Provides:
 * - Provider presets with models, base URLs, and context windows
 * - Token limit auto-inference from model names
 * - Config resolution priority chain (env > config file > UI defaults)
 */

export interface ModelPreset {
  id: string;
  name: string;
  description: string;
  contextWindow: number;
}

export interface ProviderPreset {
  id: string;
  name: string;
  logo: string;
  provider: string;
  baseUrl: string;
  models: ModelPreset[];
  apiKeyPrefix?: string;
  docsUrl?: string;
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "openai",
    name: "OpenAI",
    logo: "🟢",
    provider: "openai_compatible",
    baseUrl: "https://api.openai.com/v1",
    apiKeyPrefix: "sk-",
    docsUrl: "https://platform.openai.com/api-keys",
    models: [
      { id: "gpt-4.1", name: "GPT-4.1", description: "最新旗舰，最强推理", contextWindow: 1047576 },
      { id: "gpt-4o", name: "GPT-4o", description: "高性能多模态", contextWindow: 128000 },
      { id: "gpt-4o-mini", name: "GPT-4o Mini", description: "快速轻量", contextWindow: 128000 },
    ],
  },
  {
    id: "anthropic",
    name: "Anthropic",
    logo: "🟠",
    provider: "anthropic",
    baseUrl: "https://api.anthropic.com",
    apiKeyPrefix: "sk-ant-",
    docsUrl: "https://console.anthropic.com/settings/keys",
    models: [
      { id: "claude-sonnet-4-20250514", name: "Claude Sonnet 4", description: "高性能编码 & 推理", contextWindow: 200000 },
      { id: "claude-haiku-4-20250514", name: "Claude Haiku 4", description: "快速轻量响应", contextWindow: 200000 },
    ],
  },
  {
    id: "qwen",
    name: "通义千问",
    logo: "🔵",
    provider: "openai_compatible",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    apiKeyPrefix: "sk-",
    docsUrl: "https://help.aliyun.com/zh/model-studio/developer-reference/get-api-key",
    models: [
      { id: "qwen-max", name: "Qwen Max", description: "最强中文推理", contextWindow: 32768 },
      { id: "qwen-plus", name: "Qwen Plus", description: "均衡性能与速度", contextWindow: 131072 },
      { id: "qwen-turbo", name: "Qwen Turbo", description: "极速响应", contextWindow: 131072 },
    ],
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    logo: "🟣",
    provider: "openai_compatible",
    baseUrl: "https://api.deepseek.com/v1",
    apiKeyPrefix: "sk-",
    docsUrl: "https://platform.deepseek.com/api_keys",
    models: [
      { id: "deepseek-chat", name: "DeepSeek Chat", description: "通用对话模型", contextWindow: 65536 },
      { id: "deepseek-coder", name: "DeepSeek Coder", description: "专业编程模型", contextWindow: 65536 },
    ],
  },
  {
    id: "gemini",
    name: "Google Gemini",
    logo: "🟡",
    provider: "openai_compatible",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    apiKeyPrefix: "AIza-",
    docsUrl: "https://aistudio.google.com/apikey",
    models: [
      { id: "gemini-2.5-flash", name: "Gemini 2.5 Flash", description: "多模态推理", contextWindow: 1048576 },
      { id: "gemini-2.5-pro", name: "Gemini 2.5 Pro", description: "最强多模态", contextWindow: 1048576 },
    ],
  },
];

const TOKEN_LIMIT_PATTERNS: Array<[RegExp, number]> = [
  [/^gpt-4\.1/, 1047576],
  [/^gpt-4o/, 128000],
  [/^gpt-4-turbo/, 128000],
  [/^gpt-4(?!o|\.1|-turbo)/, 8192],
  [/^gpt-3\.5/, 16385],
  [/^o[134]-/, 200000],
  [/^claude-.*-4/, 200000],
  [/^claude-3/, 200000],
  [/^claude-2/, 100000],
  [/^gemini-2/, 1048576],
  [/^gemini-1\.5-pro/, 2097152],
  [/^gemini-1\.5-flash/, 1048576],
  [/^qwen-max/, 32768],
  [/^qwen-(plus|turbo|long)/, 131072],
  [/^qwen3-/, 131072],
  [/^deepseek-(chat|coder|reasoner)/, 65536],
  [/^deepseek-v3/, 65536],
  [/^llama-3\.3/, 131072],
  [/^llama-3\.1/, 131072],
  [/^llama-3(?!\.1|\.3)/, 8192],
  [/^mistral-large/, 131072],
  [/^mixtral/, 32768],
  [/^yi-/, 200000],
  [/^glm-4/, 131072],
];

/**
 * Infer context window size from model ID.
 * Falls back to 8192 if no pattern matches.
 */
export function inferContextWindow(modelId: string): number {
  for (const [pattern, limit] of TOKEN_LIMIT_PATTERNS) {
    if (pattern.test(modelId)) return limit;
  }
  return 8192;
}

/**
 * Find a provider preset by ID.
 */
export function getProviderPreset(providerId: string): ProviderPreset | undefined {
  return PROVIDER_PRESETS.find((p) => p.id === providerId);
}

/**
 * Find a model's context window from presets, falling back to inference.
 */
export function getModelContextWindow(modelId: string): number {
  for (const provider of PROVIDER_PRESETS) {
    const model = provider.models.find((m) => m.id === modelId);
    if (model) return model.contextWindow;
  }
  return inferContextWindow(modelId);
}

/**
 * Build a flat lookup of all preset models keyed by model ID.
 */
export function getAllPresetModels(): Map<string, ModelPreset & { providerId: string; provider: string; baseUrl: string }> {
  const map = new Map<string, ModelPreset & { providerId: string; provider: string; baseUrl: string }>();
  for (const p of PROVIDER_PRESETS) {
    for (const m of p.models) {
      map.set(m.id, { ...m, providerId: p.id, provider: p.provider, baseUrl: p.baseUrl });
    }
  }
  return map;
}

export interface ResolvedModelConfig {
  key: string;
  provider: string;
  model: string;
  baseUrl: string;
  apiKey: string;
  contextWindow: number;
  temperature: number;
  maxConcurrent: number;
  timeoutSecs: number;
  source: "env" | "config" | "ui";
}

/**
 * Snapshot of the full model config state, used for rollback on switch failure.
 */
export interface ModelConfigSnapshot {
  timestamp: number;
  models: Record<string, Record<string, unknown>>;
  credentials: Record<string, Record<string, unknown>>;
}

const SNAPSHOT_STACK_LIMIT = 5;
let snapshotStack: ModelConfigSnapshot[] = [];

/**
 * Resolve model configuration from multiple sources with priority:
 * 1. Environment variables (highest)
 * 2. Config file
 * 3. UI defaults (lowest)
 *
 * Environment variable naming: FASTCLAW_<KEY>_API_KEY, FASTCLAW_<KEY>_BASE_URL, etc.
 */
export function resolveModelConfig(
  key: string,
  configValue: Record<string, unknown> | null,
  credentialValue: Record<string, unknown> | null,
): ResolvedModelConfig {
  const envPrefix = `FASTCLAW_${key.toUpperCase().replace(/-/g, "_")}`;

  const envApiKey = typeof globalThis !== "undefined"
    ? (globalThis as Record<string, unknown>)[`${envPrefix}_API_KEY`] as string | undefined
    : undefined;
  const envBaseUrl = typeof globalThis !== "undefined"
    ? (globalThis as Record<string, unknown>)[`${envPrefix}_BASE_URL`] as string | undefined
    : undefined;

  const cfgProvider = (configValue?.provider as string) ?? "openai_compatible";
  const cfgModel = (configValue?.model as string) ?? "";
  const cfgBaseUrl = (configValue?.baseUrl as string) ?? "";
  const cfgApiKey = (credentialValue?.apiKey as string) ?? "";
  const cfgContextWindow = (configValue?.contextWindow as number) ?? 0;

  const finalApiKey = envApiKey || cfgApiKey;
  const finalBaseUrl = envBaseUrl || cfgBaseUrl;
  const finalContextWindow = cfgContextWindow > 0 ? cfgContextWindow : inferContextWindow(cfgModel);

  return {
    key,
    provider: cfgProvider,
    model: cfgModel,
    baseUrl: finalBaseUrl,
    apiKey: finalApiKey,
    contextWindow: finalContextWindow,
    temperature: (configValue?.temperature as number) ?? 0,
    maxConcurrent: (configValue?.maxConcurrent as number) ?? 10,
    timeoutSecs: (configValue?.timeoutSecs as number) ?? 120,
    source: envApiKey ? "env" : cfgApiKey ? "config" : "ui",
  };
}

/**
 * Take a snapshot of the current model configuration for rollback.
 * Call this before applying a model switch.
 */
export function takeModelSnapshot(
  models: Record<string, Record<string, unknown>>,
  credentials: Record<string, Record<string, unknown>>,
): ModelConfigSnapshot {
  const snapshot: ModelConfigSnapshot = {
    timestamp: Date.now(),
    models: structuredClone(models),
    credentials: structuredClone(credentials),
  };
  snapshotStack.push(snapshot);
  if (snapshotStack.length > SNAPSHOT_STACK_LIMIT) {
    snapshotStack = snapshotStack.slice(-SNAPSHOT_STACK_LIMIT);
  }
  return snapshot;
}

/**
 * Pop the most recent snapshot for rollback. Returns `null` if none available.
 */
export function popModelSnapshot(): ModelConfigSnapshot | null {
  return snapshotStack.pop() ?? null;
}

/**
 * Peek at the most recent snapshot without consuming it.
 */
export function peekModelSnapshot(): ModelConfigSnapshot | null {
  return snapshotStack.length > 0 ? snapshotStack[snapshotStack.length - 1] : null;
}

/**
 * Check if rollback snapshots are available.
 */
export function hasModelSnapshots(): boolean {
  return snapshotStack.length > 0;
}

/**
 * Clear all saved snapshots (e.g., after a confirmed successful switch).
 */
export function clearModelSnapshots(): void {
  snapshotStack = [];
}
