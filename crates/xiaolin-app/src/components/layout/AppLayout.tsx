import { useState, useEffect, useCallback, useRef, lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import { useGatewayStore } from "../../lib/store";
import { useUIStore } from "../../lib/stores";
import type { LayoutTier } from "../../lib/stores/ui-store";
import { MessageStream } from "../message-stream/MessageStream";
import { AutomationView } from "../automation/AutomationView";
import { PluginsView } from "../plugins/PluginsView";
import { SettingsPanel } from "../settings/SettingsPanel";
import { ElicitationDialog } from "../plugins/ElicitationDialog";
import { TitleBar } from "./TitleBar";
import { ClawIcon } from "./ClawIcon";
import { UpdateBanner } from "./UpdateBanner";
import { AppShell } from "../shell/AppShell";
import * as api from "../../lib/api";

const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

const RESIZE_HIT = 3;

function WindowResizeHandles() {
  if (!isTauri) return null;

  const start = useCallback(
    async (e: React.MouseEvent, direction: string) => {
      e.preventDefault();
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (getCurrentWindow() as any).startResizeDragging(direction);
    },
    [],
  );

  const S = RESIZE_HIT;
  const abs = (extra: React.CSSProperties): React.CSSProperties => ({
    position: "absolute",
    zIndex: 50,
    ...extra,
  });

  return (
    <>
      <div style={abs({ top: 0, left: S, right: S, height: S, cursor: "n-resize" })} onMouseDown={(e) => start(e, "North")} />
      <div style={abs({ bottom: 0, left: S, right: S, height: S, cursor: "s-resize" })} onMouseDown={(e) => start(e, "South")} />
      <div style={abs({ left: 0, top: S, bottom: S, width: S, cursor: "w-resize" })} onMouseDown={(e) => start(e, "West")} />
      <div style={abs({ right: 0, top: S, bottom: S, width: S, cursor: "e-resize" })} onMouseDown={(e) => start(e, "East")} />
      <div style={abs({ top: 0, left: 0, width: S * 2, height: S * 2, cursor: "nw-resize" })} onMouseDown={(e) => start(e, "NorthWest")} />
      <div style={abs({ top: 0, right: 0, width: S * 2, height: S * 2, cursor: "ne-resize" })} onMouseDown={(e) => start(e, "NorthEast")} />
      <div style={abs({ bottom: 0, left: 0, width: S * 2, height: S * 2, cursor: "sw-resize" })} onMouseDown={(e) => start(e, "SouthWest")} />
      <div style={abs({ bottom: 0, right: 0, width: S * 2, height: S * 2, cursor: "se-resize" })} onMouseDown={(e) => start(e, "SouthEast")} />
    </>
  );
}

const OnboardingWizard = lazy(() =>
  import("../onboarding/OnboardingWizard").then((m) => ({ default: m.OnboardingWizard })),
);

function Loading({ error }: { error: string | null }) {
  const { t } = useTranslation("common");
  if (error) {
    return (
      <div className="flex h-full flex-col items-center justify-center" style={{ background: "var(--bg-primary)" }}>
        <div style={{ animation: "scale-in var(--duration-slow) var(--ease-out)" }} className="text-center">
          <div className="mx-auto mb-5"><ClawIcon size={64} /></div>
          <p className="text-[15px] font-semibold tracking-[-0.01em]" style={{ color: "var(--fill-primary)" }}>XiaoLin</p>
          <p className="mt-1.5 text-[13px]" style={{ color: "var(--red)" }}>{t("connectionFailed", { error })}</p>
          <button
            onClick={() => window.location.reload()}
            className="mt-4 cursor-pointer rounded-[var(--radius-xs)] px-4 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80 active:scale-[0.97]"
            style={{ background: "var(--tint)", color: "#fff" }}
          >
            {t("retryConnection")}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col items-center justify-center gap-4" style={{ background: "var(--bg-primary)" }}>
      <div style={{ animation: "pulse-subtle 2s ease-in-out infinite" }}><ClawIcon size={48} /></div>
      <p className="text-[13px] font-medium" style={{ color: "var(--fill-tertiary)" }}>{t("connecting")}</p>
    </div>
  );
}

function MainContent({ connected, mode }: { connected: boolean; mode: string }) {
  const { t } = useTranslation("common");
  const mainView = useUIStore((s) => s.mainView);
  return (
    <>
      <UpdateBanner />
      <main className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        {mainView === "automations" ? <AutomationView /> : mainView === "plugins" ? <PluginsView /> : <MessageStream />}
        {!connected && mode !== "browser" && (
          <div
            className="absolute inset-x-0 top-0 z-20 flex items-center justify-center py-1.5"
            style={{
              background: "rgba(var(--bg-primary-rgb, 0, 0, 0), 0.85)",
              backdropFilter: "blur(8px)",
            }}
          >
            <span className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              {t("connectionLost")}
            </span>
          </div>
        )}
      </main>
    </>
  );
}

export function AppLayout() {
  const mode = useGatewayStore((s) => s.mode);
  const error = useGatewayStore((s) => s.error);
  const connected = useGatewayStore((s) => s.connected);

  const setLayoutTier = useUIStore((s) => s.setLayoutTier);
  const layoutTier = useUIStore((s) => s.layoutTier);
  const sidebarCollapsed = useUIStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);

  useEffect(() => {
    const computeTier = (w: number): LayoutTier =>
      w < 700 ? "compact" : w > 1100 ? "wide" : "standard";
    setLayoutTier(computeTier(window.innerWidth));
    const onResize = () => setLayoutTier(computeTier(window.innerWidth));
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [setLayoutTier]);

  const prevTierRef = useRef(layoutTier);
  useEffect(() => {
    if (layoutTier === "compact" && prevTierRef.current !== "compact" && !sidebarCollapsed) {
      toggleSidebar();
    }
    prevTierRef.current = layoutTier;
  }, [layoutTier, sidebarCollapsed, toggleSidebar]);

  const [showOnboarding, setShowOnboarding] = useState(false);
  const [onboardingChecked, setOnboardingChecked] = useState(false);
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    let unlistenFn: (() => void) | undefined;
    (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (!cancelled) setIsMaximized(await win.isMaximized());
      unlistenFn = await win.onResized(async () => {
        if (!cancelled) setIsMaximized(await win.isMaximized());
      });
    })();
    return () => {
      cancelled = true;
      unlistenFn?.();
    };
  }, []);

  useEffect(() => {
    if (mode === "shell" || mode === "connecting" || !connected) return;
    let cancelled = false;
    (async () => {
      try {
        const [cfg, models] = await Promise.all([
          api.getConfig("onboarding") as Promise<{ value?: { completed?: boolean }; completed?: boolean } | null>,
          api.listModels(),
        ]);
        if (cancelled) return;
        const val = cfg?.value ?? cfg;
        if (val && typeof val === "object" && (val as Record<string, unknown>).completed) {
          setShowOnboarding(false);
          setOnboardingChecked(true);
          return;
        }
        setShowOnboarding(models.length === 0);
        setOnboardingChecked(true);
      } catch {
        if (!cancelled) { setShowOnboarding(false); setOnboardingChecked(true); }
      }
    })();
    return () => { cancelled = true; };
  }, [mode, connected]);

  const handleOnboardingComplete = useCallback(async () => {
    try { await api.setConfig("onboarding", { completed: true }); } catch { /* best-effort */ }
    try {
      const models = await api.listModels();
      if (models.length > 0) {
        const first = models[0];
        await api.updateAgent("main", {
          model: { provider: first.provider, model: first.model, temperature: 0.7 },
        });
      }
    } catch { /* best-effort */ }
    setShowOnboarding(false);
  }, []);

  let content: React.ReactNode;

  if (mode === "shell" || mode === "connecting" || !onboardingChecked) {
    content = <Loading error={error} />;
  } else if (showOnboarding) {
    content = (
      <>
        <TitleBar />
        <Suspense fallback={<div className="flex-1" style={{ background: "var(--bg-primary)" }} />}>
          <OnboardingWizard onComplete={handleOnboardingComplete} />
        </Suspense>
      </>
    );
  } else {
    content = (
      <AppShell>
        <MainContent connected={connected} mode={mode} />
      </AppShell>
    );
  }

  const settingsOpen = useUIStore((s) => s.settingsOpen);
  const closeSettings = useUIStore((s) => s.closeSettings);

  return (
    <div className={`app-shell relative flex h-full flex-col${isMaximized ? " maximized" : ""}`} data-layout-tier={layoutTier}>
      {!isMaximized && <WindowResizeHandles />}
      {content}
      <SettingsPanel open={settingsOpen} onClose={closeSettings} />
      <ElicitationDialog />
    </div>
  );
}
