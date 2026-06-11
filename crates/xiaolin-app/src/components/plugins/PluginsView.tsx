import { useEffect, useState, useCallback } from "react";
import {
  PuzzlePiece, ToggleLeft, ToggleRight, ArrowsClockwise,
  CaretDown, CaretRight, WarningCircle, SpinnerGap,
  Wrench, Gear,
} from "@phosphor-icons/react";
import { usePluginStore, subscribePluginEvents } from "../../lib/stores/plugin-store";
import type { PluginSummary, PluginTool } from "../../lib/transport";

export function PluginsView() {
  const plugins = usePluginStore((s) => s.plugins);
  const loading = usePluginStore((s) => s.loading);
  const fetchPlugins = usePluginStore((s) => s.fetchPlugins);
  const enablePlugin = usePluginStore((s) => s.enablePlugin);
  const disablePlugin = usePluginStore((s) => s.disablePlugin);
  const restartPlugin = usePluginStore((s) => s.restartPlugin);
  const fetchTools = usePluginStore((s) => s.fetchTools);
  const toolsById = usePluginStore((s) => s.toolsById);

  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => {
    subscribePluginEvents();
    fetchPlugins();
  }, [fetchPlugins]);

  const handleToggle = useCallback(
    async (p: PluginSummary) => {
      if (p.enabled) await disablePlugin(p.id);
      else await enablePlugin(p.id);
    },
    [enablePlugin, disablePlugin],
  );

  const handleRestart = useCallback(
    async (id: string) => { await restartPlugin(id); },
    [restartPlugin],
  );

  const handleExpand = useCallback(
    (id: string) => {
      const next = expandedId === id ? null : id;
      setExpandedId(next);
      if (next && !toolsById[next]) fetchTools(next);
    },
    [expandedId, toolsById, fetchTools],
  );

  const connectedCount = plugins.filter((p) => p.status === "connected").length;

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--bg-card)" }}>
      <style>{ANIM_CSS}</style>

      {/* Header */}
      <div
        className="flex shrink-0 items-center justify-between px-6 py-5"
        style={{ borderBottom: "0.5px solid var(--separator)" }}
      >
        <div className="flex items-center gap-3">
          <div
            className="flex h-9 w-9 items-center justify-center rounded-[10px]"
            style={{ background: "color-mix(in srgb, var(--tint) 8%, transparent)" }}
          >
            <PuzzlePiece size={18} style={{ color: "var(--tint)" }} />
          </div>
          <div>
            <h1 className="text-[17px] font-bold tracking-[-0.01em]" style={{ color: "var(--fill-primary)" }}>
              Plugins
            </h1>
            <p className="text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
              Manage MCP server connections and tools
            </p>
          </div>
        </div>
        {plugins.length > 0 && (
          <div
            className="flex items-center gap-1.5 rounded-full px-3 py-1"
            style={{ background: "color-mix(in srgb, var(--green, #38A169) 8%, transparent)" }}
          >
            <span className="inline-block h-1.5 w-1.5 rounded-full" style={{ background: "var(--green, #38A169)" }} />
            <span className="text-[12px] font-semibold tabular-nums" style={{ color: "var(--green, #38A169)" }}>
              {connectedCount}/{plugins.length} connected
            </span>
          </div>
        )}
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto" style={{ overscrollBehavior: "contain" }}>
        <div className="mx-auto w-full max-w-[640px] px-6 py-5">
          {loading ? (
            <div className="flex flex-col items-center justify-center gap-3 py-20 pv-fade-in">
              <SpinnerGap size={20} className="animate-spin" style={{ color: "var(--fill-quaternary)" }} />
              <p className="text-[12px]" style={{ color: "var(--fill-quaternary)" }}>Loading plugins…</p>
            </div>
          ) : plugins.length === 0 ? (
            <EmptyState />
          ) : (
            <div className="flex flex-col gap-2">
              {plugins.map((p, idx) => (
                <PluginRow
                  key={p.id}
                  plugin={p}
                  expanded={expandedId === p.id}
                  tools={toolsById[p.id]}
                  onToggle={() => handleToggle(p)}
                  onRestart={() => handleRestart(p.id)}
                  onExpand={() => handleExpand(p.id)}
                  index={idx}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Footer */}
      <div
        className="flex shrink-0 items-center justify-center px-6 py-3"
        style={{ borderTop: "0.5px solid var(--separator)" }}
      >
        <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
          Add MCP servers in Settings to extend capabilities
        </p>
      </div>
    </div>
  );
}

function PluginRow({
  plugin: p,
  expanded,
  tools,
  onToggle,
  onRestart,
  onExpand,
  index,
}: {
  plugin: PluginSummary;
  expanded: boolean;
  tools?: PluginTool[];
  onToggle: () => void;
  onRestart: () => void;
  onExpand: () => void;
  index: number;
}) {
  const [restarting, setRestarting] = useState(false);

  const handleRestart = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setRestarting(true);
    await onRestart();
    setRestarting(false);
  };

  return (
    <div
      className="pv-stagger rounded-[var(--radius-sm)] transition-all duration-200"
      style={{
        "--stagger-i": index,
        background: expanded ? "var(--bg-primary)" : "transparent",
        border: expanded ? "0.5px solid var(--separator)" : "0.5px solid transparent",
      } as React.CSSProperties}
    >
      <div
        className="group flex items-center gap-3 px-4 py-3.5 transition-colors duration-150 hover:bg-[var(--bg-hover)]"
        style={{ cursor: "pointer", borderRadius: "var(--radius-sm)" }}
        onClick={onExpand}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onExpand(); } }}
      >
        {expanded ? (
          <CaretDown size={14} style={{ color: "var(--fill-quaternary)" }} />
        ) : (
          <CaretRight size={14} style={{ color: "var(--fill-quaternary)" }} />
        )}

        <StatusDot status={p.status} />

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2.5">
            <span className="truncate text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>{p.name}</span>
            <ScopeBadge scope={p.scope} />
          </div>
          {p.lastError && (
            <div className="mt-0.5 flex items-center gap-1 text-[11px]" style={{ color: "var(--red, #E53E3E)" }}>
              <WarningCircle size={10} />
              <span className="truncate">{p.lastError}</span>
            </div>
          )}
        </div>

        <div className="flex items-center gap-2">
          {p.toolCount > 0 && (
            <span className="text-[11px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
              {p.toolCount} tools
            </span>
          )}

          <button
            onClick={handleRestart}
            disabled={restarting || !p.enabled}
            className="rounded-[var(--radius-xs)] p-1.5 opacity-0 transition-all duration-200 group-hover:opacity-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
            style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}
            title="Restart"
            aria-label={`Restart ${p.name}`}
          >
            <ArrowsClockwise size={13} className={restarting ? "animate-spin" : ""} />
          </button>

          <button
            onClick={(e) => { e.stopPropagation(); onToggle(); }}
            className="rounded-[var(--radius-xs)] p-1.5 transition-all duration-200"
            style={{ cursor: "pointer", background: "none", border: "none" }}
            title={p.enabled ? "Disable" : "Enable"}
            aria-label={`${p.enabled ? "Disable" : "Enable"} ${p.name}`}
          >
            {p.enabled ? (
              <ToggleRight size={20} style={{ color: "var(--green, #38A169)" }} />
            ) : (
              <ToggleLeft size={20} style={{ color: "var(--fill-quaternary)" }} />
            )}
          </button>
        </div>
      </div>

      {expanded && (
        <div className="px-4 pb-4 pv-fade-in">
          <div className="ml-6 border-l pl-4 pt-1" style={{ borderColor: "var(--separator)" }}>
            {p.connectedAt && (
              <DetailRow label="Connected" value={new Date(p.connectedAt).toLocaleString()} />
            )}
            <DetailRow label="Status" value={p.status} />
            {p.lastError && <DetailRow label="Error" value={p.lastError} isError />}

            {tools && tools.length > 0 && (
              <div className="mt-3">
                <p className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
                  <Wrench size={11} /> Tools ({tools.length})
                </p>
                <div className="flex flex-col gap-1.5">
                  {tools.map((t) => (
                    <div key={t.name} className="rounded-[var(--radius-xs)] px-3 py-2" style={{ background: "var(--bg-card)" }}>
                      <p className="text-[12px] font-medium" style={{ color: "var(--fill-primary)" }}>{t.name}</p>
                      {t.description && (
                        <p className="mt-0.5 text-[11px] leading-relaxed" style={{ color: "var(--fill-quaternary)" }}>{t.description}</p>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {tools && tools.length === 0 && (
              <p className="mt-1 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>No tools available</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center gap-5 py-24 pv-fade-in">
      <div
        className="pv-float flex h-16 w-16 items-center justify-center rounded-[16px]"
        style={{ background: "color-mix(in srgb, var(--tint) 6%, transparent)" }}
      >
        <PuzzlePiece size={32} style={{ color: "var(--tint)", opacity: 0.8 }} />
      </div>
      <div className="text-center">
        <p className="text-[17px] font-bold" style={{ color: "var(--fill-primary)" }}>No plugins installed</p>
        <p className="mt-2 text-[13px] leading-relaxed" style={{ color: "var(--fill-quaternary)", maxWidth: 320 }}>
          Add MCP servers in Settings to extend your agent&apos;s capabilities with external tools.
        </p>
      </div>
      <button
        className="mt-2 flex items-center gap-2 rounded-[var(--radius-xs)] px-5 py-2.5 text-[13px] font-medium transition-colors duration-150 hover:bg-[var(--bg-hover)]"
        style={{ cursor: "pointer", background: "none", border: "1px solid var(--separator)", color: "var(--fill-secondary)" }}
      >
        <Gear size={14} /> Manage in Settings
      </button>
    </div>
  );
}

function StatusDot({ status }: { status: string }) {
  const isConnecting = status === "connecting";
  const color =
    status === "connected" ? "var(--green, #38A169)" :
    status === "failed" ? "var(--red, #E53E3E)" :
    isConnecting ? "var(--orange, #ED8936)" :
    "var(--fill-quaternary)";
  return (
    <span className="relative flex h-2.5 w-2.5 shrink-0">
      {isConnecting && <span className="absolute inline-flex h-full w-full animate-ping rounded-full opacity-40" style={{ background: color }} />}
      <span className="relative inline-flex h-2.5 w-2.5 rounded-full" style={{ background: color }} />
    </span>
  );
}

function ScopeBadge({ scope }: { scope: string }) {
  return (
    <span
      className="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
      style={{
        background: scope === "project" ? "color-mix(in srgb, var(--orange, #ED8936) 8%, transparent)" : "var(--bg-tertiary)",
        color: scope === "project" ? "var(--orange, #ED8936)" : "var(--fill-quaternary)",
      }}
    >
      {scope}
    </span>
  );
}

function DetailRow({ label, value, isError }: { label: string; value: string; isError?: boolean }) {
  return (
    <div className="flex items-start gap-3 py-0.5">
      <span className="shrink-0 text-[11px] font-medium" style={{ color: "var(--fill-quaternary)", minWidth: 72 }}>{label}</span>
      <span className="text-[12px] break-all" style={{ color: isError ? "var(--red, #E53E3E)" : "var(--fill-secondary)" }}>{value}</span>
    </div>
  );
}

const ANIM_CSS = `
@media (prefers-reduced-motion: no-preference) {
  .pv-fade-in { animation: pvFadeIn 220ms cubic-bezier(0.16, 1, 0.3, 1) both; }
  .pv-float { animation: pvFloat 4s ease-in-out infinite; }
  .pv-stagger { animation: pvFadeUp 260ms cubic-bezier(0.16, 1, 0.3, 1) both; animation-delay: calc(var(--stagger-i, 0) * 40ms); }
}
@keyframes pvFadeIn { from { opacity: 0; } to { opacity: 1; } }
@keyframes pvFloat { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-5px); } }
@keyframes pvFadeUp { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }
`;
