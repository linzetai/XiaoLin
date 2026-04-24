import { useState, useEffect, useCallback, useMemo } from "react";
import { useGatewayStore } from "../../lib/store";
import { useAgentStore } from "../../lib/agent-store";
import { AgentList } from "../agent-list/AgentList";
import { AgentDetail } from "../agent-detail/AgentDetail";
import { MessageStream } from "../message-stream/MessageStream";
import { TitleBar } from "./TitleBar";
import { ClawIcon } from "./ClawIcon";
import { OnboardingWizard } from "../onboarding/OnboardingWizard";
import * as api from "../../lib/api";

function Loading({ error }: { error: string | null }) {
  return (
    <div className="flex h-full flex-col items-center justify-center" style={{ background: "var(--bg-primary)" }}>
      <div style={{ animation: "scale-in 0.4s ease-out" }} className="text-center">
        <div className="mx-auto mb-5" style={{ animation: error ? "none" : "pulse-subtle 2s ease-in-out infinite" }}>
          <ClawIcon size={64} />
        </div>
        <p className="text-[15px] font-semibold tracking-[-0.01em]" style={{ color: "var(--fill-primary)" }}>FastClaw</p>
        <p className="mt-1.5 text-[13px]" style={{ color: error ? "var(--red)" : "var(--fill-tertiary)" }}>
          {error ? `连接失败: ${error}` : "正在启动..."}
        </p>
        {error && (
          <button
            onClick={() => window.location.reload()}
            className="mt-4 cursor-pointer rounded-[var(--radius-xs)] px-4 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80"
            style={{ background: "var(--tint)", color: "#fff" }}
          >
            重试连接
          </button>
        )}
      </div>
    </div>
  );
}

export function AppLayout() {
  const mode = useGatewayStore((s) => s.mode);
  const error = useGatewayStore((s) => s.error);
  const connected = useGatewayStore((s) => s.connected);

  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agents = useAgentStore((s) => s.agents);
  const detailOpen = useAgentStore((s) => s.detailOpen);
  const toggleDetail = useAgentStore((s) => s.toggleDetail);
  const closeDetail = useAgentStore((s) => s.closeDetail);

  const activeAgent = useMemo(
    () => agents.find((a) => a.id === activeAgentId) ?? agents[0],
    [agents, activeAgentId],
  );

  const [showOnboarding, setShowOnboarding] = useState(false);
  const [onboardingChecked, setOnboardingChecked] = useState(false);

  useEffect(() => {
    if (mode === "connecting" || (!connected && mode !== "embedded")) return;
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
    setShowOnboarding(false);
  }, []);

  if (mode === "connecting" || !activeAgent || !onboardingChecked) return <Loading error={error} />;

  if (showOnboarding) {
    return (
      <div className="flex h-full flex-col" style={{ background: "var(--bg-primary)" }}>
        <TitleBar />
        <OnboardingWizard onComplete={handleOnboardingComplete} />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--bg-primary)" }}>
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <AgentList />
        <main className="relative flex min-w-0 flex-1 flex-col">
          <MessageStream onToggleDetail={toggleDetail} detailOpen={detailOpen} />
          {!connected && mode !== "browser" && (
            <div
              className="absolute inset-x-0 top-0 z-20 flex items-center justify-center py-1.5"
              style={{
                background: "rgba(var(--bg-primary-rgb, 0, 0, 0), 0.85)",
                backdropFilter: "blur(8px)",
                animation: "fade-in 0.3s",
              }}
            >
              <span className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
                连接已断开，正在重连...
              </span>
            </div>
          )}
        </main>
        <AgentDetail
          open={detailOpen}
          onClose={closeDetail}
          agentName={activeAgent.name}
          agentInitial={activeAgent.initial}
          agentColor={activeAgent.color}
        />
      </div>
    </div>
  );
}
