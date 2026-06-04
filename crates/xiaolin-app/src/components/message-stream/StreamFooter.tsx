import {
  Image as ImageIcon, FileText, Paperclip, ArrowUp,
  Square, X, Loader2, Compass, Code2, ChevronDown,
  Plus, RefreshCw, GitBranch, Monitor,
} from "lucide-react";
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { createPortal } from "react-dom";
import { MentionInput, type MentionInputHandle, type InlineMention, type MentionOption, type SlashCommand } from "./MentionInput";
import {
  useChatMetaStore,
  useQueueStore,
  useActiveChatId,
  useChatQueue,
  useActiveStream,
} from "../../lib/stores";
import { ICON } from "../../lib/ui-tokens";
import { QuestionPanel } from "./MessageRenderer";
import { ApprovalCard, type ApprovalData } from "./ApprovalCard";
import { QueueIndicator } from "./QueueIndicator";
import { QueuePanel } from "./QueuePanel";
import { PermissionSelector } from "./PermissionSelector";
// TODO: 语音输入功能待完善本地 STT（whisper.cpp 或 API）后重新启用
// import { VoiceButton } from "../VoiceButton";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { useConfigStore } from "../../lib/stores/config-store";
import type { Chat } from "../../lib/stores/types";
import { openLightbox } from "../common/ImageLightbox";

const isMacPlatform = /Mac|iPhone|iPad/.test((navigator as { userAgentData?: { platform?: string } }).userAgentData?.platform ?? navigator.platform ?? "");
const MOD_KEY = isMacPlatform ? "⌘" : "Ctrl+";
const MOD_LABEL = isMacPlatform ? "⌘" : "Ctrl";
function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export interface AttachedFile {
  name: string;
  size: number;
  type: string;
  file: File;
  previewUrl?: string;
}

function FilePill({ file, onRemove }: { file: AttachedFile; onRemove: () => void }) {
  const isImage = file.type.startsWith("image/");
  const icon = isImage
    ? <ImageIcon {...ICON.sm} />
    : file.type.includes("pdf")
      ? <FileText {...ICON.sm} />
      : <Paperclip {...ICON.sm} />;

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
          <X size={10} strokeWidth={2} />
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
        <X size={10} strokeWidth={2} />
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
            上下文窗口
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
            <span>已用 {pct.toFixed(1)}%</span>
            <span>剩余 {formatTokens(remaining)}</span>
          </div>
          {warning && (
            <div
              className="mt-2 rounded-md px-2 py-1 text-[10px]"
              style={{
                background: critical ? "rgba(252,129,129,0.12)" : "rgba(237,137,54,0.12)",
                color: critical ? "var(--red, #FC8181)" : "var(--yellow, #ED8936)",
              }}
            >
              {critical ? "上下文即将溢出，建议开始新对话" : "上下文使用较高，较长对话可能被压缩"}
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
  const displayName = currentModel.split("/").pop() || currentModel || "选择模型";

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
        <ChevronDown {...ICON.sm} />
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
              <div className="px-3 py-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>暂无模型</div>
            )}
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}

function ModeToggle({
  mode,
  onToggle,
  disabled,
}: {
  mode: "agent" | "plan";
  onToggle: () => void;
  disabled: boolean;
}) {
  const isPlan = mode === "plan";
  return (
    <div
      className="flex h-7 items-center overflow-hidden rounded-lg text-[11px] font-medium"
      style={{ border: "1px solid var(--separator)" }}
    >
      <button
        onClick={!isPlan ? undefined : onToggle}
        disabled={disabled}
        className="flex h-full items-center gap-1 px-2.5 transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-50"
        style={{
          background: !isPlan ? "var(--tint-bg)" : "transparent",
          color: !isPlan ? "var(--tint)" : "var(--fill-tertiary)",
        }}
      >
        <Code2 size={12} strokeWidth={1.5} />
        Agent
      </button>
      <button
        onClick={isPlan ? undefined : onToggle}
        disabled={disabled}
        className="flex h-full items-center gap-1 px-2.5 transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-50"
        style={{
          background: isPlan ? "oklch(94% 0.05 310)" : "transparent",
          color: isPlan ? "oklch(56% 0.18 310)" : "var(--fill-tertiary)",
          borderLeft: "1px solid var(--separator)",
        }}
      >
        <Compass size={12} strokeWidth={1.5} />
        Plan
      </button>
    </div>
  );
}

