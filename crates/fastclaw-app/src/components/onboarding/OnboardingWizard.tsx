import { useState, useEffect, useCallback, useReducer } from "react";
import {
  ChevronRight, ChevronLeft, Bot, MessageSquare, Clock, Search,
  Wrench, Settings, Sparkles, Eye, EyeOff, Zap, CheckCircle, XCircle,
  ArrowRight,
} from "lucide-react";
import { ClawIcon } from "../layout/ClawIcon";
import * as transport from "../../lib/transport";
import { PROVIDER_PRESETS, type ProviderPreset } from "../../lib/model-registry";
import { useModelTest, saveModelConfig, type TestStatus } from "../../lib/model-utils";

// ─── Wizard Types ────────────────────────────────────────────────────

type WizardStep = "welcome" | "model" | "features" | "done";

interface ModelState {
  key: string;
  provider: string;
  model: string;
  baseUrl: string;
  apiKey: string;
  contextWindow: number;
  selectedProvider: ProviderPreset | null;
  subStep: 1 | 2 | 3;
  saving: boolean;
  saved: boolean;
}

type ModelAction =
  | { type: "SET_FIELD"; field: keyof ModelState; value: unknown }
  | { type: "SELECT_PROVIDER"; provider: ProviderPreset }
  | { type: "SELECT_CUSTOM" }
  | { type: "SELECT_MODEL"; modelId: string; contextWindow: number }
  | { type: "GO_PREV_SUB" }
  | { type: "SET_SAVING"; saving: boolean }
  | { type: "SET_SAVED" };

const INITIAL_MODEL_STATE: ModelState = {
  key: "",
  provider: "openai_compatible",
  model: "",
  baseUrl: "",
  apiKey: "",
  contextWindow: 0,
  selectedProvider: null,
  subStep: 1,
  saving: false,
  saved: false,
};

function modelReducer(state: ModelState, action: ModelAction): ModelState {
  switch (action.type) {
    case "SET_FIELD":
      return { ...state, [action.field]: action.value };
    case "SELECT_PROVIDER":
      return {
        ...state,
        selectedProvider: action.provider,
        provider: action.provider.provider,
        baseUrl: action.provider.baseUrl,
        key: action.provider.id,
        subStep: 2,
      };
    case "SELECT_CUSTOM":
      return {
        ...state,
        selectedProvider: null,
        provider: "openai_compatible",
        baseUrl: "",
        key: "",
        subStep: 3,
      };
    case "SELECT_MODEL":
      return {
        ...state,
        model: action.modelId,
        contextWindow: action.contextWindow,
        subStep: 3,
      };
    case "GO_PREV_SUB":
      return {
        ...state,
        subStep: (state.subStep > 1 ? state.subStep - 1 : 1) as 1 | 2 | 3,
      };
    case "SET_SAVING":
      return { ...state, saving: action.saving };
    case "SET_SAVED":
      return { ...state, saving: false, saved: true };
  }
}

// ─── Style Constants ─────────────────────────────────────────────────

const inputCls = "w-full rounded-[8px] px-3 py-2.5 text-[13px] outline-none transition-all focus:ring-2 focus:ring-[var(--tint)]";
const inputStyle: React.CSSProperties = {
  background: "var(--bg-base)",
  color: "var(--fill-primary)",
  border: "0.5px solid var(--separator-opaque)",
};
const labelCls = "mb-1.5 block text-[11px] font-semibold tracking-wide uppercase";
const labelStyle: React.CSSProperties = { color: "var(--fill-tertiary)" };

// ─── Wizard Root ─────────────────────────────────────────────────────

export interface OnboardingWizardProps {
  onComplete: () => void;
}

