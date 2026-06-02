import { useState, useRef, useCallback, useEffect, useMemo, memo } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { useAgentStore } from "../../lib/agent-store";
import { useActiveAgentChats } from "../../lib/stores/selectors";
import type { MentionInputHandle, MentionOption } from "./MentionInput";
import { MessageRendererRow } from "./MessageRenderer";

import { StreamFooter, type AttachedFile } from "./StreamFooter";
import { PlanPanel } from "./PlanPanel";
import { useStreamScroll, STREAM_PAGE_SIZE } from "./useStreamScroll";
import { useMessageStreamChat } from "./useMessageStreamChat";
import { X, ChevronUp, ChevronDown, Upload, Search, ArrowDown } from "lucide-react";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { StreamEmptyState } from "./StreamEmptyState";
import { StickyContextBar } from "./StickyContextBar";
import { parseTodoResult, type TodoSummary } from "./TodoCard";
import { ICON } from "../../lib/ui-tokens";

const VIEWPORT_INCREASE = { top: 200, bottom: 200 };

const VirtuosoFooter = () => <div className="h-8" />;

const VirtuosoHeaderWithMore = memo(function VirtuosoHeaderWithMore() {
  return (
    <div className="flex h-8 items-center justify-center">
      <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        ↑ 滚动加载更多
      </span>
    </div>
  );
});

const VirtuosoHeaderEmpty = () => <div className="h-8" />;

interface MessageStreamProps {
  onToggleDetail?: () => void;
  detailOpen?: boolean;
}

