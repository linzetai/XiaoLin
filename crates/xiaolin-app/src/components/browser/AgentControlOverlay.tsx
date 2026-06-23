import { Robot, HandGrabbing, Play } from "@phosphor-icons/react";
import { useBrowserStore, browserRequestTakeover, browserClearUserTakeover } from "../../lib/stores/browser-store";

interface AgentControlOverlayProps {
  pageId: string;
}

export function AgentControlOverlay({ pageId }: AgentControlOverlayProps) {
  const agentControlled = useBrowserStore((s) => s.pages[pageId]?.agentControlled ?? false);
  const userTakeoverActive = useBrowserStore((s) => s.userTakeoverActive);
  const userActionToast = useBrowserStore((s) => s.userActionToast);
  const clearUserActionToast = useBrowserStore((s) => s.clearUserActionToast);

  if (!agentControlled && !userTakeoverActive && !userActionToast) return null;

  return (
    <>
      {agentControlled && (
        <div
          aria-hidden
          style={{
            position: "absolute",
            inset: 0,
            background: "rgba(88, 166, 255, 0.08)",
            pointerEvents: "none",
            zIndex: 2,
          }}
        />
      )}
      {userActionToast && (
        <div
          style={{
            position: "absolute",
            bottom: 12,
            left: "50%",
            transform: "translateX(-50%)",
            zIndex: 4,
            padding: "8px 14px",
            borderRadius: 8,
            background: "var(--bg-card)",
            border: "1px solid var(--border-shell-subtle)",
            boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
            fontSize: 12,
            color: "var(--fill-secondary)",
            display: "flex",
            alignItems: "center",
            gap: 8,
            maxWidth: "90%",
          }}
        >
          <span style={{ flex: 1 }}>{userActionToast}</span>
          {agentControlled && (
            <button
              type="button"
              onClick={() => {
                clearUserActionToast();
                void browserRequestTakeover(pageId);
              }}
              style={{
                border: "none",
                background: "var(--tint)",
                color: "#fff",
                cursor: "pointer",
                fontSize: 11,
                padding: "3px 8px",
                borderRadius: 5,
                fontWeight: 500,
                whiteSpace: "nowrap",
              }}
            >
              中止 Agent
            </button>
          )}
          <button
            type="button"
            onClick={clearUserActionToast}
            style={{
              border: "none",
              background: "transparent",
              cursor: "pointer",
              color: "var(--fill-quaternary)",
              fontSize: 11,
            }}
          >
            Dismiss
          </button>
        </div>
      )}
      {agentControlled && (
        <div
          style={{
            position: "absolute",
            top: 8,
            right: 8,
            zIndex: 3,
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "6px 10px",
            borderRadius: 8,
            background: "var(--bg-card)",
            border: "1px solid var(--border-shell-subtle)",
            boxShadow: "0 2px 8px rgba(0,0,0,0.12)",
            fontSize: 12,
          }}
        >
          <Robot size={16} weight="fill" style={{ color: "var(--tint)" }} />
          <span style={{ color: "var(--fill-secondary)" }}>Agent operating…</span>
          <button
            type="button"
            onClick={() => void browserRequestTakeover(pageId)}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 4,
              padding: "3px 8px",
              borderRadius: 5,
              border: "none",
              background: "var(--tint)",
              color: "#fff",
              cursor: "pointer",
              fontSize: 11,
              fontWeight: 500,
            }}
          >
            <HandGrabbing size={12} />
            Take control
          </button>
        </div>
      )}
      {userTakeoverActive && !agentControlled && (
        <div
          style={{
            position: "absolute",
            top: 8,
            right: 8,
            zIndex: 3,
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "6px 10px",
            borderRadius: 8,
            background: "var(--bg-card)",
            border: "1px solid var(--border-shell-subtle)",
            boxShadow: "0 2px 8px rgba(0,0,0,0.12)",
            fontSize: 12,
          }}
        >
          <HandGrabbing size={16} weight="fill" style={{ color: "var(--fill-secondary)" }} />
          <span style={{ color: "var(--fill-secondary)" }}>You are in control</span>
          <button
            type="button"
            onClick={() => void browserClearUserTakeover()}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 4,
              padding: "3px 8px",
              borderRadius: 5,
              border: "none",
              background: "var(--tint)",
              color: "#fff",
              cursor: "pointer",
              fontSize: 11,
              fontWeight: 500,
            }}
          >
            <Play size={12} weight="fill" />
            Resume Agent
          </button>
        </div>
      )}
    </>
  );
}
