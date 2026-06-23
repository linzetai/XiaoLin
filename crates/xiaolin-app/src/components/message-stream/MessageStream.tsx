import { useState, useRef, useCallback, useEffect, useMemo, useLayoutEffect } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import { useStreamStore } from "../../lib/stores/stream-store";
import { useGoalStore } from "../../lib/stores/goal-store";
import { useSearchStore } from "../../lib/stores/search-store";
import {
  useActiveChatId,
  useActiveChatMeta,
  useActiveStream,
  useActiveSubAgentRuns,
  useChatLastSegments,
  useChatUsage,
  useActiveGoal,
} from "../../lib/stores/selectors";
import type { Chat } from "../../lib/stores/types";
import type { MentionInputHandle, MentionOption } from "./MentionInput";
import { MessageRendererRow } from "./MessageRenderer";

import { StreamFooter, type AttachedFile } from "./StreamFooter";
import { ComposerCore } from "./ComposerCore";
import { PlanApprovalCard } from "./PlanApprovalCard";
import { useStreamScroll, STREAM_PAGE_SIZE } from "./useStreamScroll";
import { useMessageStreamChat } from "./useMessageStreamChat";
import { X, CaretUp, CaretDown, UploadSimple, MagnifyingGlass, ArrowDown } from "@phosphor-icons/react";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { StreamEmptyState } from "./StreamEmptyState";
import { StickyContextBar } from "./StickyContextBar";
import { GoalStatusCard } from "../chat/GoalStatusCard";
import { parseTodoResult, type TodoSummary } from "./TodoCard";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { useWorkspaceTabs } from "../shell/workspace-tabs";

interface MessageStreamProps {
  onToggleDetail?: () => void;
  detailOpen?: boolean;
}