export type PendingToolQuestion = {
  requestId: string;
  question: string;
  options: Array<{ id: string; label: string }>;
  timeoutSecs: number;
  expiresAt: number;
  allowMultiple?: boolean;
  approvalMeta?: {
    actionType?: string;
    command?: string;
    path?: string;
    content?: string;
    diff?: string;
    riskLevel?: "danger" | "caution" | "safe";
  };
} | null;

export interface StreamFooterProps {
  mentionInputRef: React.RefObject<MentionInputHandle | null>;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  workDir: string | null;
  activeChat: Chat | null | undefined;
  streaming: boolean;
  mentionOptions: MentionOption[];
  attachedFiles: AttachedFile[];
  removeFile: (index: number) => void;
  processFiles: (files: FileList | File[]) => void;
  handleMentionSend: (txt: string, mentions: InlineMention[]) => void;
  handleNewTopic: () => void;
  setWorkDir: (agentId: string, chatId: string, path: string) => void;
  pendingQuestion: PendingToolQuestion;
  setPendingQuestion: React.Dispatch<React.SetStateAction<PendingToolQuestion>>;
  stopStream: () => void;
  onTogglePlanPanel?: () => void;
}

function parseApprovalData(q: NonNullable<PendingToolQuestion>): ApprovalData {
  const meta = q.approvalMeta;
  const reason = q.question.split("\n")[0] || q.question;
  return {
    approvalId: q.requestId.slice("approval:".length),
    reason,
    action: meta ? {
      action_type: meta.actionType,
      command: meta.command,
      path: meta.path,
      content: meta.content,
      diff: meta.diff,
    } : {
      action_type: q.question.includes("操作类型:") ? q.question.split("操作类型:")[1]?.trim() : undefined,
    },
    decisions: q.options,
    riskLevel: meta?.riskLevel ?? "caution",
  };
}