export function MessageStream(_props: MessageStreamProps) {
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const ac = useActiveAgentChats();
  const setWorkDir = useAgentStore((s) => s.setWorkDir);
  const loadChatStream = useAgentStore((s) => s.loadChatStream);

  const activeChat = ac?.chatList.find((c) => c.id === ac.activeChatId);
  const stream = activeChat?.stream ?? [];
  const workDir = activeChat?.workDir ?? null;

  const loadingChats = useRef(new Set<string>());
  const loadedChats = useRef(new Set<string>());
  const [animateMessages, setAnimateMessages] = useState(false);
  useEffect(() => {
    setAnimateMessages(false);
  }, [activeChat?.id]);
  useEffect(() => {
    if (!activeChat) return;
    if (activeChat.messageCount === 0 && activeChat.stream.length === 0) {
      setAnimateMessages(true);
      return;
    }
    if (loadingChats.current.has(activeChat.id)) return;
    if (loadedChats.current.has(activeChat.id)) {
      setAnimateMessages(true);
      return;
    }

    loadingChats.current.add(activeChat.id);
    transport.getSessionMessages(activeChat.id).then((messages) => {
      if (messages && messages.length > 0 && messages.length > activeChat.stream.length) {
        loadChatStream(activeAgentId, activeChat.id, messages);
      }
      loadedChats.current.add(activeChat.id);
    }).catch(() => {}).finally(() => {
      loadingChats.current.delete(activeChat.id);
      setAnimateMessages(true);
    });
  }, [activeChat?.id, activeChat?.messageCount, activeChat?.stream.length, activeAgentId, loadChatStream]);

  const bottomRef = useRef<HTMLDivElement>(null);
  const virtuosoRef = useRef<VirtuosoHandle>(null);
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
    const chat = ac?.chatList.find((c) => c.id === chatId);
    const stableKey = chat?.localKey ?? chatId;
    return `${activeAgentId}:${stableKey}`;
  }, [ac?.chatList, activeAgentId]);

  const {
    streaming,
    streamSegments,
    pendingQuestion,
    setPendingQuestion,
    stopStream,
    handleMentionSend,
    handleNewTopic,
    streamingChatIds: _streamingChatIds,
    attentionChatIds: _attentionChatIds,
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
      .filter(({ item }) => item.data.content.toLowerCase().includes(q));
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
    window.addEventListener("fastclaw:toggle-search", handleToggleSearch);
    return () => {
      window.removeEventListener("keydown", handleGlobalKey);
      window.removeEventListener("fastclaw:toggle-search", handleToggleSearch);
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
        { id: "s-web-search", label: "Web Search", type: "skill", desc: "搜索互联网获取实时信息" },
        { id: "s-code-exec", label: "Code Execution", type: "skill", desc: "在沙箱中执行代码片段" },
      );
    }
    return opts;
  }, [workDir, fsEntries, backendSkills]);

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
    window.addEventListener("fastclaw:edit-message", handleEdit);
    window.addEventListener("fastclaw:paste-files", handlePaste);
    return () => {
      window.removeEventListener("fastclaw:edit-message", handleEdit);
      window.removeEventListener("fastclaw:paste-files", handlePaste);
    };
  }, [processFiles]);

  const removeFile = useCallback((index: number) => {
    setAttachedFiles((prev) => {
      const removed = prev[index];
      if (removed?.previewUrl) URL.revokeObjectURL(removed.previewUrl);
      return prev.filter((_, i) => i !== index);
    });
  }, []);

  const chatKey = `${activeAgentId}:${activeChat?.localKey ?? ac?.activeChatId ?? ""}`;
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

  const { handleAtBottomChange: rawAtBottomChange, handleScroll, handleStartReached } = useStreamScroll({
    virtuosoRef,
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

  const handleAtBottomChange = useCallback((atBottom: boolean) => {
    rawAtBottomChange(atBottom);
    setShowScrollFab(!atBottom);
    if (atBottom) {
      setUnreadCount(0);
      setStreamDoneLabel(false);
    }
  }, [rawAtBottomChange]);

  useEffect(() => {
    if (!atBottomRef.current && stream.length > prevStreamLenRef.current) {
      setUnreadCount((c) => c + (stream.length - prevStreamLenRef.current));
    }
    prevStreamLenRef.current = stream.length;
  }, [stream.length]);

  useEffect(() => {
    if (prevStreamingRef.current && !streaming) {
      if (atBottomRef.current) {
        virtuosoRef.current?.scrollToIndex({ index: displayData.length - 1, align: "end", behavior: "smooth" });
      } else {
        setShowScrollFab(true);
        setStreamDoneLabel(true);
      }
    }
    prevStreamingRef.current = streaming;
  }, [streaming, displayData.length]);

  const scrollToBottom = useCallback(() => {
    virtuosoRef.current?.scrollToIndex({ index: displayData.length - 1, align: "end", behavior: "smooth" });
    setTimeout(() => {
      setShowScrollFab(false);
      setUnreadCount(0);
      setStreamDoneLabel(false);
    }, 100);
  }, [displayData.length]);

  const virtuosoComponents = useMemo(() => ({
    Header: hasMore ? VirtuosoHeaderWithMore : VirtuosoHeaderEmpty,
    Footer: VirtuosoFooter,
  }), [hasMore]);

  const prevWidthRef = useRef(0);
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width ?? 0;
      if (prevWidthRef.current > 0 && Math.abs(w - prevWidthRef.current) > 2 && virtuosoRef.current) {
        const idx = firstVisibleIndexRef.current;
        requestAnimationFrame(() => {
          virtuosoRef.current?.scrollToIndex({ index: idx, align: "start", behavior: "auto" });
        });
      }
      prevWidthRef.current = w;
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [chatKey]);

  const [visibleRange, setVisibleRange] = useState<{ startIndex: number; endIndex: number }>({ startIndex: 0, endIndex: 0 });
  const handleRangeChanged = useCallback((range: { startIndex: number; endIndex: number }) => {
    firstVisibleIndexRef.current = range.startIndex;
    setVisibleRange(range);
  }, []);

  const lastUserMessage = useMemo(() => {
    for (let i = stream.length - 1; i >= 0; i--) {
      if (stream[i].data.role === "user") return { content: stream[i].data.content, index: i };
    }
    return null;
  }, [stream]);

  const lastUserDisplayIndex = useMemo(() => {
    if (!lastUserMessage) return -1;
    const globalIdx = lastUserMessage.index;
    return globalIdx - paginationOffset;
  }, [lastUserMessage, paginationOffset]);

  const todoProgress = useMemo<TodoSummary | null>(() => {
    if (!streamSegments || streamSegments.length === 0) return null;
    for (let i = streamSegments.length - 1; i >= 0; i--) {
      const seg = streamSegments[i];
      if (seg.type === "tool" && seg.toolCall?.name === "todo_write" && seg.toolCall.result) {
        const parsed = parseTodoResult(seg.toolCall.result);
        if (parsed) return parsed.summary;
      }
    }
    if (!streaming) {
      for (let i = stream.length - 1; i >= 0; i--) {
        const msg = stream[i].data;
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
    }
    return null;
  }, [streamSegments, stream, streaming]);

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
    window.dispatchEvent(new CustomEvent("fastclaw:edit-message", { detail: { text: lastUserMessage.content } }));
  }, [lastUserMessage, streaming, stopStream]);

  const handleResendFromBar = useCallback(() => {
    if (!lastUserMessage) return;
    handleMentionSend(lastUserMessage.content, []);
  }, [lastUserMessage, handleMentionSend]);

  const isEmpty = stream.length === 0 && !streaming;
  const [showPlanPanel, setShowPlanPanel] = useState(false);
  const togglePlanPanel = useCallback(() => setShowPlanPanel((v) => !v), []);

  const chatSessionId = activeChat?.id ?? "";
  const planFilePath = activeChat?.planFilePath;
  const planFileExists = activeChat?.planFileExists ?? false;

  return (
    <div className="flex min-h-0 flex-1">
    <div
      className="relative flex min-h-0 flex-1 flex-col"
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
          style={{ background: "rgba(0, 122, 255, 0.06)", animation: "fade-in var(--duration-fast)" }}
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
            <Upload size={32} strokeWidth={1.5} style={{ color: "var(--fill-secondary)" }} />
            <span className="text-[15px] font-medium" style={{ color: "var(--fill-primary)" }}>
              拖拽文件到这里
            </span>
            <span className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              支持图片、文档、代码文件
            </span>
          </div>
        </div>
      )}

      {searchOpen && (
        <div
          className="flex shrink-0 items-center gap-2 px-4 py-2"
          style={{ background: "var(--bg-secondary)", borderBottom: `0.5px solid var(--separator)`, animation: "slide-down var(--duration-fast) var(--ease-out)" }}
        >
          <Search {...ICON.md} style={{ color: "var(--fill-tertiary)" }} />
          <input
            ref={searchInputRef}
            value={searchQuery}
            onChange={(e) => { setSearchQuery(e.target.value); setSearchIdx(0); }}
            onKeyDown={(e) => {
              if (e.key === "Escape") closeSearch();
              if (e.key === "Enter" && !e.shiftKey) setSearchIdx((i) => (i + 1) % Math.max(searchResults.length, 1));
              if (e.key === "Enter" && e.shiftKey) setSearchIdx((i) => (i - 1 + Math.max(searchResults.length, 1)) % Math.max(searchResults.length, 1));
            }}
            placeholder="搜索消息..."
            className="min-w-0 flex-1 bg-transparent text-[13px] outline-none"
            style={{ color: "var(--fill-primary)" }}
          />
          {searchQuery && (
            <span className="shrink-0 text-[11px] tabular-nums" style={{ color: "var(--fill-tertiary)" }}>
              {searchResults.length > 0 ? `${searchIdx + 1}/${searchResults.length}` : "无结果"}
            </span>
          )}
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => setSearchIdx((i) => (i - 1 + Math.max(searchResults.length, 1)) % Math.max(searchResults.length, 1))}
              disabled={searchResults.length === 0}
              className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <ChevronUp {...ICON.sm} />
            </button>
            <button
              onClick={() => setSearchIdx((i) => (i + 1) % Math.max(searchResults.length, 1))}
              disabled={searchResults.length === 0}
              className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <ChevronDown {...ICON.sm} />
            </button>
          </div>
          <button
            onClick={closeSearch}
            className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <X {...ICON.sm} />
          </button>
        </div>
      )}

      {isEmpty ? (
        <div className="flex-1 overflow-y-auto px-6 py-6">
          <StreamEmptyState onPick={(t) => {
            if (mentionInputRef.current) {
              mentionInputRef.current.setText(t);
              mentionInputRef.current.focus();
            }
          }} />
        </div>
      ) : (
        <div ref={containerRef} className="flex min-h-0 flex-1 flex-col">
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
          <Virtuoso
            ref={virtuosoRef}
            key={chatKey}
            data={displayData}
            initialTopMostItemIndex={Math.max(0, displayData.length - 1)}
            followOutput={(isAtBottom) => {
              if (isAtBottom) return "smooth";
              if (streaming && atBottomRef.current) return "smooth";
              return false;
            }}
            atBottomStateChange={handleAtBottomChange}
            atBottomThreshold={120}
            className="flex-1"
            style={{ overflowX: "hidden", overflowY: "scroll" }}
            onScroll={handleScroll}
            startReached={handleStartReached}
            rangeChanged={handleRangeChanged}
            itemContent={(idx, item) => (
              <MessageRendererRow
                item={item}
                idx={idx}
                paginationOffset={paginationOffset}
                searchQuery={searchQuery}
                searchIdx={searchIdx}
                searchResults={searchResults}
                streamSegments={streamSegments}
                subAgentRuns={activeChat?.subAgentRuns}
                bottomRef={bottomRef}
                animate={animateMessages}
                lastSegments={activeChat?.lastSegments as import("./types").StreamSegment[] | undefined}
              />
            )}
            increaseViewportBy={VIEWPORT_INCREASE}
            components={virtuosoComponents}
          />
        </div>
      )}

      {showScrollFab && !isEmpty && (
        <div className="absolute right-6 bottom-[140px] z-20" style={{ animation: "fade-in var(--duration-fast) var(--ease-out)" }}>
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
            <ArrowDown {...ICON.sm} />
            {streamDoneLabel ? (
              <span className="text-[11px] font-medium" style={{ color: "var(--tint)" }}>
                输出完成
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
    </div>

    {showPlanPanel && chatSessionId && (
      <div style={{ width: 360, minWidth: 360 }} className="shrink-0">
        <PlanPanel
          sessionId={chatSessionId}
          planFilePath={planFilePath}
          planFileExists={planFileExists}
          onClose={() => setShowPlanPanel(false)}
        />
      </div>
    )}
    </div>
  );
}
