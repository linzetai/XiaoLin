import { useState, useRef, useCallback, useEffect, useMemo, type MutableRefObject, type RefObject, type Dispatch, type SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import { useStreamStore } from "../../lib/stores/stream-store";
import { useQueueStore } from "../../lib/stores/queue-store";
import { useLocaleStore } from "../../lib/stores/locale-store";
import { useTerminalStore } from "../../lib/stores/terminal-store";
import { usePtyStore } from "../../lib/stores/pty-store";
import { handleGoalClearedForChat, handleGoalUpdatedForChat } from "../../lib/stores/goal-store";
import type { ChatMessage, SubAgentRunUI } from "../../lib/stores/types";
import { type ToolCall } from "./ToolCallCard";
import type { MentionInputHandle, InlineMention } from "./MentionInput";
import type { AttachedFile } from "./StreamFooter";
import * as transport from "../../lib/transport";
import type { StreamSegment } from "./types";
import { useWorkspaceTabs } from "../shell/workspace-tabs";
import { detachedStreams, MAX_DETACHED_STREAMS } from "./messageStreamRegistry";
import { sendNotification, isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

let _notifPermission: boolean | null = null;
async function notifyIfBackground(title: string, body: string) {
  if (document.hasFocus()) return;
  if (_notifPermission === null) {
    _notifPermission = await isPermissionGranted();
    if (!_notifPermission) {
      const perm = await requestPermission();
      _notifPermission = perm === "granted";
    }
  }
  if (_notifPermission) {
    sendNotification({ title, body });
    getCurrentWindow().setFocus().catch(() => {});
  }
}

export function useMessageStreamChat({
  activeAgentId,
  activeChat,
  workDir,
  chatScrollKey,
  scrollPositions,
  mentionInputRef,
  attachedFilesRef,
  setAttachedFiles,
}: {
  activeAgentId: string;
  activeChat: { id: string; localKey?: string; workDir?: string | null } | undefined;
  workDir: string | null;
  chatScrollKey: (chatId: string | undefined) => string | undefined;
  scrollPositions: MutableRefObject<Record<string, number>>;
  mentionInputRef: RefObject<MentionInputHandle | null>;
  attachedFilesRef: MutableRefObject<AttachedFile[]>;
  setAttachedFiles: Dispatch<SetStateAction<AttachedFile[]>>;
}) {
  const { t } = useTranslation("chat");
  const newChat = useChatMetaStore((s) => s.newChat);
  const updateChatBackendId = useChatMetaStore((s) => s.updateChatBackendId);
  const setChatExecutionMode = useChatMetaStore((s) => s.setChatExecutionMode);
  const setChatPlanFile = useChatMetaStore((s) => s.setChatPlanFile);
  const setChatPlanApprovalPending = useChatMetaStore((s) => s.setChatPlanApprovalPending);
  const updateChatUsage = useStreamStore((s) => s.updateChatUsage);
  const subAgentStart = useStreamStore((s) => s.subAgentStart);
  const subAgentDelta = useStreamStore((s) => s.subAgentDelta);
  const subAgentToolStart = useStreamStore((s) => s.subAgentToolStart);
  const subAgentToolDone = useStreamStore((s) => s.subAgentToolDone);
  const subAgentComplete = useStreamStore((s) => s.subAgentComplete);
  const subAgentNotification = useStreamStore((s) => s.subAgentNotification);
  const addBriefMessage = useStreamStore((s) => s.addBriefMessage);

  const decisionLabel = useCallback((decision: string) =>
    decision === "approved" ? t("decision_approved")
      : decision === "approved_for_session" ? t("decision_approvedSession")
      : decision === "denied" ? t("decision_denied")
      : decision === "abort" ? t("decision_abort")
      : decision,
  [t]);

  const addMessage = useCallback((
    msg: Omit<ChatMessage, "id" | "chatId">,
    targetChatId?: string,
  ) => {
    const chatId = targetChatId ?? useChatMetaStore.getState().activeChatId;
    useStreamStore.getState().addMessage(chatId, msg);
    const title = msg.role === "user" ? msg.content.slice(0, 20) : undefined;
    useChatMetaStore.getState().incrementMessageCount(chatId, title);
  }, []);

  const [streaming, setStreaming] = useState(false);
  const streamingRef = useRef(streaming);
  streamingRef.current = streaming;
  const activeAgentIdRef = useRef(activeAgentId);
  activeAgentIdRef.current = activeAgentId;
  const [streamSegments, setStreamSegments] = useState<StreamSegment[]>([]);
  const segmentsRef = useRef<StreamSegment[]>([]);
  const [pendingQuestion, setPendingQuestion] = useState<{
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
      paths?: string[];
      cwd?: string;
      content?: string;
      diff?: string;
      riskLevel?: "low" | "medium" | "high";
      policyAmendPrefix?: string[];
    };
  } | null>(null);
  const streamAccRef = useRef("");
  const rafIdRef = useRef(0);
  const currentStreamChatRef = useRef<string | null>(null);
  const cleanupRef = useRef<(() => void) | null>(null);

  const cancelScheduledFlush = useCallback(() => {
    if (rafIdRef.current) {
      cancelAnimationFrame(rafIdRef.current);
      clearTimeout(rafIdRef.current);
      rafIdRef.current = 0;
    }
  }, []);

  const drafts = useRef<Record<string, string>>({});

  const atBottomRef = useRef(true);
  const suppressScrollTrackingUntilRef = useRef(0);
  const pendingBottomScrollBehaviorRef = useRef<"auto" | "smooth" | null>(null);
  const pendingRestoreScrollTopRef = useRef<number | null>(null);
  const runProgrammaticScroll = useCallback((action: () => void, suppressMs = 280) => {
    suppressScrollTrackingUntilRef.current = Date.now() + suppressMs;
    action();
  }, []);
  const requestBottomScroll = useCallback((behavior: "auto" | "smooth") => {
    pendingBottomScrollBehaviorRef.current = behavior;
  }, []);

  const prevChatIdRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    const prevId = prevChatIdRef.current;
    const newId = activeChat?.id;
    prevChatIdRef.current = newId;

    if (prevId && prevId !== newId && mentionInputRef.current) {
      const currentText = mentionInputRef.current.getText();
      if (currentText.trim()) {
        drafts.current[prevId] = currentText;
      } else {
        delete drafts.current[prevId];
      }
    }

    if (newId && mentionInputRef.current) {
      const saved = drafts.current[newId] ?? "";
      mentionInputRef.current.setText(saved);
    }

    if (prevId && prevId !== newId && streamingRef.current) {
      if (detachedStreams.size >= MAX_DETACHED_STREAMS) {
        const oldestKey = detachedStreams.keys().next().value;
        if (oldestKey) detachedStreams.delete(oldestKey);
      }
      detachedStreams.set(prevId, {
        agentId: activeAgentIdRef.current,
        chatId: prevId,
        acc: streamAccRef.current,
        toolCalls: segmentsRef.current.filter((s) => s.type === "tool" && s.toolCall).map((s) => s.toolCall!),
        scrollPosition: (() => {
          const key = chatScrollKey(prevId);
          return key ? scrollPositions.current[key] : undefined;
        })(),
        done: false,
        error: false,
        cleanup: () => {},
        needsAttention: false,
      });
      currentStreamChatRef.current = null;
      setStreaming(false);
      setPendingQuestion(null);
      segmentsRef.current = [];
      setStreamSegments([]);
      pendingBottomScrollBehaviorRef.current = null;
    }

    if (newId && detachedStreams.has(newId)) {
      const ds = detachedStreams.get(newId)!;
      if (ds.done) {
        detachedStreams.delete(newId);
      } else {
        streamAccRef.current = ds.acc;
        currentStreamChatRef.current = newId;
        const restored: StreamSegment[] = [];
        if (ds.acc) restored.push({ id: "text-0", type: "text", content: ds.acc });
        for (const tc of ds.toolCalls) {
          restored.push({ id: `tool-${tc.id}`, type: "tool", toolCall: tc });
        }
        segmentsRef.current = restored;
        setStreamSegments([...restored]);
        setStreaming(true);
        if (ds.scrollPosition != null) {
          const key = chatScrollKey(newId);
          if (key) scrollPositions.current[key] = ds.scrollPosition;
        }
        if (ds.pendingInteraction) {
          const pi = ds.pendingInteraction;
          if (pi.type === "approval_required") {
            const d = pi.data;
            const approvalId = d.approval_id as string;
            const reason = d.reason as string;
            const action = d.action as Record<string, unknown> | undefined;
            const actionType = action?.action_type as string ?? "unknown";
            const decisions = (d.available_decisions as Array<{decision: string; prefix?: string[]}>) ?? [];
            const riskLevel = ((d.risk_level as string) || "medium") as "low" | "medium" | "high";
            const policyAmendDec = decisions.find((dec) => dec.decision === "approved_with_policy_amend");
            setPendingQuestion({
              requestId: `approval:${approvalId}`,
              question: t("approval_actionType", { reason, actionType }),
              options: decisions.map((dec) => ({
                id: dec.decision,
                label: decisionLabel(dec.decision),
                ...(dec.prefix ? { prefix: dec.prefix } : {}),
              })),
              timeoutSecs: 0,
              expiresAt: 0,
              allowMultiple: false,
              approvalMeta: {
                actionType,
                command: action?.command as string | undefined,
                path: action?.path as string | undefined,
                paths: action?.paths as string[] | undefined,
                cwd: action?.cwd as string | undefined,
                content: action?.content as string | undefined,
                diff: action?.diff as string | undefined,
                riskLevel,
                policyAmendPrefix: policyAmendDec?.prefix,
              },
            });
          } else if (pi.type === "ask_question") {
            const d = pi.data;
            const timeoutSecs = (d.timeout_secs as number) ?? 0;
            setPendingQuestion({
              requestId: d.request_id as string,
              question: d.question as string,
              options: (d.options as Array<{ id: string; label: string }>) ?? [],
              timeoutSecs,
              expiresAt: timeoutSecs > 0 ? Date.now() + timeoutSecs * 1000 : 0,
              allowMultiple: d.allow_multiple as boolean | undefined,
            });
          }
        }
        detachedStreams.delete(newId);
      }
    }
  }, [activeChat?.id, chatScrollKey, t, decisionLabel]);

  useEffect(() => {
    return () => {
      for (const [chatId, ds] of detachedStreams) {
        if (ds.agentId === activeAgentId) {
          detachedStreams.delete(chatId);
        }
      }
    };
  }, [activeAgentId]);

  const sendWithContent = async (txt: string, mentions: InlineMention[], options?: { goalMode?: boolean }) => {
    const mentionDesc = mentions.length > 0
      ? t("mentionDesc", { items: mentions.map((m) => `@${m.label} (${m.type})`).join(", ") })
      : "";
    const attached = attachedFilesRef.current;
    const nonImageFiles = attached.filter((f) => !f.type.startsWith("image/"));
    const imageFiles = attached.filter((f) => f.type.startsWith("image/"));
    const fileDesc = nonImageFiles.length > 0
      ? t("attachmentDesc", { items: nonImageFiles.map((f) => f.name).join(", ") })
      : "";

    const imageDataUrls: Array<{ url: string; alt?: string }> = [];
    if (imageFiles.length > 0) {
      await Promise.all(
        imageFiles.map(
          (af) =>
            new Promise<void>((resolve) => {
              const reader = new FileReader();
              reader.onload = () => {
                if (typeof reader.result === "string") {
                  imageDataUrls.push({ url: reader.result, alt: af.name });
                }
                resolve();
              };
              reader.onerror = () => resolve();
              reader.readAsDataURL(af.file);
            }),
        ),
      );
    }

    const metaState = useChatMetaStore.getState();
    const capturedAgentId = metaState.activeAgentId;
    const currentAgent = metaState.agents.find((a) => a.id === capturedAgentId);
    const capturedChatId = metaState.activeChatId;
    const currentActiveChat = metaState.chats[capturedChatId];

    addMessage({
      role: "user",
      content: txt + mentionDesc + fileDesc,
      timestamp: new Date(),
      images: imageDataUrls.length > 0 ? imageDataUrls : undefined,
    }, capturedChatId);
    if (cleanupRef.current) {
      cleanupRef.current();
      cleanupRef.current = null;
    }
    atBottomRef.current = true;
    setStreaming(true);
    requestBottomScroll("auto");
    segmentsRef.current = [];
    setStreamSegments([]);
    streamAccRef.current = "";
    currentStreamChatRef.current = capturedChatId;

    const isActive = () => currentStreamChatRef.current === capturedChatId;

    const flushSegments = () => {
      if (!isActive()) return;
      if (rafIdRef.current) return;
      const runFlush = () => {
        rafIdRef.current = 0;
        if (isActive()) setStreamSegments(segmentsRef.current.map((s) => ({ ...s, toolCall: s.toolCall ? { ...s.toolCall } : undefined })));
      };
      // rAF pauses when the window is hidden (WebKitGTK); fall back to setTimeout.
      rafIdRef.current =
        typeof document !== "undefined" && document.hidden
          ? window.setTimeout(runFlush, 16)
          : requestAnimationFrame(runFlush);
    };

    const appendText = (c: string) => {
      streamAccRef.current += c;
      const segs = segmentsRef.current;
      const last = segs[segs.length - 1];
      if (last && last.type === "text") {
        last.content = (last.content ?? "") + c;
      } else {
        segs.push({ id: `text-${segs.length}`, type: "text", content: c });
      }
    };

    let messageContent: string | unknown[];
    if (imageDataUrls.length > 0) {
      const parts: unknown[] = [];
      const textBody = txt + mentionDesc + fileDesc;
      if (textBody) parts.push({ type: "text", text: textBody });
      for (const img of imageDataUrls) {
        parts.push({ type: "image_url", image_url: { url: img.url } });
      }
      messageContent = parts;
      console.log(`[XiaoLin] Sending ${imageDataUrls.length} image(s), data URL sizes:`, imageDataUrls.map((img) => `${(img.url.length / 1024).toFixed(1)}KB`));
    } else {
      messageContent = txt + mentionDesc + fileDesc;
    }

    const resolvedLang = useLocaleStore.getState().resolvedResponseLang();
    const { promise: chatPromise, cleanup } = transport.chatStream(
      {
        messages: [{ role: "user", content: messageContent }],
        agentId: capturedAgentId,
        sessionId: capturedChatId,
        model: currentAgent?.model || undefined,
        workDir: currentActiveChat?.workDir ?? undefined,
        responseLanguage: resolvedLang,
        goalMode: options?.goalMode,
      },
      (event) => {
        switch (event.type) {
          case "turn_start": {
            streamAccRef.current = "";
            segmentsRef.current = [];
            const ds = detachedStreams.get(capturedChatId);
            if (ds) ds.acc = "";
            const startSid = event.data?.session_id as string | undefined;
            if (startSid && capturedChatId !== startSid) {
              updateChatBackendId(capturedChatId, startSid);
            }
            setChatPlanApprovalPending(capturedChatId, false);
            break;
          }
          case "content_delta": {
            const delta = event.data?.delta as Record<string, unknown> | undefined;
            if (!delta) return;
            const text = (delta as { choices?: Array<{ delta?: { content?: string } }> })
              ?.choices?.[0]?.delta?.content;
            if (!text) return;
            if (isActive()) {
              appendText(text);
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) ds.acc += text;
            }
            break;
          }
          case "reasoning_delta": {
            const content = (event.data?.content as string) ?? "";
            if (!content) break;
            if (isActive()) {
              const segs = segmentsRef.current;
              const last = segs[segs.length - 1];
              if (last && last.type === "reasoning") {
                last.content = (last.content ?? "") + content;
              } else {
                segs.push({ id: `reasoning-${segs.length}`, type: "reasoning", content });
              }
              flushSegments();
            }
            break;
          }
          case "iteration_boundary": {
            const iteration = (event.data?.iteration as number) ?? 0;
            if (isActive()) {
              const segs = segmentsRef.current;
              segs.push({ id: `iter-${iteration}`, type: "iteration_boundary", iteration });
              flushSegments();
            }
            break;
          }
          case "turn_end": {
            const d = event.data;
            const sid = d?.session_id as string | undefined;
            const summary = d?.summary as {
              tool_calls_made?: number;
              iterations?: number;
              usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number };
              elapsed_ms?: number;
              context_tokens?: number;
              context_window?: number;
            } | undefined;
            const ds = detachedStreams.get(capturedChatId);
            const finalContent = isActive() ? streamAccRef.current : ds?.acc ?? streamAccRef.current;
            const currentSegments = isActive() ? [...segmentsRef.current] : [];
            const savedToolCalls = currentSegments
              .filter((s) => s.type === "tool" && s.toolCall)
              .map((s) => {
                const tc = s.toolCall!;
                return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration, metadata: tc.metadata };
              });

            if (currentSegments.length > 0) {
              useStreamStore.getState().setChatLastSegments(capturedChatId, currentSegments.map((s) => ({
                id: s.id,
                type: s.type,
                content: s.content,
                iteration: s.iteration,
                toolCall: s.toolCall ? { id: s.toolCall.id, name: s.toolCall.name, status: s.toolCall.status, args: s.toolCall.args, result: s.toolCall.result, duration: s.toolCall.duration, metadata: s.toolCall.metadata } : undefined,
              })));
            }

            if (isActive()) {
              cancelScheduledFlush();
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
              setPendingQuestion(null);
            }

            addMessage({
              role: "assistant",
              content: finalContent,
              timestamp: new Date(),
              toolCalls: savedToolCalls.length > 0 ? savedToolCalls : undefined,
            }, capturedChatId);

            if (ds) {
              ds.done = true;
              detachedStreams.delete(capturedChatId);
            }

            const modeChange = d?.modeChange as { from?: string; to?: string } | undefined;
            if (modeChange?.to && (modeChange.to === "agent" || modeChange.to === "plan")) {
              setChatExecutionMode(capturedChatId, modeChange.to);
            }

            cleanup();
            useStreamStore.getState().clearToolProgress();

            if (sid && capturedChatId !== sid) {
              updateChatBackendId(capturedChatId, sid);
            }

            const usageData = summary?.usage;
            const elapsedMs = (d?.elapsedMs as number) ?? summary?.elapsed_ms ?? 0;
            const contextTokens = (d?.contextTokens as number) ?? summary?.context_tokens ?? undefined;
            const contextWindow = (d?.contextWindow as number) ?? summary?.context_window ?? undefined;
            if (usageData || elapsedMs || contextTokens) {
              const resolvedChatId = sid ?? capturedChatId;
              updateChatUsage(resolvedChatId, {
                promptTokens: usageData?.prompt_tokens ?? 0,
                completionTokens: usageData?.completion_tokens ?? 0,
                totalTokens: usageData?.total_tokens ?? 0,
                elapsedMs,
                contextTokens,
                contextWindow,
              });
            }

            if (isActive() && atBottomRef.current) {
              requestBottomScroll("smooth");
            }

            if (isActive()) {
              const queueState = useQueueStore.getState();
              const queue = queueState.queues[capturedChatId] ?? [];
              if (queue.length > 0) {
                const nextMsg = queueState.dequeueMessage(capturedChatId);
                if (nextMsg && nextMsg.status === "pending") {
                  setTimeout(() => {
                    sendRef.current(nextMsg.content, nextMsg.mentions.map((m: { type: "file" | "dir" | "skill"; id: string; label: string }) => ({
                      type: m.type,
                      id: m.id,
                      label: m.label,
                      start: 0,
                      end: 0,
                    })));
                  }, 300);
                }
              }
            }

            const turnEndReason = d?.reason as string | undefined;
            if (turnEndReason === "plan_approval_pending") {
              setChatPlanApprovalPending(sid ?? capturedChatId, true);
            }
            if (turnEndReason === "token_budget_reached") {
              const budgetUsage = summary?.usage;
              const completionTokens = budgetUsage?.completion_tokens ?? 0;
              addMessage({
                role: "system",
                content: `Token 预算已用尽（已生成 ~${completionTokens} tokens）。你可以继续追加预算或结束本次任务。`,
                timestamp: new Date(),
                metadata: {
                  action: "token_budget_reached",
                  completionTokens,
                  sessionId: sid ?? capturedChatId,
                },
              }, capturedChatId);
            }
            break;
          }
          case "turn_aborted": {
            const d = event.data;
            const reason = (d?.reason as string) ?? "interrupted";
            if (isActive()) {
              cancelScheduledFlush();
              const content = streamAccRef.current;
              const savedTC = segmentsRef.current
                .filter((s) => s.type === "tool" && s.toolCall)
                .map((s) => {
                  const tc = s.toolCall!;
                  return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration, metadata: tc.metadata };
                });
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
              setPendingQuestion(null);
              if (content) {
                addMessage({
                  role: "assistant",
                  content,
                  timestamp: new Date(),
                  toolCalls: savedTC.length > 0 ? savedTC : undefined,
                }, capturedChatId);
              }
              addMessage({
                role: "system",
                content: t("turnAborted", { reason }),
                timestamp: new Date(),
              }, capturedChatId);
            }
            const ds = detachedStreams.get(capturedChatId);
            if (ds) { ds.done = true; detachedStreams.delete(capturedChatId); }
            cleanup();
            break;
          }
          case "tool_executing": {
            const d = event.data;
            if (!d?.tool_name) return;
            const tc: ToolCall = {
              id: (d.call_id ?? d.tool_name) as string,
              name: d.tool_name as string,
              status: "running",
              args: d.args as string | undefined,
              startTime: Date.now(),
            };
            if (tc.name === "shell_exec") {
              let cmd: string | undefined;
              try { cmd = d.args ? JSON.parse(d.args as string)?.command : undefined; } catch { /* ignore */ }
              useTerminalStore.getState().startSession(tc.id, tc.name, cmd, capturedChatId);
            }
            if (isActive()) {
              const existing = segmentsRef.current.find((s) => s.type === "tool" && s.toolCall?.id === tc.id);
              if (existing?.toolCall) {
                existing.toolCall.status = "running";
                existing.toolCall.args = tc.args;
                existing.toolCall.startTime = tc.startTime;
              } else {
                segmentsRef.current.push({ id: `tool-${tc.id}`, type: "tool", toolCall: tc });
              }
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) ds.toolCalls = [...ds.toolCalls.filter((t) => t.id !== tc.id), tc];
            }
            break;
          }
          case "tool_progress": {
            const d = event.data;
            if (!d?.call_id) break;
            const callId = d.call_id as string;
            const partial = d.partial_output as string | undefined;
            if (partial) {
              useTerminalStore.getState().appendOutput(callId, partial);
            }
            useStreamStore.getState().setToolProgress(callId, {
              progress: d.progress as number | undefined,
              message: (d.message as string) || undefined,
            });
            break;
          }
          case "tool_result": {
            const d = event.data;
            if (!d?.tool_name) return;
            const callId = (d.call_id ?? d.tool_name) as string;
            const output = (d.display_output ?? d.output) as string | undefined;
            const meta = (d.metadata ?? null) as Record<string, unknown> | null;
            if (d.tool_name === "shell_exec") {
              useTerminalStore.getState().endSession(callId);
            }
            if (d.tool_name === "terminal_open" && d.success && output) {
              try {
                const parsed = JSON.parse(output);
                if (parsed.session_id) {
                  usePtyStore.getState().addSession({
                    id: parsed.session_id,
                    chatId: capturedChatId,
                    status: "connecting",
                    name: parsed.name ?? "Agent Terminal",
                    source: "agent",
                  });
                }
              } catch { /* ignore parse errors */ }
            }
            if (d.tool_name === "terminal_close" && d.success && output) {
              try {
                const parsed = JSON.parse(output);
                if (parsed.closed) {
                  usePtyStore.getState().updateSession(parsed.closed, { status: "closed" });
                }
              } catch { /* ignore parse errors */ }
            }
            if (isActive()) {
              const seg = segmentsRef.current.find((s) => s.type === "tool" && s.toolCall?.id === callId);
              if (seg?.toolCall) {
                seg.toolCall.status = d.success ? "success" : "error";
                seg.toolCall.result = output;
                seg.toolCall.duration = seg.toolCall.startTime ? Date.now() - seg.toolCall.startTime : undefined;
                seg.toolCall.metadata = meta;
              }
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) {
                ds.toolCalls = ds.toolCalls.map((t) =>
                  t.id === callId
                    ? { ...t, status: d.success ? "success" : "error", result: output, duration: t.startTime ? Date.now() - t.startTime : undefined, metadata: meta }
                    : t,
                );
              }
            }
            break;
          }
          case "ask_question": {
            const d = event.data;
            if (d?.request_id && d?.question) {
              if (isActive()) {
                const timeoutSecs = (d.timeout_secs as number) ?? 0;
                setPendingQuestion({
                  requestId: d.request_id as string,
                  question: d.question as string,
                  options: (d.options as Array<{ id: string; label: string }>) ?? [],
                  timeoutSecs,
                  expiresAt: timeoutSecs > 0 ? Date.now() + timeoutSecs * 1000 : 0,
                  allowMultiple: d.allow_multiple as boolean | undefined,
                });
                notifyIfBackground(t("notif_needAnswer"), (d.question as string).slice(0, 60));
              } else {
                const ds = detachedStreams.get(capturedChatId);
                if (ds) {
                  ds.pendingInteraction = { type: "ask_question", data: d as Record<string, unknown> };
                  ds.needsAttention = true;
                }
              }
            }
            break;
          }
          case "mode_change": {
            const d = event.data;
            const newMode = d?.to as string | undefined;
            if (newMode && (newMode === "agent" || newMode === "plan")) {
              setChatExecutionMode(capturedChatId, newMode);
            }
            break;
          }
          case "plan_file_update": {
            const d = event.data;
            if (d?.path) {
              const exists = (d.exists as boolean) ?? false;
              setChatPlanFile(capturedChatId, d.path as string, exists);
              if (exists) {
                const { panelOpen, activeTabId, tabs } = useWorkspaceTabs.getState();
                const hasPlanTab = tabs.some((t) => t.id === "plan");
                const manuallyDismissed = sessionStorage.getItem(`xiaolin:plan-panel-dismissed:${capturedChatId}`);
                if (hasPlanTab && !manuallyDismissed && !(panelOpen && activeTabId === "plan")) {
                  useWorkspaceTabs.getState().setActiveTab("plan");
                }
              }
            }
            break;
          }
          case "plan_delta": {
            // Handled directly by PlanPanel via onWsEvent subscription
            break;
          }
          case "plan_update": {
            // Handled directly by PlanPanel via onWsEvent subscription
            break;
          }
          case "goal_updated": {
            handleGoalUpdatedForChat(event, capturedChatId);
            break;
          }
          case "goal_cleared": {
            handleGoalClearedForChat(event, capturedChatId);
            break;
          }
          case "context_warning": {
            const d = event.data;
            if (d?.message && isActive()) {
              addMessage({
                role: "system",
                content: d.message as string,
                timestamp: new Date(),
              }, capturedChatId);
            }
            break;
          }
          case "context_usage_update": {
            const d = event.data;
            if (d?.used_tokens != null && d?.limit_tokens != null && isActive()) {
              const resolvedChatId = capturedChatId;
              updateChatUsage(resolvedChatId, {
                promptTokens: 0,
                completionTokens: 0,
                totalTokens: 0,
                elapsedMs: 0,
                contextTokens: d.used_tokens as number,
                contextWindow: d.limit_tokens as number,
              });
              if (d.compressed && (d.tokens_saved as number) > 0) {
                addMessage({
                  role: "system",
                  content: t("contextCompressed", {
                    tokens: Math.round((d.tokens_saved as number) / 1000 * 10) / 10,
                  }),
                  timestamp: new Date(),
                }, resolvedChatId);
              }
            }
            break;
          }
          case "sub_agent_start": {
            const d = event.data;
            if (!d?.run_id) break;
            const run: SubAgentRunUI = {
              runId: d.run_id as string,
              agentId: (d.agent_id ?? "default") as string,
              subagentType: (d.subagent_type ?? "general") as string,
              task: (d.task ?? "") as string,
              depth: (d.depth as number) ?? 1,
              status: "running",
              content: "",
              toolCalls: [],
              toolCallsMade: 0,
              iterations: 0,
              notifications: [],
            };
            subAgentStart(capturedChatId, run);
            break;
          }
          case "sub_agent_delta": {
            const d = event.data;
            if (d?.run_id && d?.content) {
              subAgentDelta(capturedChatId, d.run_id as string, d.content as string);
            }
            break;
          }
          case "sub_agent_tool_executing": {
            const d = event.data;
            if (d?.run_id && d?.tool_name) {
              subAgentToolStart(capturedChatId, d.run_id as string, {
                id: (d.call_id ?? d.tool_name) as string,
                name: d.tool_name as string,
                status: "running",
                args: d.args as string | undefined,
              });
            }
            break;
          }
          case "sub_agent_tool_result": {
            const d = event.data;
            if (d?.run_id && d?.call_id) {
              subAgentToolDone(
                capturedChatId,
                d.run_id as string, d.call_id as string,
                (d.output ?? "") as string, d.success as boolean,
              );
            }
            break;
          }
          case "sub_agent_complete": {
            const d = event.data;
            if (d?.run_id) {
              subAgentComplete(
                capturedChatId,
                d.run_id as string, (d.status ?? "completed") as string,
                d.result as string | undefined,
                d.tool_calls_made as number | undefined,
                d.iterations as number | undefined,
                d.elapsed_ms as number | undefined,
              );
            }
            break;
          }
          case "sub_agent_notification": {
            const d = event.data;
            if (d?.run_id && d?.message) {
              subAgentNotification(
                capturedChatId,
                d.run_id as string,
                d.message as string,
              );
            }
            break;
          }
          case "error": {
            const e = (event.data?.message as string) ?? event.error?.message ?? t("unknownError");
            // Always clear streaming on fatal error, even if chat switched (isActive() false)
            setStreaming(false);
            if (isActive()) {
              cancelScheduledFlush();
              streamAccRef.current = "";
              // Persist tool call segments (especially todo_write) before clearing
              const toolSegs = segmentsRef.current.filter(
                (s) => s.type === "tool" && s.toolCall && s.toolCall.status !== "running"
              );
              if (toolSegs.length > 0) {
                const toolCalls = toolSegs.map((s) => ({
                  id: s.toolCall!.id,
                  name: s.toolCall!.name,
                  status: s.toolCall!.status,
                  args: s.toolCall!.args,
                  result: s.toolCall!.result,
                  duration: s.toolCall!.duration,
                  metadata: s.toolCall!.metadata,
                }));
                const textSegs = segmentsRef.current.filter((s) => s.type === "text" && s.content);
                const partialContent = textSegs.map((s) => s.content).join("");
                addMessage({
                  role: "assistant",
                  content: partialContent,
                  timestamp: new Date(),
                  toolCalls,
                  metadata: { partial: true },
                }, capturedChatId);
              }
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setPendingQuestion(null);
            }
            addMessage({ role: "system", content: t("error_prefix", { message: e }), timestamp: new Date() }, capturedChatId);
            const ds = detachedStreams.get(capturedChatId);
            if (ds) { ds.error = true; ds.done = true; detachedStreams.delete(capturedChatId); }
            cleanup();

            if (isActive() && atBottomRef.current) {
              requestBottomScroll("smooth");
            }

            // 标记队列第一条为失败，继续处理下一条
            if (isActive()) {
              const queueState = useQueueStore.getState();
              const queue = queueState.queues[capturedChatId] ?? [];
              if (queue.length > 0) {
                const firstItem = queue[0];
                if (firstItem.status === "pending") {
                  queueState.updateQueuedMessage(capturedChatId, firstItem.id, {
                    status: "failed",
                    error: e,
                  });
                  // 继续处理下一个 pending
                  const nextPending = queue.find((m: { status: string; id: string }) => m.status === "pending" && m.id !== firstItem.id);
                  if (nextPending) {
                    setTimeout(() => {
                      sendRef.current(nextPending.content, nextPending.mentions.map((m: { type: "file" | "dir" | "skill"; id: string; label: string }) => ({
                        type: m.type,
                        id: m.id,
                        label: m.label,
                        start: 0,
                        end: 0,
                      })));
                    }, 300);
                  }
                }
              }
            }
            break;
          }
          case "stream_error": {
            const d = event.data;
            const msg = (d?.message as string) ?? t("streamError");
            const code = (d?.error_code as string) ?? "";
            const retry = (d?.retry_attempt as number) ?? 0;
            const recoverable = (d?.recoverable as boolean) ?? true;
            if (!recoverable) {
              setStreaming(false);
            }
            if (isActive()) {
              addMessage({
                role: "system",
                content: t("streamErrorDetail", {
                  msg,
                  codePart: code ? ` [${code}]` : "",
                  retryPart: retry > 0 ? t("streamErrorRetryPart", { retry }) : "",
                }),
                timestamp: new Date(),
              }, capturedChatId);
            }
            break;
          }
          case "warning": {
            const d = event.data;
            const msg = (d?.message as string) ?? "";
            if (msg && isActive()) {
              addMessage({
                role: "system",
                content: `⚠ ${msg}`,
                timestamp: new Date(),
              }, capturedChatId);
            }
            break;
          }
          case "approval_required": {
            const d = event.data;
            if (d?.approval_id && d?.reason) {
              if (isActive()) {
                const approvalId = d.approval_id as string;
                const reason = d.reason as string;
                const action = d.action as Record<string, unknown> | undefined;
                const actionType = action?.action_type as string ?? "unknown";
                const decisions = (d.available_decisions as Array<{decision: string; prefix?: string[]}>) ?? [];

                const riskLevel = ((d.risk_level as string) || "medium") as "low" | "medium" | "high";
                const policyAmendDec = decisions.find((dec) => dec.decision === "approved_with_policy_amend");

                setPendingQuestion({
                  requestId: `approval:${approvalId}`,
                  question: t("approval_actionType", { reason, actionType }),
                  options: decisions.map((dec) => ({
                    id: dec.decision,
                    label: decisionLabel(dec.decision),
                    ...(dec.prefix ? { prefix: dec.prefix } : {}),
                  })),
                  timeoutSecs: 0,
                  expiresAt: 0,
                  allowMultiple: false,
                  approvalMeta: {
                    actionType,
                    command: action?.command as string | undefined,
                    path: action?.path as string | undefined,
                    paths: action?.paths as string[] | undefined,
                    cwd: action?.cwd as string | undefined,
                    content: action?.content as string | undefined,
                    diff: action?.diff as string | undefined,
                    riskLevel,
                    policyAmendPrefix: policyAmendDec?.prefix,
                  },
                });
                notifyIfBackground(t("notif_needApproval"), reason);
              } else {
                const ds = detachedStreams.get(capturedChatId);
                if (ds) {
                  ds.pendingInteraction = { type: "approval_required", data: d as Record<string, unknown> };
                  ds.needsAttention = true;
                }
              }
            }
            break;
          }
          case "approval_resolved": {
            if (isActive()) {
              setPendingQuestion(null);
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) {
                ds.pendingInteraction = undefined;
                ds.needsAttention = false;
              }
            }
            break;
          }
          case "brief_message": {
            const d = event.data;
            if (d?.content && isActive()) {
              addBriefMessage(capturedChatId, {
                id: `brief-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
                content: d.content as string,
                mode: (d.mode as "normal" | "proactive") ?? "normal",
                timestamp: Date.now(),
              });
            }
            break;
          }
        }
      },
    );

    cleanupRef.current = cleanup;

    chatPromise.catch((err: unknown) => {
      if (isActive()) {
        setStreaming(false);
        setPendingQuestion(null);
        segmentsRef.current = [];
        currentStreamChatRef.current = null;
        setStreamSegments([]);
      }
      const errMsg = err instanceof Error ? err.message : t("connectionFailed");
      addMessage({ role: "system", content: t("error_prefix", { message: errMsg }), timestamp: new Date() }, capturedChatId);
      cleanup();
    });
  };

  const sendRef = useRef(sendWithContent);
  sendRef.current = sendWithContent;

  const handleSlashCommand = useCallback(
    async (command: string): Promise<boolean> => {
      const trimmed = command.trim();
      if (trimmed === "/init") {
        const wd = workDir ?? undefined;
        addMessage({ role: "user", content: "/init", timestamp: new Date() });
        try {
          const result = await transport.workspaceInit(wd);
          addMessage({
            role: "assistant",
            content: result.alreadyExists
              ? `\u2705 ${result.message}`
              : `\u2705 ${result.message}\n\nCreated:\n${(result.created ?? []).map(f => `- \`${f}\``).join("\n")}`,
            timestamp: new Date(),
          });
        } catch (e) {
          addMessage({
            role: "assistant",
            content: `\u274c init failed: ${e instanceof Error ? e.message : String(e)}`,
            timestamp: new Date(),
          });
        }
        return true;
      }
      return false;
    },
    [workDir, addMessage],
  );

  const handleMentionSend = useCallback(
    (txt: string, _mentions: InlineMention[], options?: { goalMode?: boolean }) => {
      if (!txt.trim()) return;
      const SLASH_COMMANDS = ["/init"];
      if (SLASH_COMMANDS.some(cmd => txt.trim() === cmd || txt.trim().startsWith(cmd + " "))) {
        mentionInputRef.current?.clear();
        handleSlashCommand(txt.trim());
        return;
      }
      mentionInputRef.current?.clear();

      if (streamingRef.current) {
        const sessionId = currentStreamChatRef.current;
        if (sessionId) {
          transport.chatSteer(sessionId, [{ role: "user", content: txt.trim() }]).catch(() => {});
        }
        addMessage({
          role: "user",
          content: txt.trim(),
          timestamp: new Date(),
          isSteer: true,
        }, activeChat?.id);
        setAttachedFiles([]);
        return;
      }

      setAttachedFiles((prev) => {
        prev.forEach((f) => { if (f.previewUrl) URL.revokeObjectURL(f.previewUrl); });
        return [];
      });
      sendRef.current(txt.trim(), _mentions, options);
    },
    [activeChat?.id, addMessage, handleSlashCommand],
  );

  const stopStream = useCallback(() => {
    const sessionId = currentStreamChatRef.current
      ?? useChatMetaStore.getState().activeChatId
      ?? undefined;

    if (sessionId) {
      transport.chatCancel(sessionId).catch(() => {});
    }

    if (cleanupRef.current) {
      cleanupRef.current();
      cleanupRef.current = null;
    }
    const content = streamAccRef.current;
    const savedTC = segmentsRef.current
      .filter((s) => s.type === "tool" && s.toolCall)
      .map((s) => {
        const tc = s.toolCall!;
        return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration, metadata: tc.metadata };
      });
    cancelScheduledFlush();
    streamAccRef.current = "";
    segmentsRef.current = [];
    setStreamSegments([]);
    if (content) {
      addMessage({
        role: "assistant",
        content,
        timestamp: new Date(),
        toolCalls: savedTC.length > 0 ? savedTC : undefined,
      }, currentStreamChatRef.current ?? undefined);
    }
    currentStreamChatRef.current = null;
    setStreaming(false);
    setPendingQuestion((prev) => {
      if (prev && !prev.requestId.startsWith("approval:")) {
        transport.submitToolAnswer(prev.requestId, "", sessionId);
      }
      return null;
    });
  }, [addMessage]);

  const handleNewTopic = useCallback(() => {
    if (streaming) return;
    newChat(workDir ?? undefined);
  }, [streaming, newChat, workDir]);

  const streamingChatIds = useMemo(() => {
    const ids = new Set<string>();
    if (streaming && currentStreamChatRef.current) ids.add(currentStreamChatRef.current);
    for (const [chatId, ds] of detachedStreams) {
      if (!ds.done) ids.add(chatId);
    }
    return ids;
  }, [streaming, detachedStreams]);

  const attentionChatIds = useMemo(() => {
    const ids = new Set<string>();
    for (const [chatId, ds] of detachedStreams) {
      if (ds.needsAttention) ids.add(chatId);
    }
    return ids;
  }, [streaming, detachedStreams]);

  useEffect(() => {
    getCurrentWindow().emit("tray-pending-update", pendingQuestion != null).catch(() => {});
  }, [pendingQuestion]);

  useEffect(() => {
    const unlisten = listen<{ content: string }>("quick-action-send", (event) => {
      const { content } = event.payload;
      if (content) handleMentionSend(content, []);
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [handleMentionSend]);

  return {
    streaming,
    streamSegments,
    pendingQuestion,
    setPendingQuestion,
    stopStream,
    handleMentionSend,
    handleNewTopic,
    streamingChatIds,
    attentionChatIds,
    atBottomRef,
    suppressScrollTrackingUntilRef,
    pendingBottomScrollBehaviorRef,
    pendingRestoreScrollTopRef,
    runProgrammaticScroll,
    requestBottomScroll,
  };
}
