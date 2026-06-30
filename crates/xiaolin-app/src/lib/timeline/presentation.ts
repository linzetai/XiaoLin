import type {
  ApprovalNode,
  IterationBoundaryNode,
  SystemNoticeNode,
  ToolGroupNode,
  ToolStepNode,
  TurnDisplayNode,
  TurnStatusNode,
} from "./types";

export type AssistantPresentationMode = "active" | "completed" | "abnormal";

export type AssistantProcessNode =
  | ToolStepNode
  | ToolGroupNode
  | SystemNoticeNode;

export type AssistantVisibleNode = Exclude<TurnDisplayNode, IterationBoundaryNode>;

// ============================================================================
// ProcessInterval
// ============================================================================

export interface ProcessInterval {
  id: string;
  nodes: AssistantProcessNode[];
  startMs: number;
  endMs: number;
  durationMs: number;
}

// ============================================================================
// Presentation items
// ============================================================================

export type ActivityNarrationSource = "runtime" | "model_public";

export interface RuntimeActivityNarration {
  id: string;
  text: string;
  source: ActivityNarrationSource;
  phaseKey: string;
  createdAtMs: number;
  anchorNodeId?: string;
}

export type AssistantPresentationItem =
  | { kind: "narration"; narration: RuntimeActivityNarration }
  | { kind: "completed_batch"; interval: ProcessInterval }
  | { kind: "running_tool"; node: ToolStepNode }
  | { kind: "approval"; node: ApprovalNode }
  | { kind: "failed_tool"; node: ToolStepNode }
  | { kind: "error"; node: TurnStatusNode | SystemNoticeNode }
  | { kind: "visible"; node: AssistantVisibleNode }
  // Compatibility for older tests/callers while the renderer migrates.
  | { kind: "attention"; node: AssistantVisibleNode }
  | { kind: "process_interval"; interval: ProcessInterval };

export interface AssistantTurnPresentation {
  mode: AssistantPresentationMode;
  items: AssistantPresentationItem[];
  processNodes: AssistantProcessNode[];
  terminalStatus?: TurnStatusNode;
}

// ============================================================================
// Approval decision normalization
// ============================================================================

export type ApprovalDecision =
  | "allow_once"
  | "allow_always"
  | "deny"
  | "abort"
  | "other";

export function normalizeApprovalDecision(raw?: string): ApprovalDecision {
  switch (raw) {
    case "allow_once":
    case "approved":
      return "allow_once";
    case "allow_always":
    case "approved_for_session":
      return "allow_always";
    case "deny":
    case "denied":
      return "deny";
    case "abort":
    case "aborted":
      return "abort";
    default:
      return "other";
  }
}

// ============================================================================
// Selector
// ============================================================================

export function selectAssistantTurnPresentation(
  nodes: TurnDisplayNode[],
  options: { showDiagnostics?: boolean } = {},
): AssistantTurnPresentation {
  const mode = derivePresentationMode(nodes);
  const terminalStatus = nodes.find(isTerminalStatus);
  const items = buildPresentationItems(nodes, options);

  return {
    mode,
    items,
    processNodes: collectProcessNodes(nodes),
    terminalStatus,
  };
}

// ============================================================================
// Presentation item builder
// ============================================================================

