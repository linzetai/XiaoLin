import { useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { CaretDown, CaretUp, Robot, Trash } from "@phosphor-icons/react";
import { useBrowserStore } from "../../lib/stores/browser-store";
import type { TFunction } from "i18next";

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

const ACTION_LABEL_KEYS: Record<string, string> = {
  navigate: "actionNavigate",
  click: "actionClick",
  fill: "actionFill",
  hover: "actionHover",
  scroll: "actionScroll",
  screenshot: "actionScreenshot",
  take_snapshot: "actionSnapshot",
  get_content: "actionGetContent",
  evaluate: "actionEvaluate",
  select_page: "actionSelectPage",
  new_page: "actionNewPage",
  close_page: "actionClosePage",
  type_text: "actionTypeText",
  press_key: "actionPressKey",
};

function getActionLabel(t: TFunction<"browser">, action: string): string {
  const key = ACTION_LABEL_KEYS[action];
  return key ? t(key) : action;
}

export function AgentOperationLog() {
  const { t } = useTranslation("browser");
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
          {t("agentOps", { count: operations.length })}
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
              {getActionLabel(t, op.action)}
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
            {t("clearOps")}
          </button>
        </div>
      )}
    </div>
  );
}
