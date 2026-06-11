import { useState, useCallback, useRef, useEffect, type CSSProperties } from "react";
import { Plus, X, TerminalSquare } from "lucide-react";
import { TerminalPanel } from "./TerminalPanel";
import { InteractiveTerminal } from "./InteractiveTerminal";
import { usePtyStore } from "../../lib/stores";

function shortenPath(cwd: string | undefined): string {
  if (!cwd) return "";
  const parts = cwd.split("/").filter(Boolean);
  // /home/user/... → ~/lastDir
  if (parts.length >= 2 && parts[0] === "home") {
    const afterUser = parts.slice(2);
    if (afterUser.length === 0) return "~";
    return "~/" + afterUser[afterUser.length - 1];
  }
  return parts[parts.length - 1] || "/";
}

type SubView = "output" | "shell";

const subTabBarStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 2,
  padding: "4px 8px",
  borderBottom: "1px solid var(--border-shell-subtle)",
  flexShrink: 0,
};

const subTabBtnStyle: CSSProperties = {
  fontSize: 11,
  padding: "3px 8px",
  borderRadius: 4,
  border: "none",
  cursor: "pointer",
  fontWeight: 500,
  transition: "all 0.1s",
};

const sessionTabStyle: CSSProperties = {
  fontSize: 10,
  padding: "2px 6px",
  borderRadius: 3,
  border: "none",
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  gap: 3,
  transition: "all 0.1s",
};

export function TerminalTabContent() {
  const [subView, setSubView] = useState<SubView>("output");
  const [editingId, setEditingId] = useState<string | null>(null);
  const editInputRef = useRef<HTMLInputElement>(null);
  const sessions = usePtyStore((s) => s.sessions);
  const activeSessionId = usePtyStore((s) => s.activeSessionId);
  const addSession = usePtyStore((s) => s.addSession);
  const removeSession = usePtyStore((s) => s.removeSession);
  const setActiveSession = usePtyStore((s) => s.setActiveSession);
  const updateSession = usePtyStore((s) => s.updateSession);

  const createNewSession = useCallback(() => {
    const tempId = `pty-${Date.now()}`;
    addSession({ id: tempId, status: "connecting" });
    setSubView("shell");
  }, [addSession]);

  const handleRename = useCallback((id: string, newName: string) => {
    const trimmed = newName.trim();
    if (trimmed) updateSession(id, { name: trimmed });
    setEditingId(null);
  }, [updateSession]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (!e.ctrlKey || !e.shiftKey) return;

      if (e.key === "`") {
        e.preventDefault();
        createNewSession();
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        const currentSessions = usePtyStore.getState().sessions;
        const currentActive = usePtyStore.getState().activeSessionId;
        if (currentSessions.length < 2 || !currentActive) return;
        const idx = currentSessions.findIndex((s) => s.id === currentActive);
        const next = e.key === "ArrowRight"
          ? currentSessions[(idx + 1) % currentSessions.length]
          : currentSessions[(idx - 1 + currentSessions.length) % currentSessions.length];
        setActiveSession(next.id);
        setSubView("shell");
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [createNewSession, setActiveSession]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div style={subTabBarStyle}>
        <button
          type="button"
          style={{
            ...subTabBtnStyle,
            color: subView === "output" ? "var(--fill-primary)" : "var(--fill-quaternary)",
            background: subView === "output" ? "var(--bg-hover)" : "transparent",
          }}
          onClick={() => setSubView("output")}
        >
          Output
        </button>
        <button
          type="button"
          style={{
            ...subTabBtnStyle,
            color: subView === "shell" ? "var(--fill-primary)" : "var(--fill-quaternary)",
            background: subView === "shell" ? "var(--bg-hover)" : "transparent",
          }}
          onClick={() => {
            setSubView("shell");
            if (sessions.length === 0) {
              createNewSession();
            }
          }}
        >
          Shell
        </button>

        {subView === "shell" && (
          <>
            <div style={{ width: 1, height: 14, background: "var(--border-shell-subtle)", margin: "0 4px" }} />
            {sessions.map((sess) => (
              <button
                key={sess.id}
                type="button"
                style={{
                  ...sessionTabStyle,
                  color: sess.id === activeSessionId ? "var(--fill-primary)" : "var(--fill-quaternary)",
                  background: sess.id === activeSessionId ? "var(--bg-hover)" : "transparent",
                }}
                title={sess.cwd ?? undefined}
                onClick={() => setActiveSession(sess.id)}
                onDoubleClick={() => {
                  setEditingId(sess.id);
                  setTimeout(() => editInputRef.current?.select(), 0);
                }}
              >
                <TerminalSquare size={10} strokeWidth={1.5} />
                {editingId === sess.id ? (
                  <input
                    ref={editInputRef}
                    defaultValue={sess.name ?? "sh"}
                    style={{
                      width: 60,
                      fontSize: 10,
                      padding: "0 2px",
                      border: "1px solid var(--border-shell-subtle)",
                      borderRadius: 2,
                      background: "var(--bg-primary)",
                      color: "inherit",
                      outline: "none",
                    }}
                    onClick={(e) => e.stopPropagation()}
                    onBlur={(e) => handleRename(sess.id, e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleRename(sess.id, e.currentTarget.value);
                      if (e.key === "Escape") setEditingId(null);
                    }}
                  />
                ) : (
                  <span style={{ maxWidth: 120, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {sess.status === "closed"
                      ? "[exited]"
                      : sess.cwd
                        ? `${sess.name ?? "sh"}: ${shortenPath(sess.cwd)}`
                        : sess.name ?? "sh"}
                  </span>
                )}
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    marginLeft: 2,
                    opacity: 0.6,
                  }}
                  onClick={(e) => {
                    e.stopPropagation();
                    removeSession(sess.id);
                  }}
                >
                  <X size={9} strokeWidth={2} />
                </span>
              </button>
            ))}
            <button
              type="button"
              style={{
                ...sessionTabStyle,
                color: "var(--fill-quaternary)",
                background: "transparent",
              }}
              onClick={createNewSession}
              title="New terminal"
            >
              <Plus size={10} strokeWidth={2} />
            </button>
          </>
        )}
      </div>

      <div style={{ flex: 1, minHeight: 0, overflow: "hidden", position: "relative" }}>
        {subView === "output" ? (
          <TerminalPanel />
        ) : sessions.length > 0 ? (
          sessions.map((sess) => (
            <div
              key={sess.id}
              style={{
                position: "absolute",
                inset: 0,
                visibility: sess.id === activeSessionId ? "visible" : "hidden",
              }}
            >
              <InteractiveTerminal sessionId={sess.id} />
            </div>
          ))
        ) : (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              height: "100%",
              color: "var(--fill-quaternary)",
              fontSize: 12,
            }}
          >
            No active terminal session
          </div>
        )}
      </div>
    </div>
  );
}
