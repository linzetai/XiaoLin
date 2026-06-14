import type { ToolCall } from "./ToolCallCard";

export interface StreamSegment {
  id: string;
  type: "text" | "tool" | "reasoning" | "iteration_boundary";
  content?: string;
  toolCall?: ToolCall;
  iteration?: number;
}
