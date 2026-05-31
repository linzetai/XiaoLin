/**
 * Shared model utilities — connection testing and config persistence.
 *
 * Used by both the OnboardingWizard and the Settings ModelTab to avoid
 * duplicating the test / save logic in two places.
 */

import { useState, useCallback, useRef } from "react";
import * as api from "./api";
import * as transport from "./transport";
import { inferContextWindow } from "./model-registry";

// ─── Types ───────────────────────────────────────────────────────────

export type TestStatus = "idle" | "testing" | "success" | "error";

export interface ModelFormData {
  key: string;
  provider: string;
  model: string;
  baseUrl: string;
  apiKey: string;
  temperature: number;
  maxConcurrent: number;
  timeoutSecs: number;
  contextWindow: number;
}

export const DEFAULT_MODEL_FORM: ModelFormData = {
  key: "",
  provider: "openai_compatible",
  model: "",
  baseUrl: "",
  apiKey: "",
  temperature: 0,
  maxConcurrent: 10,
  timeoutSecs: 120,
  contextWindow: 0,
};

// ─── useModelTest ────────────────────────────────────────────────────

export interface UseModelTestReturn {
  testStatus: TestStatus;
  testMsg: string;
  runTest: (baseUrl: string, apiKey: string, model?: string) => Promise<void>;
  resetTest: () => void;
}

/**
 * Reusable hook for testing a model connection.
 * Handles both Tauri IPC and browser-based fetch paths.
 */
export function useModelTest(): UseModelTestReturn {
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [testMsg, setTestMsg] = useState("");
  const abortRef = useRef<AbortController | null>(null);

  const runTest = useCallback(async (baseUrl: string, apiKey: string, model?: string) => {
    const url = baseUrl.replace(/\/+$/, "");
    if (!url) { setTestStatus("error"); setTestMsg("请填写 Base URL"); return; }
    if (!apiKey || apiKey.startsWith("***")) {
      setTestStatus("error"); setTestMsg("请填写有效的 API Key"); return;
    }

    abortRef.current?.abort();
    const ac = new AbortController();
    abortRef.current = ac;

    setTestStatus("testing");
    setTestMsg("");

    try {
      if (transport.isTauri) {
        await transport.testModelConnection(url, apiKey, model || undefined);
        if (ac.signal.aborted) return;
        setTestStatus("success");
        setTestMsg("连接成功");
      } else {
        const resp = await fetch(`${url}/models`, {
          method: "GET",
          headers: { Authorization: `Bearer ${apiKey}` },
          signal: AbortSignal.timeout(10000),
        });
        if (ac.signal.aborted) return;
        if (resp.ok) {
          setTestStatus("success");
          setTestMsg("连接成功");
        } else {
          const body = await resp.text().catch(() => "");
          setTestStatus("error");
          setTestMsg(`HTTP ${resp.status}${body ? `: ${body.slice(0, 80)}` : ""}`);
        }
      }
    } catch (err: unknown) {
      if (ac.signal.aborted) return;
      setTestStatus("error");
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : "连接失败";
      setTestMsg(msg.length > 120 ? msg.slice(0, 120) + "…" : msg);
    }
  }, []);

  const resetTest = useCallback(() => {
    abortRef.current?.abort();
    setTestStatus("idle");
    setTestMsg("");
  }, []);

  return { testStatus, testMsg, runTest, resetTest };
}

// ─── saveModelConfig ─────────────────────────────────────────────────

export interface SaveModelOpts {
  key: string;
  provider: string;
  model: string;
  baseUrl: string;
  apiKey: string;
  temperature?: number;
  maxConcurrent?: number;
  timeoutSecs?: number;
  contextWindow?: number;
}

/**
 * Persist a model + credential entry to the config store.
 * Emits the `fastclaw:models-updated` event on success.
 */
export async function saveModelConfig(opts: SaveModelOpts): Promise<void> {
  const {
    key, provider, model, baseUrl, apiKey,
    temperature = 0, maxConcurrent = 10, timeoutSecs = 120,
    contextWindow: explicitCw,
  } = opts;

  const contextWindow = (explicitCw && explicitCw > 0)
    ? explicitCw
    : inferContextWindow(model);

  const existingModels = ((await api.getConfig("models")) as Record<string, unknown> | null) ?? {};
  const mv = (existingModels as { value?: Record<string, unknown> }).value ?? existingModels;

  const newModels = {
    ...(typeof mv === "object" && mv !== null ? mv : {}),
    [key]: { provider, model, baseUrl, temperature, maxConcurrent, timeoutSecs, contextWindow },
  };
  await api.setConfig("models", newModels);

  if (apiKey || baseUrl) {
    const existingCreds = ((await api.getConfig("credentials")) as Record<string, unknown> | null) ?? {};
    const cv = (existingCreds as { value?: Record<string, unknown> }).value ?? existingCreds;
    const newCreds = {
      ...(typeof cv === "object" && cv !== null ? cv : {}),
      [key]: { apiKey, baseUrl },
    };
    await api.setConfig("credentials", newCreds);
  }

  await api.setConfig("agents", { defaults: { model: `${key}/${model}` } });

  window.dispatchEvent(new CustomEvent("fastclaw:models-updated"));
}
