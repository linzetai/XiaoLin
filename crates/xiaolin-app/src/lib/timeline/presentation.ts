import type {
  ApprovalNode,
  IterationBoundaryNode,
  ReasoningNode,
  SystemNoticeNode,
  ToolGroupNode,
  ToolStepNode,
  TurnDisplayNode,
  TurnStatusNode,
} from "./types";

export type AssistantPresentationMode = "active" | "completed" | "abnormal";

export type AssistantProcessNode =
  | ReasoningNode
  | ToolStepNode
  | ToolGroupNode
  | ApprovalNode
  | SystemNoticeNode;

export type AssistantVisibleNode = Exclude<TurnDisplayNode, IterationBoundaryNode>;

export type AssistantPresentationItem =
  | { kind: "visible"; node: AssistantVisibleNode }
  | {
      kind: "process_summary";
      id: string;
      nodes: AssistantProcessNode[];
      elapsedMs?: number;
    };

export interface AssistantTurnPresentation {
  mode: AssistantPresentationMode;
  items: AssistantPresentationItem[];
  processNodes: AssistantProcessNode[];
  terminalStatus?: TurnStatusNode;
}

export function selectAssistantTurnPresentation(
  nodes: TurnDisplayNode[],
  options: { showDiagnostics?: boolean } = {},
): AssistantTurnPresentation {
  const mode = derivePresentationMode(nodes);
  const terminalStatus = nodes.find(isTerminalStatus);

  if (mode === "active") {
    const activeItems = nodes
      .filter((node) => options.showDiagnostics || node.kind !== "iteration_boundary")
      .map((node): AssistantPresentationItem => ({ kind: "visible", node: node as AssistantVisibleNode }));
    return {
      mode,
      items: activeItems,
      processNodes: collectProcessNodes(nodes),
      terminalStatus,
    };
  }

  const items: AssistantPresentationItem[] = [];
  const processNodes: AssistantProcessNode[] = [];
  let summaryInserted = false;
  const elapsedMs = terminalStatus?.elapsed_ms;

  const flushSummary = () => {
    if (summaryInserted || processNodes.length === 0) return;
    items.push({
      kind: "process_summary",
      id: `process-summary-${processNodes[0].turn_id}-${processNodes[0].node_id}`,
      nodes: processNodes,
      elapsedMs,
    });
    summaryInserted = true;
  };

  for (const node of nodes) {
    if (node.kind === "iteration_boundary") {
      if (options.showDiagnostics) {
        // Iteration boundaries are diagnostic-only. Keep them out of the
        // user-facing completed process transcript unless a diagnostic view
        // renders raw nodes directly.
      }
      continue;
    }

    if (isNormalCompletionStatus(node)) {
      continue;
    }

    if (isFoldedProcessNode(node)) {
      if (!summaryInserted) {
        processNodes.push(node);
        continue;
      }
      // Once the summary is inserted, later process nodes are still folded into
      // the same summary so completed turns have one process affordance.
      processNodes.push(node);
      continue;
    }

    if (mode === "abnormal" && node.kind === "turn_status") {
      flushSummary();
      items.push({ kind: "visible", node });
      continue;
    }

    flushSummary();
    items.push({ kind: "visible", node });
  }

  flushSummary();

  return {
    mode,
    items,
    processNodes,
    terminalStatus,
  };
}

export function derivePresentationMode(nodes: TurnDisplayNode[]): AssistantPresentationMode {
  const abnormal = nodes.some((node) => node.kind === "turn_status" && isAbnormalTurnStatus(node));
  if (abnormal) return "abnormal";

  const normalComplete = nodes.some(isNormalCompletionStatus);
  const active = nodes.some((node) => node.status === "pending" || node.status === "running");

  if (!normalComplete || active) return "active";
  return "completed";
}

export function isFoldedProcessNode(node: TurnDisplayNode): node is AssistantProcessNode {
  return (
    node.kind === "reasoning" ||
    node.kind === "tool_step" ||
    node.kind === "tool_group" ||
    node.kind === "approval" ||
    node.kind === "system_notice"
  );
}

export function isAbnormalTurnStatus(node: TurnStatusNode): boolean {
  return (
    node.end_reason === "tool_loop" ||
    node.end_reason === "interrupted" ||
    node.end_reason === "replaced" ||
    node.end_reason === "budget_limited" ||
    node.end_reason === "cancelled" ||
    node.diagnosis?.severity === "error" ||
    node.status === "failed" ||
    node.status === "cancelled"
  );
}

function collectProcessNodes(nodes: TurnDisplayNode[]): AssistantProcessNode[] {
  return nodes.filter(isFoldedProcessNode);
}

function isTerminalStatus(node: TurnDisplayNode): node is TurnStatusNode {
  return node.kind === "turn_status";
}

function isNormalCompletionStatus(node: TurnDisplayNode): boolean {
  return (
    node.kind === "turn_status" &&
    node.end_reason === "completed" &&
    node.diagnosis == null &&
    node.status !== "failed" &&
    node.status !== "cancelled"
  );
}
