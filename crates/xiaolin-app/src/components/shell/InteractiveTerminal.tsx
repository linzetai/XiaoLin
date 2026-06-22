import { useEffect, useRef, useState, useCallback } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { useTranslation } from "react-i18next";
import "@xterm/xterm/css/xterm.css";

import { useGatewayStore } from "../../lib/store";
import { usePtyStore, useChatMetaStore } from "../../lib/stores";

interface InteractiveTerminalProps {
  sessionId: string;
}

const MAX_RECONNECT_ATTEMPTS = 5;
const BASE_DELAY_MS = 1000;

type ConnState = "connecting" | "connected" | "reconnecting" | "closed";

function getTerminalTheme() {
  const style = getComputedStyle(document.documentElement);
  const getCssVar = (name: string, fallback: string) =>
    style.getPropertyValue(name).trim() || fallback;

  const bgPrimary = getCssVar("--bg-primary", "#1a1a1a");
  const fgPrimary = getCssVar("--fill-primary", "#e6edf3");
  const accent = getCssVar("--fill-accent", getCssVar("--tint", "#58a6ff"));

  const hex = bgPrimary.replace("#", "");
  let isDark = true;
  if (hex.length >= 6) {
    const r = parseInt(hex.slice(0, 2), 16);
    const g = parseInt(hex.slice(2, 4), 16);
    const b = parseInt(hex.slice(4, 6), 16);
    isDark = (r * 299 + g * 587 + b * 114) / 1000 < 128;
  }

  return {
    background: bgPrimary,
    foreground: fgPrimary,
    cursor: accent,
    selectionBackground: isDark ? "#264f78" : "#b4d5fe",
    black: isDark ? "#484f58" : "#24292e",
    red: isDark ? "#ff7b72" : "#d73a49",
    green: isDark ? "#3fb950" : "#22863a",
    yellow: isDark ? "#d29922" : "#b08800",
    blue: isDark ? "#58a6ff" : "#0366d6",
    magenta: isDark ? "#bc8cff" : "#6f42c1",
    cyan: isDark ? "#39c5cf" : "#1b7c83",
    white: isDark ? "#b1bac4" : "#6a737d",
  };
}