export function OnboardingWizard({ onComplete }: OnboardingWizardProps) {
  const [step, setStep] = useState<WizardStep>("welcome");
  const [fadeClass, setFadeClass] = useState("ob-fade-in");
  const [ms, dispatch] = useReducer(modelReducer, INITIAL_MODEL_STATE);

  const goTo = useCallback((next: WizardStep) => {
    setFadeClass("ob-fade-out");
    setTimeout(() => {
      setStep(next);
      setFadeClass("ob-fade-in");
    }, 250);
  }, []);

  const handleImportClick = useCallback(async () => {
    try {
      if (!transport.isTauri) {
        alert("迁移功能仅在桌面应用中可用");
        return;
      }
      const selected = await window.__TAURI__.dialog.open({
        filters: [{ name: "FastClaw Migration Files", extensions: ["json", "fcdata"] }],
        multiple: false,
      });
      if (!selected) return;
      const fileContents = await window.__TAURI__.fs.readBinaryFile(selected);
      await transport.importData(new Uint8Array(fileContents), {
        merge: false,
        overwriteConfig: true,
        overwriteAgents: true,
        overwriteSessions: true,
        overwriteSkills: true,
      });
      goTo("model");
    } catch (error) {
      console.error("导入失败:", error);
      alert("导入失败: " + (error as Error).message);
    }
  }, [goTo]);

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center"
      style={{ background: "var(--bg-primary)" }}
    >
      <div className={`w-full max-w-[560px] px-6 ${fadeClass}`}>
        {step === "welcome" && (
          <WelcomeStep onNext={() => goTo("model")} onImport={handleImportClick} />
        )}
        {step === "model" && (
          <ModelStep
            state={ms}
            dispatch={dispatch}
            onNext={() => goTo("features")}
            onPrev={() => goTo("welcome")}
          />
        )}
        {step === "features" && (
          <FeaturesStep onNext={() => goTo("done")} onPrev={() => goTo("model")} />
        )}
        {step === "done" && <DoneStep onComplete={onComplete} />}
      </div>

      <div className="fixed bottom-8 left-1/2 flex -translate-x-1/2 items-center gap-2">
        {(["welcome", "model", "features", "done"] as WizardStep[]).map((s) => (
          <div
            key={s}
            className={`ob-dot ${step === s ? "ob-dot-active" : ""}`}
            style={{
              background: step === s ? "var(--fill-primary)" : "var(--fill-quaternary)",
            }}
          />
        ))}
      </div>
    </div>
  );
}

// ─── Step 1: Welcome ─────────────────────────────────────────────────

function WelcomeStep({ onNext, onImport }: { onNext: () => void; onImport: () => void }) {
  return (
    <div className="flex flex-col items-center text-center">
      <div style={{ animation: "scale-in 0.5s ease-out" }}>
        <ClawIcon size={72} />
      </div>
      <h1
        className="mt-6 text-[28px] font-bold tracking-tight"
        style={{ color: "var(--fill-primary)" }}
      >
        欢迎使用 FastClaw
      </h1>
      <p
        className="mt-3 max-w-[380px] text-[15px] leading-relaxed"
        style={{ color: "var(--fill-secondary)" }}
      >
        FastClaw 是一个本地优先的 AI Agent 平台。
        <br />
        支持多 Agent 管理、工具调用、定时任务和联网搜索。
      </p>
      <p className="mt-6 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
        如何开始？
      </p>
      <div className="mt-4 flex w-full max-w-[280px] flex-col gap-3">
        <button
          onClick={onNext}
          className="flex cursor-pointer items-center justify-center gap-2 rounded-full px-6 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          新手配置
          <Settings size={16} strokeWidth={2} />
        </button>
        <button
          onClick={onImport}
          className="flex cursor-pointer items-center justify-center gap-2 rounded-full px-6 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{
            background: "var(--bg-elevated)",
            color: "var(--fill-primary)",
            border: "1px solid var(--separator-opaque)",
          }}
        >
          导入现有配置
          <ArrowRight size={16} strokeWidth={2} />
        </button>
      </div>
    </div>
  );
}

// ─── Step 2: Model Setup (orchestrator) ──────────────────────────────

