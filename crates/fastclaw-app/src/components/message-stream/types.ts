import type { ToolCall } from "./ToolCallCard";

export interface StreamSegment {
  id: string;
  type: "text" | "tool";
  content?: string;
  toolCall?: ToolCall;
}
