import i18n from "../../i18n";
import { useState, useEffect, useCallback, type MouseEvent as RME } from "react";
import { useGatewayStore } from "../../lib/store";
// Agent store import removed — single-agent mode
import { NotificationCenter } from "../notification/NotificationCenter";
import { NotificationDetailPanel } from "../notification/NotificationDetailPanel";
import { Minus, Square, ArrowsOut, X } from "@phosphor-icons/react";
import type { AppNotification } from "../../lib/transport";

const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

async function onDragMouseDown(e: RME) {
  if (!isTauri || e.button !== 0) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  getCurrentWindow().startDragging();
}

async function onDragDoubleClick() {
  if (!isTauri) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  getCurrentWindow().toggleMaximize();
}

function WindowControls() {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    let unlistenFn: (() => void) | undefined;
    (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      setIsMaximized(await win.isMaximized());
      unlistenFn = await win.onResized(async () => {
        if (!cancelled) setIsMaximized(await win.isMaximized());
      });
    })();
    return () => {
      cancelled = true;
      unlistenFn?.();
    };
  }, []);

  const minimize = useCallback(async () => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().minimize();
  }, []);

  const toggleMaximize = useCallback(async () => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().toggleMaximize();
  }, []);

  const close = useCallback(async () => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().close();
  }, []);

  if (!isTauri) return null;

  const btn = "flex items-center justify-center transition-all duration-150";
  const iconProps = { weight: "light" as const };

  return (
    <div className="ml-1 flex h-full items-stretch">
      <div className="my-auto h-3.5 w-px" style={{ background: "var(--separator)" }} />
      <button
        onClick={minimize}
        className={`${btn} w-[36px] hover:bg-[var(--bg-hover)] active:scale-95`}
        style={{ color: "var(--fill-quaternary)" }}
        title={i18n.t("common:minimize")}
      >
        <Minus {...iconProps} />
      </button>
      <button
        onClick={toggleMaximize}
        className={`${btn} w-[36px] hover:bg-[var(--bg-hover)] active:scale-95`}
        style={{ color: "var(--fill-quaternary)" }}
        title={isMaximized ? i18n.t("common:restore") : i18n.t("common:maximize")}
      >
        {isMaximized ? <ArrowsOut {...iconProps} /> : <Square {...iconProps} />}
      </button>
      <button
        onClick={close}
        className={`${btn} w-[36px] hover:bg-[#E81123] hover:text-white active:scale-95`}
        style={{ color: "var(--fill-quaternary)", transition: "background 0.15s, color 0.15s, transform 0.1s" }}
        title={i18n.t("common:close")}
      >
        <X {...iconProps} />
      </button>
    </div>
  );
}

function ConnectionDot() {
  const connected = useGatewayStore((s) => s.connected);
  return (
    <div
      className="flex h-7 w-7 items-center justify-center"
      title={connected ? i18n.t("common:titleBarConnected") : i18n.t("common:titleBarDisconnected")}
    >
      <span className="relative inline-flex items-center justify-center">
        <span
          className="inline-block h-[7px] w-[7px] rounded-full transition-colors duration-150"
          style={{ background: connected ? "var(--green)" : "var(--red)" }}
        />
        {connected && (
          <span
            className="absolute inline-block h-[7px] w-[7px] rounded-full"
            style={{
              background: "var(--green)",
              animation: "pulse-ring 2s ease-out infinite",
            }}
          />
        )}
        {!connected && (
          <span
            className="absolute inline-block h-[7px] w-[7px] rounded-full"
            style={{
              background: "var(--red)",
              animation: "shake 0.5s ease-in-out",
            }}
          />
        )}
      </span>
    </div>
  );
}

function AgentLabel() {
  return (
    <div className="pointer-events-none flex items-center gap-2 pl-4" data-tauri-drag-region="">
      <span className="text-[12px] font-medium" style={{ color: "var(--fill-secondary)" }}>
        {i18n.t("common:appName")}
      </span>
    </div>
  );
}

export function TitleBar() {
  const [detailNotification, setDetailNotification] = useState<AppNotification | null>(null);

  return (
    <>
      {detailNotification && (
        <NotificationDetailPanel
          notification={detailNotification}
          onClose={() => setDetailNotification(null)}
        />
      )}
      <header
        className="relative z-30 flex shrink-0 select-none items-stretch"
        style={{
          height: "var(--titlebar-h)",
          background: "var(--bg-sidebar)",
        }}
      >
        <div
          className="absolute inset-x-0 bottom-0 pointer-events-none h-px"
          style={{ background: "linear-gradient(90deg, transparent 5%, var(--separator) 50%, transparent 95%)" }}
        />
        <AgentLabel />
        <div
          className="h-full flex-1"
          data-tauri-drag-region=""
          onMouseDown={onDragMouseDown}
          onDoubleClick={onDragDoubleClick}
          style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        />

        <div className="flex h-full items-center gap-0.5">
          <ConnectionDot />
          <NotificationCenter onDetailOpen={setDetailNotification} />
          <WindowControls />
        </div>
      </header>
    </>
  );
}
