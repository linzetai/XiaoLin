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
  const subAgentStart = useAgentStore((s) => s.subAgentStart);
  const subAgentDelta = useAgentStore((s) => s.subAgentDelta);
  const subAgentToolStart = useAgentStore((s) => s.subAgentToolStart);
  const subAgentToolDone = useAgentStore((s) => s.subAgentToolDone);
  const subAgentComplete = useAgentStore((s) => s.subAgentComplete);

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
    const currentAc = currentState.agentChats[capturedAgentId];
    const currentActiveChat = currentAc?.chatList.find((c) => c.id === currentAc.activeChatId);
    const capturedChatId = currentActiveChat?.id ?? "temp";

    addMessage(capturedAgentId, {
      role: "user",
      content: txt + mentionDesc + fileDesc,
      timestamp: new Date(),
      images: imageDataUrls.length > 0 ? imageDataUrls : undefined,
    }, capturedChatId);
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
    } else {
      messageContent = txt + mentionDesc + fileDesc;
    }

    const { promise: chatPromise, cleanup } = transport.chatStream(
      {
        messages: [{ role: "user", content: messageContent }],
        agentId: capturedAgentId,
        sessionId: capturedChatId,
        workDir: currentActiveChat?.workDir ?? undefined,
      },
      (event) => {
        switch (event.type) {
          case "chat.start": {
            streamAccRef.current = "";
            segmentsRef.current = [];
            const ds = detachedStreams.get(capturedChatId);
            if (ds) ds.acc = "";
            break;
          }
          case "chat.delta": {
            const c = event.data?.content as string | undefined;
            if (!c) return;
            if (isActive()) {
              appendText(c);
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) ds.acc += c;
            }
            break;
          }
          case "chat.complete": {
            const sid = event.data?.sessionId as string | undefined;
            const ds = detachedStreams.get(capturedChatId);
            const finalContent = isActive() ? streamAccRef.current : ds?.acc ?? streamAccRef.current;
            const savedToolCalls = (isActive() ? segmentsRef.current : [])
              .filter((s) => s.type === "tool" && s.toolCall)
              .map((s) => {
                const tc = s.toolCall!;
                return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration };
              });

            if (isActive()) {
              cancelAnimationFrame(rafIdRef.current);
              rafIdRef.current = 0;
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
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

            const usageData = event.data?.usage as { promptTokens?: number; completionTokens?: number; totalTokens?: number } | undefined;
            const elapsedMs = (event.data?.elapsedMs as number) ?? 0;
            const contextTokens = (event.data?.contextTokens as number) || undefined;
            const contextWindow = (event.data?.contextWindow as number) || undefined;
            if (usageData || elapsedMs || contextTokens) {
              const resolvedChatId = sid ?? capturedChatId;
              updateChatUsage(capturedAgentId, resolvedChatId, {
                promptTokens: usageData?.promptTokens ?? 0,
                completionTokens: usageData?.completionTokens ?? 0,
                totalTokens: usageData?.totalTokens ?? 0,
                elapsedMs,
                contextTokens,
                contextWindow,
              });
            }

            if (isActive() && atBottomRef.current) {
              requestBottomScroll("smooth");
            }
            break;
          }
          case "chat.tool.start": {
            const d = event.data;
            if (!d?.tool) return;
            const tc: ToolCall = {
              id: (d.callId ?? d.tool) as string,
              name: d.tool as string,
              status: "running",
              args: d.args as string | undefined,
              startTime: Date.now(),
            };
            if (isActive()) {
              segmentsRef.current.push({ id: `tool-${tc.id}`, type: "tool", toolCall: tc });
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) ds.toolCalls = [...ds.toolCalls.filter((t) => t.id !== tc.id), tc];
            }
            break;
          }
          case "chat.tool.done": {
            const d = event.data;
            if (!d?.tool) return;
            const callId = (d.callId ?? d.tool) as string;
            if (isActive()) {
              const seg = segmentsRef.current.find((s) => s.type === "tool" && s.toolCall?.id === callId);
              if (seg?.toolCall) {
                seg.toolCall.status = d.success ? "success" : "error";
                seg.toolCall.result = d.output as string | undefined;
                seg.toolCall.duration = seg.toolCall.startTime ? Date.now() - seg.toolCall.startTime : undefined;
              }
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) {
                ds.toolCalls = ds.toolCalls.map((t) =>
                  t.id === callId
                    ? { ...t, status: d.success ? "success" : "error", result: d.output as string | undefined, duration: t.startTime ? Date.now() - t.startTime : undefined }
                    : t,
                );
              }
            }
            break;
          }
          case "chat.ask_question": {
            const d = event.data;
            if (d?.requestId && d?.question && isActive()) {
              const timeoutSecs = (d.timeoutSecs as number) ?? 60;
              setPendingQuestion({
                requestId: d.requestId as string,
                question: d.question as string,
                options: (d.options as Array<{ id: string; label: string }>) ?? [],
                timeoutSecs,
                expiresAt: Date.now() + timeoutSecs * 1000,
                allowMultiple: d.allowMultiple as boolean | undefined,
              });
            }
            break;
          }
          case "chat.context.warning": {
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
          case "chat.subagent.start": {
            const d = event.data;
            if (!d?.runId) break;
            const run: SubAgentRunUI = {
              runId: d.runId as string,
              agentId: (d.agentId ?? "default") as string,
              subagentType: (d.subagentType ?? "general") as string,
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
          case "chat.subagent.delta": {
            const d = event.data;
            if (d?.runId && d?.content) {
              subAgentDelta(capturedAgentId, capturedChatId, d.runId as string, d.content as string);
            }
            break;
          }
          case "chat.subagent.tool.start": {
            const d = event.data;
            if (d?.runId && d?.tool) {
              subAgentToolStart(capturedAgentId, capturedChatId, d.runId as string, {
                id: (d.callId ?? d.tool) as string,
                name: d.tool as string,
                status: "running",
                args: d.args as string | undefined,
              });
            }
            break;
          }
          case "chat.subagent.tool.done": {
            const d = event.data;
            if (d?.runId && d?.callId) {
              subAgentToolDone(
                capturedAgentId, capturedChatId,
                d.runId as string, d.callId as string,
                (d.output ?? "") as string, d.success as boolean,
              );
            }
            break;
          }
          case "chat.subagent.complete": {
            const d = event.data;
            if (d?.runId) {
              subAgentComplete(
                capturedAgentId, capturedChatId,
                d.runId as string, (d.status ?? "completed") as string,
                d.result as string | undefined,
                d.toolCallsMade as number | undefined,
                d.iterations as number | undefined,
                d.elapsedMs as number | undefined,
              );
            }
            break;
          }
          case "chat.error": {
            const e = event.error?.message ?? "未知错误";
            if (isActive()) {
              cancelAnimationFrame(rafIdRef.current);
              rafIdRef.current = 0;
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
            }
            addMessage(capturedAgentId, { role: "system", content: `错误: ${e}`, timestamp: new Date() }, capturedChatId);
            const ds = detachedStreams.get(capturedChatId);
            if (ds) { ds.error = true; ds.done = true; detachedStreams.delete(capturedChatId); }
            cleanup();

            if (isActive() && atBottomRef.current) {
              requestBottomScroll("smooth");
            }
            break;
          }
        }
      },
    );

    cleanupRef.current = cleanup;

    chatPromise.catch(() => {
      if (isActive()) { setStreaming(false); }
      cleanup();
    });
  };

  const sendRef = useRef(sendWithContent);
  sendRef.current = sendWithContent;
  const streamingRef = useRef(streaming);
  streamingRef.current = streaming;

  const handleMentionSend = useCallback(
    (txt: string, _mentions: InlineMention[]) => {
      if (!txt.trim() || streamingRef.current) return;
      mentionInputRef.current?.clear();
      setAttachedFiles((prev) => {
        prev.forEach((f) => { if (f.previewUrl) URL.revokeObjectURL(f.previewUrl); });
        return [];
      });
      sendRef.current(txt.trim(), _mentions);
    },
    [],
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
    currentStreamChatRef.current = null;
    setStreaming(false);
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
