import { useState, useCallback } from "react";
import { CaretDown, CaretUp, Robot, Trash } from "@phosphor-icons/react";
import { useBrowserStore } from "../../lib/stores/browser-store";

function formatTime(ts: number): string {
  try {
    return new Date(ts).toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return "";
  }
}

const ACTION_LABELS: Record<string, string> = {
  navigate: "导航",
  click: "点击",
  fill: "填写",
  hover: "悬停",
  scroll: "滚动",
  screenshot: "截图",
  take_snapshot: "快照",
  get_content: "获取内容",
  evaluate: "执行脚本",
  select_page: "切换标签",
  new_page: "新建标签",
  close_page: "关闭标签",
  type_text: "输入",
  press_key: "按键",
};

export function AgentOperationLog() {
  const operations = useBrowserStore((s) => s.agentOperations);
  const clearAgentOperations = useBrowserStore((s) => s.clearAgentOperations);
  const [expanded, setExpanded] = useState(false);

  const toggle = useCallback(() => setExpanded((v) => !v), []);
  const handleClear = useCallback(() => clearAgentOperations(), [clearAgentOperations]);

  if (operations.length === 0) return null;

  const visible = expanded ? operations : [];

  return (
    <div
      style={{
        flexShrink: 0,
        borderTop: "1px solid var(--border-shell-subtle)",
        background: "var(--bg-secondary)",
        fontSize: 11,
      }}
    >
      <button
        type="button"
        onClick={toggle}
        style={{
          display: "flex",
          width: "100%",
          alignItems: "center",
          gap: 6,
          padding: "6px 10px",
          border: "none",
          background: "transparent",
          cursor: "pointer",
          color: "var(--fill-secondary)",
        }}
      >
        <Robot size={14} weight="duotone" />
        <span style={{ flex: 1, textAlign: "left" }}>
          Agent 操作 ({operations.length})
        </span>
        {expanded ? <CaretDown size={12} /> : <CaretUp size={12} />}
      </button>

      {expanded && (
        <div
          style={{
            maxHeight: 160,
            overflowY: "auto",
            padding: "0 10px 8px",
            display: "flex",
            flexDirection: "column",
            gap: 4,
          }}
        >
          {visible.map((op) => (
          <div
            key={op.id}
            style={{
              display: "flex",
              gap: 8,
              alignItems: "baseline",
              padding: "3px 6px",
              borderRadius: 4,
              background: "var(--bg-hover)",
              color: "var(--fill-primary)",
            }}
          >
            <span
              style={{
                flexShrink: 0,
                color: "var(--fill-quaternary)",
                fontVariantNumeric: "tabular-nums",
              }}
            >
              {formatTime(op.ts)}
            </span>
            <span style={{ flexShrink: 0, color: "var(--tint, #4299E1)" }}>
              {ACTION_LABELS[op.action] ?? op.action}
            </span>
            <span
              style={{
                flex: 1,
                minWidth: 0,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                color: "var(--fill-secondary)",
              }}
              title={op.description}
            >
              {op.description}
            </span>
          </div>
        ))}
        </div>
      )}

      {expanded && (
        <div style={{ display: "flex", justifyContent: "flex-end", padding: "0 10px 8px" }}>
          <button
            type="button"
            onClick={handleClear}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 4,
              padding: "2px 6px",
              border: "none",
              borderRadius: 4,
              background: "var(--bg-hover)",
              cursor: "pointer",
              color: "var(--fill-tertiary)",
              fontSize: 10,
            }}
          >
            <Trash size={10} />
            清空
          </button>
        </div>
      )}
    </div>
  );
}