export function buildPresentationItems(
  nodes: TurnDisplayNode[],
  options: { showDiagnostics?: boolean } = {},
): AssistantPresentationItem[] {
  const items: AssistantPresentationItem[] = [];
  let currentInterval: AssistantProcessNode[] = [];
  let currentBatchFamily: string | null = null;
  let lastNarrationPhase: string | null = null;

  const flushInterval = () => {
    if (currentInterval.length === 0) return;
    const first = currentInterval[0];
    const last = currentInterval[currentInterval.length - 1];
    items.push({
      kind: "completed_batch",
      interval: {
        id: `process-${first.turn_id}-${first.node_id}`,
        nodes: [...currentInterval],
        startMs: first.created_at_ms,
        endMs: last.updated_at_ms,
        durationMs: Math.max(0, last.updated_at_ms - first.created_at_ms),
      },
    });
    currentInterval = [];
    currentBatchFamily = null;
  };

  const pushRuntimeNarration = (phase: RuntimeNarrationPhase | null, anchor: TurnDisplayNode) => {
    if (!phase) return;
    if (phase.phaseKey === lastNarrationPhase) return;
    flushInterval();
    items.push({
      kind: "narration",
      narration: {
        id: `runtime-narration:${anchor.turn_id}:${phase.phaseKey}:${anchor.node_id}`,
        text: phase.text,
        source: "runtime",
        phaseKey: phase.phaseKey,
        createdAtMs: anchor.created_at_ms,
        anchorNodeId: anchor.node_id,
      },
    });
    lastNarrationPhase = phase.phaseKey;
  };

  const pushCompletedProcessNode = (node: AssistantProcessNode, family: string) => {
    if (currentBatchFamily != null && currentBatchFamily !== family) {
      flushInterval();
    }
    currentInterval.push(node);
    currentBatchFamily = family;
  };

  for (const node of nodes) {
    if (
      (node.kind === "assistant_text" || node.kind === "reasoning") &&
      !node.content.trim()
    ) {
      continue;
    }

    if (node.kind === "iteration_boundary") {
      if (!options.showDiagnostics) continue;
      flushInterval();
      items.push({ kind: "visible", node: node as unknown as AssistantVisibleNode });
      continue;
    }

    if (isNormalCompletionStatus(node)) continue;

    // Raw reasoning / CoT is intentionally not shown in the Turn Flow UI.
    if (node.kind === "reasoning") {
      continue;
    }

    if (node.kind === "tool_step" && (node.status === "failed" || node.status === "cancelled")) {
      pushRuntimeNarration(runtimeNarrationForNode(node), node);
      flushInterval();
      items.push({ kind: "failed_tool", node });
      continue;
    }

    if (node.kind === "turn_status" && isAbnormalTurnStatus(node)) {
      flushInterval();
      items.push({ kind: "error", node });
      continue;
    }

    if (node.kind === "system_notice" && (node.level === "error" || node.level === "warning")) {
      flushInterval();
      items.push({ kind: "error", node });
      continue;
    }

    if (node.kind === "approval") {
      flushInterval();
      items.push({ kind: "approval", node });
      continue;
    }

    if (node.kind === "assistant_text") {
      flushInterval();
      if (node.text_role === "activity") {
        const phaseKey = `model_public:${node.node_id}`;
        items.push({
          kind: "narration",
          narration: {
            id: `model-public-narration:${node.turn_id}:${node.node_id}`,
            text: node.content,
            source: "model_public",
            phaseKey,
            createdAtMs: node.created_at_ms,
            anchorNodeId: node.node_id,
          },
        });
        lastNarrationPhase = phaseKey;
      } else {
        items.push({ kind: "visible", node });
      }
      continue;
    }

    if (node.kind === "tool_step" && (node.status === "running" || node.status === "pending")) {
      pushRuntimeNarration(runtimeNarrationForNode(node), node);
      flushInterval();
      items.push({ kind: "running_tool", node });
      continue;
    }

    if (node.kind === "tool_step" && node.status === "completed") {
      pushRuntimeNarration(runtimeNarrationForNode(node), node);
      pushCompletedProcessNode(node, completedToolFamily(node));
      continue;
    }

    if (node.kind === "tool_group") {
      const firstStep = node.steps[0];
      if (firstStep) pushRuntimeNarration(runtimeNarrationForNode(firstStep), node);
      pushCompletedProcessNode(node, completedToolFamily(firstStep));
      continue;
    }

    if (node.kind === "system_notice" && node.level !== "error" && node.level !== "warning") {
      pushCompletedProcessNode(node, "notice");
      continue;
    }

    flushInterval();
    items.push({ kind: "visible", node: node as AssistantVisibleNode });
  }

  flushInterval();
  return items;
}

// ============================================================================
// Helpers
// ============================================================================

export function derivePresentationMode(nodes: TurnDisplayNode[]): AssistantPresentationMode {
  const abnormal = nodes.some((node) => node.kind === "turn_status" && isAbnormalTurnStatus(node));
  if (abnormal) return "abnormal";

  const normalComplete = nodes.some(isNormalCompletionStatus);
  const active = nodes.some((node) => node.status === "pending" || node.status === "running");

  if (!normalComplete || active) return "active";
  return "completed";
}

