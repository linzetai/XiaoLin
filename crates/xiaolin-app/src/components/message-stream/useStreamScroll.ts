import { useRef, useCallback, useEffect, type RefObject, type MutableRefObject, type Dispatch, type SetStateAction } from "react";
import type { Virtualizer } from "@tanstack/react-virtual";

export const STREAM_PAGE_SIZE = 50;

export function useStreamScroll({
  virtualizer,
  scrollContainerRef,
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
  virtualizer: Virtualizer<HTMLDivElement, Element> | null;
  scrollContainerRef: RefObject<HTMLDivElement | null>;
  scrollPositions: MutableRefObject<Record<string, number>>;
  chatKey: string;
  displayDataLength: number;
  streamLength: number;
  hasMore: boolean;
  setVisibleCount: Dispatch<SetStateAction<number>>;
  paginationOffsetRef: MutableRefObject<number>;
  searchIdx: number;
  searchResults: Array<{ item: unknown; idx: number }>;
  atBottomRef: MutableRefObject<boolean>;
  pendingBottomScrollBehaviorRef: MutableRefObject<"auto" | "smooth" | null>;
  pendingRestoreScrollTopRef: MutableRefObject<number | null>;
  suppressScrollTrackingUntilRef: MutableRefObject<number>;
  runProgrammaticScroll: (action: () => void, suppressMs?: number) => void;
}) {
  const loadingMore = useRef(false);
  const handleStartReached = useCallback(() => {
    if (!hasMore || loadingMore.current) return;
    loadingMore.current = true;
    setVisibleCount((prev) => {
      const next = Math.min(prev + STREAM_PAGE_SIZE, streamLength);
      loadingMore.current = false;
      return next;
    });
  }, [hasMore, streamLength, setVisibleCount]);

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    if (Date.now() < suppressScrollTrackingUntilRef.current) return;
    if (!e.nativeEvent.isTrusted) return;

    const el = e.target as HTMLDivElement;
    const top = el.scrollTop;

    if (atBottomRef.current) {
      scrollPositions.current[chatKey] = 0;
      return;
    }
    scrollPositions.current[chatKey] = top;

    if (hasMore && top < 200) {
      handleStartReached();
    }
  }, [chatKey, scrollPositions, atBottomRef, suppressScrollTrackingUntilRef, hasMore, handleStartReached]);

  const prevChatKey = useRef(chatKey);

  useEffect(() => {
    if (prevChatKey.current !== chatKey) {
      const prevKey = prevChatKey.current;
      if (atBottomRef.current) {
        scrollPositions.current[prevKey] = 0;
      }
      prevChatKey.current = chatKey;
      pendingRestoreScrollTopRef.current = scrollPositions.current[chatKey] ?? null;
    }
  }, [chatKey, scrollPositions, atBottomRef, pendingRestoreScrollTopRef]);

  useEffect(() => {
    if (pendingBottomScrollBehaviorRef.current == null || !virtualizer) return;
    const behavior = pendingBottomScrollBehaviorRef.current;
    pendingBottomScrollBehaviorRef.current = null;
    requestAnimationFrame(() => {
      runProgrammaticScroll(() => {
        virtualizer.scrollToEnd({ behavior });
      });
    });
  }, [displayDataLength, chatKey, runProgrammaticScroll, virtualizer, pendingBottomScrollBehaviorRef]);

  useEffect(() => {
    if (pendingRestoreScrollTopRef.current == null || !scrollContainerRef.current) return;
    const restoreTop = pendingRestoreScrollTopRef.current;
    pendingRestoreScrollTopRef.current = null;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        runProgrammaticScroll(() => {
          if (scrollContainerRef.current) {
            scrollContainerRef.current.scrollTop = restoreTop;
          }
        }, 360);
      });
    });
  }, [chatKey, displayDataLength, runProgrammaticScroll, scrollContainerRef, pendingRestoreScrollTopRef]);

  useEffect(() => {
    if (searchResults.length > 0 && virtualizer) {
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
            virtualizer.scrollToIndex(visibleIdx, { align: "center", behavior: "auto" });
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
  }, [searchIdx, searchResults, displayDataLength, streamLength, runProgrammaticScroll, virtualizer, setVisibleCount, paginationOffsetRef]);

  return {
    handleScroll,
    handleStartReached,
  };
}
