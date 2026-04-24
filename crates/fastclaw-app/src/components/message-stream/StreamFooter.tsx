import {
  Image as ImageIcon, FileText, Paperclip, Settings2, FolderOpen, ArrowUp,
  Square, X,
} from "lucide-react";
import { useState } from "react";
import { MentionInput, type MentionInputHandle, type InlineMention, type MentionOption } from "./MentionInput";
import { useAgentStore } from "../../lib/agent-store";
import { QuestionPanel } from "./MessageRenderer";
import * as transport from "../../lib/transport";
import type { Chat } from "../../lib/agent-store";

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
    ? <ImageIcon size={12} strokeWidth={1.5} />
    : file.type.includes("pdf")
      ? <FileText size={12} strokeWidth={1.5} />
      : <Paperclip size={12} strokeWidth={1.5} />;

  if (isImage && file.previewUrl) {
    return (
      <div
        className="relative inline-block rounded-lg overflow-hidden"
        style={{
          border: `0.5px solid var(--separator)`,
          animation: "pop 0.2s ease-out",
        }}
      >
        <img
          src={file.previewUrl}
          alt={file.name}
          className="block max-h-[80px] max-w-[120px] object-cover"
        />
        <button
          onClick={onRemove}
          className="absolute top-0.5 right-0.5 flex h-4 w-4 cursor-pointer items-center justify-center rounded-full transition-colors duration-100"
          style={{ background: "rgba(0,0,0,0.5)", color: "#fff" }}
        >
          <X size={8} strokeWidth={2.5} />
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
        animation: "pop 0.2s ease-out",
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
        <X size={8} strokeWidth={2.5} />
      </button>
    </div>
  );
}

