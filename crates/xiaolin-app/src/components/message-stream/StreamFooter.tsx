import { Paperclip } from "@phosphor-icons/react";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { useState, useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";
import type { MentionInputHandle, InlineMention, MentionOption } from "./MentionInput";
import { useActiveStream } from "../../lib/stores";
import { QuestionPanel } from "./MessageRenderer";
import { ApprovalCard, type ApprovalData } from "./ApprovalCard";
import { ComposerCore, type AttachedFile } from "./ComposerCore";
import * as transport from "../../lib/transport";
import type { Chat } from "../../lib/stores/types";

export type { AttachedFile } from "./ComposerCore";

export type PendingToolQuestion = {
  requestId: string;
  question: string;
  options: Array<{ id: string; label: string; prefix?: string[] }>;
  timeoutSecs: number;
  expiresAt: number;
  allowMultiple?: boolean;
  approvalMeta?: {
    actionType?: string;
    command?: string;
    path?: string;
    paths?: string[];
    cwd?: string;
    content?: string;
    diff?: string;
    riskLevel?: "low" | "medium" | "high";
    policyAmendPrefix?: string[];
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
      paths: meta.paths,
      cwd: meta.cwd,
      content: meta.content,
      diff: meta.diff,
    } : {
      action_type: q.question.includes("操作类型:") ? q.question.split("操作类型:")[1]?.trim()
        : q.question.includes("Action type:") ? q.question.split("Action type:")[1]?.trim() : undefined,
    },
    decisions: q.options,
    riskLevel: meta?.riskLevel ?? "medium",
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
  const { t } = useTranslation("chat");
  const [dragOver, setDragOver] = useState(false);

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

  const stream = useActiveStream();
  const handleRecallLastMessage = useCallback((): string | null => {
    for (let i = stream.length - 1; i >= 0; i--) {
      const item = stream[i];
      if (item.type === "message" && item.data.role === "user") {
        const content = item.data.content;
        if (Array.isArray(content)) {
          const textPart = content.find(
            (p: { type?: string }) => p.type === "text",
          );
          return textPart?.text ?? null;
        }
        return content;
      }
    }
    return null;
  }, [stream]);

  return (
    <div className="input-wrap relative shrink-0" style={{ padding: "6px clamp(24px, 5%, 80px) 12px" }}>
      {dragOver && (
        <div className="fixed inset-0 z-[9998] flex items-center justify-center" style={{ background: "rgba(0,0,0,0.4)" }}>
          <div
            className="flex h-48 w-72 flex-col items-center justify-center gap-3 rounded-2xl"
            style={{ background: "var(--bg-elevated)", border: "2px dashed var(--tint)", boxShadow: "var(--glow-tint)", animation: "drop-zone-pulse 2s ease-in-out infinite" }}
          >
            <Paperclip size={ICON_SIZE["2xl"]} style={{ color: "var(--tint)", animation: "icon-float 1.5s ease-in-out infinite" }} />
            <span className="text-[14px] font-medium" style={{ color: "var(--fill-primary)" }}>{t("dropFileToAttach")}</span>
          </div>
        </div>
      )}

      {pendingQuestion && (
        pendingQuestion.requestId.startsWith("approval:") ? (
          <ApprovalCard
            data={parseApprovalData(pendingQuestion)}
            sessionId={activeChat?.id}
            onDecision={async (decision, extra) => {
              const approvalId = pendingQuestion.requestId.slice("approval:".length);
              await transport.resolveApproval(approvalId, decision, activeChat?.id, extra);
              setPendingQuestion(null);
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

      <ComposerCore
        mentionInputRef={mentionInputRef}
        fileInputRef={fileInputRef}
        workDir={workDir}
        activeChat={activeChat}
        streaming={streaming}
        mentionOptions={mentionOptions}
        attachedFiles={attachedFiles}
        removeFile={removeFile}
        processFiles={processFiles}
        handleMentionSend={handleMentionSend}
        handleNewTopic={handleNewTopic}
        setWorkDir={setWorkDir}
        stopStream={stopStream}
        onTogglePlanPanel={onTogglePlanPanel}
        onRecallLastMessage={handleRecallLastMessage}
      />
    </div>
  );
}

const isMacPlatform = /Mac|iPhone|iPad/.test((navigator as { userAgentData?: { platform?: string } }).userAgentData?.platform ?? navigator.platform ?? "");
const MOD_KEY = isMacPlatform ? "⌘" : "Ctrl+";
const MOD_LABEL = isMacPlatform ? "⌘" : "Ctrl";

export { isMacPlatform, MOD_KEY, MOD_LABEL };
