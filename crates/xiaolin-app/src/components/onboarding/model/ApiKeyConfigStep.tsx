import { useState } from "react";
import {
  ChevronRight, Eye, EyeOff, Zap, CheckCircle, XCircle,
} from "lucide-react";
import { ICON } from "../../../lib/ui-tokens";
import { inferContextWindow } from "../../../lib/model-registry";
import type { TestStatus } from "../../../lib/model-utils";
import { inputCls, inputStyle, labelCls, labelStyle } from "../shared";
import type { ModelState, ModelAction } from "./model-state";

export function ApiKeyConfigStep({
  state,
  dispatch,
  testStatus,
  testMsg,
  onTest,
  onSave,
}: {
  state: ModelState;
  dispatch: React.Dispatch<ModelAction>;
  testStatus: TestStatus;
  testMsg: string;
  onTest: () => void;
  onSave: () => void;
}) {
  const [showApiKey, setShowApiKey] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const canSave = state.key.trim() && state.model.trim() && state.contextWindow >= 1024;

  const setField = (field: keyof ModelState, value: unknown) =>
    dispatch({ type: "SET_FIELD", field, value });

  return (
    <div
      className="overflow-hidden rounded-[var(--radius-md)]"
      style={{
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator-opaque)",
        boxShadow: "var(--shadow-md)",
      }}
    >
      <div className="space-y-3.5 p-5">
        {state.selectedProvider && (
          <div>
            <label className={labelCls} style={labelStyle}>提供商</label>
            <div
              className="flex items-center gap-2 rounded-[8px] px-3 py-2.5 text-[13px]"
              style={{
                background: "var(--bg-base)",
                color: "var(--fill-primary)",
                border: "0.5px solid var(--separator-opaque)",
              }}
            >
              <span className="text-[16px]">{state.selectedProvider.logo}</span>
              <span className="font-medium">{state.selectedProvider.name}</span>
            </div>
          </div>
        )}

        <div>
          <label htmlFor="ob-key" className={labelCls} style={labelStyle}>名称</label>
          <input
            id="ob-key"
            value={state.key}
            onChange={(e) => setField("key", e.target.value)}
            placeholder="如 openai / qwen"
            className={inputCls}
            style={inputStyle}
          />
        </div>

        <div>
          <label htmlFor="ob-model" className={labelCls} style={labelStyle}>模型名称</label>
          <input
            id="ob-model"
            value={state.model}
            onChange={(e) => {
              const modelId = e.target.value;
              setField("model", modelId);
              if (state.contextWindow === 0 && modelId) {
                const inferred = inferContextWindow(modelId);
                if (inferred !== 8192) setField("contextWindow", inferred);
              }
            }}
            placeholder="如 gpt-4o / qwen-max / claude-sonnet-4-20250514"
            className={inputCls}
            style={inputStyle}
          />
        </div>

        <div>
          <label htmlFor="ob-baseurl" className={labelCls} style={labelStyle}>Base URL</label>
          <input
            id="ob-baseurl"
            value={state.baseUrl}
            onChange={(e) => setField("baseUrl", e.target.value)}
            placeholder="如 https://api.openai.com/v1"
            className={inputCls}
            style={inputStyle}
          />
        </div>

        <div>
          <label htmlFor="ob-apikey" className={labelCls} style={labelStyle}>
            API Key
            {state.selectedProvider?.apiKeyPrefix && (
              <span
                className="ml-1.5 font-normal normal-case"
                style={{ color: "var(--fill-quaternary)" }}
              >
                (以 {state.selectedProvider.apiKeyPrefix} 开头)
              </span>
            )}
          </label>
          <div className="relative">
            <input
              id="ob-apikey"
              type={showApiKey ? "text" : "password"}
              value={state.apiKey}
              onChange={(e) => setField("apiKey", e.target.value)}
              placeholder={state.selectedProvider?.apiKeyPrefix || "sk-..."}
              className={`${inputCls} pr-20`}
              style={inputStyle}
            />
            <div className="absolute right-2 top-1/2 flex -translate-y-1/2 gap-1">
              <button
                onClick={() => setShowApiKey(!showApiKey)}
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-md transition-colors hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--fill-tertiary)" }}
              >
                {showApiKey ? <EyeOff {...ICON.sm} /> : <Eye {...ICON.sm} />}
              </button>
              <button
                onClick={onTest}
                disabled={testStatus === "testing"}
                className="flex h-7 cursor-pointer items-center gap-1 rounded-md px-2 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
                style={{
                  color:
                    testStatus === "success"
                      ? "var(--green)"
                      : testStatus === "error"
                        ? "var(--red)"
                        : "var(--fill-secondary)",
                }}
              >
                {testStatus === "testing" ? (
                  <Zap {...ICON.sm} className="animate-pulse" />
                ) : testStatus === "success" ? (
                  <CheckCircle {...ICON.sm} />
                ) : testStatus === "error" ? (
                  <XCircle {...ICON.sm} />
                ) : (
                  <Zap {...ICON.sm} />
                )}
                测试
              </button>
            </div>
          </div>
          {testMsg && (
            <p
              className="mt-1.5 text-[11px]"
              style={{ color: testStatus === "success" ? "var(--green)" : "var(--red)" }}
            >
              {testMsg}
            </p>
          )}
        </div>

        <div>
          <label htmlFor="ob-ctx" className={labelCls} style={labelStyle}>
            上下文窗口 (tokens)
          </label>
          <input
            id="ob-ctx"
            type="number"
            min="1024"
            step="1024"
            value={state.contextWindow || ""}
            onChange={(e) => setField("contextWindow", parseInt(e.target.value) || 0)}
            placeholder="如 128000（选预设模型时自动填入）"
            className={inputCls}
            style={{
              ...inputStyle,
              borderColor: state.contextWindow > 0 ? undefined : "var(--fill-quaternary)",
            }}
          />
          <p className="mt-1 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
            {state.contextWindow > 0
              ? `当前 ${state.contextWindow.toLocaleString()} tokens`
              : "输入模型名可自动推断，也可手动填写"}
          </p>
        </div>

        {showAdvanced && (
          <div className="rounded-[8px] p-3" style={{ background: "var(--bg-base)" }}>
            <p className="mb-2 text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
              高级选项（可保持默认）
            </p>
            <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              Temperature: 0 · 并发: 10 · 超时: 120s
            </p>
          </div>
        )}
      </div>

      <div
        className="flex items-center justify-between px-5 py-3"
        style={{ borderTop: "0.5px solid var(--separator)" }}
      >
        <button
          onClick={() => setShowAdvanced(!showAdvanced)}
          className="flex cursor-pointer items-center gap-1 text-[12px] transition-colors hover:opacity-80"
          style={{ color: "var(--fill-tertiary)" }}
        >
          <ChevronRight
            {...ICON.sm}
            className={`transition-transform ${showAdvanced ? "rotate-90" : ""}`}
          />
          高级选项
        </button>
        <button
          onClick={onSave}
          disabled={!canSave || state.saving}
          className="cursor-pointer rounded-full px-6 py-2 text-[13px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          {state.saving ? "保存中..." : "保存模型"}
        </button>
      </div>
    </div>
  );
}