export function MessageStream(_props: MessageStreamProps) {
  const { t } = useTranslation("chat");
  const { t: tCommon } = useTranslation("common");
  const activeAgentId = useChatMetaStore((s) => s.activeAgentId);
  const activeChatId = useActiveChatId();
  const activeChatMeta = useActiveChatMeta();
  const stream = useActiveStream();
  const subAgentRuns = useActiveSubAgentRuns();
  const lastSegments = useChatLastSegments(activeChatId);
  const usage = useChatUsage(activeChatId);
  const activeGoal = useActiveGoal();
  const setWorkDirRaw = useChatMetaStore((s) => s.setWorkDir);
  const loadChatStream = useStreamStore((s) => s.loadChatStream);
  const pendingScrollTurnId = useSearchStore((s) => s.pendingScrollTurnId);
  const pendingScrollSessionId = useSearchStore((s) => s.pendingScrollSessionId);
  const highlightTurnId = useSearchStore((s) => s.highlightTurnId);
  const navError = useSearchStore((s) => s.navError);
  const clearPendingScroll = useSearchStore((s) => s.clearPendingScroll);
  const clearHighlight = useSearchStore((s) => s.clearHighlight);

  const activeChat = useMemo((): Chat | undefined => {
    if (!activeChatMeta) return undefined;
    return {
      ...activeChatMeta,
      stream,
      usage,
      subAgentRuns,
      lastSegments,
    };
  }, [activeChatMeta, stream, usage, subAgentRuns, lastSegments]);

  const workDir = activeChatMeta?.workDir ?? null;

  const setWorkDir = useCallback(
    (_agentId: string, chatId: string, path: string | null) => {
      setWorkDirRaw(chatId, path);
    },
    [setWorkDirRaw],
  );

  useEffect(() => {
    if (!import.meta.env.DEV) return;

    (window as any).__xiaolin_setWorkDir = (path: string | null) => {
      const state = useChatMetaStore.getState();
      const chatId = state.activeChatId;
      if (chatId) state.setWorkDir(chatId, path);
      return { chatId, messageCount: state.chats[chatId ?? ""]?.messageCount };
    };

    (window as any).__xiaolin_setMode = async (mode: "agent" | "plan" | "goal") => {
      const chatMetaState = useChatMetaStore.getState();
      const chatId = chatMetaState.activeChatId;
      if (!chatId) return { error: "no active chat" };

      const chat = chatMetaState.chats[chatId];
      const currentExecMode = chat?.executionMode ?? "agent";

      if (mode === "goal") {
        useGoalStore.getState().setGoalMode(chatId, true);
        if (currentExecMode === "plan") {
          const resp = await transport.setExecutionModeIpc("agent", chatId);
          if (resp.ok) chatMetaState.setChatExecutionMode(chatId, "agent");
        }
      } else {
        useGoalStore.getState().setGoalMode(chatId, false);
        const backendMode = mode === "plan" ? "plan" : "agent";
        if (backendMode !== currentExecMode) {
          const resp = await transport.setExecutionModeIpc(backendMode, chatId);
          if (resp.ok) chatMetaState.setChatExecutionMode(chatId, backendMode);
        }
      }

      const updated = useChatMetaStore.getState().chats[chatId];
      return {
        chatId,
        goalMode: useGoalStore.getState().goalMode[chatId] ?? false,
        executionMode: updated?.executionMode,
      };
    };

    return () => {
      delete (window as any).__xiaolin_setWorkDir;
      delete (window as any).__xiaolin_setMode;
    };
  }, []);

  const loadingChats = useRef(new Set<string>());
  const loadedChats = useRef(new Set<string>());
  useEffect(() => {
    if (!activeChatMeta) return;
    const chatId = activeChatMeta.id;
    if (activeChatMeta.messageCount === 0 && stream.length === 0) return;
    if (stream.length > 0) return;
    if (loadingChats.current.has(chatId)) return;
    if (loadedChats.current.has(chatId)) return;

    const ac = new AbortController();
    loadingChats.current.add(chatId);
    transport.getSessionMessages(chatId).then((messages) => {
      if (ac.signal.aborted) return;
      const currentActiveId = useChatMetaStore.getState().activeChatId;
      if (currentActiveId !== chatId) return;
      const currentStream = useStreamStore.getState().streams[chatId] ?? [];
      if (currentStream.length > 0) return;
      if (messages && messages.length > 0) {
        loadChatStream(chatId, messages);
      }
      loadedChats.current.add(chatId);
    }).catch(() => {
      if (ac.signal.aborted) return;
    }).finally(() => {
      loadingChats.current.delete(chatId);
    });

    return () => {
      ac.abort();
    };
  }, [activeChatMeta?.id, activeChatMeta?.messageCount, stream.length, loadChatStream]);

  const hydratedSessions = useRef(new Set<string>());
  useEffect(() => {
    if (!activeChatMeta) return;
    const sid = activeChatMeta.id;
    if (sid.startsWith("new-") || hydratedSessions.current.has(sid)) return;
    const chat = useChatMetaStore.getState().chats[sid];
    if (chat?.planFilePath) return;
    hydratedSessions.current.add(sid);
    useChatMetaStore.getState().hydratePlanMeta(sid);
  }, [activeChatMeta?.id]);

  const bottomRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const scrollPositions = useRef<Record<string, number>>({});
  const mentionInputRef = useRef<MentionInputHandle>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [attachedFiles, setAttachedFiles] = useState<AttachedFile[]>([]);
  const attachedFilesRef = useRef<AttachedFile[]>([]);
  attachedFilesRef.current = attachedFiles;

  const draftsRef = useRef<Record<string, { text: string; files: AttachedFile[] }>>({});
  const prevAgentChatKey = useRef<string>("");

  const [isDragging, setIsDragging] = useState(false);
  const dragCounter = useRef(0);

  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchIdx, setSearchIdx] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [showScrollFab, setShowScrollFab] = useState(false);
  const [unreadCount, setUnreadCount] = useState(0);
  const [streamDoneLabel, setStreamDoneLabel] = useState(false);
  const prevStreamLenRef = useRef(stream.length);
  const prevStreamingRef = useRef(false);
  const chatScrollKey = useCallback((chatId: string | undefined) => {
    if (!chatId) return undefined;
    const chat = useChatMetaStore.getState().chats[chatId];
    return chat?.localKey ?? chatId;
  }, []);

  const {
    streaming,
    streamSegments,
    pendingQuestion,
    setPendingQuestion,
    stopStream,
    handleMentionSend,
    handleNewTopic,
    atBottomRef,
    suppressScrollTrackingUntilRef,
    pendingBottomScrollBehaviorRef,
    pendingRestoreScrollTopRef,
    runProgrammaticScroll,
  } = useMessageStreamChat({
    activeAgentId,
    activeChat,
    workDir,
    chatScrollKey,
    scrollPositions,
    mentionInputRef,
    attachedFilesRef,
    setAttachedFiles,
  });

  const searchResults = useMemo(() => {
    if (!searchQuery.trim()) return [];
    const q = searchQuery.toLowerCase();
    const results = stream
      .map((item, idx) => ({ item, idx }))
      .filter(({ item }) => item.type === "message" && item.data.content.toLowerCase().includes(q));
    return results;
  }, [stream, searchQuery]);

  const openSearch = useCallback(() => {
    setSearchOpen(true);
    setSearchQuery("");
    setSearchIdx(0);
    setTimeout(() => searchInputRef.current?.focus(), 0);
  }, []);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setSearchQuery("");
  }, []);

  const paginationOffsetRef = useRef(0);

  useEffect(() => {
    const handleGlobalKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        if (searchOpen) closeSearch();
        else openSearch();
      }
    };
    const handleToggleSearch = () => {
      if (searchOpen) closeSearch();
      else openSearch();
    };
    window.addEventListener("keydown", handleGlobalKey);
    window.addEventListener("xiaolin:toggle-search", handleToggleSearch);
    return () => {
      window.removeEventListener("keydown", handleGlobalKey);
      window.removeEventListener("xiaolin:toggle-search", handleToggleSearch);
    };
  }, [searchOpen, openSearch, closeSearch]);

  const [fsEntries, setFsEntries] = useState<{ files: string[]; dirs: string[] }>({ files: [], dirs: [] });
  useEffect(() => {
    if (!workDir) { setFsEntries({ files: [], dirs: [] }); return; }
    api.listFiles(workDir).then(setFsEntries).catch(() => setFsEntries({ files: [], dirs: [] }));
  }, [workDir]);

  const [backendSkills, setBackendSkills] = useState<api.SkillInfo[]>([]);
  useEffect(() => {
    api.listSkills().then(setBackendSkills).catch(() => {});
  }, []);

  const mentionOptions: MentionOption[] = useMemo(() => {
    const opts: MentionOption[] = [];
    if (workDir) {
      for (const f of fsEntries.files) {
        opts.push({ id: `f-${f}`, label: f, type: "file", desc: f });
      }
      for (const d of fsEntries.dirs) {
        opts.push({ id: `d-${d}`, label: `${d}/`, type: "dir", desc: `${d}/` });
      }
    }
    if (backendSkills.length > 0) {
      for (const s of backendSkills) {
        opts.push({ id: `s-${s.id}`, label: s.name, type: "skill", desc: s.description });
      }
    } else {
      opts.push(
        { id: "s-web-search", label: t("skill_webSearch"), type: "skill", desc: t("webSearch") },
        { id: "s-code-exec", label: t("skill_codeExec"), type: "skill", desc: t("codeExec") },
      );
    }
    return opts;
  }, [workDir, fsEntries, backendSkills, t]);

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current++;
    if (e.dataTransfer.types.includes("Files")) {
      setIsDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current--;
    if (dragCounter.current === 0) {
      setIsDragging(false);
    }
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const processFiles = useCallback((files: FileList | File[]) => {
    const newFiles: AttachedFile[] = Array.from(files).map((f) => ({
      name: f.name,
      size: f.size,
      type: f.type,
      file: f,
      previewUrl: f.type.startsWith("image/") ? URL.createObjectURL(f) : undefined,
    }));
    setAttachedFiles((prev) => [...prev, ...newFiles]);
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current = 0;
    setIsDragging(false);
    if (e.dataTransfer.files.length > 0) {
      processFiles(e.dataTransfer.files);
    }
  }, [processFiles]);

  useEffect(() => {
    const handleEdit = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      const text = detail?.text;
      if (text && mentionInputRef.current) {
        mentionInputRef.current.setText(text);
        mentionInputRef.current.focus();
      }
      const images = detail?.images as Array<{ url: string; alt?: string }> | undefined;
      if (images?.length) {
        const restored: AttachedFile[] = [];
        for (let i = 0; i < images.length; i++) {
          const img = images[i];
          const match = img.url.match(/^data:([^;]+);base64,(.+)$/);
          if (!match) continue;
          const mime = match[1];
          const b64 = match[2];
          const bin = atob(b64);
          const arr = new Uint8Array(bin.length);
          for (let j = 0; j < bin.length; j++) arr[j] = bin.charCodeAt(j);
          const blob = new Blob([arr], { type: mime });
          const file = new File([blob], img.alt || `image-${i}.png`, { type: mime });
          restored.push({ name: file.name, size: file.size, type: file.type, file, previewUrl: URL.createObjectURL(blob) });
        }
        if (restored.length) setAttachedFiles((prev) => [...prev, ...restored]);
      }
    };
    const handlePaste = (e: Event) => {
      const files = (e as CustomEvent).detail?.files as File[] | undefined;
      if (files?.length) processFiles(files);
    };
    window.addEventListener("xiaolin:edit-message", handleEdit);
    window.addEventListener("xiaolin:paste-files", handlePaste);
    return () => {
      window.removeEventListener("xiaolin:edit-message", handleEdit);
      window.removeEventListener("xiaolin:paste-files", handlePaste);
    };
  }, [processFiles]);

  const removeFile = useCallback((index: number) => {
    setAttachedFiles((prev) => {
      const removed = prev[index];
      if (removed?.previewUrl) URL.revokeObjectURL(removed.previewUrl);
      return prev.filter((_, i) => i !== index);
    });
  }, []);

  useEffect(() => {
    return () => {
      for (const f of attachedFilesRef.current) {
        if (f.previewUrl) URL.revokeObjectURL(f.previewUrl);
      }
      for (const draft of Object.values(draftsRef.current)) {
        for (const f of draft.files) {
          if (f.previewUrl) URL.revokeObjectURL(f.previewUrl);
        }
      }
    };
  }, []);

  const chatKey = activeChatMeta?.localKey ?? activeChatId ?? "";
  const firstVisibleIndexRef = useRef(0);
  const containerRef = useRef<HTMLDivElement>(null);

  const [visibleCount, setVisibleCount] = useState(STREAM_PAGE_SIZE);
  useEffect(() => {
    setVisibleCount(STREAM_PAGE_SIZE);
  }, [chatKey]);

  useEffect(() => {
    const key = chatKey;
    if (prevAgentChatKey.current && prevAgentChatKey.current !== key) {
      const text = mentionInputRef.current?.getText() ?? "";
      draftsRef.current[prevAgentChatKey.current] = { text, files: [...attachedFilesRef.current] };
      mentionInputRef.current?.clear();
      setAttachedFiles([]);
    }
    const draft = draftsRef.current[key];
    if (draft) {
      mentionInputRef.current?.setText(draft.text);
      setAttachedFiles(draft.files);
      delete draftsRef.current[key];
    }
    prevAgentChatKey.current = key;
  }, [chatKey]);

  const hasMore = stream.length > visibleCount;
  const paginationOffset = hasMore ? stream.length - visibleCount : 0;
  paginationOffsetRef.current = paginationOffset;
  const visibleStream = hasMore ? stream.slice(paginationOffset) : stream;

  const displayData = useMemo(() => {
    if (streaming) {
      return [
        ...visibleStream,
        { key: "_streaming_", data: { role: "streaming" as const, content: "", timestamp: new Date() } },
      ];
    }
    return visibleStream;
  }, [visibleStream, streaming]);

  const getItemKey = useCallback((index: number) => {
    const item = displayData[index];
    if (!item) return index;
    if ("key" in item && (item as { key?: string }).key === "_streaming_") return "_streaming_";
    if ("type" in item) {
      const typed = item as { type: string; data: { id: string | number } };
      if (typed.type === "message") return `msg-${typed.data.id}`;
      if (typed.type === "brief") return `brief-${typed.data.id}`;
    }
    return index;
  }, [displayData]);

  const virtualizer = useVirtualizer({
    count: displayData.length,
    getScrollElement: () => scrollContainerRef.current,
    estimateSize: () => 80,
    getItemKey,
    overscan: 6,
    anchorTo: "end",
    followOnAppend: "smooth",
    scrollEndThreshold: 120,
    useFlushSync: false,
  });

  const virtualizerRef = useRef(virtualizer);
  virtualizerRef.current = virtualizer;

  const measureElementRef = useCallback((node: Element | null) => {
    virtualizerRef.current.measureElement(node);
  }, []);

  useLayoutEffect(() => {
    virtualizer.scrollToEnd();
  }, [chatKey]);

  useEffect(() => {
    if (!pendingScrollTurnId || !pendingScrollSessionId) return;
    if (activeChatId !== pendingScrollSessionId) return;

    const fullIdx = stream.findIndex(
      (item) => item.type === "message" && String(item.data.id) === pendingScrollTurnId,
    );
    if (fullIdx < 0) return;

    const neededVisible = stream.length - fullIdx;
    if (neededVisible > visibleCount) {
      setVisibleCount(neededVisible);
      return;
    }

    const visibleIdx = fullIdx - paginationOffsetRef.current;
    if (visibleIdx < 0 || visibleIdx >= displayData.length) return;

    runProgrammaticScroll(() => {
      virtualizer.scrollToIndex(visibleIdx, { align: "center", behavior: "smooth" });
    });

    clearPendingScroll();
    setTimeout(() => clearHighlight(), 2800);
  }, [
    pendingScrollTurnId,
    pendingScrollSessionId,
    activeChatId,
    stream,
    visibleCount,
    displayData.length,
    virtualizer,
    runProgrammaticScroll,
    clearPendingScroll,
    clearHighlight,
  ]);

  const { t: tSidebar } = useTranslation("sidebar");

  const { handleScroll, handleStartReached: _handleStartReached } = useStreamScroll({
    virtualizer,
    scrollContainerRef,
    scrollPositions,
    chatKey,
    displayDataLength: displayData.length,
    streamLength: stream.length,
    hasMore,
    setVisibleCount,
    paginationOffsetRef,
    searchIdx,
    searchResults,
    atBottomRef,
    pendingBottomScrollBehaviorRef,
    pendingRestoreScrollTopRef,
    suppressScrollTrackingUntilRef,
    runProgrammaticScroll,
  });

  const handleScrollWithAtBottom = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    handleScroll(e);
    const isAtEnd = virtualizer.isAtEnd();
    const wasAtBottom = atBottomRef.current;
    atBottomRef.current = isAtEnd;
    if (isAtEnd !== wasAtBottom) {
      setShowScrollFab(!isAtEnd);
      if (isAtEnd) {
        setUnreadCount(0);
        setStreamDoneLabel(false);
      }
    }
  }, [handleScroll, virtualizer]);

  useEffect(() => {
    if (!atBottomRef.current && stream.length > prevStreamLenRef.current) {
      setUnreadCount((c) => c + (stream.length - prevStreamLenRef.current));
    }
    prevStreamLenRef.current = stream.length;
  }, [stream.length]);

  useEffect(() => {
    if (prevStreamingRef.current && !streaming) {
      if (atBottomRef.current) {
        virtualizer.scrollToEnd({ behavior: "smooth" });
      } else {
        setShowScrollFab(true);
        setStreamDoneLabel(true);
      }
    }
    prevStreamingRef.current = streaming;
  }, [streaming, displayData.length]);

  const scrollToBottom = useCallback(() => {
    virtualizer.scrollToEnd({ behavior: "smooth" });
    setTimeout(() => {
      setShowScrollFab(false);
      setUnreadCount(0);
      setStreamDoneLabel(false);
    }, 100);
  }, [virtualizer]);

  const prevWidthRef = useRef(0);
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width ?? 0;
      if (prevWidthRef.current > 0 && Math.abs(w - prevWidthRef.current) > 2) {
        const idx = firstVisibleIndexRef.current;
        requestAnimationFrame(() => {
          virtualizer.scrollToIndex(idx, { align: "start", behavior: "auto" });
        });
      }
      prevWidthRef.current = w;
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [chatKey, virtualizer]);

  const visibleRange = virtualizer.range ?? { startIndex: 0, endIndex: 0 };
  useEffect(() => {
    if (visibleRange) {
      firstVisibleIndexRef.current = visibleRange.startIndex;
    }
  }, [visibleRange?.startIndex]);

  const lastUserMessage = useMemo(() => {
    for (let i = stream.length - 1; i >= 0; i--) {
      const item = stream[i];
      if (item.type === "message" && item.data.role === "user") return { content: item.data.content, index: i };
    }
    return null;
  }, [stream]);

  const lastUserDisplayIndex = useMemo(() => {
    if (!lastUserMessage) return -1;
    const globalIdx = lastUserMessage.index;
    return globalIdx - paginationOffset;
  }, [lastUserMessage, paginationOffset]);

  const lastAssistantDisplayIdx = useMemo(() => {
    for (let i = displayData.length - 1; i >= 0; i--) {
      const it = displayData[i];
      if ("type" in it && it.type === "message" && (it.data as { role: string }).role === "assistant") return i;
    }
    return -1;
  }, [displayData]);

  const todoProgress = useMemo<TodoSummary | null>(() => {
    // Check current streaming segments first (most recent data)
    if (streamSegments && streamSegments.length > 0) {
      for (let i = streamSegments.length - 1; i >= 0; i--) {
        const seg = streamSegments[i];
        if (seg.type === "tool" && seg.toolCall?.name === "todo_write" && seg.toolCall.result) {
          const parsed = parseTodoResult(seg.toolCall.result);
          if (parsed) return parsed.summary;
        }
      }
    }
    // Always check full message history as fallback (including during streaming/Goal mode)
    for (let i = stream.length - 1; i >= 0; i--) {
      const item = stream[i];
      if (item.type !== "message") continue;
      const msg = item.data;
      if (msg.role === "assistant" && msg.toolCalls) {
        for (let j = msg.toolCalls.length - 1; j >= 0; j--) {
          const tc = msg.toolCalls[j];
          if (tc.name === "todo_write" && tc.result) {
            const parsed = parseTodoResult(tc.result);
            if (parsed) return parsed.summary;
          }
        }
      }
    }
    return null;
  }, [streamSegments, stream]);

  const showContextBar = useMemo(() => {
    if (!lastUserMessage) return false;
    if (streaming) return true;
    if (lastUserDisplayIndex >= 0) {
      return lastUserDisplayIndex < visibleRange.startIndex || lastUserDisplayIndex > visibleRange.endIndex;
    }
    return lastUserDisplayIndex < 0;
  }, [lastUserMessage, streaming, lastUserDisplayIndex, visibleRange]);

  const handleEditFromBar = useCallback(() => {
    if (!lastUserMessage) return;
    if (streaming) stopStream();
    window.dispatchEvent(new CustomEvent("xiaolin:edit-message", { detail: { text: lastUserMessage.content } }));
  }, [lastUserMessage, streaming, stopStream]);

  const handleResendFromBar = useCallback(() => {
    if (!lastUserMessage) return;
    handleMentionSend(lastUserMessage.content, []);
  }, [lastUserMessage, handleMentionSend]);

  const isEmpty = stream.length === 0 && !streaming;
  const chatSessionId = activeChatMeta?.id ?? "";
  const planFilePath = activeChatMeta?.planFilePath;
  const planFileExists = activeChatMeta?.planFileExists ?? false;
  const executionMode = activeChatMeta?.executionMode ?? "agent";
  const planApprovalPending = activeChatMeta?.planApprovalPending ?? false;

  const togglePlanPanel = useCallback(() => {
    const { tabs, panelOpen, activeTabId: curTab } = useWorkspaceTabs.getState();
    const hasPlanTab = tabs.some((t) => t.id === "plan");
    if (!hasPlanTab) return;
    if (panelOpen && curTab === "plan") {
      useWorkspaceTabs.getState().togglePanel();
      if (chatSessionId) {
        sessionStorage.setItem(`xiaolin:plan-panel-dismissed:${chatSessionId}`, "1");
      }
    } else {
      useWorkspaceTabs.getState().setActiveTab("plan");
      if (chatSessionId) {
        sessionStorage.removeItem(`xiaolin:plan-panel-dismissed:${chatSessionId}`);
      }
    }
  }, [chatSessionId]);

  const showFallbackPlanApproval = planApprovalPending && planFileExists && executionMode === "plan" && !streaming;

  const handleFallbackPlanApprove = useCallback(async (mode: "agent" | "plan") => {
    const chatId = activeChatMeta?.id;
    if (!chatId) return;
    const { setChatExecutionMode, setChatPlanApprovalPending } = useChatMetaStore.getState();
    await transport.approvePlan(chatId, mode);
    setChatExecutionMode(chatId, mode);
    setChatPlanApprovalPending(chatId, false);
    if (mode === "agent") {
      const planPath = activeChatMeta?.planFilePath ?? "";
      handleMentionSend(
        `Plan approved. Execute the plan now. The plan file is at: ${planPath}\n\nStrategy:\n1. Read the plan file and identify ALL tasks.\n2. Use \`todo_write\` to track progress.\n3. PARALLELIZATION: If there are 3+ independent tasks (no data dependency), you MUST use \`spawn_subagent\` to run them concurrently. Give each subagent a clear, self-contained prompt.\n4. For sequential/dependent tasks, execute them yourself in order.\n5. Self-verify: before marking complete, test the result.\n6. Do NOT ask for clarification — start immediately.`,
        [],
      );
    }
  }, [activeChatMeta, handleMentionSend]);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail as { planPath?: string } | undefined;
      const planPath = detail?.planPath ?? activeChatMeta?.planFilePath ?? "";
      handleMentionSend(
        `Plan approved. Execute the plan now. The plan file is at: ${planPath}\n\nStrategy:\n1. Read the plan file and identify ALL tasks.\n2. Use \`todo_write\` to track progress.\n3. PARALLELIZATION: If there are 3+ independent tasks (no data dependency), you MUST use \`spawn_subagent\` to run them concurrently. Give each subagent a clear, self-contained prompt.\n4. For sequential/dependent tasks, execute them yourself in order.\n5. Self-verify: before marking complete, test the result.\n6. Do NOT ask for clarification — start immediately.`,
        [],
      );
    };
    window.addEventListener("xiaolin:plan-approved", handler);
    return () => window.removeEventListener("xiaolin:plan-approved", handler);
  }, [handleMentionSend, activeChatMeta?.planFilePath]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
    {navError && (
      <div
        style={{
          position: "absolute",
          top: 12,
          left: "50%",
          transform: "translateX(-50%)",
          zIndex: 40,
          padding: "8px 14px",
          borderRadius: 8,
          fontSize: 12,
          background: "var(--bg-elevated)",
          border: "0.5px solid var(--separator)",
          boxShadow: "var(--shadow-lg)",
          color: "var(--fill-secondary)",
          animation: "fade-slide-up var(--duration-fast) var(--ease-out)",
        }}
      >
        {tSidebar(navError)}
      </div>
    )}
    <div className="flex min-h-0 min-w-0 flex-1">
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
      style={{ background: "var(--bg-primary)" }}
      data-streaming={streaming ? "true" : undefined}
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
    >
      {isDragging && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center"
          style={{ background: "rgba(0, 122, 255, 0.06)" }}
        >
          <div
            className="flex flex-col items-center gap-3 rounded-[var(--radius-xl)] border-2 border-dashed px-12 py-10"
            style={{
              borderColor: "var(--tint)",
              background: "var(--bg-elevated)",
              boxShadow: "var(--shadow-lg)",
              animation: "drop-zone-pulse 1.5s ease-in-out infinite",
            }}
          >
            <UploadSimple size={ICON_SIZE["2xl"]} style={{ color: "var(--fill-secondary)" }} />
            <span className="text-[15px] font-medium" style={{ color: "var(--fill-primary)" }}>
              {t("dropFilesHere")}
            </span>
            <span className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              {t("dropFilesSupport")}
            </span>
          </div>
        </div>
      )}

      {searchOpen && (
        <div
          className="flex shrink-0 items-center gap-2 py-2"
          style={{ background: "var(--bg-secondary)", borderBottom: `0.5px solid var(--separator)`, animation: "slide-down var(--duration-fast) var(--ease-out)", padding: "8px clamp(24px, 5%, 80px)" }}
        >
          <MagnifyingGlass size={ICON_SIZE.md} style={{ color: "var(--fill-tertiary)" }} />
          <input
            ref={searchInputRef}
            value={searchQuery}
            onChange={(e) => { setSearchQuery(e.target.value); setSearchIdx(0); }}
            onKeyDown={(e) => {
              if (e.key === "Escape") closeSearch();
              if (e.key === "Enter" && !e.shiftKey) setSearchIdx((i) => (i + 1) % Math.max(searchResults.length, 1));
              if (e.key === "Enter" && e.shiftKey) setSearchIdx((i) => (i - 1 + Math.max(searchResults.length, 1)) % Math.max(searchResults.length, 1));
            }}
            placeholder={t("searchMessages")}
            className="min-w-0 flex-1 bg-transparent text-[13px] outline-none"
            style={{ color: "var(--fill-primary)" }}
          />
          {searchQuery && (
            <span className="shrink-0 text-[11px] tabular-nums" style={{ color: "var(--fill-tertiary)" }}>
              {searchResults.length > 0
                ? t("searchResult", { current: searchIdx + 1, total: searchResults.length })
                : tCommon("noResults")}
            </span>
          )}
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => setSearchIdx((i) => (i - 1 + Math.max(searchResults.length, 1)) % Math.max(searchResults.length, 1))}
              disabled={searchResults.length === 0}
              className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <CaretUp />
            </button>
            <button
              onClick={() => setSearchIdx((i) => (i + 1) % Math.max(searchResults.length, 1))}
              disabled={searchResults.length === 0}
              className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <CaretDown />
            </button>
          </div>
          <button
            onClick={closeSearch}
            className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <X />
          </button>
        </div>
      )}

      {isEmpty ? (
        <div className="flex min-h-0 flex-1 flex-col">
          {activeGoal && chatSessionId && (
            <GoalStatusCard sessionId={chatSessionId} goal={activeGoal} />
          )}
          <div className="flex-1 overflow-y-auto" style={{ padding: "24px clamp(24px, 5%, 80px)" }}>
          <StreamEmptyState
            workDir={workDir}
            composerSlot={
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
                onTogglePlanPanel={togglePlanPanel}
              />
            }
            onPick={(t) => {
              if (mentionInputRef.current) {
                mentionInputRef.current.setText(t);
                mentionInputRef.current.focus();
              }
            }}
          />
          </div>
        </div>
      ) : (
        <div ref={containerRef} className="flex min-h-0 min-w-0 flex-1 flex-col">
          {activeGoal && chatSessionId && (
            <GoalStatusCard sessionId={chatSessionId} goal={activeGoal} />
          )}
          {showContextBar && lastUserMessage && (
            <StickyContextBar
              userMessage={lastUserMessage.content}
              streaming={streaming}
              todoProgress={todoProgress}
              onStop={stopStream}
              onEdit={handleEditFromBar}
              onResend={handleResendFromBar}
            />
          )}
          <div
            ref={scrollContainerRef}
            key={chatKey}
            className="min-w-0 flex-1"
            style={{ overflowX: "hidden", overflowY: "auto" }}
            onScroll={handleScrollWithAtBottom}
          >
            {hasMore && (
              <div className="m-prev flex h-8 cursor-pointer items-center justify-center">
                <span className="text-[13px] transition-colors" style={{ color: "var(--fill-tertiary)" }}>
                  {paginationOffset} previous messages ›
                </span>
              </div>
            )}
            {!hasMore && <div className="h-8" />}
            <div style={{ height: virtualizer.getTotalSize(), width: "100%", position: "relative" }}>
              {virtualizer.getVirtualItems().map((virtualItem) => (
                <div
                  key={virtualItem.key}
                  ref={measureElementRef}
                  data-index={virtualItem.index}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translate3d(0, ${Math.round(virtualItem.start)}px, 0)`,
                    willChange: "transform",
                  }}
                >
                  <MessageRendererRow
                    item={displayData[virtualItem.index]}
                    idx={virtualItem.index}
                    paginationOffset={paginationOffset}
                    searchQuery={searchQuery}
                    searchIdx={searchIdx}
                    searchResults={searchResults}
                    streamSegments={streamSegments}
                    subAgentRuns={subAgentRuns}
                    bottomRef={bottomRef}
                    lastSegments={virtualItem.index === lastAssistantDisplayIdx ? lastSegments as import("./types").StreamSegment[] | undefined : undefined}
                    highlightTurnId={highlightTurnId}
                    executionMode={executionMode}
                  />
                </div>
              ))}
            </div>
            {showFallbackPlanApproval && (
              <div style={{ padding: "8px clamp(24px, 5%, 80px)" }}>
                <PlanApprovalCard
                  result={`Plan complete — waiting for user approval.\n\nPlan file: ${planFilePath}`}
                  metadata={{ approval_pending: true, plan_path: planFilePath, plan_exists: true }}
                  onApprove={handleFallbackPlanApprove}
                />
              </div>
            )}
            <div className="h-8" />
          </div>
        </div>
      )}

      {showScrollFab && !isEmpty && (
        <div className="absolute right-6 bottom-[140px] z-20">
          <button
            onClick={scrollToBottom}
            className="flex h-9 items-center gap-1.5 rounded-full px-3 shadow-lg transition-all duration-150 hover:scale-105 active:scale-95"
            style={{
              background: "var(--bg-elevated)",
              border: "1px solid var(--separator)",
              color: "var(--fill-secondary)",
              boxShadow: "var(--shadow-lg)",
            }}
          >
            <ArrowDown />
            {streamDoneLabel ? (
              <span className="text-[11px] font-medium" style={{ color: "var(--tint)" }}>
                {tCommon("outputComplete")}
              </span>
            ) : unreadCount > 0 ? (
              <span
                className="flex h-[18px] min-w-[18px] items-center justify-center rounded-full px-1 text-[10px] font-semibold tabular-nums"
                style={{ background: "var(--tint)", color: "#fff" }}
              >
                {unreadCount > 99 ? "99+" : unreadCount}
              </span>
            ) : null}
          </button>
        </div>
      )}

      {!isEmpty && (
        <StreamFooter
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
          pendingQuestion={pendingQuestion}
          setPendingQuestion={setPendingQuestion}
          stopStream={stopStream}
          onTogglePlanPanel={togglePlanPanel}
        />
      )}
    </div>

    </div>
    </div>
  );
}
