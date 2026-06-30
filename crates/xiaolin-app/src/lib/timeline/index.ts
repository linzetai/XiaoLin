// Canonical turn timeline module.
//
// This module provides the types, reducer, selectors, fixtures, and
// normalization helpers for the app's single-source-of-truth chat transcript.
//
// Both live WebSocket delivery and history replay use the same reducer
// semantics.  See Decision D3 in the design document.

export type {
  TimelineEventId,
  TimelineEventType,
  TurnTimelineEvent,
  TurnDisplayNode,
  UserMessageNode,
  AssistantTextNode,
  ReasoningNode,
  ToolStepNode,
  ToolGroupNode,
  ApprovalNode,
  IterationBoundaryNode,
  TurnStatusNode,
  SystemNoticeNode,
  NodeStatus,
  ToolCategory,
  TimelineState,
  SourceEventTrace,
  OutputPreview,
  OutputDetailReference,
  ToolTargetMetadata,
  TerminalDiagnosisMetadata,
  // Payload types
  TurnStartedPayload,
  UserMessageCreatedPayload,
  AssistantTextDeltaPayload,
  AssistantTextSnapshotPayload,
  ReasoningDeltaPayload,
  ReasoningSnapshotPayload,
  ToolCallStartedPayload,
  ToolCallProgressPayload,
  ToolCallFinishedPayload,
  ApprovalRequestedPayload,
  ApprovalResolvedPayload,
  IterationBoundaryPayload,
  AssistantMessageFinalizedPayload,
  TurnFinishedPayload,
  CompactBoundaryPayload,
  SystemNoticePayload,
  // Helpers
  TurnDisplayNodeKind,
} from "./types";

export {
  TIMELINE_SCHEMA_VERSION,
  SMALL_OUTPUT_MAX_BYTES,
  SMALL_OUTPUT_MAX_LINES,
  SMALL_OUTPUT_MAX_TOKENS,
  isSmallOutput,
  emptyTimelineState,
} from "./types";

export {
  reduceTimelineEvent,
  reduceTimelineEvents,
  materializeNodes,
} from "./reducer";

export {
  selectNodes,
  selectNodesForTurn,
  selectNodesByKind,
  selectMaxSeq,
  selectLastNode,
  selectTurnIds,
  selectActiveNodes,
  selectEventTypeCounts,
  selectTurnGroups,
} from "./selectors";
export type { TurnGroup } from "./selectors";

export {
  derivePresentationMode,
  isAbnormalTurnStatus,
  isFoldedProcessNode,
  selectAssistantTurnPresentation,
} from "./presentation";
export type {
  AssistantPresentationItem,
  AssistantPresentationMode,
  AssistantProcessNode,
  AssistantTurnPresentation,
} from "./presentation";

export {
  normalizeNodeForComparison,
  nodesAreEquivalent,
  diffNodes,
  sortEventsBySeq,
  deduplicateEvents,
} from "./normalize";

export {
  makeTurnStarted,
  makeUserMessageCreated,
  makeTextDelta,
  makeTextSnapshot,
  makeReasoningDelta,
  makeReasoningSnapshot,
  makeToolStarted,
  makeToolProgress,
  makeToolFinished,
  makeApprovalRequested,
  makeApprovalResolved,
  makeIterationBoundary,
  makeAssistantMessageFinalized,
  makeTurnFinished,
  makeCompactBoundary,
  makeSystemNotice,
  complexTurnFixture,
  simpleTextTurnFixture,
  toolLoopTerminationFixture,
} from "./fixtures";

export {
  recoverTimelineAfterReconnect,
  initTimelineForSession,
} from "./reconnect";
