// TimelineTranscript — renders TurnDisplayNode[] from the canonical timeline
// store with virtualization for long transcript performance.
//
// Nodes are grouped into turn-level message blocks (Codex app / ChatGPT-style)
// before virtualization. Each turn group is one virtual item, keeping the user
// message and its assistant response together visually.
//
// This component uses @tanstack/react-virtual to only render visible turn
// blocks, keeping long transcripts and high-frequency text deltas responsive.

import { useRef, useEffect, useCallback, memo, useMemo } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTimelineStore } from "../../lib/stores/timeline-store";
import { selectTurnGroups } from "../../lib/timeline/selectors";
import type { TurnGroup } from "../../lib/timeline/selectors";
import type { TurnDisplayNode } from "../../lib/timeline/types";
import { TurnBlock } from "./TurnBlock";

const EMPTY_NODES: TurnDisplayNode[] = [];

export interface TimelineTranscriptProps {
  /** Session ID to render the transcript for. */
  sessionId: string;
  /** When true, pending nodes show streaming animations. */
  isLive?: boolean;
  /** Called when the user scrolls to the top (for infinite scroll). */
  onScrollToTop?: () => void;
  /** Ref to the scroll container element. */
  scrollContainerRef?: React.RefObject<HTMLDivElement | null>;
  /** When true, iteration boundary labels and other diagnostics are visible. */
  showDiagnostics?: boolean;
}

/**
 * Virtualized transcript backed by the canonical timeline store.
 *
 * Groups flat TurnDisplayNode[] into TurnGroup[] (one per turn), then
 * virtualizes at the turn-group level. Each turn block contains a user
 * message bubble and an assistant response block with all assistant-side
 * nodes in timeline order.
 *
 * Uses the same TurnNodeRenderer for both live streaming and history replay.
 * Only visible turn blocks are rendered, keeping scroll performance smooth
 * even with hundreds of tool steps and long text content.
 */
export const TimelineTranscript = memo(function TimelineTranscript({
  sessionId,
  isLive,
  scrollContainerRef: externalScrollRef,
  showDiagnostics = false,
}: TimelineTranscriptProps) {
  const internalScrollRef = useRef<HTMLDivElement>(null);
  const usesExternalScroll = externalScrollRef != null;
  const scrollRef = externalScrollRef ?? internalScrollRef;

  // Subscribe only to a stable store reference. Zustand 5 + React 19 can loop
  // if the selector returns a freshly derived array on each snapshot read.
  const nodes = useTimelineStore((s) => {
    const state = s.states[sessionId];
    if (!state) return EMPTY_NODES;
    return state.nodes;
  });
  const turnGroups: TurnGroup[] = useMemo(
    () => selectTurnGroups({ sessionId, nodes, events: [], maxSeq: 0, turnIndex: {}, eventTraces: {} }),
    [nodes, sessionId],
  );

  const activityKey = useMemo(() => {
    const lastGroup = turnGroups[turnGroups.length - 1];
    if (!lastGroup) return "empty";
    const lastNode = lastGroup.assistantNodes[lastGroup.assistantNodes.length - 1] ?? lastGroup.userMessageNode;
    if (!lastNode) return `${lastGroup.groupId}:empty`;
    const contentLength =
      "content" in lastNode && typeof lastNode.content === "string"
        ? lastNode.content.length
        : 0;
    return `${turnGroups.length}:${lastGroup.groupId}:${lastNode.node_id}:${lastNode.updated_at_ms}:${lastNode.status}:${contentLength}`;
  }, [turnGroups]);

  // Virtualizer for bounded rendering — one virtual item per turn group
  const virtualizer = useVirtualizer({
    count: turnGroups.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 120, // rough estimate; measureElement refines
    overscan: 3,
    getItemKey: (index) => turnGroups[index]?.groupId ?? `missing-turn-${index}`,
  });

  // Keep virtualizer ref for scroll-to-bottom
  const virtualizerRef = useRef(virtualizer);
  virtualizerRef.current = virtualizer;

  useEffect(() => {
    requestAnimationFrame(() => {
      virtualizerRef.current.measure();
    });
  }, [sessionId, turnGroups.length]);

  // Auto-scroll to bottom when new turn groups arrive in live mode
  const prevActivityKeyRef = useRef(activityKey);
  useEffect(() => {
    if (isLive && activityKey !== prevActivityKeyRef.current) {
      // Only auto-scroll if user is already near the bottom
      if (virtualizerRef.current.isAtEnd()) {
        // Use rAF to let React finish rendering
        requestAnimationFrame(() => {
          virtualizerRef.current.scrollToEnd({ behavior: "smooth" });
        });
      }
    }
    prevActivityKeyRef.current = activityKey;
  }, [activityKey, isLive]);

  const measureElement = useCallback(
    (node: HTMLElement | null) => {
      virtualizerRef.current.measureElement(node);
    },
    [],
  );

  if (turnGroups.length === 0) {
    return null;
  }

  const virtualItems = virtualizer.getVirtualItems();
  const useStaticFallback = virtualItems.length === 0 && turnGroups.length > 0;

  const content = useStaticFallback ? (
    <div
      style={{
        width: "100%",
        position: "relative",
      }}
      data-virtualizer-fallback="true"
    >
      {turnGroups.map((turnGroup, index) => (
        <div
          key={turnGroup.groupId}
          data-index={index}
          ref={measureElement}
          style={{ padding: "0 clamp(24px, 5%, 80px)" }}
        >
          <TurnBlock
            turnGroup={turnGroup}
            isLive={isLive}
            sessionId={sessionId}
            showDiagnostics={showDiagnostics}
          />
        </div>
      ))}
    </div>
  ) : (
    <div
      style={{
        height: virtualizer.getTotalSize(),
        width: "100%",
        position: "relative",
      }}
    >
      {virtualItems.map((virtualItem) => {
        const turnGroup = turnGroups[virtualItem.index];
        if (!turnGroup) return null;
        return (
          <div
            key={virtualItem.key}
            data-index={virtualItem.index}
            ref={measureElement}
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              transform: `translateY(${virtualItem.start}px)`,
              padding: "0 clamp(24px, 5%, 80px)",
            }}
          >
            <TurnBlock
              turnGroup={turnGroup}
              isLive={isLive}
              sessionId={sessionId}
              showDiagnostics={showDiagnostics}
            />
          </div>
        );
      })}
    </div>
  );

  if (usesExternalScroll) {
    return (
      <div data-diagnostics={showDiagnostics ? "true" : undefined}>
        {content}
      </div>
    );
  }

  return (
    <div
      ref={scrollRef}
      data-diagnostics={showDiagnostics ? "true" : undefined}
      style={{
        height: "100%",
        overflowY: "auto",
        contain: "strict",
      }}
    >
      {content}
    </div>
  );
});
