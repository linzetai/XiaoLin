import {
  Image as ImageIcon, FileText, Paperclip, ArrowUp,
  Square, X, SpinnerGap, Compass, Code, CaretDown,
  Plus, GitBranch, Monitor, Crosshair,
} from "@phosphor-icons/react";
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import { MentionInput, type MentionInputHandle, type InlineMention, type MentionOption, type SlashCommand } from "./MentionInput";
import { usePlanNudge } from "./usePlanNudge";
import {
  useChatMetaStore,
  useQueueStore,
  useActiveChatId,
  useChatQueue,
  useGoalStore,
} from "../../lib/stores";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { QueueIndicator } from "./QueueIndicator";
import { QueuePanel } from "./QueuePanel";
import { PermissionSelector } from "./PermissionSelector";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { useConfigStore } from "../../lib/stores/config-store";
import { useStreamStore } from "../../lib/stores/stream-store";
import type { Chat } from "../../lib/stores/types";
import { openLightbox } from "../common/ImageLightbox";

const isMacPlatform = /Mac|iPhone|iPad/.test((navigator as { userAgentData?: { platform?: string } }).userAgentData?.platform ?? navigator.platform ?? "");
const MOD_KEY = isMacPlatform ? "⌘" : "Ctrl+";

export interface AttachedFile {
  name: string;
  size: number;
  type: string;
  file: File;
  previewUrl?: string;
}

function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function FilePill({ file, onRemove }: { file: AttachedFile; onRemove: () => void }) {
  const isImage = file.type.startsWith("image/");
  const icon = isImage
    ? <ImageIcon />
    : file.type.includes("pdf")
      ? <FileText />
      : <Paperclip />;

  if (isImage && file.previewUrl) {
    return (
      <div
        className="relative inline-block"
        style={{ animation: "pop var(--duration-normal) var(--ease-spring)" }}
      >
        <img
          src={file.previewUrl}
          alt={file.name}
          className="block max-h-[80px] max-w-[120px] cursor-pointer rounded-lg object-cover"
          style={{ border: `0.5px solid var(--separator)` }}
          onClick={() => openLightbox(file.previewUrl!, file.name)}
        />
        <button
          onClick={(e) => { e.stopPropagation(); onRemove(); }}
          className="absolute -top-1.5 -right-1.5 z-10 flex h-4 w-4 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[rgba(0,0,0,0.7)]"
          style={{ background: "rgba(0,0,0,0.5)", color: "#fff" }}
        >
          <X size={10} weight="bold" />
        </button>
      </div>
    );
  }

  return (
    <div
      className="flex min-w-0 max-w-full items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px]"
      style={{
        background: "var(--bg-secondary)",
        border: `0.5px solid var(--separator)`,
        animation: "pop var(--duration-normal) var(--ease-spring)",
      }}
    >
      <span className="shrink-0" style={{ color: "var(--fill-tertiary)" }}>{icon}</span>
      <span className="min-w-0 max-w-[120px] truncate" style={{ color: "var(--fill-primary)" }} title={file.name}>{file.name}</span>
      <span className="shrink-0" style={{ color: "var(--fill-quaternary)" }}>{formatSize(file.size)}</span>
      <button
        onClick={onRemove}
        className="ml-0.5 flex h-4 w-4 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
        style={{ color: "var(--fill-tertiary)" }}
      >
        <X size={10} weight="bold" />
      </button>
    </div>
  );
}

function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

