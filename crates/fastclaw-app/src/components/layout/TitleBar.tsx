import { useState, useEffect, useCallback, type MouseEvent as RME } from "react";
import { useThemeStore, type ThemeMode } from "../../lib/theme";
import { useGatewayStore } from "../../lib/store";
import { SettingsPanel } from "../settings/SettingsPanel";
import { Sun, Moon, Monitor, Settings, Minus, Square, Maximize2, X } from "lucide-react";
import { ClawIcon } from "./ClawIcon";

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

function ThemeToggle() {
  const { mode, setMode } = useThemeStore();
  const next = () => {
    const o: ThemeMode[] = ["light", "dark", "system"];
    setMode(o[(o.indexOf(mode) + 1) % o.length]);
  };

  return (
    <button
      onClick={next}
      className="flex h-full w-11 items-center justify-center transition-colors duration-100 hover:bg-[var(--bg-hover)]"
      style={{ color: "var(--fill-tertiary)" }}
      title={mode === "light" ? "浅色" : mode === "dark" ? "深色" : "自动"}
    >
      {mode === "light" ? (
        <Sun size={14} strokeWidth={1.5} />
      ) : mode === "dark" ? (
        <Moon size={14} strokeWidth={1.5} />
      ) : (
        <Monitor size={14} strokeWidth={1.5} />
      )}
    </button>
  );
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

  const btn = "flex items-center justify-center transition-colors duration-100";

  return (
    <>
      <button
        onClick={minimize}
        className={`${btn} w-11 h-full hover:bg-[var(--bg-hover)]`}
        style={{ color: "var(--fill-secondary)" }}
        title="最小化"
      >
        <Minus size={12} strokeWidth={1.5} />
      </button>
      <button
        onClick={toggleMaximize}
        className={`${btn} w-11 h-full hover:bg-[var(--bg-hover)]`}
        style={{ color: "var(--fill-secondary)" }}
        title={isMaximized ? "还原" : "最大化"}
      >
        {isMaximized ? <Maximize2 size={10} strokeWidth={1.5} /> : <Square size={10} strokeWidth={1.5} />}
      </button>
      <button
        onClick={close}
        className={`${btn} w-11 h-full hover:bg-[var(--bg-active)]`}
        style={{ color: "var(--fill-secondary)" }}
        title="关闭"
      >
        <X size={12} strokeWidth={1.5} />
      </button>
    </>
  );
}

function ConnectionDot() {
  const connected = useGatewayStore((s) => s.connected);
  return (
    <div
      className="flex items-center justify-center px-2"
      title={connected ? "已连接" : "未连接"}
    >
      <span
        className="inline-block h-[6px] w-[6px] rounded-full transition-colors duration-300"
        style={{ background: connected ? "var(--green)" : "var(--red)" }}
      />
    </div>
  );
}

export function TitleBar() {
  const [settingsOpen, setSettingsOpen] = useState(false);

  return (
    <>
      <SettingsPanel open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      <header
        className="relative z-30 flex shrink-0 select-none items-stretch"
        style={{
          height: "var(--titlebar-h)",
          background: "var(--bg-sidebar)",
          borderBottom: `0.5px solid var(--separator)`,
        }}
      >
        <div
          className="flex flex-1 items-center gap-2 pl-4"
          onMouseDown={onDragMouseDown}
          onDoubleClick={onDragDoubleClick}
        >
          <ClawIcon size={22} />
          <span
            className="text-[13px] font-semibold tracking-[-0.01em]"
            style={{ color: "var(--fill-primary)" }}
          >
            FastClaw
          </span>
        </div>

        <ConnectionDot />
        <button
          onClick={() => setSettingsOpen(true)}
          className="flex w-11 items-center justify-center transition-colors duration-100 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-tertiary)" }}
          title="设置"
        >
          <Settings size={15} strokeWidth={1.5} />
        </button>
        <ThemeToggle />
        <WindowControls />
      </header>
    </>
  );
}
