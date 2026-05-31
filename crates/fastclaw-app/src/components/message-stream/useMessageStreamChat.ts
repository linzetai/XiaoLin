import { useState, useRef, useCallback, useEffect, useMemo, type MutableRefObject, type RefObject, type Dispatch, type SetStateAction } from "react";
import { useAgentStore, type SubAgentRunUI } from "../../lib/agent-store";
import { type ToolCall } from "./ToolCallCard";
import type { MentionInputHandle, InlineMention } from "./MentionInput";
import type { AttachedFile } from "./StreamFooter";
import * as transport from "../../lib/transport";
import type { StreamSegment } from "./types";
import { detachedStreams, MAX_DETACHED_STREAMS } from "./messageStreamRegistry";

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
  const addMessage = useAgentStore((s) => s.addMessage);
  const newChat = useAgentStore((s) => s.newChat);
  const updateChatBackendId = useAgentStore((s) => s.updateChatBackendId);
  const updateChatUsage = useAgentStore((s) => s.updateChatUsage);
  const setChatExecutionMode = useAgentStore((s) => s.setChatExecutionMode);
  const setChatPlanFile = useAgentStore((s) => s.setChatPlanFile);
  const subAgentStart = useAgentStore((s) => s.subAgentStart);
  const subAgentDelta = useAgentStore((s) => s.subAgentDelta);
  const subAgentToolStart = useAgentStore((s) => s.subAgentToolStart);
  const subAgentToolDone = useAgentStore((s) => s.subAgentToolDone);
  const subAgentComplete = useAgentStore((s) => s.subAgentComplete);
  const enqueueMessage = useAgentStore((s) => s.enqueueMessage);

  const [streaming, setStreaming] = useState(false);
  const [streamSegments, setStreamSegments] = useState<StreamSegment[]>([]);
  const segmentsRef = useRef<StreamSegment[]>([]);
  const [pendingQuestion, setPendingQuestion] = useState<{
    requestId: string;
    question: string;
    options: Array<{ id: string; label: string }>;
    timeoutSecs: number;
    expiresAt: number;
    allowMultiple?: boolean;
  } | null>(null);
  const streamAccRef = useRef("");
  const rafIdRef = useRef(0);
  const currentStreamChatRef = useRef<string | null>(null);
  const cleanupRef = useRef<(() => void) | null>(null);

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

    if (prevId && prevId !== newId && streaming) {
      if (detachedStreams.size >= MAX_DETACHED_STREAMS) {
        const oldestKey = detachedStreams.keys().next().value;
        if (oldestKey) detachedStreams.delete(oldestKey);
      }
      detachedStreams.set(prevId, {
        agentId: activeAgentId,
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
        detachedStreams.delete(newId);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeChat?.id, chatScrollKey]);

  useEffect(() => {
    return () => {
      for (const [chatId, ds] of detachedStreams) {
        if (ds.agentId === activeAgentId) {
          detachedStreams.delete(chatId);
        }
      }
    };
  }, [activeAgentId]);

  const sendWithContent = async (txt: string, mentions: InlineMention[]) => {
    const mentionDesc = mentions.length > 0
      ? `\n\n[引用: ${mentions.map((m) => `@${m.label} (${m.type})`).join(", ")}]`
      : "";
    const attached = attachedFilesRef.current;
    const nonImageFiles = attached.filter((f) => !f.type.startsWith("image/"));
    const imageFiles = attached.filter((f) => f.type.startsWith("image/"));
    const fileDesc = nonImageFiles.length > 0
      ? `\n\n[附件: ${nonImageFiles.map((f) => f.name).join(", ")}]`
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

    const currentState = useAgentStore.getState();
    const capturedAgentId = currentState.activeAgentId;
    const currentAgent = currentState.agents.find((a) => a.id === capturedAgentId);
    const currentAc = currentState.agentChats[capturedAgentId];
    const currentActiveChat = currentAc?.chatList.find((c) => c.id === currentAc.activeChatId);
    const capturedChatId = currentActiveChat?.id ?? "temp";

    addMessage(capturedAgentId, {
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
      rafIdRef.current = requestAnimationFrame(() => {
        rafIdRef.current = 0;
        if (isActive()) setStreamSegments(segmentsRef.current.map((s) => ({ ...s, toolCall: s.toolCall ? { ...s.toolCall } : undefined })));
      });
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
      console.log(`[FastClaw] Sending ${imageDataUrls.length} image(s), data URL sizes:`, imageDataUrls.map((img) => `${(img.url.length / 1024).toFixed(1)}KB`));
    } else {
      messageContent = txt + mentionDesc + fileDesc;
    }

    const { promise: chatPromise, cleanup } = transport.chatStream(
      {
        messages: [{ role: "user", content: messageContent }],
        agentId: capturedAgentId,
        sessionId: capturedChatId,
        model: currentAgent?.model || undefined,
        workDir: currentActiveChat?.workDir ?? undefined,
      },
      (event) => {
        switch (event.type) {
          case "turn_start": {
            streamAccRef.current = "";
            segmentsRef.current = [];
            const ds = detachedStreams.get(capturedChatId);
            if (ds) ds.acc = "";
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
                return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration };
              });

            if (currentSegments.length > 0) {
              const setChatLastSegments = useAgentStore.getState().setChatLastSegments;
              setChatLastSegments(capturedAgentId, capturedChatId, currentSegments.map((s) => ({
                id: s.id,
                type: s.type,
                content: s.content,
                toolCall: s.toolCall ? { id: s.toolCall.id, name: s.toolCall.name, status: s.toolCall.status, args: s.toolCall.args, result: s.toolCall.result, duration: s.toolCall.duration } : undefined,
              })));
            }

            if (isActive()) {
              cancelAnimationFrame(rafIdRef.current);
              rafIdRef.current = 0;
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
              setPendingQuestion(null);
            }

            addMessage(capturedAgentId, {
              role: "assistant",
              content: finalContent,
              timestamp: new Date(),
              toolCalls: savedToolCalls.length > 0 ? savedToolCalls : undefined,
            }, capturedChatId);

            if (ds) {
              ds.done = true;
              detachedStreams.delete(capturedChatId);
            }
            cleanup();

            if (sid && capturedChatId !== sid) {
              updateChatBackendId(capturedAgentId, capturedChatId, sid);
            }

            const usageData = summary?.usage;
            const elapsedMs = (d?.elapsedMs as number) ?? summary?.elapsed_ms ?? 0;
            const contextTokens = (d?.contextTokens as number) ?? summary?.context_tokens ?? undefined;
            const contextWindow = (d?.contextWindow as number) ?? summary?.context_window ?? undefined;
            if (usageData || elapsedMs || contextTokens) {
              const resolvedChatId = sid ?? capturedChatId;
              updateChatUsage(capturedAgentId, resolvedChatId, {
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
              const state = useAgentStore.getState();
              const ac = state.agentChats[capturedAgentId];
              const queue = ac?.messageQueue ?? [];
              if (queue.length > 0) {
                const nextMsg = state.dequeueMessage(capturedAgentId, capturedChatId);
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
            break;
          }
          case "turn_aborted": {
            const d = event.data;
            const reason = (d?.reason as string) ?? "interrupted";
            if (isActive()) {
              cancelAnimationFrame(rafIdRef.current);
              rafIdRef.current = 0;
              const content = streamAccRef.current;
              const savedTC = segmentsRef.current
                .filter((s) => s.type === "tool" && s.toolCall)
                .map((s) => {
                  const tc = s.toolCall!;
                  return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration };
                });
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
              setPendingQuestion(null);
              if (content) {
                addMessage(capturedAgentId, {
                  role: "assistant",
                  content,
                  timestamp: new Date(),
                  toolCalls: savedTC.length > 0 ? savedTC : undefined,
                }, capturedChatId);
              }
              addMessage(capturedAgentId, {
                role: "system",
                content: `回合已中止: ${reason}`,
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
          case "tool_result": {
            const d = event.data;
            if (!d?.tool_name) return;
            const callId = (d.call_id ?? d.tool_name) as string;
            const output = (d.display_output ?? d.output) as string | undefined;
            if (isActive()) {
              const seg = segmentsRef.current.find((s) => s.type === "tool" && s.toolCall?.id === callId);
              if (seg?.toolCall) {
                seg.toolCall.status = d.success ? "success" : "error";
                seg.toolCall.result = output;
                seg.toolCall.duration = seg.toolCall.startTime ? Date.now() - seg.toolCall.startTime : undefined;
              }
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) {
                ds.toolCalls = ds.toolCalls.map((t) =>
                  t.id === callId
                    ? { ...t, status: d.success ? "success" : "error", result: output, duration: t.startTime ? Date.now() - t.startTime : undefined }
                    : t,
                );
              }
            }
            break;
          }
          case "ask_question": {
            const d = event.data;
            if (d?.request_id && d?.question && isActive()) {
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
            break;
          }
          case "mode_change": {
            const d = event.data;
            const newMode = d?.to as string | undefined;
            if (newMode && (newMode === "agent" || newMode === "plan")) {
              setChatExecutionMode(capturedAgentId, capturedChatId, newMode);
            }
            break;
          }
          case "plan_file_update": {
            const d = event.data;
            if (d?.path) {
              setChatPlanFile(capturedAgentId, capturedChatId, d.path as string, (d.exists as boolean) ?? false);
            }
            break;
          }
          case "context_warning": {
            const d = event.data;
            if (d?.message && isActive()) {
              addMessage(capturedAgentId, {
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
              updateChatUsage(capturedAgentId, resolvedChatId, {
                promptTokens: 0,
                completionTokens: 0,
                totalTokens: 0,
                elapsedMs: 0,
                contextTokens: d.used_tokens as number,
                contextWindow: d.limit_tokens as number,
              });
              if (d.compressed && (d.tokens_saved as number) > 0) {
                addMessage(capturedAgentId, {
                  role: "system",
                  content: `上下文已压缩，节省了约 ${Math.round((d.tokens_saved as number) / 1000 * 10) / 10}k tokens`,
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
            };
            subAgentStart(capturedAgentId, capturedChatId, run);
            break;
          }
          case "sub_agent_delta": {
            const d = event.data;
            if (d?.run_id && d?.content) {
              subAgentDelta(capturedAgentId, capturedChatId, d.run_id as string, d.content as string);
            }
            break;
          }
          case "sub_agent_tool_executing": {
            const d = event.data;
            if (d?.run_id && d?.tool_name) {
              subAgentToolStart(capturedAgentId, capturedChatId, d.run_id as string, {
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
                capturedAgentId, capturedChatId,
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
                capturedAgentId, capturedChatId,
                d.run_id as string, (d.status ?? "completed") as string,
                d.result as string | undefined,
                d.tool_calls_made as number | undefined,
                d.iterations as number | undefined,
                d.elapsed_ms as number | undefined,
              );
            }
            break;
          }
          case "error": {
            const e = (event.data?.message as string) ?? event.error?.message ?? "未知错误";
            if (isActive()) {
              cancelAnimationFrame(rafIdRef.current);
              rafIdRef.current = 0;
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
              setPendingQuestion(null);
            }
            addMessage(capturedAgentId, { role: "system", content: `错误: ${e}`, timestamp: new Date() }, capturedChatId);
            const ds = detachedStreams.get(capturedChatId);
            if (ds) { ds.error = true; ds.done = true; detachedStreams.delete(capturedChatId); }
            cleanup();

            if (isActive() && atBottomRef.current) {
              requestBottomScroll("smooth");
            }

            // 标记队列第一条为失败，继续处理下一条
            if (isActive()) {
              const state = useAgentStore.getState();
              const ac = state.agentChats[capturedAgentId];
              const queue = ac?.messageQueue ?? [];
              if (queue.length > 0) {
                const firstItem = queue[0];
                if (firstItem.status === "pending") {
                  state.updateQueuedMessage(capturedAgentId, capturedChatId, firstItem.id, {
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
            const msg = (d?.message as string) ?? "流错误";
            const code = (d?.error_code as string) ?? "";
            const retry = (d?.retry_attempt as number) ?? 0;
            if (isActive()) {
              addMessage(capturedAgentId, {
                role: "system",
                content: `流错误${code ? ` [${code}]` : ""}: ${msg}${retry > 0 ? ` (重试 #${retry})` : ""}`,
                timestamp: new Date(),
              }, capturedChatId);
            }
            break;
          }
          case "warning": {
            const d = event.data;
            const msg = (d?.message as string) ?? "";
            if (msg && isActive()) {
              addMessage(capturedAgentId, {
                role: "system",
                content: `⚠ ${msg}`,
                timestamp: new Date(),
              }, capturedChatId);
            }
            break;
          }
          case "approval_required": {
            const d = event.data;
            if (d?.approval_id && d?.reason && isActive()) {
              const approvalId = d.approval_id as string;
              const reason = d.reason as string;
              const action = d.action as Record<string, unknown> | undefined;
              const actionType = action?.action_type as string ?? "unknown";
              const decisions = (d.available_decisions as Array<{decision: string}>) ?? [];

              setPendingQuestion({
                requestId: `approval:${approvalId}`,
                question: `${reason}\n操作类型: ${actionType}`,
                options: decisions.map((dec) => {
                  const label = dec.decision === "approved" ? "批准"
                    : dec.decision === "approved_for_session" ? "本次全部批准"
                    : dec.decision === "denied" ? "拒绝"
                    : dec.decision === "abort" ? "中止"
                    : dec.decision;
                  return { id: dec.decision, label };
                }),
                timeoutSecs: 0,
                expiresAt: 0,
                allowMultiple: false,
              });
            }
            break;
          }
          case "approval_resolved": {
            if (isActive()) {
              setPendingQuestion(null);
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
      const errMsg = err instanceof Error ? err.message : "连接失败";
      addMessage(capturedAgentId, { role: "system", content: `错误: ${errMsg}`, timestamp: new Date() }, capturedChatId);
      cleanup();
    });
  };

  const sendRef = useRef(sendWithContent);
  sendRef.current = sendWithContent;
  const streamingRef = useRef(streaming);
  streamingRef.current = streaming;

  const handleSlashCommand = useCallback(
    async (command: string): Promise<boolean> => {
      const trimmed = command.trim();
      if (trimmed === "/init") {
        const wd = workDir ?? undefined;
        addMessage(activeAgentId, { role: "user", content: "/init", timestamp: new Date() });
        try {
          const result = await transport.workspaceInit(wd);
          addMessage(activeAgentId, {
            role: "assistant",
            content: result.alreadyExists
              ? `\u2705 ${result.message}`
              : `\u2705 ${result.message}\n\nCreated:\n${(result.created ?? []).map(f => `- \`${f}\``).join("\n")}`,
            timestamp: new Date(),
          });
        } catch (e) {
          addMessage(activeAgentId, {
            role: "assistant",
            content: `\u274c init failed: ${e instanceof Error ? e.message : String(e)}`,
            timestamp: new Date(),
          });
        }
        return true;
      }
      return false;
    },
    [activeAgentId, workDir, addMessage],
  );

  const handleMentionSend = useCallback(
    (txt: string, _mentions: InlineMention[]) => {
      if (!txt.trim()) return;
      const SLASH_COMMANDS = ["/init"];
      if (SLASH_COMMANDS.some(cmd => txt.trim() === cmd || txt.trim().startsWith(cmd + " "))) {
        mentionInputRef.current?.clear();
        handleSlashCommand(txt.trim());
        return;
      }
      mentionInputRef.current?.clear();

      if (streamingRef.current) {
        // 流式响应期间：加入队列
        const imageDataUrls = attachedFilesRef.current
          .filter(f => f.type.startsWith("image/"))
          .map(f => ({ url: f.previewUrl ?? "", alt: f.name }));
        enqueueMessage(activeAgentId, activeChat?.id ?? "", {
          content: txt.trim(),
          mentions: _mentions.map(m => ({ type: m.type, id: m.id, label: m.label })),
          images: imageDataUrls,
          status: "pending",
          createdAt: new Date(),
        });
        setAttachedFiles([]);
        return;
      }

      setAttachedFiles((prev) => {
        prev.forEach((f) => { if (f.previewUrl) URL.revokeObjectURL(f.previewUrl); });
        return [];
      });
      sendRef.current(txt.trim(), _mentions);
    },
    [activeAgentId, activeChat?.id, enqueueMessage, handleSlashCommand],
  );

  const stopStream = useCallback(() => {
    if (cleanupRef.current) {
      cleanupRef.current();
      cleanupRef.current = null;
    }
    const content = streamAccRef.current;
    const savedTC = segmentsRef.current
      .filter((s) => s.type === "tool" && s.toolCall)
      .map((s) => {
        const tc = s.toolCall!;
        return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration };
      });
    cancelAnimationFrame(rafIdRef.current);
    rafIdRef.current = 0;
    streamAccRef.current = "";
    segmentsRef.current = [];
    setStreamSegments([]);
    if (content) {
      addMessage(activeAgentId, {
        role: "assistant",
        content,
        timestamp: new Date(),
        toolCalls: savedTC.length > 0 ? savedTC : undefined,
      }, currentStreamChatRef.current ?? undefined);
    }
    const sessionId = currentStreamChatRef.current ?? undefined;
    currentStreamChatRef.current = null;
    setStreaming(false);
    setPendingQuestion((prev) => {
      if (prev && !prev.requestId.startsWith("approval:")) {
        transport.submitToolAnswer(prev.requestId, "", sessionId);
      }
      return null;
    });
  }, [activeAgentId, addMessage]);

  const handleNewTopic = useCallback(() => {
    if (streaming) return;
    newChat(activeAgentId, workDir ?? undefined);
  }, [streaming, newChat, activeAgentId, workDir]);

  const streamingChatIds = useMemo(() => {
    const ids = new Set<string>();
    if (streaming && currentStreamChatRef.current) ids.add(currentStreamChatRef.current);
    for (const [chatId, ds] of detachedStreams) {
      if (!ds.done) ids.add(chatId);
    }
    return ids;
  }, [streaming, detachedStreams]);

  return {
    streaming,
    streamSegments,
    pendingQuestion,
    setPendingQuestion,
    stopStream,
    handleMentionSend,
    handleNewTopic,
    streamingChatIds,
    atBottomRef,
    suppressScrollTrackingUntilRef,
    pendingBottomScrollBehaviorRef,
    pendingRestoreScrollTopRef,
    runProgrammaticScroll,
    requestBottomScroll,
  };
}