export function isFoldableProcessNode(node: TurnDisplayNode): node is AssistantProcessNode {
  return (
    (node.kind === "tool_step" && node.status === "completed") ||
    node.kind === "tool_group" ||
    (node.kind === "system_notice" && node.level !== "error" && node.level !== "warning")
  );
}

export function isAttentionItem(node: TurnDisplayNode): boolean {
  if (node.kind === "tool_step") {
    return node.status === "failed" || node.status === "cancelled";
  }
  if (node.kind === "system_notice") {
    return node.level === "error" || node.level === "warning";
  }
  if (node.kind === "turn_status") {
    return isAbnormalTurnStatus(node);
  }
  return false;
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
  return nodes.filter(isFoldableProcessNode);
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

interface RuntimeNarrationPhase {
  phaseKey: string;
  text: string;
}

function runtimeNarrationForNode(node: ToolStepNode): RuntimeNarrationPhase | null {
  if (node.status === "failed" || node.status === "cancelled") {
    return { phaseKey: "tool.failed", text: "工具执行失败，正在等待处理" };
  }

  if (isSubAgentTool(node)) {
    return { phaseKey: "subagent.parallel", text: "正在并行检查多个模块" };
  }

  if (node.tool_category === "file" || node.tool_name.toLowerCase().includes("read")) {
    const target = readToolTarget(node).toLowerCase();
    if (isDesignOrConfigTarget(target)) {
      return { phaseKey: "design.config", text: "正在核对设计与配置" };
    }
    return { phaseKey: "file.read", text: "正在阅读相关实现" };
  }

  if (node.tool_category === "search" || isSearchTool(node)) {
    return { phaseKey: "search.refs", text: "正在搜索相关引用" };
  }

  if (node.tool_category === "shell") {
    const command = readCommand(node).toLowerCase();
    if (/\b(git\s+(diff|status|log))\b/.test(command)) {
      return { phaseKey: "scope.inspect", text: "正在整理本次改动范围" };
    }
    if (/\b(test|build|lint|check|clippy)\b/.test(command)) {
      return { phaseKey: "verify.run", text: "正在验证构建与测试结果" };
    }
    return { phaseKey: "tool.shell", text: "正在整理工具执行结果" };
  }

  return null;
}

function completedToolFamily(step?: ToolStepNode): string {
  if (!step) return "tool:other";
  if (isSubAgentTool(step)) return "tool:sub_agent";
  return `tool:${step.tool_category ?? "other"}`;
}

function readToolTarget(node: ToolStepNode): string {
  return node.target?.path
    ?? node.target?.query
    ?? node.target?.url
    ?? node.target?.label
    ?? node.target?.command
    ?? readStringArg(node.args, "file_path")
    ?? readStringArg(node.args, "path")
    ?? readStringArg(node.args, "query")
    ?? readStringArg(node.args, "command")
    ?? "";
}

function readCommand(node: ToolStepNode): string {
  return node.target?.command ?? readStringArg(node.args, "command") ?? "";
}

function readStringArg(args: string | undefined, key: string): string | undefined {
  if (!args) return undefined;
  try {
    const parsed = JSON.parse(args) as Record<string, unknown>;
    const value = parsed[key];
    return typeof value === "string" ? value : undefined;
  } catch {
    return undefined;
  }
}

function isSearchTool(node: ToolStepNode): boolean {
  const name = node.tool_name.toLowerCase();
  return name.includes("grep") || name.includes("search") || name === "rg";
}

function isSubAgentTool(node: ToolStepNode): boolean {
  const name = node.tool_name.toLowerCase();
  return node.tool_category === "sub_agent" || name.includes("subagent") || name.includes("sub_agent");
}

function isDesignOrConfigTarget(target: string): boolean {
  return (
    target.includes("design") ||
    target.includes("docs/") ||
    /\.(toml|json|ya?ml|config|conf|md)$/i.test(target)
  );
}