function ShortcutsHint() {
  return (
    <div
      className="flex items-center gap-3 text-[11px]"
      style={{ color: "var(--fill-quaternary)" }}
    >
      <span><kbd className="font-mono text-[10px]">Enter</kbd> 发送</span>
      <span><kbd className="font-mono text-[10px]">Shift+Enter</kbd> 换行</span>
      <span><kbd className="font-mono text-[10px]">{MOD_LABEL}+K</kbd> 新话题</span>
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
          style={{ transition: "stroke-dashoffset 0.4s ease, stroke 0.3s ease" }}
        />
      </svg>
      <span
        className="absolute text-[7px] font-bold tabular-nums leading-none"
        style={{ color }}
      >
        {pct < 1 ? "<1" : Math.round(pct)}
      </span>
      {hover && (
        <div
          className="absolute bottom-full mb-2 rounded-xl px-3 py-2.5"
          style={{
            background: "var(--bg-elevated)",
            border: "1px solid var(--separator)",
            boxShadow: "var(--shadow-lg)",
            color: "var(--fill-primary)",
            zIndex: 50,
            right: -8,
            minWidth: 180,
            animation: "fade-in 0.1s ease-out",
          }}
        >
          <div className="mb-1.5 text-[11px] font-semibold" style={{ color: "var(--fill-secondary)" }}>
            上下文窗口
          </div>
          <div className="mb-2 flex items-baseline gap-1">
            <span className="text-[16px] font-bold tabular-nums" style={{ color }}>{formatTokens(used)}</span>
            <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>/ {formatTokens(limit)} tokens</span>
          </div>
          <div
            className="mb-2 h-[4px] w-full overflow-hidden rounded-full"
            style={{ background: "var(--separator-opaque, #E2E8F0)" }}
          >
            <div
              className="h-full rounded-full"
              style={{
                width: `${pct}%`,
                background: color,
                transition: "width 0.3s ease",
              }}
            />
          </div>
          <div className="flex justify-between text-[10px]" style={{ color: "var(--fill-tertiary)" }}>
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
      )}
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
} | null;

export interface StreamFooterProps {
  mentionInputRef: React.RefObject<MentionInputHandle | null>;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  workDir: string | null;
  activeChat: Chat | null | undefined;
  streaming: boolean;
  detailOpen?: boolean;
  onToggleDetail?: () => void;
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
}

export function StreamFooter({
  mentionInputRef,
  fileInputRef,
  workDir,
  activeChat,
  streaming,
  detailOpen,
  onToggleDetail,
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
}: StreamFooterProps) {
  return (
    <div className="relative shrink-0 px-6 pb-5 pt-2">
      {pendingQuestion && (
        <QuestionPanel
          question={pendingQuestion}
          onAnswer={async (answer) => {
            setPendingQuestion(null);
            await transport.submitToolAnswerIpc(pendingQuestion.requestId, answer);
          }}
          onTimeout={() => setPendingQuestion(null)}
        />
      )}

      <div
        className="overflow-hidden rounded-2xl transition-shadow duration-200"
        style={{
          background: "var(--bg-elevated)",
          border: `1px solid var(--separator)`,
          boxShadow: "var(--shadow-md)",
        }}
      >
        {attachedFiles.length > 0 && (
          <div className="flex flex-wrap gap-1.5 px-4 pt-3" style={{ animation: "slide-up 0.15s ease-out" }}>
            {attachedFiles.map((f, i) => (
              <FilePill key={`${f.name}-${i}`} file={f} onRemove={() => removeFile(i)} />
            ))}
          </div>
        )}

        <MentionInput
          ref={mentionInputRef}
          disabled={streaming}
          placeholder="描述任务，或输入 @ 引用文件、目录、Skill..."
          options={mentionOptions}
          onSend={handleMentionSend}
          onNewTopic={handleNewTopic}
          onAttach={() => fileInputRef.current?.click()}
          onPasteFiles={processFiles}
        />

        <div className="flex items-center justify-between gap-2 px-3.5 pb-3">
          <div className="flex min-w-0 items-center gap-0.5">
            <button
              onClick={() => fileInputRef.current?.click()}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-tertiary)" }}
              title={`附件 (${MOD_KEY}${isMacPlatform ? "⇧" : "Shift+"}A)`}
            >
              <Paperclip size={16} strokeWidth={1.5} />
            </button>
            <button
              onClick={onToggleDetail}
              className="flex shrink-0 cursor-pointer items-center gap-1 rounded-lg px-2 py-1 text-[12px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{ color: detailOpen ? "var(--fill-secondary)" : "var(--fill-tertiary)" }}
              title="工具"
            >
              <Settings2 size={14} strokeWidth={1.5} />
              工具
            </button>

            <div className="mx-1 h-4 w-px shrink-0" style={{ background: "var(--separator)" }} />

            <button
              onClick={async () => {
                const currentState = useAgentStore.getState();
                const curAgentId = currentState.activeAgentId;
                const curAc = currentState.agentChats[curAgentId];
                const curChat = curAc?.chatList.find((c) => c.id === curAc.activeChatId);
                if (!curChat) return;
                let selected: string | null = null;
                try {
                  const { open: tauriOpenDialog } = await import("@tauri-apps/plugin-dialog");
                  selected = await tauriOpenDialog({ directory: true, multiple: false, defaultPath: curChat.workDir ?? undefined }) as string | null;
                } catch {
                  selected = prompt("输入工作目录路径:", curChat.workDir ?? "");
                }
                if (typeof selected === "string" && selected) {
                  setWorkDir(curAgentId, curChat.id, selected);
                }
              }}
              className="flex min-w-0 items-center gap-1.5 rounded-lg px-2 py-1 text-[12px] transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{ color: workDir ? "var(--fill-secondary)" : "var(--fill-quaternary)" }}
              title={workDir ? `工作目录: ${workDir}` : "设置工作目录"}
            >
              <FolderOpen className="shrink-0" size={13} strokeWidth={1.5} />
              <span className="max-w-[120px] truncate font-mono text-[11px]">
                {workDir ? workDir.replace(/^\/home\/[^/]+\//, "~/") : "工作目录"}
              </span>
            </button>

            {!detailOpen && (
              <>
                <div className="mx-1 h-4 w-px shrink-0" style={{ background: "var(--separator)" }} />
                <ShortcutsHint />
              </>
            )}
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {activeChat?.usage?.contextTokens != null && activeChat?.usage?.contextWindow != null && activeChat.usage.contextWindow > 0 && (
              <ContextRing
                used={activeChat.usage.contextTokens}
                limit={activeChat.usage.contextWindow}
              />
            )}
            {streaming ? (
              <button
                key="stop"
                onClick={stopStream}
                className="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors duration-150"
                style={{
                  background: "var(--fill-warning, #ED8936)",
                  color: "#fff",
                }}
                title="停止生成"
              >
                <Square size={12} strokeWidth={2.5} fill="currentColor" />
              </button>
            ) : (
              <button
                key="send"
                onClick={() => {
                  const ref = mentionInputRef.current;
                  if (ref) {
                    const t = ref.getText().trim();
                    if (t) handleMentionSend(t, ref.getMentions());
                  }
                }}
                className="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full transition-all duration-150 hover:brightness-110 active:scale-95 disabled:opacity-25"
                style={{
                  background: "var(--tint)",
                  color: "#fff",
                }}
                title="发送 ↩"
              >
                <ArrowUp size={16} strokeWidth={2} />
              </button>
            )}
          </div>
        </div>
      </div>

      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden"
        onChange={(e) => { if (e.target.files) processFiles(e.target.files); e.target.value = ""; }}
      />
    </div>
  );
}

export { isMacPlatform, MOD_KEY, MOD_LABEL };
