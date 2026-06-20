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

    const FILE_TOOLS = new Set(["edit_file", "write_file", "create_file", "multi_edit", "apply_patch", "str_replace_editor"]);
    if (savedSegments && savedSegments.length > 0) {
      for (const seg of savedSegments) {
        const tc = seg.toolCall;
        if (tc && FILE_TOOLS.has(tc.name) && tc.result) {
          results.push(tc.result);
        }
      }
    } else if (toolCalls) {
      for (const tc of toolCalls) {
        if (FILE_TOOLS.has(tc.name) && tc.result) {
          results.push(tc.result);
        }
      }
    }

    return aggregateFileChanges(results);
  }, [toolCalls, savedSegments]);
}
