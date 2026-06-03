import { useMemo } from "react";
import { aggregateFileChanges, type FileChangeSummary } from "./edit-result-utils";
import type { ChatMessageToolCall } from "../../lib/stores/types";
import type { StreamSegment } from "./types";

export function useFileChangeSummary(
  toolCalls?: ChatMessageToolCall[],
  savedSegments?: StreamSegment[],
): FileChangeSummary | null {
  return useMemo(() => {
    const results: string[] = [];

    if (savedSegments && savedSegments.length > 0) {
      for (const seg of savedSegments) {
        const tc = seg.toolCall;
        if (tc?.name === "edit_file" && tc.result) {
          results.push(tc.result);
        }
      }
    } else if (toolCalls) {
      for (const tc of toolCalls) {
        if (tc.name === "edit_file" && tc.result) {
          results.push(tc.result);
        }
      }
    }

    return aggregateFileChanges(results);
  }, [toolCalls, savedSegments]);
}
