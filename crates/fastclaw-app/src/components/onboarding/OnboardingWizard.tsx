import { useState, useEffect, useCallback, useRef } from "react";
import { ChevronRight, ChevronDown, ChevronLeft, Bot, MessageSquare, Clock, Search, Wrench, Settings, Sparkles, Eye, EyeOff, Zap, CheckCircle, XCircle, ArrowRight, Upload } from "lucide-react";
import { ClawIcon } from "../layout/ClawIcon";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { ImportChoiceStep } from "./ImportChoiceStep";

type Step = "welcome" | "model" | "features" | "done";

type ImportChoice = "new" | "import";

// 用于存储各步骤状态的类型
interface StepState {
  model: {
    key: string;
    provider: string;
    model: string;
    baseUrl: string;
    apiKey: string;
    showApiKey: boolean;
    showAdvanced: boolean;
    testStatus: TestStatus;
    testMsg: string;
    saving: boolean;
    saved: boolean;
  };
}

type TestStatus = "idle" | "testing" | "success" | "error";

export interface OnboardingWizardProps {
  onComplete: () => void;
}

export function OnboardingWizard({ onComplete }: OnboardingWizardProps) {
  const [step, setStep] = useState<Step | "import_choice">("welcome");
  const [importChoice, setImportChoice] = useState<ImportChoice | null>(null);
  const [fadeClass, setFadeClass] = useState("ob-fade-in");
  // 存储各步骤的状态
  const [stepStates, setStepStates] = useState<StepState>({
    model: {
      key: "",
      provider: "openai_compatible",
      model: "",
      baseUrl: "",
      apiKey: "",
      showApiKey: false,
      showAdvanced: false,
      testStatus: "idle",
      testMsg: "",
      saving: false,
      saved: false,
    }
  });

  const updateStepState = useCallback((newState: Partial<StepState>) => {
    setStepStates(prev => ({ ...prev, ...newState }));
  }, []);

  const goTo = useCallback((next: Step | "import_choice") => {
    setFadeClass("ob-fade-out");
    setTimeout(() => {
      setStep(next);
      setFadeClass("ob-fade-in");
    }, 250);
  }, []);

  const handleImportChoice = (choice: ImportChoice) => {
    setImportChoice(choice);
    if (choice === "import") {
      // 触发导入文件对话框
      handleImportClick();
    } else {
      goTo("model");
    }
  };

  const handleImportClick = async () => {
    try {
      // 调用 Tauri 命令来打开文件选择对话框并导入数据
      if (transport.isTauri) {
        // 使用 Tauri dialog 打开文件选择器
        const selected = await window.__TAURI__.dialog.open({
          filters: [{
            name: "FastClaw Migration Files",
            extensions: ["json", "fcdata"]
          }],
          multiple: false
        });
        
        if (selected) {
          // 读取文件内容
          const fileContents = await window.__TAURI__.fs.readBinaryFile(selected);
          
          // 使用导入功能
          await transport.importData(new Uint8Array(fileContents), {
            merge: false,
            overwriteConfig: true,
            overwriteAgents: true,
            overwriteSessions: true,
            overwriteSkills: true
          });
          
          // 导入成功后，跳转到下一个步骤
          goTo("model");
        }
      } else {
        alert("迁移功能仅在桌面应用中可用");
      }
    } catch (error) {
      console.error("导入失败:", error);
      alert("导入失败: " + (error as Error).message);
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center" style={{ background: "var(--bg-primary)" }}>
      <div className={`w-full max-w-[560px] px-6 ${fadeClass}`}>
        {step === "welcome" && <WelcomeStep onNext={() => handleImportChoice("new")} onImport={() => handleImportChoice("import")} />}
        {step === "import_choice" && <ImportChoiceStep onSelect={handleImportChoice} />}
        {step === "model" && <ModelStep 
          onNext={() => goTo("features")} 
          onPrev={() => goTo("welcome")} 
          stepStates={stepStates}
          updateStepState={updateStepState}
        />}
        {step === "features" && <FeaturesStep onNext={() => goTo("done")} onPrev={() => goTo("model")} />}
        {step === "done" && <DoneStep onComplete={onComplete} />}
      </div>

      <div className="fixed bottom-8 left-1/2 flex -translate-x-1/2 items-center gap-2">
        {step !== "import_choice" && (["welcome", "model", "features", "done"] as Step[]).map((s) => (
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

/* ━━━ Step 1: Welcome ━━━ */

function WelcomeStep({ onNext, onImport }: { onNext: () => void, onImport: () => void }) {
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
      <p
        className="mt-6 text-[13px]"
        style={{ color: "var(--fill-tertiary)" }}
      >
        如何开始？
      </p>
      <div className="mt-4 flex flex-col gap-3 w-full max-w-[280px]">
        <button
          onClick={onNext}
          className="flex cursor-pointer items-center justify-center gap-2 rounded-full px-6 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{
            background: "var(--fill-primary)",
            color: "var(--fill-inverse)",
          }}
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
            border: "1px solid var(--separator-opaque)"
          }}
        >
          导入现有配置
          <ArrowRight size={16} strokeWidth={2} />
        </button>
      </div>
    </div>
  );
}

/* ━━━ Step 2: Model Setup ━━━ */

// 预设模型配置
const PRESET_MODELS = [
  {
    id: "qwen-coding-plan",
    name: "通义千问 - 编程规划",
    provider: "openai_compatible",
    model: "qwen-max",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    description: "适合编程任务的模型配置"
  },
  {
    id: "qwen-api",
    name: "通义千问 - API",
    provider: "openai_compatible",
    model: "qwen-plus",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    description: "通用API模型配置"
  },
  {
    id: "gpt-4o",
    name: "GPT-4o",
    provider: "openai_compatible",
    model: "gpt-4o",
    baseUrl: "https://api.openai.com/v1",
    description: "OpenAI最新旗舰模型"
  },
  {
    id: "claude-sonnet",
    name: "Claude Sonnet 4",
    provider: "openai_compatible",
    model: "claude-sonnet-4-20250514",
    baseUrl: "https://api.anthropic.com/v1",
    description: "Anthropic高性能模型"
  }
];

function ModelStep({ onNext, onPrev, stepStates, updateStepState }: { 
  onNext: () => void, 
  onPrev: () => void,
  stepStates: StepState,
  updateStepState: (newState: Partial<StepState>) => void 
}) {
  const { model: modelState } = stepStates;
  const [key, setKey] = useState(modelState.key);
  const [provider, setProvider] = useState(modelState.provider);
  const [model, setModel] = useState(modelState.model);
  const [baseUrl, setBaseUrl] = useState(modelState.baseUrl);
  const [apiKey, setApiKey] = useState(modelState.apiKey);
  const [showApiKey, setShowApiKey] = useState(modelState.showApiKey);
  const [showAdvanced, setShowAdvanced] = useState(modelState.showAdvanced);
  const [testStatus, setTestStatus] = useState<TestStatus>(modelState.testStatus);
  const [testMsg, setTestMsg] = useState(modelState.testMsg);
  const [saving, setSaving] = useState(modelState.saving);
  const [saved, setSaved] = useState(modelState.saved);
  const [activeTab, setActiveTab] = useState<"preset" | "custom">("preset");

  // 当状态变化时，更新父组件的状态
  useEffect(() => {
    updateStepState({
      model: {
        key,
        provider,
        model,
        baseUrl,
        apiKey,
        showApiKey,
        showAdvanced,
        testStatus,
        testMsg,
        saving,
        saved
      }
    });
  }, [key, provider, model, baseUrl, apiKey, showApiKey, showAdvanced, testStatus, testMsg, saving, saved]);

  const canSave = key.trim() && model.trim();

  const handleTest = async () => {
    const url = baseUrl.replace(/\/+$/, "");
    if (!url) { setTestStatus("error"); setTestMsg("请填写 Base URL"); return; }
    if (!apiKey) { setTestStatus("error"); setTestMsg("请填写 API Key"); return; }
    setTestStatus("testing");
    setTestMsg("");
    try {
      if (transport.isTauri) {
        await transport.testModelConnection(url, apiKey, model || undefined);
        setTestStatus("success");
        setTestMsg("连接成功");
      } else {
        const resp = await fetch(`${url}/models`, {
          method: "GET",
          headers: { Authorization: `Bearer ${apiKey}` },
          signal: AbortSignal.timeout(10000),
        });
        if (resp.ok) { setTestStatus("success"); setTestMsg("连接成功"); }
        else {
          const body = await resp.text().catch(() => "");
          setTestStatus("error");
          setTestMsg(`HTTP ${resp.status}${body ? `: ${body.slice(0, 80)}` : ""}`);
        }
      }
    } catch (err: unknown) {
      setTestStatus("error");
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : "连接失败";
      setTestMsg(msg.length > 120 ? msg.slice(0, 120) + "…" : msg);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      const existingModels = ((await api.getConfig("models")) as Record<string, unknown> | null) ?? {};
      const mv = (existingModels as { value?: Record<string, unknown> }).value ?? existingModels;
      const newModels = {
        ...(typeof mv === "object" && mv !== null ? mv : {}),
        [key]: { provider, model, baseUrl, temperature: 0, maxConcurrent: 10, timeoutSecs: 120 },
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

      window.dispatchEvent(new CustomEvent("fastclaw:models-updated"));
      setSaved(true);
    } catch {
      setTestMsg("保存失败，请重试");
    }
    setSaving(false);
  };

  const handlePresetSelect = (preset: typeof PRESET_MODELS[number]) => {
    setKey(preset.id);
    setProvider(preset.provider);
    setModel(preset.model);
    setBaseUrl(preset.baseUrl);
  };

  const inputCls = "w-full rounded-[8px] px-3 py-2.5 text-[13px] outline-none transition-all focus:ring-2 focus:ring-[var(--tint)]";
  const inputStyle: React.CSSProperties = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };
  const labelCls = "mb-1.5 block text-[11px] font-semibold tracking-wide uppercase";
  const labelStyle: React.CSSProperties = { color: "var(--fill-tertiary)" };

  if (saved) {
    return (
      <div className="flex flex-col items-center text-center">
        <div className="flex h-16 w-16 items-center justify-center rounded-full" style={{ background: "color-mix(in srgb, var(--green) 12%, transparent)" }}>
          <CheckCircle size={32} strokeWidth={1.5} style={{ color: "var(--green)" }} />
        </div>
        <h2 className="mt-5 text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
          模型配置完成
        </h2>
        <p className="mt-2 text-[14px]" style={{ color: "var(--fill-secondary)" }}>
          <span className="font-medium">{model}</span> 已就绪，接下来了解一下 FastClaw 的核心功能
        </p>
        <button
          onClick={onNext}
          className="mt-8 flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          了解功能
          <ArrowRight size={16} strokeWidth={2} />
        </button>
      </div>
    );
  }

  return (
    <div className="relative">
      {/* 顶部导航栏 */}
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
          添加你的第一个模型
        </h2>
        <p className="mt-2 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
          FastClaw 兼容任何 OpenAI 格式的 API（通义千问、DeepSeek、GPT-4o、Claude 等）
        </p>
      </div>

      {/* Tab 切 */}
      <div className="flex mb-4 rounded-[var(--radius-sm)] p-1" style={{ background: "var(--bg-base)" }}>
        <button
          className={`flex-1 rounded-[var(--radius-xs)] py-2 text-[13px] font-medium transition-colors ${
            activeTab === "preset" ? "" : "hover:bg-[var(--bg-hover)]"
          }`}
          style={{
            background: activeTab === "preset" ? "var(--fill-primary)" : "transparent",
            color: activeTab === "preset" ? "var(--fill-inverse)" : "var(--fill-secondary)"
          }}
          onClick={() => setActiveTab("preset")}
        >
          颺定模型
        </button>
        <button
          className={`flex-1 rounded-[var(--radius-xs)] py-2 text-[13px] font-medium transition-colors ${
            activeTab === "custom" ? "" : "hover:bg-[var(--bg-hover)]"
          }`}
          style={{
            background: activeTab === "custom" ? "var(--fill-primary)" : "transparent",
            color: activeTab === "custom" ? "var(--fill-inverse)" : "var(--fill-secondary)"
          }}
          onClick={() => setActiveTab("custom")}
        >
          自定模型
        </button>
      </div>

      <div
        className="overflow-hidden rounded-[var(--radius-md)]"
        style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)", boxShadow: "var(--shadow-md)" }}
      >
        {activeTab === "preset" && (
          <div className="p-5">
            <p className="text-[13px] mb-4" style={{ color: "var(--fill-secondary)" }}>
              选择一个预设模型配置快速开始
            </p>
            <div className="space-y-3">
              {PRESET_MODELS.map((preset) => (
                <div
                  key={preset.id}
                  className="rounded-[var(--radius-sm)] p-3 cursor-pointer transition-all hover:scale-[1.02]"
                  style={{ 
                    background: key === preset.id ? "var(--bg-selected)" : "var(--bg-base)",
                    border: `1px solid ${key === preset.id ? "var(--fill-primary)" : "var(--separator-opaque)"}`,
                  }}
                  onClick={() => handlePresetSelect(preset)}
                >
                  <div className="flex justify-between items-center">
                    <div>
                      <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>
                        {preset.name}
                      </h3>
                      <p className="text-[12px] mt-1" style={{ color: "var(--fill-tertiary)" }}>
                        {preset.description}
                      </p>
                    </div>
                    <div className="text-[10px] px-2 py-1 rounded" style={{ background: "var(--bg-elevated)", color: "var(--fill-tertiary)" }}>
                      {preset.provider}
                    </div>
                  </div>
                  <div className="mt-2 text-[11px] font-mono" style={{ color: "var(--fill-quaternary)" }}>
                    {preset.model} • {preset.baseUrl}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {activeTab === "custom" && (
          <div className="space-y-3.5 p-5">
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label htmlFor="ob-key" className={labelCls} style={labelStyle}>名称</label>
                <input id="ob-key" value={key} onChange={(e) => setKey(e.target.value)} placeholder="如 dashscope" className={inputCls} style={inputStyle} />
              </div>
              <div>
                <label htmlFor="ob-provider" className={labelCls} style={labelStyle}>Provider</label>
                <div className="relative">
                  <select id="ob-provider" value={provider} onChange={(e) => setProvider(e.target.value)} className={`${inputCls} cursor-pointer pr-8`} style={{ ...inputStyle, appearance: "none" }}>
                    <option value="openai_compatible">OpenAI Compatible</option>
                    <option value="openai">OpenAI</option>
                    <option value="anthropic">Anthropic</option>
                  </select>
                  <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
                </div>
              </div>
            </div>

            <div>
              <label htmlFor="ob-model" className={labelCls} style={labelStyle}>模型名称</label>
              <input id="ob-model" value={model} onChange={(e) => setModel(e.target.value)} placeholder="如 qwen-plus / gpt-4o / claude-sonnet-4-20250514" className={inputCls} style={inputStyle} />
            </div>

            <div>
              <label htmlFor="ob-baseurl" className={labelCls} style={labelStyle}>Base URL</label>
              <input id="ob-baseurl" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="如 https://dashscope.aliyuncs.com/compatible-mode/v1" className={inputCls} style={inputStyle} />
            </div>

            <div>
              <label htmlFor="ob-apikey" className={labelCls} style={labelStyle}>API Key</label>
              <div className="relative">
                <input
                  id="ob-apikey"
                  type={showApiKey ? "text" : "password"}
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder="sk-..."
                  className={`${inputCls} pr-20`}
                  style={inputStyle}
                />
                <div className="absolute top-1/2 right-2 flex -translate-y-1/2 gap-1">
                  <button
                    onClick={() => setShowApiKey(!showApiKey)}
                    className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-md transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--fill-tertiary)" }}
                  >
                    {showApiKey ? <EyeOff size={13} /> : <Eye size={13} />}
                  </button>
                  <button
                    onClick={handleTest}
                    disabled={testStatus === "testing"}
                    className="flex h-7 cursor-pointer items-center gap-1 rounded-md px-2 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: testStatus === "success" ? "var(--green)" : testStatus === "error" ? "var(--red)" : "var(--fill-secondary)" }}
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
                <p className="mt-1.5 text-[11px]" style={{ color: testStatus === "success" ? "var(--green)" : "var(--red)" }}>
                  {testMsg}
                </p>
              )}
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
        )}

        <div
          className="flex items-center justify-between px-5 py-3"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          <button
            onClick={() => setShowAdvanced(!showAdvanced)}
            className="flex cursor-pointer items-center gap-1 text-[12px] transition-colors hover:opacity-80"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <ChevronRight size={12} className={`transition-transform ${showAdvanced ? "rotate-90" : ""}`} />
            高级选项
          </button>
          <button
            onClick={handleSave}
            disabled={!canSave || saving}
            className="cursor-pointer rounded-full px-6 py-2 text-[13px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40"
            style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
          >
            {saving ? "保存中..." : "保存模型"}
          </button>
        </div>
      </div>
    </div>
  );
}

/* ━━━ Step 3: Feature Tour ━━━ */

const FEATURES = [
  {
    icon: Bot,
    title: "多 Agent 管理",
    desc: "创建多个 Agent，每个拥有独立身份、模型和工具配置。在左侧边栏切换。",
    cssColor: "var(--tint)",
  },
  {
    icon: MessageSquare,
    title: "智能对话",
    desc: "流式输出，支持工具调用和多轮推理。Agent 可以读写文件、执行命令、搜索网络。",
    cssColor: "var(--green)",
  },
  {
    icon: Wrench,
    title: "工具 & Skills",
    desc: "内置文件操作、Shell 执行、代码分析等工具。可上传自定义 Skill 扩展能力。",
    cssColor: "var(--orange)",
  },
  {
    icon: Clock,
    title: "定时任务",
    desc: "为每个 Agent 配置 Cron 定时任务，定期执行对话或发送 Webhook。",
    cssColor: "var(--accent, #af52de)",
  },
  {
    icon: Search,
    title: "联网搜索",
    desc: "内置 Google、百度、Bing 等多引擎搜索。也支持 Tavily API 和自建 SearXNG。",
    cssColor: "var(--red)",
  },
  {
    icon: Settings,
    title: "设置中心",
    desc: "点击右上角齿轮图标管理模型、搜索引擎、MCP 服务器和更多配置。",
    cssColor: "var(--fill-tertiary)",
  },
];

function FeaturesStep({ onNext, onPrev }: { onNext: () => void, onPrev: () => void }) {
  return (
    <div className="relative">
      {/* 顶部导航栏 */}
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
            <p className="mt-1 text-[11px] leading-relaxed" style={{ color: "var(--fill-secondary)" }}>
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

/* ━━━ Step 4: Done ━━━ */

function DoneStep({ onComplete }: { onComplete: () => void }) {
  const [delayedComplete, setDelayedComplete] = useState(false);

  useEffect(() => {
    const t = setTimeout(() => {
      setDelayedComplete(true);
    }, 1200);
    return () => clearTimeout(t);
  }, []);

  return (
    <div className="flex flex-col items-center text-center">
      <div
        className="flex h-16 w-16 items-center justify-center rounded-full"
        style={{ background: "color-mix(in srgb, var(--green) 12%, transparent)", animation: "scale-in 0.4s ease-out" }}
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
        {delayedComplete && (
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