export function StreamFooter({
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
  pendingQuestion,
  setPendingQuestion,
  stopStream,
  onTogglePlanPanel,
}: StreamFooterProps) {
  const [inputHasContent, setInputHasContent] = useState(false);
  const [sendPending, setSendPending] = useState(false);
  const [dragOver, setDragOver] = useState(false);
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

  const handleCompact = useCallback(() => {
    if (streaming) return;
    handleMentionSend("/compact", []);
  }, [streaming, handleMentionSend]);

  const handleToggleMode = useCallback(async () => {
    if (streaming) return;
    const newMode = executionMode === "plan" ? "agent" : "plan";
    const sessionId = activeChat?.id;
    const resp = await transport.setExecutionModeIpc(newMode, sessionId ?? undefined);
    if (resp.ok) {
      const { activeChatId: chatId, setChatExecutionMode } = useChatMetaStore.getState();
      setChatExecutionMode(chatId, newMode);
    }
  }, [streaming, executionMode, activeChat?.id]);

  const handlePlanSlash = useCallback(() => {
    if (streaming) return;
    handleToggleMode();
  }, [streaming, handleToggleMode]);

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
    { id: "new", label: "new", desc: "开始新话题", action: handleNewTopic },
    { id: "clear", label: "clear", desc: "新建对话（清空当前）", action: handleNewTopic },
    { id: "compact", label: "compact", desc: "压缩上下文以释放空间", action: handleCompact },
    { id: "plan", label: "plan", desc: executionMode === "plan" ? "切换到 Agent 模式" : "切换到 Plan 模式（只读探索）", action: handlePlanSlash },
    { id: "export-md", label: "export md", desc: "导出当前会话为 Markdown 文件", action: handleExportMd },
    { id: "export-json", label: "export json", desc: "导出当前会话为 JSON 文件", action: handleExportJson },
    { id: "model", label: "model", desc: "在消息中指定模型，如 /model gpt-4o" },
    { id: "tools", label: "tools", desc: "在消息中指定工具，如 /tools search" },
  ], [handleNewTopic, handleCompact, handlePlanSlash, handleExportMd, handleExportJson, executionMode]);

  const wrappedSend = useCallback((txt: string, mentions: InlineMention[]) => {
    setSendPending(true);
    setInputHasContent(false);
    handleMentionSend(txt, mentions);
  }, [handleMentionSend]);

  const stream = useActiveStream();
  const handleRecallLastMessage = useCallback((): string | null => {
    for (let i = stream.length - 1; i >= 0; i--) {
      const item = stream[i];
      if (item.type === "message" && item.data.role === "user") return item.data.content;
    }
    return null;
  }, [stream]);

  useEffect(() => {
    const handleDragEnter = (e: DragEvent) => {
      if (e.dataTransfer?.types.includes("Files")) {
        setDragOver(true);
      }
    };
    const handleDragLeave = (e: DragEvent) => {
      if (e.relatedTarget === null || !(e.currentTarget as Node)?.contains(e.relatedTarget as Node)) {
        setDragOver(false);
      }
    };
    const handleDragOver = (e: DragEvent) => {
      if (e.dataTransfer?.types.includes("Files")) {
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
      }
    };
    const handleDrop = (e: DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      if (e.dataTransfer?.files.length) {
        processFiles(e.dataTransfer.files);
      }
    };

    document.addEventListener("dragenter", handleDragEnter);
    document.addEventListener("dragleave", handleDragLeave);
    document.addEventListener("dragover", handleDragOver);
    document.addEventListener("drop", handleDrop);
    return () => {
      document.removeEventListener("dragenter", handleDragEnter);
      document.removeEventListener("dragleave", handleDragLeave);
      document.removeEventListener("dragover", handleDragOver);
      document.removeEventListener("drop", handleDrop);
    };
  }, [processFiles]);

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
      selected = prompt("输入工作目录路径:", curChat.workDir ?? "");
    }
    if (typeof selected === "string" && selected) {
      setWorkDir("", chatId, selected);
    }
  }, [setWorkDir]);

  const handleSendClick = useCallback(() => {
    const ref = mentionInputRef.current;
    if (ref) {
      const t = ref.getText().trim();
      if (t) wrappedSend(t, ref.getMentions());
    }
  }, [mentionInputRef, wrappedSend]);

  const comingSoon = useCallback((e: React.MouseEvent<HTMLButtonElement>) => {
    const btn = e.currentTarget;
    btn.style.background = "var(--tint-subtle)";
    btn.style.color = "var(--tint)";
    setTimeout(() => { btn.style.background = "transparent"; btn.style.color = "var(--fill-tertiary)"; }, 600);
  }, []);

  /* ── chip / icon shared styles ── */
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
  const ibIconStyle: React.CSSProperties = {
    width: 26, height: 26, borderRadius: 6,
    border: "none", background: "transparent",
    color: "var(--fill-quaternary)", cursor: "pointer",
    display: "flex", alignItems: "center", justifyContent: "center",
    transition: "color 0.12s, background 0.12s",
  };
  const ibIconHover = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.currentTarget.style.color = "var(--fill-tertiary)";
    e.currentTarget.style.background = "var(--bg-hover)";
  };
  const ibIconLeave = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.currentTarget.style.color = "var(--fill-quaternary)";
    e.currentTarget.style.background = "transparent";
  };

  return (
    <div className="input-wrap relative shrink-0" style={{ padding: "6px 28px 12px" }}>
      {/* Drag overlay */}
      {dragOver && (
        <div className="fixed inset-0 z-[9998] flex items-center justify-center" style={{ background: "rgba(0,0,0,0.4)" }}>
          <div
            className="flex h-48 w-72 flex-col items-center justify-center gap-3 rounded-2xl"
            style={{ background: "var(--bg-elevated)", border: "2px dashed var(--tint)", boxShadow: "var(--glow-tint)", animation: "drop-zone-pulse 2s ease-in-out infinite" }}
          >
            <Paperclip size={32} strokeWidth={1.5} style={{ color: "var(--tint)", animation: "icon-float 1.5s ease-in-out infinite" }} />
            <span className="text-[14px] font-medium" style={{ color: "var(--fill-primary)" }}>拖放文件到此处</span>
          </div>
        </div>
      )}

      {/* Pending question / approval */}
      {pendingQuestion && (
        pendingQuestion.requestId.startsWith("approval:") ? (
          <ApprovalCard
            data={parseApprovalData(pendingQuestion)}
            sessionId={activeChat?.id}
            onDecision={async (decision) => {
              const approvalId = pendingQuestion.requestId.slice("approval:".length);
              setPendingQuestion(null);
              await transport.resolveApproval(approvalId, decision, activeChat?.id);
            }}
          />
        ) : (
          <QuestionPanel
            question={pendingQuestion}
            onAnswer={async (answer) => {
              setPendingQuestion(null);
              await transport.submitToolAnswerIpc(pendingQuestion.requestId, answer, activeChat?.id);
            }}
            onTimeout={() => {
              const q = pendingQuestion;
              setPendingQuestion(null);
              if (q) transport.submitToolAnswerIpc(q.requestId, "", activeChat?.id);
            }}
          />
        )
      )}

      {/* ═══ input-box container ═══ */}
      <div
        className="input-box overflow-hidden"
        style={{
          border: "1.5px solid var(--bg-input-border)",
          borderRadius: 12,
          background: "var(--bg-card)",
          transition: "border-color 0.15s, box-shadow 0.15s",
        }}
        onFocusCapture={(e) => {
          const box = e.currentTarget;
          box.style.borderColor = "var(--accent, var(--tint))";
          box.style.boxShadow = "0 0 0 3px color-mix(in srgb, var(--accent, var(--tint)) 8%, transparent)";
        }}
        onBlurCapture={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node)) {
            const box = e.currentTarget;
            box.style.borderColor = "var(--bg-input-border)";
            box.style.boxShadow = "none";
          }
        }}
      >
        {/* Queue indicator */}
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

        {/* Attached files preview */}
        {attachedFiles.length > 0 && (
          <div className="flex flex-wrap gap-2 px-4 pt-3">
            {attachedFiles.map((f, i) => (
              <div key={`${f.name}-${i}`} style={{ animation: `fade-slide-up var(--duration-normal) var(--ease-out) ${i * 50}ms backwards` }}>
                <FilePill file={f} onRemove={() => removeFile(i)} />
              </div>
            ))}
          </div>
        )}

        {/* Plan mode indicator */}
        {executionMode === "plan" && (
          <button
            type="button" onClick={onTogglePlanPanel}
            className="flex w-full items-center gap-2 px-4 py-2 text-left text-[11px] transition-colors hover:brightness-110"
            style={{ background: "color-mix(in srgb, var(--tint, #4299E1) 6%, transparent)", borderBottom: "0.5px solid color-mix(in srgb, var(--tint, #4299E1) 15%, transparent)", color: "var(--tint, #4299E1)" }}
          >
            <Compass {...ICON.md} className="shrink-0" />
            <span className="min-w-0 truncate">
              Plan Mode — 只读探索模式
              {planFilePath && <span style={{ opacity: 0.7 }}>{" · "}{planFileExists ? "" : "(未创建) "}{planFilePath.replace(/^\/home\/[^/]+\//, "~/")}</span>}
            </span>
            <FileText {...ICON.sm} className="ml-auto shrink-0" style={{ opacity: 0.6 }} />
          </button>
        )}
        {executionMode === "agent" && planFileExists && planFilePath && (
          <button
            type="button" onClick={onTogglePlanPanel}
            className="flex w-full items-center gap-2 px-4 py-1.5 text-left text-[10px] transition-colors hover:brightness-110"
            style={{ background: "color-mix(in srgb, var(--tint, #4299E1) 3%, transparent)", borderBottom: "0.5px solid color-mix(in srgb, var(--tint, #4299E1) 10%, transparent)", color: "var(--fill-tertiary)" }}
          >
            <FileText {...ICON.sm} className="shrink-0" style={{ color: "var(--tint, #4299E1)", opacity: 0.7 }} />
            <span className="min-w-0 truncate">计划文件: {planFilePath.replace(/^\/home\/[^/]+\//, "~/")}</span>
          </button>
        )}

        {/* ── Textarea ── */}
        <div style={{ padding: "11px 14px 6px" }}>
          <MentionInput
            ref={mentionInputRef}
            placeholder={streaming ? "追加指令..." : executionMode === "plan" ? "描述规划任务，或输入 /plan 切换到 Agent..." : "描述任务，或输入 @ 引用文件、/ 命令..."}
            options={mentionOptions}
            slashCommands={slashCommands}
            onSend={wrappedSend}
            onNewTopic={handleNewTopic}
            onAttach={() => fileInputRef.current?.click()}
            onPasteFiles={processFiles}
            onRecallLastMessage={handleRecallLastMessage}
            onContentChange={setInputHasContent}
          />
        </div>

        {/* ── Inline toolbar (ib-bar) ── */}
        <div style={{ display: "flex", alignItems: "center", padding: "3px 10px 8px" }}>
          {/* ib-left */}
          <div style={{ display: "flex", alignItems: "center", gap: 2, flex: 1, minWidth: 0, overflow: "hidden" }}>
            {/* [1] + add button */}
            <button type="button" style={chipStyle} onMouseEnter={chipHover} onMouseLeave={chipLeave}
              onClick={() => fileInputRef.current?.click()} title={`附加文件 (${MOD_KEY}${isMacPlatform ? "⇧" : "Shift+"}A)`}
            >
              <Plus size={13} strokeWidth={1.6} />
            </button>
            {/* [2] permissions selector */}
            <PermissionSelector sessionId={activeChat?.id} disabled={streaming} />
            {/* [3] refresh placeholder */}
            <button type="button" style={chipStyle} onMouseEnter={chipHover} onMouseLeave={chipLeave}
              onClick={comingSoon} title="预留"
            >
              <RefreshCw size={13} strokeWidth={1.6} />
            </button>
            {/* [4] model selector */}
            <ModelSelector />
          </div>

          {/* ib-right */}
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            {/* attach icon */}
            <button type="button" style={ibIconStyle} onMouseEnter={ibIconHover} onMouseLeave={ibIconLeave}
              onClick={() => fileInputRef.current?.click()} title={`附件 (${MOD_KEY}${isMacPlatform ? "⇧" : "Shift+"}A)`}
            >
              <Paperclip size={15} strokeWidth={1.6} />
            </button>
            {/* send / stop button */}
            {streaming ? (
              <button
                key="stop" onClick={stopStream}
                className="flex shrink-0 cursor-pointer items-center justify-center rounded-full transition-all duration-150 hover:scale-105 active:scale-90"
                style={{ width: 28, height: 28, background: "var(--red, #FF3B30)", color: "#fff", boxShadow: "0 0 0 3px color-mix(in srgb, var(--red, #FF3B30) 20%, transparent)", animation: "glow-pulse 1.5s ease-in-out infinite" }}
                title="停止生成"
              >
                <Square size={12} strokeWidth={2.5} fill="currentColor" />
              </button>
            ) : sendPending ? (
              <button key="loading" disabled className="flex shrink-0 items-center justify-center rounded-full"
                style={{ width: 28, height: 28, background: "var(--tint)", color: "#fff", opacity: 0.7 }} title="发送中..."
              >
                <Loader2 size={14} strokeWidth={2} className="animate-spin" />
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
                title={messageQueue.length >= 10 ? "队列已满（最多10条）" : streaming ? "追加指令 ↩" : "发送 ↩"}
              >
                <ArrowUp size={14} strokeWidth={2.5} />
              </button>
            )}
          </div>
        </div>
      </div>

      {/* ═══ below-input metadata row ═══ */}
      <div style={{ display: "flex", alignItems: "center", gap: 2, padding: "4px 4px 0" }}>
        {/* Work locally / workDir */}
        <button type="button"
          style={{ ...chipStyle, color: "var(--fill-quaternary)", fontSize: 11 }}
          onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; e.currentTarget.style.color = "var(--fill-tertiary)"; }}
          onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "var(--fill-quaternary)"; }}
          onClick={handleOpenWorkDir}
          title={workDir ? `工作目录: ${workDir}` : "设置工作目录"}
        >
          <Monitor size={12} strokeWidth={1.8} />
          <span>{workDir ? workDir.replace(/^\/home\/[^/]+\//, "~/").replace(/^(.{24}).+/, "$1…") : "Work locally"}</span>
          <span style={{ fontSize: 8, opacity: 0.4 }}>▾</span>
        </button>
        {/* Branch */}
        <button type="button"
          style={{ ...chipStyle, color: "var(--fill-quaternary)", fontSize: 11 }}
          onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; e.currentTarget.style.color = "var(--fill-tertiary)"; }}
          onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "var(--fill-quaternary)"; }}
          onClick={comingSoon} title="Git 分支（即将推出）"
        >
          <GitBranch size={12} strokeWidth={1.8} />
          <span>main</span>
          <span style={{ fontSize: 8, opacity: 0.4 }}>▾</span>
        </button>

        <div style={{ flex: 1 }} />

        {/* Mode toggle + context ring */}
        <ModeToggle mode={executionMode} onToggle={handleToggleMode} disabled={streaming} />
        {activeChat?.usage?.contextTokens != null && activeChat?.usage?.contextWindow != null && activeChat.usage.contextWindow > 0 && (
          <ContextRing used={activeChat.usage.contextTokens} limit={activeChat.usage.contextWindow} />
        )}
      </div>

      <input ref={fileInputRef} type="file" multiple className="hidden"
        onChange={(e) => { if (e.target.files) processFiles(e.target.files); e.target.value = ""; }}
      />
    </div>
  );
}

export { isMacPlatform, MOD_KEY, MOD_LABEL };
