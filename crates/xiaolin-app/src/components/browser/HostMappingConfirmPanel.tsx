import { useState, useEffect, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { ShieldWarning, Check, X } from "@phosphor-icons/react";
import { isTauri } from "../../lib/transport";
import * as transport from "../../lib/transport";

export interface NetworkConfirmPayload {
  requestId: string;
  kind: "set_hosts" | "set_proxy" | string;
  reason?: string | null;
  mappings?: Array<{ pattern: string; targetIp: string }>;
  proxyUrl?: string | null;
  expiresAt: number;
}

interface HostMappingConfirmPanelProps {
  request: NetworkConfirmPayload;
  onResolved: () => void;
}

function normalizePayload(raw: Record<string, unknown>): NetworkConfirmPayload {
  const mappings = Array.isArray(raw.mappings)
    ? (raw.mappings as Record<string, unknown>[]).map((m) => ({
        pattern: String(m.pattern ?? ""),
        targetIp: String(m.target_ip ?? m.targetIp ?? ""),
      }))
    : undefined;
  return {
    requestId: String(raw.requestId ?? raw.request_id ?? ""),
    kind: String(raw.kind ?? ""),
    reason: (raw.reason as string | null) ?? null,
    mappings,
    proxyUrl: (raw.proxyUrl ?? raw.proxy_url) as string | null | undefined,
    expiresAt: Number(raw.expiresAt ?? raw.expires_at ?? 0),
  };
}

async function resolveConfirm(requestId: string, approved: boolean): Promise<void> {
  if (!isTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("browser_network_confirm_resolve", { requestId, approved });
}

export function HostMappingConfirmPanel({ request, onResolved }: HostMappingConfirmPanelProps) {
  const { t } = useTranslation("browser");
  const [remaining, setRemaining] = useState(30);
  const [submitting, setSubmitting] = useState(false);
  const resolvedRef = useRef(false);

  const handleDecision = useCallback(
    async (approved: boolean) => {
      if (resolvedRef.current || submitting) return;
      setSubmitting(true);
      try {
        await resolveConfirm(request.requestId, approved);
        resolvedRef.current = true;
        onResolved();
      } catch (e) {
        console.warn("[browser-network] confirm resolve failed:", e);
      } finally {
        setSubmitting(false);
      }
    },
    [request.requestId, submitting, onResolved],
  );

  const handleDecisionRef = useRef(handleDecision);
  handleDecisionRef.current = handleDecision;

  useEffect(() => {
    resolvedRef.current = false;
    setRemaining(30);
    setSubmitting(false);
  }, [request.requestId]);

  useEffect(() => {
    const tick = () => {
      if (request.expiresAt > 0) {
        const left = Math.max(0, Math.ceil((request.expiresAt - Date.now()) / 1000));
        setRemaining(left);
        if (left <= 0 && !resolvedRef.current) {
          void handleDecisionRef.current(false);
        }
      } else {
        setRemaining((r) => {
          if (r <= 1 && !resolvedRef.current) {
            void handleDecisionRef.current(false);
            return 0;
          }
          return r - 1;
        });
      }
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [request.requestId, request.expiresAt]);

  const title =
    request.kind === "set_proxy" ? t("agentRequestProxy") : t("agentRequestHostMapping");

  return (
    <div
      className="mx-4 mb-3 overflow-hidden rounded-xl"
      style={{
        border: "1px solid var(--color-amber-500, #f59e0b)",
        background: "var(--bg-elevated)",
        boxShadow: "var(--shadow-md)",
      }}
    >
      <div
        className="flex items-center gap-2 px-4 py-3"
        style={{ background: "rgba(245, 158, 11, 0.08)", borderBottom: "0.5px solid var(--separator)" }}
      >
        <ShieldWarning size={20} weight="fill" style={{ color: "var(--color-amber-500, #f59e0b)" }} />
        <span className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
          {title}
        </span>
        <span className="ml-auto text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          {remaining}s
        </span>
      </div>

      <div className="px-4 py-3">
        {request.reason && (
          <p className="mb-3 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
            {request.reason}
          </p>
        )}

        {request.kind === "set_hosts" && request.mappings && request.mappings.length > 0 && (
          <div className="mb-3 rounded-md text-[12px]" style={{ background: "var(--bg-secondary)" }}>
            {request.mappings.map((m, i) => (
              <div
                key={`${m.pattern}-${m.targetIp}`}
                className="flex items-center gap-2 px-3 py-2"
                style={{
                  borderBottom:
                    i < request.mappings!.length - 1 ? "0.5px solid var(--separator)" : undefined,
                }}
              >
                <code style={{ color: "var(--fill-primary)" }}>{m.pattern}</code>
                <span style={{ color: "var(--fill-tertiary)" }}>→</span>
                <code style={{ color: "var(--accent)" }}>{m.targetIp}</code>
              </div>
            ))}
          </div>
        )}

        {request.kind === "set_proxy" && (
          <div className="mb-3 rounded-md px-3 py-2 text-[12px]" style={{ background: "var(--bg-secondary)" }}>
            <span style={{ color: "var(--fill-tertiary)" }}>{t("upstreamProxyLabel")}</span>
            <code style={{ color: "var(--fill-primary)" }}>
              {request.proxyUrl ?? t("clearUpstreamProxy")}
            </code>
          </div>
        )}

        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("hostMappingSecurityWarning")}
        </p>

        <div className="flex gap-2">
          <button
            type="button"
            disabled={submitting}
            onClick={() => void handleDecision(true)}
            className="flex flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-lg py-2 text-[13px] font-medium disabled:opacity-50"
            style={{ background: "var(--accent)", color: "var(--accent-fg, #fff)" }}
          >
            <Check size={16} weight="bold" />
            {t("approve")}
          </button>
          <button
            type="button"
            disabled={submitting}
            onClick={() => void handleDecision(false)}
            className="flex flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-lg py-2 text-[13px] font-medium disabled:opacity-50"
            style={{
              background: "var(--bg-secondary)",
              border: "0.5px solid var(--separator)",
              color: "var(--fill-primary)",
            }}
          >
            <X size={16} weight="bold" />
            {t("deny")}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Global confirm queue — listens to Tauri + WS events. */
export function useBrowserNetworkConfirmListener(): {
  pendingConfirm: NetworkConfirmPayload | null;
  pendingCount: number;
  dismissConfirm: () => void;
} {
  const [pendingConfirms, setPendingConfirms] = useState<NetworkConfirmPayload[]>([]);

  useEffect(() => {
    if (!isTauri) return;

    let cancelled = false;
    const unsubs: Array<() => void> = [];

    function enqueueConfirm(payload: Record<string, unknown>) {
      const item = normalizePayload(payload);
      setPendingConfirms((prev) => {
        if (prev.some((p) => p.requestId === item.requestId)) return prev;
        return [...prev, item];
      });
    }

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const unlisten = await listen<Record<string, unknown>>(
        "browser-network-confirm-request",
        (ev) => enqueueConfirm(ev.payload),
      );
      if (cancelled) {
        unlisten();
        return;
      }
      unsubs.push(unlisten);
    })();

    const wsUnsub = transport.onWsEvent("browser_network_confirm", (msg: unknown) => {
      const data = (msg as { data?: Record<string, unknown> })?.data;
      if (data) enqueueConfirm(data);
    });
    unsubs.push(wsUnsub);

    return () => {
      cancelled = true;
      for (const u of unsubs) u();
    };
  }, []);

  const dismissConfirm = useCallback(() => {
    setPendingConfirms((prev) => prev.slice(1));
  }, []);

  return {
    pendingConfirm: pendingConfirms[0] ?? null,
    pendingCount: pendingConfirms.length,
    dismissConfirm,
  };
}