function ModelStep({
  state,
  dispatch,
  onNext,
  onPrev,
}: {
  state: ModelState;
  dispatch: React.Dispatch<ModelAction>;
  onNext: () => void;
  onPrev: () => void;
}) {
  const { testStatus, testMsg, runTest, resetTest } = useModelTest();

  const handleSave = useCallback(async () => {
    dispatch({ type: "SET_SAVING", saving: true });
    try {
      await saveModelConfig({
        key: state.key,
        provider: state.provider,
        model: state.model,
        baseUrl: state.baseUrl,
        apiKey: state.apiKey,
        contextWindow: state.contextWindow,
      });
      dispatch({ type: "SET_SAVED" });
    } catch {
      resetTest();
      dispatch({ type: "SET_SAVING", saving: false });
    }
  }, [state, dispatch, resetTest]);

  if (state.saved) {
    return <ModelSavedConfirmation model={state.model} onNext={onNext} />;
  }

  return (
    <div className="relative">
      {/* Back button */}
      <div className="absolute -top-12 left-0 flex">
        <button
          onClick={state.subStep > 1 ? () => dispatch({ type: "GO_PREV_SUB" }) : onPrev}
          className="flex cursor-pointer items-center gap-1 text-[13px] font-medium transition-colors hover:opacity-80"
          style={{ color: "var(--fill-tertiary)" }}
        >
          <ChevronLeft size={16} />
          {state.subStep > 1 ? "上一步" : "返回"}
        </button>
      </div>

      <div className="mb-5 text-center">
        <h2 className="text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
          添加你的第一个模型
        </h2>
        <p className="mt-1.5 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
          选择 AI 提供商，配置 API 密钥即可开始
        </p>
      </div>

      <SubStepBreadcrumb current={state.subStep} />

      {state.subStep === 1 && (
        <ProviderSelectStep dispatch={dispatch} />
      )}
      {state.subStep === 2 && state.selectedProvider && (
        <ModelSelectStep provider={state.selectedProvider} dispatch={dispatch} />
      )}
      {state.subStep === 3 && (
        <ApiKeyConfigStep
          state={state}
          dispatch={dispatch}
          testStatus={testStatus}
          testMsg={testMsg}
          onTest={() => runTest(state.baseUrl, state.apiKey, state.model)}
          onSave={handleSave}
        />
      )}
    </div>
  );
}

// ─── Sub-step Breadcrumb ─────────────────────────────────────────────

function SubStepBreadcrumb({ current }: { current: 1 | 2 | 3 }) {
  const labels = ["选择提供商", "选择模型", "配置密钥"];
  return (
    <div className="mb-5 flex items-center justify-center gap-1">
      {[1, 2, 3].map((s) => (
        <div key={s} className="flex items-center gap-1">
          <div
            className={`flex h-5 w-5 items-center justify-center rounded-full text-[10px] font-bold ${
              s <= current ? "" : "opacity-30"
            }`}
            style={{
              background:
                s < current
                  ? "var(--green)"
                  : s === current
                    ? "var(--fill-primary)"
                    : "var(--fill-quaternary)",
              color: s <= current ? (s < current ? "#fff" : "var(--fill-inverse)") : "var(--fill-inverse)",
            }}
          >
            {s < current ? <CheckCircle size={10} strokeWidth={3} /> : s}
          </div>
          <span
            className={`text-[11px] ${s <= current ? "" : "opacity-30"}`}
            style={{ color: s <= current ? "var(--fill-primary)" : "var(--fill-tertiary)" }}
          >
            {labels[s - 1]}
          </span>
          {s < 3 && (
            <div className="ml-1 mr-1 h-px w-4" style={{ background: "var(--separator)" }} />
          )}
        </div>
      ))}
    </div>
  );
}

// ─── Sub-step 1: Provider Selection ──────────────────────────────────

