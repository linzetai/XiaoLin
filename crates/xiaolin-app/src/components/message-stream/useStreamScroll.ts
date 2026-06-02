import { useRef, useCallback, useEffect, type RefObject, type MutableRefObject, type Dispatch, type SetStateAction } from "react";
import type { VirtuosoHandle } from "react-virtuoso";
import type { ChatMessage } from "../../lib/agent-store";

export const STREAM_PAGE_SIZE = 50;

export function useStreamScroll({
  virtuosoRef,
  scrollPositions,
  chatKey,
  displayDataLength,
  streamLength,
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
}: {
  virtuosoRef: RefObject<VirtuosoHandle | null>;
  scrollPositions: MutableRefObject<Record<string, number>>;
  chatKey: string;
  displayDataLength: number;
  streamLength: number;
  hasMore: boolean;
  setVisibleCount: Dispatch<SetStateAction<number>>;
  paginationOffsetRef: MutableRefObject<number>;
  searchIdx: number;
  searchResults: Array<{ item: { data: ChatMessage }; idx: number }>;
  atBottomRef: MutableRefObject<boolean>;
  pendingBottomScrollBehaviorRef: MutableRefObject<"auto" | "smooth" | null>;
  pendingRestoreScrollTopRef: MutableRefObject<number | null>;
  suppressScrollTrackingUntilRef: MutableRefObject<number>;
  runProgrammaticScroll: (action: () => void, suppressMs?: number) => void;
}) {
  const handleAtBottomChange = useCallback((atBottom: boolean) => {
    atBottomRef.current = atBottom;
  }, [atBottomRef]);

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    if (Date.now() < suppressScrollTrackingUntilRef.current) return;
    if (!e.nativeEvent.isTrusted) return;
    const top = (e.target as HTMLDivElement).scrollTop;
    if (atBottomRef.current) {
      scrollPositions.current[chatKey] = 0;
      return;
    }
    scrollPositions.current[chatKey] = top;
  }, [chatKey, scrollPositions, atBottomRef, suppressScrollTrackingUntilRef]);

  const prevChatKey = useRef(chatKey);

  useEffect(() => {
    if (prevChatKey.current !== chatKey) {
      const prevKey = prevChatKey.current;
      if (virtuosoRef.current && atBottomRef.current) {
        scrollPositions.current[prevKey] = 0;
      }
      prevChatKey.current = chatKey;
      pendingRestoreScrollTopRef.current = scrollPositions.current[chatKey] ?? null;
    }
  }, [chatKey, scrollPositions, virtuosoRef, atBottomRef, pendingRestoreScrollTopRef]);

  useEffect(() => {
    if (pendingBottomScrollBehaviorRef.current == null || !virtuosoRef.current) return;
    const behavior = pendingBottomScrollBehaviorRef.current;
    pendingBottomScrollBehaviorRef.current = null;
    requestAnimationFrame(() => {
      runProgrammaticScroll(() => {
        virtuosoRef.current?.scrollToIndex({ index: "LAST", align: "end", behavior });
      });
    });
  }, [displayDataLength, chatKey, runProgrammaticScroll, virtuosoRef, pendingBottomScrollBehaviorRef]);

  useEffect(() => {
    if (pendingRestoreScrollTopRef.current == null || !virtuosoRef.current) return;
    const restoreTop = pendingRestoreScrollTopRef.current;
    pendingRestoreScrollTopRef.current = null;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        runProgrammaticScroll(() => {
          virtuosoRef.current?.scrollTo({ top: restoreTop });
        }, 360);
      });
    });
  }, [chatKey, displayDataLength, runProgrammaticScroll, virtuosoRef, pendingRestoreScrollTopRef]);

  const handleStartReached = useCallback(() => {
    if (hasMore) {
      let startIndex = 0;
      virtuosoRef.current?.getState((state) => {
        startIndex = state.ranges?.[0]?.startIndex ?? 0;
      });
      setVisibleCount((prev) => {
        const next = Math.min(prev + STREAM_PAGE_SIZE, streamLength);
        const added = next - prev;
        if (added > 0) {
          requestAnimationFrame(() => {
            runProgrammaticScroll(() => {
              virtuosoRef.current?.scrollToIndex({
                index: startIndex + added,
                align: "start",
                behavior: "auto",
              });
            });
          });
        }
        return next;
      });
    }
  }, [hasMore, streamLength, setVisibleCount, runProgrammaticScroll, virtuosoRef]);

  useEffect(() => {
    if (searchResults.length > 0 && virtuosoRef.current) {
      const fullIdx = searchResults[searchIdx]?.idx;
      if (fullIdx != null) {
        const visibleIdx = fullIdx - paginationOffsetRef.current;
        if (visibleIdx < 0) {
          const neededVisibleCount = streamLength - fullIdx;
          setVisibleCount((prev) => Math.max(prev, neededVisibleCount));
          return;
        }
        if (visibleIdx >= 0 && visibleIdx < displayDataLength) {
          runProgrammaticScroll(() => {
            virtuosoRef.current?.scrollToIndex({ index: visibleIdx, align: "center", behavior: "auto" });
          });
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              const mark = document.querySelector('mark[data-search-highlight="current"]');
              if (mark) {
                mark.scrollIntoView({ behavior: "smooth", block: "center" });
              } else {
                setTimeout(() => {
                  const markRetry = document.querySelector('mark[data-search-highlight="current"]');
                  if (markRetry) {
                    markRetry.scrollIntoView({ behavior: "smooth", block: "center" });
                  }
                }, 200);
              }
            });
          });
        }
      }
    }
  }, [searchIdx, searchResults, displayDataLength, streamLength, runProgrammaticScroll, virtuosoRef, setVisibleCount, paginationOffsetRef]);

  return {
    handleAtBottomChange,
    handleScroll,
    handleStartReached,
  };
}