export function InteractiveTerminal({ sessionId }: InteractiveTerminalProps) {
  const { t } = useTranslation("sidebar");
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [connState, setConnState] = useState<ConnState>("connecting");
  const [bgColor, setBgColor] = useState(() => getTerminalTheme().background);

  const info = useGatewayStore((s) => s.info);

  const applyTheme = useCallback(() => {
    const theme = getTerminalTheme();
    setBgColor(theme.background);
    if (termRef.current) {
      termRef.current.options.theme = theme;
      const viewport = containerRef.current?.querySelector<HTMLElement>(".xterm-viewport");
      if (viewport) viewport.style.backgroundColor = theme.background;
    }
  }, []);

  // React to theme changes via attribute/class mutations on <html>
  useEffect(() => {
    const observer = new MutationObserver(() => {
      requestAnimationFrame(applyTheme);
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "data-theme", "style"],
    });
    return () => observer.disconnect();
  }, [applyTheme]);

  useEffect(() => {
    if (!containerRef.current || !info) return;

    const theme = getTerminalTheme();
    setBgColor(theme.background);

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: "var(--font-mono)",
      theme,
      scrollback: 5000,
      allowProposedApi: true,
    });

    const fit = new FitAddon();
    term.loadAddon(fit);
    fitRef.current = fit;

    term.open(containerRef.current);

    try {
      const webgl = new WebglAddon();
      term.loadAddon(webgl);
    } catch {
      // WebGL not available, fall back to canvas
    }

    // Override xterm.css hardcoded viewport background
    const viewport = containerRef.current.querySelector<HTMLElement>(".xterm-viewport");
    if (viewport) viewport.style.backgroundColor = theme.background;

    fit.fit();
    termRef.current = term;

    let disposed = false;
    let reconnectAttempt = 0;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    function buildWsUrl() {
      const httpUrl = info!.httpUrl;
      const wsProtocol = httpUrl.startsWith("https") ? "wss" : "ws";
      const host = httpUrl.replace(/^https?:\/\//, "");
      const activeChatId = useChatMetaStore.getState().activeChatId;
      const chatMeta = useChatMetaStore.getState().chats[activeChatId];
      const cwd = chatMeta?.workDir || "";
      const params = new URLSearchParams({ cols: String(term.cols), rows: String(term.rows) });
      if (cwd) params.set("cwd", cwd);
      return `${wsProtocol}://${host}/api/v1/pty?${params.toString()}`;
    }

    function connect() {
      if (disposed) return;

      const url = buildWsUrl();
      const ws = new WebSocket(url);
      ws.binaryType = "arraybuffer";
      wsRef.current = ws;

      ws.onopen = () => {
        if (disposed) return;
        reconnectAttempt = 0;
        setConnState("connected");
        usePtyStore.getState().updateSession(sessionId, { status: "connecting" });
      };

      ws.onmessage = (event) => {
        if (disposed) return;
        if (event.data instanceof ArrayBuffer) {
          term.write(new Uint8Array(event.data));
        } else if (typeof event.data === "string") {
          try {
            const msg = JSON.parse(event.data);
            if (msg.type === "session_created") {
              const patch: Record<string, unknown> = { status: "connected" };
              if (msg.cwd) patch.cwd = msg.cwd;
              usePtyStore.getState().updateSession(sessionId, patch);
            } else if (msg.type === "cwd_changed") {
              if (msg.cwd) {
                usePtyStore.getState().updateSession(sessionId, { cwd: msg.cwd });
              }
            } else if (msg.type === "session_closed") {
              usePtyStore.getState().updateSession(sessionId, { status: "closed", exitCode: msg.exit_code });
              setConnState("closed");
            } else if (msg.type === "error") {
              console.error("[PTY] Error:", msg.error);
              usePtyStore.getState().updateSession(sessionId, { status: "closed" });
              setConnState("closed");
            }
          } catch {
            // ignore non-JSON text
          }
        }
      };

      ws.onclose = (event) => {
        if (disposed) return;
        wsRef.current = null;

        if (event.code === 1000 || reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
          setConnState("closed");
          usePtyStore.getState().updateSession(sessionId, { status: "closed" });
          return;
        }

        reconnectAttempt++;
        const delay = BASE_DELAY_MS * Math.pow(2, reconnectAttempt - 1);
        setConnState("reconnecting");
        term.write(`\r\n\x1b[33m[Connection lost, reconnecting in ${Math.round(delay / 1000)}s...]\x1b[0m\r\n`);
        reconnectTimer = setTimeout(connect, delay);
      };

      ws.onerror = () => {
        // onclose will fire after onerror — handling is done there
      };
    }

    // OSC 7: shell reports current working directory
    term.parser.registerOscHandler(7, (data) => {
      if (disposed) return true;
      let path = data;
      try {
        const url = new URL(data);
        path = decodeURIComponent(url.pathname);
      } catch {
        if (data.startsWith("file://")) {
          const slashIdx = data.indexOf("/", 7);
          path = slashIdx >= 0 ? decodeURIComponent(data.slice(slashIdx)) : data;
        }
      }
      if (path && path.startsWith("/")) {
        usePtyStore.getState().updateSession(sessionId, { cwd: path });
      }
      return true;
    });

    term.onData((data) => {
      const ws = wsRef.current;
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(new TextEncoder().encode(data));
      }
    });

    term.onBinary((data) => {
      const ws = wsRef.current;
      if (ws && ws.readyState === WebSocket.OPEN) {
        const buf = new Uint8Array(data.length);
        for (let i = 0; i < data.length; i++) {
          buf[i] = data.charCodeAt(i);
        }
        ws.send(buf);
      }
    });

    connect();

    return () => {
      disposed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      wsRef.current?.close(1000);
      wsRef.current = null;
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [info, sessionId]);

  // ResizeObserver for dynamic terminal sizing
  useEffect(() => {
    if (!containerRef.current) return;
    const container = containerRef.current;

    const observer = new ResizeObserver(() => {
      fitRef.current?.fit();
      const term = termRef.current;
      const ws = wsRef.current;
      if (term && ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
      }
    });

    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  return (
    <div style={{ width: "100%", height: "100%", position: "relative" }}>
      <div
        ref={containerRef}
        style={{
          width: "100%",
          height: "100%",
          background: bgColor,
          padding: 4,
        }}
      />
      {connState !== "connected" && (
        <div
          style={{
            position: "absolute",
            top: 4,
            right: 8,
            fontSize: 10,
            padding: "2px 6px",
            borderRadius: 3,
            background: connState === "reconnecting" ? "#d2992233" : connState === "closed" ? "#ff7b7233" : "#58a6ff33",
            color: connState === "reconnecting" ? "#d29922" : connState === "closed" ? "#ff7b72" : "#58a6ff",
            pointerEvents: "none",
          }}
        >
          {connState === "connecting" && t("connecting")}
          {connState === "reconnecting" && t("reconnecting")}
          {connState === "closed" && t("disconnected")}
        </div>
      )}
    </div>
  );
}
