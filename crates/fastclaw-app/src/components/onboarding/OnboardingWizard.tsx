import { useState, useEffect, useCallback } from "react";
import { ChevronRight, ChevronDown, Bot, MessageSquare, Clock, Search, Wrench, Settings, Sparkles, Eye, EyeOff, Zap, CheckCircle, XCircle, ArrowRight } from "lucide-react";
import { ClawIcon } from "../layout/ClawIcon";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";

type Step = "welcome" | "model" | "features" | "done";

interface OnboardingWizardProps {
  onComplete: () => void;
}

export function OnboardingWizard({ onComplete }: OnboardingWizardProps) {
  const [step, setStep] = useState<Step>("welcome");
  const [fadeClass, setFadeClass] = useState("ob-fade-in");

  const goTo = useCallback((next: Step) => {
    setFadeClass("ob-fade-out");
    setTimeout(() => {
      setStep(next);
      setFadeClass("ob-fade-in");
    }, 250);
  }, []);

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center" style={{ background: "var(--bg-primary)" }}>
      <div className={`w-full max-w-[560px] px-6 ${fadeClass}`}>
        {step === "welcome" && <WelcomeStep onNext={() => goTo("model")} />}
        {step === "model" && <ModelStep onNext={() => goTo("features")} />}
        {step === "features" && <FeaturesStep onNext={() => goTo("done")} />}
        {step === "done" && <DoneStep onComplete={onComplete} />}
      </div>

      <div className="fixed bottom-8 left-1/2 flex -translate-x-1/2 items-center gap-2">
        {(["welcome", "model", "features", "done"] as Step[]).map((s) => (
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

function WelcomeStep({ onNext }: { onNext: () => void }) {
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
        开始之前，需要配置一个语言模型
      </p>
      <button
        onClick={onNext}
        className="mt-8 flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
        style={{
          background: "var(--fill-primary)",
          color: "var(--fill-inverse)",
        }}
      >
        开始配置
        <ArrowRight size={16} strokeWidth={2} />
      </button>
    </div>
  );
}

/* ━━━ Step 2: Model Setup ━━━ */

type TestStatus = "idle" | "testing" | "success" | "error";

function ModelStep({ onNext }: { onNext: () => void }) {
  const [key, setKey] = useState("");
  const [provider, setProvider] = useState("openai_compatible");
  const [model, setModel] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [showApiKey, setShowApiKey] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [testMsg, setTestMsg] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

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
    <div>
      <div className="mb-6 text-center">
        <h2 className="text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
          添加你的第一个模型
        </h2>
        <p className="mt-2 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
          FastClaw 兼容任何 OpenAI 格式的 API（通义千问、DeepSeek、GPT-4o、Claude 等）
        </p>
      </div>

      <div
        className="overflow-hidden rounded-[var(--radius-md)]"
        style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)", boxShadow: "var(--shadow-md)" }}
      >
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

function FeaturesStep({ onNext }: { onNext: () => void }) {
  return (
    <div>
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

      <div className="mt-6 flex justify-center">
        <button
          onClick={onNext}
          className="flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          开始使用
          <Sparkles size={16} strokeWidth={2} />
        </button>
      </div>
    </div>
  );
}

/* ━━━ Step 4: Done ━━━ */

function DoneStep({ onComplete }: { onComplete: () => void }) {
  useEffect(() => {
    const t = setTimeout(onComplete, 1200);
    return () => clearTimeout(t);
  }, [onComplete]);

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
    </div>
  );
}
