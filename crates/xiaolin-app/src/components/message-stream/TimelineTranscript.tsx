// TimelineTranscript — renders TurnDisplayNode[] from the canonical timeline
// store with virtualization for long transcript performance.
//
// This component uses @tanstack/react-virtual to only render visible nodes,
// keeping long transcripts and high-frequency text deltas responsive.

import { useRef, useEffect, useCallback, memo } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTimelineStore } from "../../lib/stores/timeline-store";
import type { TurnDisplayNode } from "../../lib/timeline/types";
import { TurnNodeRenderer } from "./TurnNodeRenderer";

export interface TimelineTranscriptProps {
  /** Session ID to render the transcript for. */
  sessionId: string;
  /** When true, pending nodes show streaming animations. */
  isLive?: boolean;
  /** Called when the user scrolls to the top (for infinite scroll). */
  onScrollToTop?: () => void;
  /** Ref to the scroll container element. */
  scrollContainerRef?: React.RefObject<HTMLDivElement | null>;
}

/**
 * Virtualized transcript backed by the canonical timeline store.
 *
 * Uses the same TurnNodeRenderer for both live streaming and history replay.
 * Only visible nodes are rendered, keeping scroll performance smooth even
 * with hundreds of tool steps and long text content.
 */
export const TimelineTranscript = memo(function TimelineTranscript({
  sessionId,
  isLive,
  scrollContainerRef: externalScrollRef,
}: TimelineTranscriptProps) {
  const internalScrollRef = useRef<HTMLDivElement>(null);
  const scrollRef = externalScrollRef ?? internalScrollRef;

  // Read display nodes from the timeline store
  const nodes = useTimelineStore((s) => s.states[sessionId]?.nodes ?? []);

  // Virtualizer for bounded rendering
  const virtualizer = useVirtualizer({
    count: nodes.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 80, // rough estimate; measureElement refines
    overscan: 5,
    getItemKey: (index) => nodes[index]?.node_id ?? `missing-${index}`,
  });

  // Keep virtualizer ref for scroll-to-bottom
  const virtualizerRef = useRef(virtualizer);
  virtualizerRef.current = virtualizer;

  // Auto-scroll to bottom when new nodes arrive in live mode
  const prevLengthRef = useRef(nodes.length);
  useEffect(() => {
    if (isLive && nodes.length > prevLengthRef.current) {
      // Only auto-scroll if user is already near the bottom
      if (virtualizerRef.current.isAtEnd()) {
        // Use rAF to let React finish rendering
        requestAnimationFrame(() => {
          virtualizerRef.current.scrollToEnd({ behavior: "smooth" });
        });
      }
    }
    prevLengthRef.current = nodes.length;
  }, [nodes.length, isLive]);

  const measureElement = useCallback(
    (node: HTMLElement | null) => {
      virtualizerRef.current.measureElement(node);
    },
    [],
  );

  if (nodes.length === 0) {
    return null;
  }

  return (
    <div
      ref={scrollRef}
      style={{
        height: "100%",
        overflowY: "auto",
        contain: "strict",
      }}
    >
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((virtualItem) => {
          const node = nodes[virtualItem.index];
          if (!node) return null;
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
              <TurnNodeViewItem node={node} isLive={isLive} />
            </div>
          );
        })}
      </div>
    </div>
  );
});

// Wrapper for a single node to pass to TurnNodeRenderer
const TurnNodeViewItem = memo(function TurnNodeViewItem({
  node,
  isLive,
}: {
  node: TurnDisplayNode;
  isLive?: boolean;
}) {
  return (
    <div className="msg-row">
      <TurnNodeRenderer nodes={[node]} isLive={isLive} />
    </div>
  );
});
