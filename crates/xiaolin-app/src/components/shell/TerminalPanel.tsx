import { useEffect, useRef, useMemo, type CSSProperties, type ReactNode } from "react";
import { Terminal, Trash, X as XIcon } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { useTerminalStore, useChatMetaStore, type TerminalSession } from "../../lib/stores";
import { ICON_SIZE } from "../../lib/ui-tokens";

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
  fontFamily: "var(--font-mono)",
  fontSize: 12,
  lineHeight: 1.5,
  whiteSpace: "pre-wrap",
  wordBreak: "break-all",
  background: "var(--bg-primary)",
  color: "var(--fill-primary)",
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
    <div style={outputStyle} className="terminal-output">
      {session.command && (
        <div style={{ color: "var(--tint)", marginBottom: 4, opacity: 0.8 }}>
          $ {session.command}
        </div>
      )}
      <AnsiText text={content} />
      {session.status === "running" && (
        <span style={{ opacity: 0.5 }} className="terminal-cursor">▌</span>
      )}
      <div ref={bottomRef} />
    </div>
  );
}

const ANSI_COLOR_CLASSES: Record<string, string> = {
  "31": "ansi-red",
  "32": "ansi-green",
  "33": "ansi-yellow",
  "34": "ansi-blue",
};

function renderAnsiText(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const pattern = /\x1b\[(\d+)m|\x1b\[0m/g;
  let lastIndex = 0;
  let colorClass: string | undefined;
  let match: RegExpExecArray | null;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > lastIndex) {
      const chunk = text.slice(lastIndex, match.index);
      nodes.push(
        colorClass
          ? <span key={`${lastIndex}-text`} className={colorClass}>{chunk}</span>
          : chunk,
      );
    }

    const code = match[1];
    if (code) {
      colorClass = ANSI_COLOR_CLASSES[code];
    } else {
      colorClass = undefined;
    }
    lastIndex = pattern.lastIndex;
  }

  if (lastIndex < text.length) {
    const chunk = text.slice(lastIndex);
    nodes.push(
      colorClass
        ? <span key={`${lastIndex}-tail`} className={colorClass}>{chunk}</span>
        : chunk,
    );
  }

  return nodes;
}

function AnsiText({ text }: { text: string }) {
  const nodes = useMemo(() => renderAnsiText(text), [text]);
  return <>{nodes}</>;
}

export function TerminalPanel() {
  const { t } = useTranslation("sidebar");
  const sessions = useTerminalStore((s) => s.sessions);
  const activeCallId = useTerminalStore((s) => s.activeCallId);
  const setActive = useTerminalStore((s) => s.setActive);
  const clear = useTerminalStore((s) => s.clear);
  const activeChatId = useChatMetaStore((s) => s.activeChatId);

  const sessionList = useMemo(
    () => Object.values(sessions)
      .filter((s) => !s.chatId || s.chatId === activeChatId)
      .reverse(),
    [sessions, activeChatId],
  );

  const activeSession = activeCallId ? sessions[activeCallId] : sessionList[0];

  if (sessionList.length === 0) {
    return (
      <div style={emptyStyle}>
        <Terminal size={ICON_SIZE.xl} weight="light" />
        <span>{t("noOutputYet")}</span>
        <span style={{ fontSize: 11, opacity: 0.6 }}>
          {t("outputHint")}
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
                <span
                  role="button"
                  tabIndex={0}
                  style={{
                    marginLeft: 2,
                    opacity: 0.4,
                    cursor: "pointer",
                    display: "inline-flex",
                    alignItems: "center",
                    borderRadius: 3,
                    padding: 1,
                  }}
                  title={t("close", { ns: "common" })}
                  onClick={(e) => { e.stopPropagation(); clear(s.callId); }}
                  onKeyDown={(e) => { if (e.key === "Enter") { e.stopPropagation(); clear(s.callId); } }}
                  onMouseEnter={(e) => { e.currentTarget.style.opacity = "1"; e.currentTarget.style.background = "var(--bg-hover)"; }}
                  onMouseLeave={(e) => { e.currentTarget.style.opacity = "0.4"; e.currentTarget.style.background = "transparent"; }}
                >
                  <XIcon size={10} />
                </span>
              </button>
            );
          })}
        </div>
      )}

      {activeSession && <SessionOutput session={activeSession} />}

      {activeSession && (
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
            <Trash />
            {activeSession.status === "done" ? t("clear") : t("close", { ns: "common" })}
          </button>
        </div>
      )}
    </div>
  );
}
