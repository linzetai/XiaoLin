import { useEffect, useRef, useMemo, type CSSProperties } from "react";
import { Terminal, Trash2 } from "lucide-react";
import { useTerminalStore, type TerminalSession } from "../../lib/stores";
import { ICON } from "../../lib/ui-tokens";

const containerStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  height: "100%",
  minHeight: 0,
};

const sessionListStyle: CSSProperties = {
  display: "flex",
  gap: 4,
  padding: "6px 10px",
  overflowX: "auto",
  borderBottom: "1px solid var(--border-shell-subtle)",
  flexShrink: 0,
};

const sessionBtnStyle: CSSProperties = {
  fontSize: 11,
  padding: "3px 8px",
  borderRadius: 4,
  border: "none",
  cursor: "pointer",
  whiteSpace: "nowrap",
  transition: "all 0.1s",
  display: "flex",
  alignItems: "center",
  gap: 4,
};

const outputStyle: CSSProperties = {
  flex: 1,
  overflowY: "auto",
  padding: "8px 12px",
  fontFamily: "var(--font-mono, 'JetBrains Mono', 'Fira Code', monospace)",
  fontSize: 12,
  lineHeight: 1.5,
  whiteSpace: "pre-wrap",
  wordBreak: "break-all",
  background: "var(--bg-shell-deep, #0d1117)",
  color: "var(--fill-terminal, #e6edf3)",
  minHeight: 0,
};

const emptyStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  justifyContent: "center",
  height: "100%",
  gap: 8,
  color: "var(--fill-quaternary)",
  fontSize: 12,
};

function SessionOutput({ session }: { session: TerminalSession }) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [session.lines.length]);

  const content = useMemo(
    () => session.lines.map((l) => l.text).join(""),
    [session.lines],
  );

  return (
    <div style={outputStyle}>
      {session.command && (
        <div style={{ color: "var(--tint)", marginBottom: 4, opacity: 0.8 }}>
          $ {session.command}
        </div>
      )}
      <div dangerouslySetInnerHTML={{ __html: ansiToHtml(content) }} />
      {session.status === "running" && (
        <span style={{ opacity: 0.5 }} className="terminal-cursor">▌</span>
      )}
      <div ref={bottomRef} />
    </div>
  );
}

function ansiToHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\x1b\[31m(.*?)\x1b\[0m/g, '<span style="color:#f85149">$1</span>')
    .replace(/\x1b\[32m(.*?)\x1b\[0m/g, '<span style="color:#56d364">$1</span>')
    .replace(/\x1b\[33m(.*?)\x1b\[0m/g, '<span style="color:#e3b341">$1</span>')
    .replace(/\x1b\[34m(.*?)\x1b\[0m/g, '<span style="color:#79c0ff">$1</span>')
    .replace(/\x1b\[\d+m/g, "");
}

export function TerminalPanel() {
  const sessions = useTerminalStore((s) => s.sessions);
  const activeCallId = useTerminalStore((s) => s.activeCallId);
  const setActive = useTerminalStore((s) => s.setActive);
  const clear = useTerminalStore((s) => s.clear);

  const sessionList = useMemo(
    () => Object.values(sessions).reverse(),
    [sessions],
  );

  const activeSession = activeCallId ? sessions[activeCallId] : sessionList[0];

  if (sessionList.length === 0) {
    return (
      <div style={emptyStyle}>
        <Terminal size={24} strokeWidth={1.2} />
        <span>No terminal output yet</span>
        <span style={{ fontSize: 11, opacity: 0.6 }}>
          Output will appear when shell commands execute
        </span>
      </div>
    );
  }

  return (
    <div style={containerStyle}>
      {sessionList.length > 1 && (
        <div style={sessionListStyle}>
          {sessionList.map((s) => {
            const active = s.callId === activeSession?.callId;
            const label = s.command
              ? s.command.length > 20 ? s.command.slice(0, 20) + "…" : s.command
              : s.callId.slice(0, 8);
            return (
              <button
                key={s.callId}
                type="button"
                style={{
                  ...sessionBtnStyle,
                  background: active ? "var(--bg-hover)" : "transparent",
                  color: active ? "var(--fill-primary)" : "var(--fill-tertiary)",
                }}
                onClick={() => setActive(s.callId)}
              >
                <span
                  style={{
                    width: 6,
                    height: 6,
                    borderRadius: "50%",
                    background: s.status === "running" ? "#56d364" : "var(--fill-quaternary)",
                    flexShrink: 0,
                  }}
                />
                {label}
              </button>
            );
          })}
        </div>
      )}

      {activeSession && <SessionOutput session={activeSession} />}

      {activeSession && activeSession.status === "done" && (
        <div style={{ padding: "4px 10px", borderTop: "1px solid var(--border-shell-subtle)", display: "flex", justifyContent: "flex-end" }}>
          <button
            type="button"
            style={{
              border: "none",
              background: "transparent",
              cursor: "pointer",
              color: "var(--fill-quaternary)",
              padding: "2px 6px",
              borderRadius: 4,
              fontSize: 11,
              display: "flex",
              alignItems: "center",
              gap: 4,
            }}
            onClick={() => clear(activeSession.callId)}
            onMouseEnter={(e) => { e.currentTarget.style.color = "var(--fill-secondary)"; }}
            onMouseLeave={(e) => { e.currentTarget.style.color = "var(--fill-quaternary)"; }}
          >
            <Trash2 {...ICON.sm} />
            Clear
          </button>
        </div>
      )}
    </div>
  );
}
