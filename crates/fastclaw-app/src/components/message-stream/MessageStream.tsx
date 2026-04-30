import { useState, useRef, useCallback, useEffect, useMemo, memo } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { useAgentStore } from "../../lib/agent-store";
import { useActiveAgentChats } from "../../lib/stores/selectors";
import type { MentionInputHandle, MentionOption } from "./MentionInput";
import { MessageRendererRow } from "./MessageRenderer";

import { StreamFooter, type AttachedFile, MOD_KEY } from "./StreamFooter";
import { useStreamScroll, STREAM_PAGE_SIZE } from "./useStreamScroll";
import { useMessageStreamChat } from "./useMessageStreamChat";
import { X, ChevronUp, ChevronDown, Settings2, Upload, Search, ArrowDown } from "lucide-react";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { useAvatarUrl } from "../../lib/use-avatar-url";
import { ChatTabsBar } from "./ChatTabsBar";
import { StreamEmptyState } from "./StreamEmptyState";

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

export function MessageStream({ onToggleDetail, detailOpen }: MessageStreamProps) {
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agents = useAgentStore((s) => s.agents);
  const ac = useActiveAgentChats();
  const newChat = useAgentStore((s) => s.newChat);
  const setWorkDir = useAgentStore((s) => s.setWorkDir);
  const setActiveChat = useAgentStore((s) => s.setActiveChat);
  const closeChat = useAgentStore((s) => s.closeChat);
  const renameChat = useAgentStore((s) => s.renameChat);
  const reorderChats = useAgentStore((s) => s.reorderChats);
  const loadChatStream = useAgentStore((s) => s.loadChatStream);

  const agent = agents.find((a) => a.id === activeAgentId) ?? agents[0];
  const agentAvatarUrl = useAvatarUrl(agent?.avatar);
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
    if (loadedChats.current.has(activeChat.id) && activeChat.stream.length > 0) {
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
  const prevStreamLenRef = useRef(stream.length);
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
    streamingChatIds,
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
    return stream
      .map((item, idx) => ({ item, idx }))
      .filter(({ item }) => item.data.content.toLowerCase().includes(q));
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
    window.addEventListener("keydown", handleGlobalKey);
    return () => window.removeEventListener("keydown", handleGlobalKey);
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

  const removeFile = useCallback((index: number) => {
    setAttachedFiles((prev) => {
      const removed = prev[index];
      if (removed?.previewUrl) URL.revokeObjectURL(removed.previewUrl);
      return prev.filter((_, i) => i !== index);
    });
  }, []);

  const chatKey = `${activeAgentId}:${activeChat?.localKey ?? ac?.activeChatId ?? ""}`;

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
    if (atBottom) setUnreadCount(0);
  }, [rawAtBottomChange]);

  useEffect(() => {
    if (!atBottomRef.current && stream.length > prevStreamLenRef.current) {
      setUnreadCount((c) => c + (stream.length - prevStreamLenRef.current));
    }
    prevStreamLenRef.current = stream.length;
  }, [stream.length]);

  const scrollToBottom = useCallback(() => {
    virtuosoRef.current?.scrollToIndex({ index: displayData.length - 1, align: "end", behavior: "smooth" });
    setTimeout(() => {
      setShowScrollFab(false);
      setUnreadCount(0);
    }, 100);
  }, [displayData.length]);

  const virtuosoComponents = useMemo(() => ({
    Header: hasMore ? VirtuosoHeaderWithMore : VirtuosoHeaderEmpty,
    Footer: VirtuosoFooter,
  }), [hasMore]);

  const isEmpty = stream.length === 0 && !streaming;

  return (
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

      <div
        className="vibrancy flex shrink-0 items-center justify-between px-6 py-3"
        style={{ background: "var(--bg-sidebar)", borderBottom: `0.5px solid var(--separator)` }}
      >
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <div
            className="flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-full text-[13px] font-semibold"
            style={{ background: agent.color, color: "white" }}
          >
            {agentAvatarUrl ? (
              <img src={agentAvatarUrl} alt="" className="h-full w-full object-cover" />
            ) : (
              agent.initial
            )}
          </div>
          <div className="min-w-0">
            <div className="truncate text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }} title={agent.name}>{agent.name}</div>
            <div className="mt-0.5 flex items-center gap-1.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
              <span className="inline-block h-[6px] w-[6px] rounded-full" style={{ background: agent.online ? "var(--green)" : "var(--fill-quaternary)" }} />
              {agent.online ? "在线" : "离线"}
            </div>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            onClick={openSearch}
            className="flex h-8 w-8 items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: searchOpen ? "var(--tint)" : "var(--fill-tertiary)" }}
            title={`搜索消息 (${MOD_KEY}F)`}
          >
            <Search size={15} strokeWidth={1.5} />
          </button>
          <button
            onClick={onToggleDetail}
            className="flex h-8 w-8 items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: detailOpen ? "var(--fill-primary)" : "var(--fill-tertiary)" }}
            title={detailOpen ? "关闭详情" : "打开详情"}
          >
            <Settings2 size={16} strokeWidth={1.5} />
          </button>
        </div>
      </div>

      {ac && <ChatTabsBar
        agentId={activeAgentId}
        chats={ac.chatList}
        activeChatId={ac.activeChatId}
        streamingChatIds={streamingChatIds}
        onSelect={(id) => setActiveChat(activeAgentId, id)}
        onClose={(id) => closeChat(activeAgentId, id)}
        onNew={() => newChat(activeAgentId, workDir ?? undefined)}
        onRename={(id, t) => renameChat(activeAgentId, id, t)}
        onReorder={(from, to) => reorderChats(activeAgentId, from, to)}
      />}

      {searchOpen && (
        <div
          className="flex shrink-0 items-center gap-2 px-4 py-2"
          style={{ background: "var(--bg-secondary)", borderBottom: `0.5px solid var(--separator)`, animation: "slide-down var(--duration-fast) var(--ease-out)" }}
        >
          <Search size={14} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
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
              <ChevronUp size={10} strokeWidth={2} />
            </button>
            <button
              onClick={() => setSearchIdx((i) => (i + 1) % Math.max(searchResults.length, 1))}
              disabled={searchResults.length === 0}
              className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <ChevronDown size={10} strokeWidth={2} />
            </button>
          </div>
          <button
            onClick={closeSearch}
            className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <X size={10} strokeWidth={2} />
          </button>
        </div>
      )}

      {isEmpty ? (
        <div className="flex-1 overflow-y-auto px-8 py-6">
          <StreamEmptyState onPick={(t) => {
            if (mentionInputRef.current) {
              mentionInputRef.current.clear();
              handleMentionSend(t, []);
            }
          }} />
        </div>
      ) : (
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
            />
          )}
          increaseViewportBy={VIEWPORT_INCREASE}
          components={virtuosoComponents}
        />
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
            <ArrowDown size={14} strokeWidth={2} />
            {unreadCount > 0 && (
              <span
                className="flex h-[18px] min-w-[18px] items-center justify-center rounded-full px-1 text-[10px] font-semibold tabular-nums"
                style={{ background: "var(--tint)", color: "#fff" }}
              >
                {unreadCount > 99 ? "99+" : unreadCount}
              </span>
            )}
          </button>
        </div>
      )}

      <StreamFooter
        mentionInputRef={mentionInputRef}
        fileInputRef={fileInputRef}
        workDir={workDir}
        activeChat={activeChat}
        streaming={streaming}
        detailOpen={detailOpen}
        onToggleDetail={onToggleDetail}
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
      />
    </div>
  );
}