function ProviderSelectStep({ dispatch }: { dispatch: React.Dispatch<ModelAction> }) {
  return (
    <div
      className="overflow-hidden rounded-[var(--radius-md)]"
      style={{
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator-opaque)",
        boxShadow: "var(--shadow-md)",
      }}
    >
      <div className="p-4">
        <div className="grid grid-cols-2 gap-2.5">
          {PROVIDER_PRESETS.map((p) => (
            <div
              key={p.id}
              className="cursor-pointer rounded-[var(--radius-sm)] border p-3.5 transition-all hover:scale-[1.01]"
              style={{ background: "var(--bg-base)", borderColor: "var(--separator-opaque)" }}
              onClick={() => dispatch({ type: "SELECT_PROVIDER", provider: p })}
            >
              <div className="flex items-center gap-2.5">
                <span className="text-[20px] leading-none">{p.logo}</span>
                <div>
                  <div className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
                    {p.name}
                  </div>
                  <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                    {p.models.length} 个模型
                  </div>
                </div>
              </div>
            </div>
          ))}
          {/* Custom provider */}
          <div
            className="flex cursor-pointer items-center justify-center gap-2 rounded-[var(--radius-sm)] border border-dashed p-3.5 transition-all hover:scale-[1.01]"
            style={{ borderColor: "var(--separator)" }}
            onClick={() => dispatch({ type: "SELECT_CUSTOM" })}
          >
            <Settings size={16} style={{ color: "var(--fill-tertiary)" }} />
            <span className="text-[13px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              自定义
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Sub-step 2: Model Selection ─────────────────────────────────────

function ModelSelectStep({
  provider,
  dispatch,
}: {
  provider: ProviderPreset;
  dispatch: React.Dispatch<ModelAction>;
}) {
  return (
    <div
      className="overflow-hidden rounded-[var(--radius-md)]"
      style={{
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator-opaque)",
        boxShadow: "var(--shadow-md)",
      }}
    >
      <div className="px-4 pb-2 pt-4">
        <div className="mb-3 flex items-center gap-2">
          <span className="text-[18px]">{provider.logo}</span>
          <span className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {provider.name}
          </span>
        </div>
        <div className="space-y-1.5">
          {provider.models.map((m) => (
            <div
              key={m.id}
              className="flex cursor-pointer items-center justify-between rounded-[var(--radius-sm)] px-3 py-2.5 transition-all hover:scale-[1.01]"
              style={{ background: "var(--bg-base)", border: "0.5px solid var(--separator-opaque)" }}
              onClick={() =>
                dispatch({ type: "SELECT_MODEL", modelId: m.id, contextWindow: m.contextWindow })
              }
            >
              <div>
                <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
                  {m.name}
                </div>
                <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                  {m.description}
                </div>
              </div>
              <ChevronRight size={14} style={{ color: "var(--fill-tertiary)" }} />
            </div>
          ))}
        </div>
      </div>
      {provider.docsUrl && (
        <div
          className="flex items-center gap-2 px-4 py-2.5"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          <Sparkles size={12} style={{ color: "var(--tint)" }} />
          <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            还没有 API Key？
          </span>
          <a
            href={provider.docsUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="text-[11px] font-medium underline"
            style={{ color: "var(--tint)" }}
          >
            前往获取
          </a>
        </div>
      )}
    </div>
  );
}

// ─── Sub-step 3: API Key + Save ──────────────────────────────────────

function ApiKeyConfigStep({
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
  const canSave = state.key.trim() && state.model.trim();

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
        {/* Provider label (read-only) */}
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

        {/* Name */}
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

        {/* Model */}
        <div>
          <label htmlFor="ob-model" className={labelCls} style={labelStyle}>模型名称</label>
          <input
            id="ob-model"
            value={state.model}
            onChange={(e) => setField("model", e.target.value)}
            placeholder="如 gpt-4o / qwen-max / claude-sonnet-4-20250514"
            className={inputCls}
            style={inputStyle}
          />
        </div>

        {/* Base URL */}
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

        {/* API Key */}
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
                {showApiKey ? <EyeOff size={13} /> : <Eye size={13} />}
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
                  <Zap size={11} className="animate-pulse" />
                ) : testStatus === "success" ? (
                  <CheckCircle size={11} />
                ) : testStatus === "error" ? (
                  <XCircle size={11} />
                ) : (
                  <Zap size={11} />
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

        {/* Advanced (collapsed) */}
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

      {/* Footer */}
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
            size={12}
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

// ─── Model Saved Confirmation ────────────────────────────────────────

function ModelSavedConfirmation({ model, onNext }: { model: string; onNext: () => void }) {
  return (
    <div className="flex flex-col items-center text-center">
      <div
        className="flex h-16 w-16 items-center justify-center rounded-full"
        style={{ background: "color-mix(in srgb, var(--green) 12%, transparent)" }}
      >
        <CheckCircle size={32} strokeWidth={1.5} style={{ color: "var(--green)" }} />
      </div>
      <h2 className="mt-5 text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
        模型配置完成
      </h2>
      <p className="mt-2 text-[14px]" style={{ color: "var(--fill-secondary)" }}>
        <span className="font-medium">{model || "模型"}</span> 已就绪，接下来了解一下 FastClaw 的核心功能
      </p>
      <button
        onClick={onNext}
        className="mt-8 flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
        style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
      >
        了解功能 <ArrowRight size={16} strokeWidth={2} />
      </button>
    </div>
  );
}

// ─── Step 3: Feature Tour ────────────────────────────────────────────

const FEATURES = [
  { icon: Bot, cssColor: "var(--tint)", title: "多 Agent 管理", desc: "创建和管理多个 AI Agent，各自独立配置模型、人设和工具" },
  { icon: Wrench, cssColor: "var(--orange, #ED8936)", title: "工具调用", desc: "Agent 可调用内置工具和 MCP 服务器扩展能力" },
  { icon: Clock, cssColor: "var(--purple, #B794F4)", title: "定时任务", desc: "通过 Cron 表达式设置周期任务，自动化日常工作" },
  { icon: Search, cssColor: "var(--green)", title: "联网搜索", desc: "Agent 可实时搜索互联网获取最新信息" },
  { icon: MessageSquare, cssColor: "var(--blue, #63B3ED)", title: "多轮对话", desc: "支持上下文感知的多轮对话，自动管理会话历史" },
  { icon: Sparkles, cssColor: "var(--yellow, #F6E05E)", title: "技能系统", desc: "通过技能扩展 Agent 的专业能力，支持自定义和社区共享" },
];

function FeaturesStep({ onNext, onPrev }: { onNext: () => void; onPrev: () => void }) {
  return (
    <div className="relative">
      <div className="absolute -top-12 left-0 flex">
        <button
          onClick={onPrev}
          className="flex cursor-pointer items-center gap-1 text-[13px] font-medium transition-colors hover:opacity-80"
          style={{ color: "var(--fill-tertiary)" }}
        >
          <ChevronLeft size={16} />
          返回
        </button>
      </div>

      <div className="mb-6 text-center">
        <h2 className="text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
          核心功能一览
        </h2>
        <p className="mt-2 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
          FastClaw 的主要能力
        </p>
      </div>

      <div className="grid grid-cols-2 gap-3">
        {FEATURES.map((f) => (
          <div
            key={f.title}
            className="rounded-[var(--radius-sm)] p-4 transition-all duration-200 hover:scale-[1.01]"
            style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
          >
            <div
              className="mb-3 flex h-9 w-9 items-center justify-center rounded-[8px]"
              style={{ background: `color-mix(in srgb, ${f.cssColor} 10%, transparent)` }}
            >
              <f.icon size={18} strokeWidth={1.5} style={{ color: f.cssColor }} />
            </div>
            <h3 className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              {f.title}
            </h3>
            <p
              className="mt-1 text-[11px] leading-relaxed"
              style={{ color: "var(--fill-secondary)" }}
            >
              {f.desc}
            </p>
          </div>
        ))}
      </div>

      <div className="mt-6 flex justify-end">
        <button
          onClick={onNext}
          className="flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          开始使用
          <ArrowRight size={16} strokeWidth={2} />
        </button>
      </div>
    </div>
  );
}

// ─── Step 4: Done ────────────────────────────────────────────────────

function DoneStep({ onComplete }: { onComplete: () => void }) {
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const t = setTimeout(() => setReady(true), 1200);
    return () => clearTimeout(t);
  }, []);

  return (
    <div className="flex flex-col items-center text-center">
      <div
        className="flex h-16 w-16 items-center justify-center rounded-full"
        style={{
          background: "color-mix(in srgb, var(--green) 12%, transparent)",
          animation: "scale-in 0.4s ease-out",
        }}
      >
        <Sparkles size={32} strokeWidth={1.5} style={{ color: "var(--green)" }} />
      </div>
      <h2 className="mt-5 text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
        一切就绪
      </h2>
      <p className="mt-2 text-[14px]" style={{ color: "var(--fill-secondary)" }}>
        准备好和你的 Agent 开始对话了
      </p>

      <div className="mt-6 flex gap-3">
        {ready && (
          <button
            onClick={onComplete}
            className="flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
            style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
          >
            开始使用
            <ArrowRight size={16} strokeWidth={2} />
          </button>
        )}
      </div>
    </div>
  );
}