function ContextRing({ used, limit }: { used: number; limit: number }) {
  const { t } = useTranslation("chat");
  const [hover, setHover] = useState(false);
  const ringRef = useRef<HTMLDivElement>(null);
  const ratio = limit > 0 ? used / limit : 0;
  const clampedRatio = Math.min(ratio, 1);
  const pct = clampedRatio * 100;
  const color = ratio < 0.5
    ? "var(--green, #68D391)"
    : ratio < 0.8
      ? "var(--yellow, #ED8936)"
      : "var(--red, #FC8181)";

  const size = 24;
  const strokeWidth = 2.5;
  const r = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * r;
  const offset = circumference * (1 - clampedRatio);

  const remaining = Math.max(0, limit - used);
  const warning = ratio >= 0.8;
  const critical = ratio >= 0.95;

  return (
    <div
      ref={ringRef}
      className="relative flex items-center justify-center"
      style={{
        width: size,
        height: size,
        cursor: "default",
        animation: critical ? "pulse 2s ease-in-out infinite" : undefined,
      }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      <svg width={size} height={size} style={{ transform: "rotate(-90deg)" }}>
        <circle
          cx={size / 2} cy={size / 2} r={r}
          fill="none"
          stroke="var(--separator-opaque, #E2E8F0)"
          strokeWidth={strokeWidth}
          opacity={0.6}
        />
        <circle
          cx={size / 2} cy={size / 2} r={r}
          fill="none"
          stroke={color}
          strokeWidth={strokeWidth}
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          strokeLinecap="round"
          style={{ transition: "stroke-dashoffset var(--duration-slower) var(--ease-in-out), stroke var(--duration-slow) var(--ease-in-out)" }}
        />
      </svg>
      <span
        className="absolute text-[7px] font-bold tabular-nums leading-none"
        style={{ color }}
      >
        {pct < 1 ? "<1" : Math.round(pct)}
      </span>
      {hover && createPortal(
        <div
          className="fixed rounded-xl px-4 py-3.5"
          style={{
            background: "var(--bg-elevated)",
            border: "0.5px solid var(--border-subtle)",
            boxShadow: "var(--shadow-lg), inset 0 1px 0 var(--highlight-top)",
            color: "var(--fill-primary)",
            zIndex: 9999,
            bottom: window.innerHeight - (ringRef.current?.getBoundingClientRect().top ?? 0) + 8,
            right: window.innerWidth - (ringRef.current?.getBoundingClientRect().right ?? 0) - 8,
            minWidth: 240,
            animation: "scale-spring var(--duration-fast) var(--ease-spring-subtle)",
            transformOrigin: "bottom right",
          }}
        >
          <div className="mb-2 text-[11px] font-semibold" style={{ color: "var(--fill-secondary)" }}>
            {t("contextWindow")}
          </div>
          <div className="mb-2.5 flex items-baseline gap-1.5">
            <span className="text-[15px] font-bold tabular-nums" style={{ color }}>{formatTokens(used)}</span>
            <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>/ {formatTokens(limit)} tokens</span>
          </div>
          <div
            className="mb-2.5 h-[4px] w-full overflow-hidden rounded-full"
            style={{ background: "var(--separator-opaque, #E2E8F0)" }}
          >
            <div
              className="h-full rounded-full"
              style={{
                width: `${pct}%`,
                background: color,
                transition: "width var(--duration-slow) var(--ease-in-out)",
              }}
            />
          </div>
          <div className="flex justify-between text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            <span>{t("used", { pct: pct.toFixed(1) })}</span>
            <span>{t("remaining", { tokens: formatTokens(remaining) })}</span>
          </div>
          {warning && (
            <div
              className="mt-2 rounded-md px-2 py-1 text-[10px]"
              style={{
                background: critical ? "rgba(252,129,129,0.12)" : "rgba(237,137,54,0.12)",
                color: critical ? "var(--red, #FC8181)" : "var(--yellow, #ED8936)",
              }}
            >
              {critical ? t("contextOverflow") : t("contextHigh")}
            </div>
          )}
        </div>
      , document.body)}
    </div>
  );
}

const PROVIDER_COLORS: Record<string, string> = {
  openai: "#10A37F",
  anthropic: "#D97706",
  google: "#4285F4",
  deepseek: "#6366F1",
  mistral: "#FF7000",
  default: "var(--fill-tertiary)",
};

function ModelSelector() {
  const { t } = useTranslation("chat");
  const models = useConfigStore((s) => s.models);
  const modelsLoaded = useConfigStore((s) => s.modelsLoaded);
  const refreshModels = useConfigStore((s) => s.refreshModels);
  const [open, setOpen] = useState(false);
  const agents = useChatMetaStore((s) => s.agents);
  const updateAgentProps = useChatMetaStore((s) => s.updateAgentProps);
  const agent = agents.find((a) => a.id === "main") ?? agents[0];
  const currentModel = agent?.model ?? "";
  const btnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!modelsLoaded) refreshModels();
  }, [modelsLoaded, refreshModels]);

  const currentMeta = models.find((m) => m.model === currentModel);
  const dotColor = PROVIDER_COLORS[currentMeta?.provider ?? ""] ?? PROVIDER_COLORS.default;
  const displayName = currentModel.split("/").pop() || currentModel || t("selectModel");

  return (
    <div className="relative">
      <button
        ref={btnRef}
        onClick={() => { if (!open) refreshModels(); setOpen(!open); }}
        className="flex items-center gap-1.5 rounded-lg px-2 py-1 text-[11px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
        style={{ color: "var(--fill-tertiary)" }}
      >
        <span className="h-2 w-2 rounded-full" style={{ background: dotColor }} />
        <span className="max-w-[100px] truncate">{displayName}</span>
        <CaretDown />
      </button>
      {open && createPortal(
        <div className="fixed inset-0 z-[60]" onClick={() => setOpen(false)}>
          <div
            className="fixed max-h-[200px] overflow-y-auto rounded-lg py-1"
            style={{
              left: btnRef.current?.getBoundingClientRect().left ?? 0,
              bottom: window.innerHeight - (btnRef.current?.getBoundingClientRect().top ?? 0) + 4,
              minWidth: 180,
              background: "var(--bg-elevated)",
              border: "0.5px solid var(--separator)",
              boxShadow: "var(--shadow-lg)",
              animation: "scale-in var(--duration-fast) var(--ease-out)",
              transformOrigin: "bottom left",
            }}
            onClick={(e) => e.stopPropagation()}
          >
            {models.map((m) => {
              const active = m.model === currentModel;
              const mColor = PROVIDER_COLORS[m.provider ?? ""] ?? PROVIDER_COLORS.default;
              return (
                <button
                  key={`${m.provider}/${m.model}`}
                  onClick={() => {
                    updateAgentProps({ model: m.model });
                    setOpen(false);
                  }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                  style={{
                    color: active ? "var(--tint)" : "var(--fill-secondary)",
                    fontWeight: active ? 600 : 400,
                  }}
                >
                  <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: mColor }} />
                  <span className="min-w-0 truncate">{m.model}</span>
                </button>
              );
            })}
            {models.length === 0 && (
              <div className="px-3 py-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>{t("noModels")}</div>
            )}
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}

type ComposerMode = "agent" | "plan" | "goal";

function ModeSelector({
  mode,
  onSelectMode,
  disabled,
}: {
  mode: ComposerMode;
  onSelectMode: (m: ComposerMode) => void;
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement>(null);
  const Icon = mode === "plan" ? Compass : mode === "goal" ? Crosshair : Code;
  const label = mode === "plan" ? "Plan" : mode === "goal" ? "Goal" : "Agent";

  const options: Array<{ id: ComposerMode; icon: typeof Code; label: string; color: string }> = [
    { id: "agent", icon: Code, label: "Agent", color: "var(--tint)" },
    { id: "plan", icon: Compass, label: "Plan", color: "var(--plan-tint)" },
    { id: "goal", icon: Crosshair, label: "Goal", color: "var(--orange, #ED8936)" },
  ];

  const activeColor = options.find((o) => o.id === mode)?.color ?? "var(--fill-quaternary)";

  return (
    <div className="relative">
      <button
        ref={btnRef}
        onClick={() => !disabled && setOpen(!open)}
        disabled={disabled}
        className="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:cursor-not-allowed disabled:opacity-50"
        style={{ color: activeColor }}
      >
        <Icon size={12} />
        <span>{label}</span>
        <span style={{ fontSize: 8, opacity: 0.6 }}>▾</span>
      </button>
      {open && createPortal(
        <div className="fixed inset-0 z-[60]" onClick={() => setOpen(false)}>
          <div
            className="fixed rounded-lg py-1"
            style={{
              left: btnRef.current?.getBoundingClientRect().left ?? 0,
              bottom: window.innerHeight - (btnRef.current?.getBoundingClientRect().top ?? 0) + 4,
              minWidth: 140,
              background: "var(--bg-elevated)",
              border: "0.5px solid var(--separator)",
              boxShadow: "var(--shadow-lg)",
              animation: "scale-in var(--duration-fast) var(--ease-out)",
              transformOrigin: "bottom left",
            }}
            onClick={(e) => e.stopPropagation()}
          >
            {options.map((opt) => {
              const isActive = opt.id === mode;
              return (
                <button
                  key={opt.id}
                  onClick={() => { onSelectMode(opt.id); setOpen(false); }}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                  style={{
                    color: isActive ? opt.color : "var(--fill-secondary)",
                    fontWeight: isActive ? 600 : 400,
                  }}
                >
                  <opt.icon size={13} />
                  {opt.label}
                </button>
              );
            })}
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}

export interface ComposerCoreProps {
  mentionInputRef: React.RefObject<MentionInputHandle | null>;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  workDir: string | null;
  activeChat: Chat | null | undefined;
  streaming: boolean;
  mentionOptions: MentionOption[];
  attachedFiles: AttachedFile[];
  removeFile: (index: number) => void;
  processFiles: (files: FileList | File[]) => void;
  handleMentionSend: (txt: string, mentions: InlineMention[], options?: { goalMode?: boolean }) => void;
  handleNewTopic: () => void;
  setWorkDir: (agentId: string, chatId: string, path: string) => void;
  stopStream: () => void;
  onTogglePlanPanel?: () => void;
  onRecallLastMessage?: () => string | null;
}

export function ComposerCore({
  mentionInputRef,
  fileInputRef,
  workDir,
  activeChat,
  streaming,
  mentionOptions,
  attachedFiles,
  removeFile,
  processFiles,
  handleMentionSend,
  handleNewTopic,
  setWorkDir,
  stopStream,
  onTogglePlanPanel,
  onRecallLastMessage,
}: ComposerCoreProps) {
  const { t } = useTranslation("chat");
  const [inputHasContent, setInputHasContent] = useState(false);
  const [inputText, setInputText] = useState("");
  const [inputMentions, setInputMentions] = useState<InlineMention[]>([]);
  const [sendPending, setSendPending] = useState(false);
  const [queueExpanded, setQueueExpanded] = useState(false);

  const activeChatId = useActiveChatId();
  const messageQueue = useChatQueue(activeChatId);
  const updateQueuedMessage = useQueueStore((s) => s.updateQueuedMessage);
  const removeQueuedMessage = useQueueStore((s) => s.removeQueuedMessage);
  const reorderQueue = useQueueStore((s) => s.reorderQueue);

  useEffect(() => {
    if (streaming) setSendPending(false);
  }, [streaming]);

  const executionMode = activeChat?.executionMode ?? "agent";
  const planFilePath = activeChat?.planFilePath;
  const planFileExists = activeChat?.planFileExists ?? false;
  const isGoalMode = useGoalStore((s) => activeChatId ? !!s.goalMode[activeChatId] : false);
  const hasActiveGoal = useGoalStore((s) => {
    if (!activeChatId) return false;
    const goal = s.goals[activeChatId];
    if (!goal) return false;
    return !["completed", "failed", "cancelled"].includes(goal.status);
  });

  const composerMode: ComposerMode = isGoalMode
    ? "goal"
    : executionMode === "plan"
      ? "plan"
      : "agent";

  const handleCompact = useCallback(() => {
    if (streaming) return;
    handleMentionSend("/compact", []);
  }, [streaming, handleMentionSend]);

  const handleSkillify = useCallback(() => {
    if (streaming) return;
    handleMentionSend("/skillify", []);
  }, [streaming, handleMentionSend]);

  const handleSelectMode = useCallback(async (newMode: ComposerMode) => {
    if (streaming) return;
    if (newMode === "goal") {
      if (activeChatId) {
        useGoalStore.getState().setGoalMode(activeChatId, true);
      }
      if (executionMode === "plan") {
        const sessionId = activeChat?.id;
        const resp = await transport.setExecutionModeIpc("agent", sessionId ?? undefined);
        if (resp.ok) {
          const { activeChatId: chatId, setChatExecutionMode } = useChatMetaStore.getState();
          setChatExecutionMode(chatId, "agent");
          if (chatId) {
            useStreamStore.getState().addBriefMessage(chatId, {
              id: `mode-switch-${Date.now()}`,
              content: t("slashPlanToggle_toAgent"),
              mode: "normal",
              timestamp: Date.now(),
            });
          }
        }
      }
    } else {
      if (activeChatId) {
        useGoalStore.getState().setGoalMode(activeChatId, false);
      }
      const backendMode = newMode === "plan" ? "plan" : "agent";
      if (backendMode !== executionMode) {
        const sessionId = activeChat?.id;
        const resp = await transport.setExecutionModeIpc(backendMode, sessionId ?? undefined);
        if (resp.ok) {
          const { activeChatId: chatId, setChatExecutionMode } = useChatMetaStore.getState();
          setChatExecutionMode(chatId, backendMode);
          if (backendMode === "plan") {
            try { localStorage.setItem("xiaolin:plan-mode-ever-used", String(Date.now())); } catch {}
          }
          if (chatId) {
            useStreamStore.getState().addBriefMessage(chatId, {
              id: `mode-switch-${Date.now()}`,
              content: backendMode === "plan" ? t("slashPlanToggle_toPlan") : t("slashPlanToggle_toAgent"),
              mode: "normal",
              timestamp: Date.now(),
            });
          }
        }
      }
    }
  }, [streaming, executionMode, activeChat?.id, activeChatId, t]);

  const messageCount = useStreamStore((s) => (activeChatId ? (s.streams[activeChatId]?.length ?? 0) : 0));
  const discoveryNudge = usePlanNudge(inputText, inputMentions, executionMode, activeChatId ?? "", messageCount);

  const planShortcutHandler = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const isMod = e.metaKey || e.ctrlKey;
    if (isMod && e.shiftKey && (e.key === "P" || e.key === "p")) {
      e.preventDefault();
      if (!streaming) {
        handleSelectMode(executionMode === "plan" ? "agent" : "plan");
      }
      return true;
    }
    if (e.key === "Escape" && discoveryNudge.visible) {
      discoveryNudge.dismiss();
      return true;
    }
    return false;
  }, [streaming, executionMode, handleSelectMode, discoveryNudge]);

  const handlePlanSlash = useCallback(() => {
    if (streaming) return;
    handleSelectMode(executionMode === "plan" ? "agent" : "plan");
  }, [streaming, handleSelectMode, executionMode]);

  const handleExportMd = useCallback(async () => {
    const chatId = activeChat?.id;
    if (!chatId) return;
    await api.exportSession(chatId, "markdown");
  }, [activeChat?.id]);

  const handleExportJson = useCallback(async () => {
    const chatId = activeChat?.id;
    if (!chatId) return;
    await api.exportSession(chatId, "json");
  }, [activeChat?.id]);

  const slashCommands = useMemo((): SlashCommand[] => [
    { id: "new", label: "new", desc: t("slashNew"), action: handleNewTopic },
    { id: "clear", label: "clear", desc: t("slashClear"), action: handleNewTopic },
    { id: "compact", label: "compact", desc: t("slashCompact"), action: handleCompact },
    { id: "skillify", label: "skillify", desc: t("slashSkillify"), action: handleSkillify },
    { id: "plan", label: "plan", desc: executionMode === "plan" ? t("slashPlanToggle_toAgent") : t("slashPlanToggle_toPlan"), action: handlePlanSlash },
    { id: "export-md", label: "export md", desc: t("slashExportMd"), action: handleExportMd },
    { id: "export-json", label: "export json", desc: t("slashExportJson"), action: handleExportJson },
    { id: "model", label: "model", desc: t("slashModel") },
    { id: "tools", label: "tools", desc: t("slashTools") },
  ], [t, handleNewTopic, handleCompact, handleSkillify, handlePlanSlash, handleExportMd, handleExportJson, executionMode]);

  const handleInputTextChange = useCallback((text: string, mentions: InlineMention[]) => {
    setInputText(text);
    setInputMentions(mentions);
  }, []);

  const wrappedSend = useCallback((txt: string, mentions: InlineMention[]) => {
    setSendPending(true);
    setInputHasContent(false);
    setInputText("");
    setInputMentions([]);
    const storeState = useGoalStore.getState();
    const currentGoalMode = activeChatId ? !!storeState.goalMode[activeChatId] : false;
    const currentGoal = activeChatId ? storeState.goals[activeChatId] : undefined;
    const goalActive = currentGoal && !["completed", "failed", "cancelled"].includes(currentGoal.status);
    const shouldCreateGoal = currentGoalMode && !goalActive && txt.trim() && !txt.startsWith("/");
    handleMentionSend(txt, mentions, shouldCreateGoal ? { goalMode: true } : undefined);
  }, [handleMentionSend, activeChatId]);

  const defaultRecall = useCallback((): string | null => null, []);
  const handleRecallLastMessage = onRecallLastMessage ?? defaultRecall;

  const canSend = (inputHasContent || attachedFiles.length > 0) && messageQueue.length < 10;

  const handleOpenWorkDir = useCallback(async () => {
    const { activeChatId: chatId, chats } = useChatMetaStore.getState();
    const curChat = chats[chatId];
    if (!curChat) return;
    let selected: string | null = null;
    try {
      const { open: tauriOpenDialog } = await import("@tauri-apps/plugin-dialog");
      selected = await tauriOpenDialog({ directory: true, multiple: false, defaultPath: curChat.workDir ?? undefined }) as string | null;
    } catch {
      selected = prompt(t("enterWorkDir"), curChat.workDir ?? "");
    }
    if (typeof selected === "string" && selected) {
      setWorkDir("", chatId, selected);
    }
  }, [setWorkDir, t]);

  const handleSendClick = useCallback(() => {
    const ref = mentionInputRef.current;
    if (ref) {
      const text = ref.getText().trim();
      if (text) wrappedSend(text, ref.getMentions());
    }
  }, [mentionInputRef, wrappedSend]);

  const comingSoon = useCallback((e: React.MouseEvent<HTMLButtonElement>) => {
    const btn = e.currentTarget;
    btn.style.background = "var(--tint-subtle)";
    btn.style.color = "var(--tint)";
    setTimeout(() => { btn.style.background = "transparent"; btn.style.color = "var(--fill-tertiary)"; }, 600);
  }, []);

  const chipStyle: React.CSSProperties = {
    display: "flex", alignItems: "center", gap: 4,
    padding: "3px 7px", borderRadius: 5,
    fontSize: 11, fontWeight: 500, color: "var(--fill-tertiary)",
    cursor: "pointer", border: "none", background: "transparent",
    transition: "background 0.1s, color 0.1s", whiteSpace: "nowrap",
  };
  const chipHover = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.currentTarget.style.background = "var(--bg-hover)";
    e.currentTarget.style.color = "var(--fill-secondary)";
  };
  const chipLeave = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.currentTarget.style.background = "transparent";
    e.currentTarget.style.color = "var(--fill-tertiary)";
  };
  const [nudgeDismissed, setNudgeDismissed] = useState(() => {
    try {
      const raw = localStorage.getItem("xiaolin:plan-nudge-dismissed");
      return raw ? new Set(JSON.parse(raw) as string[]) : new Set<string>();
    } catch { return new Set<string>(); }
  });

  const showPlanNudge = planFileExists && executionMode !== "plan" && !streaming
    && activeChatId != null && !nudgeDismissed.has(activeChatId);

  const dismissNudge = useCallback(() => {
    if (!activeChatId) return;
    setNudgeDismissed((prev) => {
      const next = new Set(prev);
      next.add(activeChatId);
      try { localStorage.setItem("xiaolin:plan-nudge-dismissed", JSON.stringify([...next])); } catch {}
      return next;
    });
  }, [activeChatId]);

  const [nudgeHovered, setNudgeHovered] = useState(false);
  useEffect(() => {
    if (!discoveryNudge.visible || nudgeHovered) return;
    const timer = setTimeout(() => discoveryNudge.dismiss(), 10_000);
    return () => clearTimeout(timer);
  }, [discoveryNudge.visible, discoveryNudge.dismiss, nudgeHovered]);

  const showDiscoveryNudge = discoveryNudge.visible && !showPlanNudge && executionMode !== "plan";

  return (
    <>
      {showPlanNudge && (
        <div
          className="flex w-full items-center gap-1.5 px-3 py-1 text-[11px]"
          style={{
            color: "var(--plan-tint)",
            background: "var(--plan-tint-subtle, rgba(13,148,136,0.03))",
            borderRadius: "8px 8px 0 0",
            marginBottom: -1,
          }}
        >
          <button
            className="flex flex-1 items-center gap-1.5 transition-opacity hover:opacity-80"
            onClick={() => handleSelectMode("plan")}
          >
            <FileText size={12} weight="bold" />
            {t("plan_nudge_hasUnfinished")}
          </button>
          <button
            className="flex-shrink-0 rounded p-0.5 transition-opacity hover:opacity-60"
            onClick={dismissNudge}
            style={{ color: "var(--plan-tint)" }}
          >
            <X size={10} weight="bold" />
          </button>
        </div>
      )}

      {showDiscoveryNudge && (
        <div
          className="flex w-full items-center gap-1.5 px-3 py-1 text-[11px]"
          style={{
            color: "var(--plan-tint)",
            background: "var(--plan-tint-subtle, rgba(13,148,136,0.03))",
            borderRadius: "8px 8px 0 0",
            marginBottom: -1,
          }}
          onMouseEnter={() => setNudgeHovered(true)}
          onMouseLeave={() => setNudgeHovered(false)}
        >
          <Compass size={12} weight="bold" style={{ color: "var(--plan-tint)", flexShrink: 0 }} />
          <span className="flex-1 truncate">{t(discoveryNudge.messageKey)}</span>
          <button
            className="flex-shrink-0 rounded px-2 py-0.5 text-[10px] font-medium transition-opacity hover:opacity-80"
            style={{ background: "var(--plan-tint)", color: "#fff", borderRadius: 4 }}
            onClick={() => { handleSelectMode("plan"); discoveryNudge.dismiss(); }}
          >
            {t("plan_nudge_switch")}
          </button>
          <button
            className="flex-shrink-0 rounded p-0.5 transition-opacity hover:opacity-60"
            onClick={() => discoveryNudge.dismiss()}
            style={{ color: "var(--plan-tint)" }}
          >
            <X size={10} weight="bold" />
          </button>
        </div>
      )}

      {/* ═══ input-box container ═══ */}
      <div
        className="input-box overflow-hidden"
        style={{
          border: `1.5px solid ${executionMode === "plan" ? "var(--plan-tint-border)" : "var(--bg-input-border)"}`,
          borderRadius: 12,
          background: "var(--bg-card)",
          transition: "border-color 0.3s, box-shadow 0.15s",
        }}
        onFocusCapture={(e) => {
          const box = e.currentTarget;
          const tint = executionMode === "plan" ? "var(--plan-tint)" : "var(--accent, var(--tint))";
          box.style.borderColor = tint;
          box.style.boxShadow = `0 0 0 3px color-mix(in srgb, ${tint} 8%, transparent)`;
        }}
        onBlurCapture={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node)) {
            const box = e.currentTarget;
            box.style.borderColor = executionMode === "plan" ? "var(--plan-tint-border)" : "var(--bg-input-border)";
            box.style.boxShadow = "none";
          }
        }}
      >
        {messageQueue.length > 0 && (
          <div className="px-3 pt-2">
            <QueueIndicator count={messageQueue.length} expanded={queueExpanded} onToggle={() => setQueueExpanded(!queueExpanded)} />
          </div>
        )}
        {queueExpanded && messageQueue.length > 0 && (
          <QueuePanel
            queue={messageQueue}
            onEdit={(id, content) => updateQueuedMessage(activeChatId, id, { content })}
            onRemove={(id) => removeQueuedMessage(activeChatId, id)}
            onReorder={(from, to) => reorderQueue(activeChatId, from, to)}
            onRetry={(id) => updateQueuedMessage(activeChatId, id, { status: "pending", error: undefined })}
          />
        )}

        {attachedFiles.length > 0 && (
          <div className="flex flex-wrap gap-2 px-4 pt-3">
            {attachedFiles.map((f, i) => (
              <div key={`${f.name}-${i}`} style={{ animation: `fade-slide-up var(--duration-normal) var(--ease-out) ${i * 50}ms backwards` }}>
                <FilePill file={f} onRemove={() => removeFile(i)} />
              </div>
            ))}
          </div>
        )}

        {executionMode === "plan" && (
          <button
            type="button" onClick={onTogglePlanPanel}
            className="flex w-full items-center gap-2 px-4 py-2 text-left text-[11px] transition-colors hover:brightness-110"
            style={{
              background: "var(--plan-tint-bg)",
              borderBottom: "0.5px solid var(--plan-tint-border)",
              color: "var(--plan-tint)",
              animation: "slide-down var(--duration-normal, 200ms) var(--ease-out, ease-out)",
            }}
          >
            <Compass size={ICON_SIZE.md} className="shrink-0" />
            <span className="min-w-0 truncate">
              {t("planMode")}
              {planFilePath && <span style={{ opacity: 0.7 }}>{" · "}{planFileExists ? "" : t("notCreated")}{planFilePath.replace(/^\/home\/[^/]+\//, "~/")}</span>}
            </span>
            <FileText className="ml-auto shrink-0" style={{ opacity: 0.6 }} />
          </button>
        )}
        {isGoalMode && !hasActiveGoal && (
          <div
            className="flex w-full items-center gap-2 px-4 py-2 text-[11px]"
            style={{ background: "color-mix(in srgb, var(--orange, #ED8936) 6%, transparent)", borderBottom: "0.5px solid color-mix(in srgb, var(--orange, #ED8936) 15%, transparent)", color: "var(--orange, #ED8936)" }}
          >
            <Crosshair className="shrink-0" />
            <span>Goal 模式 — 描述目标后将自主工作直到完成</span>
          </div>
        )}
        {executionMode === "agent" && planFileExists && planFilePath && (
          <button
            type="button" onClick={onTogglePlanPanel}
            className="flex w-full items-center gap-2 px-4 py-1.5 text-left text-[10px] transition-colors hover:brightness-110"
            style={{ background: "var(--plan-tint-subtle)", borderBottom: "0.5px solid var(--plan-tint-border)", color: "var(--fill-tertiary)" }}
          >
            <FileText className="shrink-0" style={{ color: "var(--tint, #4299E1)", opacity: 0.7 }} />
            <span className="min-w-0 truncate">{t("planFile", { path: planFilePath.replace(/^\/home\/[^/]+\//, "~/") })}</span>
          </button>
        )}

        <div style={{ padding: "11px 14px 6px" }}>
          <MentionInput
            ref={mentionInputRef}
            placeholder={streaming ? t("placeholderStreaming") : isGoalMode && !hasActiveGoal ? "描述你的目标，按 Enter 开始自主工作..." : executionMode === "plan" ? t("placeholderPlan") : t("placeholderDefault")}
            options={mentionOptions}
            slashCommands={slashCommands}
            onSend={wrappedSend}
            onNewTopic={handleNewTopic}
            onAttach={() => fileInputRef.current?.click()}
            onPasteFiles={processFiles}
            onRecallLastMessage={handleRecallLastMessage}
            onContentChange={setInputHasContent}
            onTextChange={handleInputTextChange}
            extraKeyHandler={planShortcutHandler}
          />
        </div>

        <div style={{ display: "flex", alignItems: "center", padding: "3px 10px 8px" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 2, flex: 1, minWidth: 0, overflow: "hidden" }}>
            <button type="button" style={chipStyle} onMouseEnter={chipHover} onMouseLeave={chipLeave}
              onClick={() => fileInputRef.current?.click()} title={t("attachFile", { shortcut: `${MOD_KEY}${isMacPlatform ? "⇧" : "Shift+"}A` })}
            >
              <Plus size={13} />
            </button>
            <PermissionSelector sessionId={activeChat?.id} disabled={streaming} />
          </div>

          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <ModelSelector />
            {streaming ? (
              <button
                key="stop" onClick={stopStream}
                className="flex shrink-0 cursor-pointer items-center justify-center rounded-full transition-all duration-150 hover:scale-105 active:scale-90"
                style={{ width: 28, height: 28, background: "var(--red, #FF3B30)", color: "#fff", boxShadow: "0 0 0 3px color-mix(in srgb, var(--red, #FF3B30) 20%, transparent)", animation: "glow-pulse 1.5s ease-in-out infinite" }}
                title={t("stopGenerate")}
              >
                <Square size={12} weight="fill" />
              </button>
            ) : sendPending ? (
              <button key="loading" disabled className="flex shrink-0 items-center justify-center rounded-full"
                style={{ width: 28, height: 28, background: "var(--tint)", color: "#fff", opacity: 0.7 }} title={t("sending")}
              >
                <SpinnerGap size={14} weight="bold" className="animate-spin" />
              </button>
            ) : (
              <button
                key="send" onClick={handleSendClick}
                disabled={!canSend}
                className="flex shrink-0 items-center justify-center rounded-full transition-all duration-150 hover:scale-[1.06] active:scale-90 disabled:cursor-default disabled:hover:scale-100"
                style={{
                  width: 28, height: 28,
                  background: "var(--tint)", color: "#fff",
                  opacity: canSend ? 1 : 0.3,
                  boxShadow: canSend ? "0 2px 8px color-mix(in srgb, var(--tint) 20%, transparent)" : "none",
                }}
                title={messageQueue.length >= 10 ? t("queueFull") : streaming ? t("appendInstruction") : t("sendHint")}
              >
                <ArrowUp size={14} weight="bold" />
              </button>
            )}
          </div>
        </div>
      </div>

      {/* ═══ below-input metadata row ═══ */}
      <div style={{ display: "flex", alignItems: "center", gap: 2, padding: "4px 4px 0" }}>
        <button type="button"
          style={{ ...chipStyle, color: "var(--fill-quaternary)", fontSize: 11 }}
          onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; e.currentTarget.style.color = "var(--fill-tertiary)"; }}
          onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "var(--fill-quaternary)"; }}
          onClick={handleOpenWorkDir}
          title={workDir ? t("workDirTitle", { dir: workDir }) : t("setWorkDir")}
        >
          <Monitor size={12} />
          <span>{workDir ? workDir.replace(/^\/home\/[^/]+\//, "~/").replace(/^(.{24}).+/, "$1…") : t("workLocally", { ns: "common" })}</span>
          <span style={{ fontSize: 8, opacity: 0.4 }}>▾</span>
        </button>
        <button type="button"
          style={{ ...chipStyle, color: "var(--fill-quaternary)", fontSize: 11 }}
          onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; e.currentTarget.style.color = "var(--fill-tertiary)"; }}
          onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "var(--fill-quaternary)"; }}
          onClick={comingSoon} title={t("gitBranch")}
        >
          <GitBranch size={12} />
          <span>main</span>
          <span style={{ fontSize: 8, opacity: 0.4 }}>▾</span>
        </button>

        <div style={{ flex: 1 }} />

        <ModeSelector mode={composerMode} onSelectMode={handleSelectMode} disabled={streaming} />
        {activeChat?.usage?.contextTokens != null && activeChat?.usage?.contextWindow != null && activeChat.usage.contextWindow > 0 && (
          <ContextRing used={activeChat.usage.contextTokens} limit={activeChat.usage.contextWindow} />
        )}
      </div>

      <input ref={fileInputRef} type="file" multiple className="hidden"
        onChange={(e) => { if (e.target.files) processFiles(e.target.files); e.target.value = ""; }}
      />
    </>
  );
}
