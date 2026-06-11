import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  HardDrives,
  ArrowsClockwise,
  Plus,
  Trash,
  X,
  WifiHigh,
  WifiSlash,
  Wrench,
  Link,
  LinkBreak,
  QrCode,
  SpinnerGap,
  CheckCircle,
  DeviceMobile,
  Key,
  Terminal,
  PencilSimple,
  ArrowCounterClockwise,
  FloppyDisk,
  MagnifyingGlass,
  CaretDown,
  CaretUp,
} from "@phosphor-icons/react";
import { ICON_SIZE } from "../../lib/ui-tokens";
import * as api from "../../lib/api";
import type { McpServerStatus, McpDetailResult, ChannelDetailResult } from "../../lib/transport";
import type { ChannelStatus } from "../../lib/transport";

function StatusDot({ status }: { status: string }) {
  const color =
    status === "connected"
      ? "var(--green)"
      : status === "failed"
        ? "var(--red)"
        : status === "connecting"
          ? "var(--yellow)"
          : "var(--fill-quaternary)";

  return (
    <span
      className="inline-block h-2 w-2 shrink-0 rounded-full"
      style={{
        background: color,
        boxShadow: status === "connected" ? `0 0 6px ${color}` : undefined,
        animation: status === "connecting" ? "pulse 1.5s ease-in-out infinite" : undefined,
      }}
    />
  );
}

function McpCard({
  server,
  onRemove,
  onClick,
}: {
  server: McpServerStatus;
  onRemove: (id: string) => void;
  onClick: (id: string) => void;
}) {
  const { t } = useTranslation("common");
  const [confirming, setConfirming] = useState(false);

  return (
    <div
      className="flex cursor-pointer items-center gap-3 rounded-[var(--radius-md)] px-4 py-3 transition-colors duration-150 hover:brightness-[0.97]"
      style={{
        background: "var(--bg-secondary)",
        border: "0.5px solid var(--border-subtle)",
      }}
      onClick={() => onClick(server.id)}
    >
      <StatusDot status={server.status} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span
            className="truncate text-[13px] font-medium"
            style={{ color: "var(--fill-primary)" }}
          >
            {server.id}
          </span>
          {server.status === "connected" && (
            <span
              className="flex items-center gap-1 text-[11px]"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <Wrench size={11} />
              {t("conn_tools", { count: server.toolCount })}
            </span>
          )}
        </div>
        {server.error && (
          <p className="mt-0.5 truncate text-[11px]" style={{ color: "var(--red)" }}>
            {server.error}
          </p>
        )}
        {server.connectedAt && (
          <p className="mt-0.5 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
            {new Date(server.connectedAt).toLocaleString()}
          </p>
        )}
      </div>
      {confirming ? (
        <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
          <button
            onClick={() => {
              onRemove(server.id);
              setConfirming(false);
            }}
            className="rounded-[var(--radius-xs)] px-2 py-1 text-[11px] font-medium transition-colors"
            style={{ background: "var(--red)", color: "#fff" }}
          >
            {t("conn_confirm")}
          </button>
          <button
            onClick={() => setConfirming(false)}
            className="rounded-[var(--radius-xs)] px-2 py-1 text-[11px] transition-colors"
            style={{ color: "var(--fill-tertiary)" }}
          >
            {t("cancel")}
          </button>
        </div>
      ) : (
        <button
          onClick={(e) => { e.stopPropagation(); setConfirming(true); }}
          className="flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] opacity-0 transition-all duration-150 group-hover:opacity-100 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-tertiary)" }}
          title={t("delete")}
        >
          <Trash />
        </button>
      )}
    </div>
  );
}

const CAP_LABELS: Record<string, string> = {
  directMessage: "conn_directMessage",
  groupChat: "conn_groupChat",
  media: "conn_media",
  streaming: "conn_streaming",
  reactions: "conn_reactions",
  threads: "conn_threads",
};

const STATUS_CONFIG: Record<string, { labelKey: string; bg: string; fg: string }> = {
  connected: { labelKey: "conn_status_connected", bg: "rgba(72,187,120,0.12)", fg: "var(--green)" },
  disconnected: { labelKey: "conn_status_disconnected", bg: "var(--bg-tertiary)", fg: "var(--fill-quaternary)" },
  configured: { labelKey: "conn_status_configured", bg: "rgba(237,137,54,0.12)", fg: "var(--yellow)" },
  available: { labelKey: "conn_status_available", bg: "var(--bg-tertiary)", fg: "var(--fill-quaternary)" },
};

function ChannelCard({
  channel,
  onConnect,
  onDisconnect,
  onClick,
}: {
  channel: ChannelStatus;
  onConnect: (ch: ChannelStatus) => void;
  onDisconnect: (ch: ChannelStatus) => void;
  onClick: (id: string) => void;
}) {
  const { t } = useTranslation("common");
  const [disconnecting, setDisconnecting] = useState(false);
  const connected = channel.status === "connected";
  const activeCaps = Object.entries(channel.capabilities ?? {})
    .filter(([, v]) => v)
    .map(([k]) => k);
  const statusCfg = STATUS_CONFIG[channel.status] ?? STATUS_CONFIG.available;

  const handleDisconnect = async () => {
    setDisconnecting(true);
    onDisconnect(channel);
    setDisconnecting(false);
  };

  return (
    <div
      className="flex cursor-pointer items-center gap-3 rounded-[var(--radius-md)] px-4 py-3 transition-colors duration-150 hover:brightness-[0.97]"
      style={{
        background: "var(--bg-secondary)",
        border: "0.5px solid var(--border-subtle)",
      }}
      onClick={() => onClick(channel.id)}
    >
      <StatusDot status={connected ? "connected" : "disconnected"} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
            {channel.name}
          </span>
          <span
            className="rounded-full px-1.5 py-0.5 text-[10px]"
            style={{ background: statusCfg.bg, color: statusCfg.fg }}
          >
            {t(statusCfg.labelKey)}
          </span>
          {channel.connectionMode && (
            <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              {channel.connectionMode}
            </span>
          )}
        </div>
        <p className="mt-0.5 truncate text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          {channel.description}
        </p>
        {activeCaps.length > 0 && (
          <div className="mt-1.5 flex flex-wrap gap-1">
            {activeCaps.map((cap) => (
              <span
                key={cap}
                className="rounded-full px-1.5 py-0.5 text-[10px]"
                style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)" }}
              >
                {t(CAP_LABELS[cap] ?? cap)}
              </span>
            ))}
          </div>
        )}
      </div>

      {connected ? (
        <button
          onClick={(e) => { e.stopPropagation(); handleDisconnect(); }}
          disabled={disconnecting}
          className="flex items-center gap-1 rounded-[var(--radius-sm)] px-2 py-1 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--red)" }}
          title={t("conn_disconnectConn")}
        >
          <LinkBreak size={12} />
          {t("conn_disconnect")}
        </button>
      ) : (
        <button
          onClick={(e) => { e.stopPropagation(); onConnect(channel); }}
          className="flex items-center gap-1 rounded-[var(--radius-sm)] px-2 py-1 text-[11px] font-medium transition-colors"
          style={{ background: "var(--tint)", color: "#fff" }}
        >
          <Link size={12} />
          {t("conn_connect")}
        </button>
      )}
    </div>
  );
}

type QrStep = "idle" | "loading" | "scanning" | "scanned" | "verify_code" | "confirmed" | "error";

function WechatQrModal({
  open,
  onClose,
  onSuccess,
}: {
  open: boolean;
  onClose: () => void;
  onSuccess: () => void;
}) {
  const { t } = useTranslation("common");
  const [step, setStep] = useState<QrStep>("idle");
  const [qrUrl, setQrUrl] = useState("");
  const [sessionKey, setSessionKey] = useState("");
  const [verifyCode, setVerifyCode] = useState("");
  const [message, setMessage] = useState("");
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const cleanup = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (!open) {
      cleanup();
      setStep("idle");
      setQrUrl("");
      setSessionKey("");
      setVerifyCode("");
      setMessage("");
    }
  }, [open, cleanup]);

  useEffect(() => () => cleanup(), [cleanup]);

  const startLogin = async () => {
    setStep("loading");
    try {
      const resp = await api.channelsWechatLogin();
      if (!resp.sessionKey) {
        setStep("error");
        setMessage(t("conn_cannotGetQr"));
        return;
      }
      setSessionKey(resp.sessionKey);
      setQrUrl(resp.qrUrl);
      setStep("scanning");

      pollRef.current = setInterval(async () => {
        try {
          const poll = await api.channelsWechatPoll(resp.sessionKey);
          switch (poll.status) {
            case "waiting":
              break;
            case "scanned":
              setStep("scanned");
              break;
            case "need_verify_code":
              setStep("verify_code");
              setMessage(poll.message ?? t("conn_enterPairCode"));
              cleanup();
              break;
            case "confirmed":
              setStep("confirmed");
              setMessage(poll.message ?? t("conn_connectionSuccess"));
              cleanup();
              setTimeout(() => onSuccess(), 1500);
              break;
            case "already_connected":
              setStep("confirmed");
              setMessage(poll.message ?? t("conn_connected"));
              cleanup();
              setTimeout(() => onSuccess(), 1500);
              break;
            case "expired_refreshed":
              if (poll.qrUrl) setQrUrl(poll.qrUrl);
              setStep("scanning");
              break;
            default:
              setStep("error");
              setMessage(poll.message ?? t("conn_connectionFailed"));
              cleanup();
          }
        } catch {
          setStep("error");
          setMessage(t("conn_pollFailed"));
          cleanup();
        }
      }, 1500);
    } catch {
      setStep("error");
      setMessage(t("conn_startFailed"));
    }
  };

  const submitVerifyCode = async () => {
    if (!verifyCode.trim()) return;
    await api.channelsWechatVerify(sessionKey, verifyCode.trim());
    setStep("scanning");
    setVerifyCode("");

    pollRef.current = setInterval(async () => {
      try {
        const poll = await api.channelsWechatPoll(sessionKey);
        if (poll.status === "confirmed") {
          setStep("confirmed");
          setMessage(poll.message ?? t("conn_connectionSuccess"));
          cleanup();
          setTimeout(() => onSuccess(), 1500);
        } else if (poll.status === "verify_blocked") {
          setStep("verify_code");
          setMessage(t("conn_codeRejected"));
          cleanup();
        } else if (poll.status !== "waiting" && poll.status !== "scanned") {
          setStep("error");
          setMessage(poll.message ?? t("conn_connectionFailed"));
          cleanup();
        }
      } catch {
        cleanup();
      }
    }, 1500);
  };

  if (!open) return null;

  const inputStyle: React.CSSProperties = {
    background: "var(--bg-primary)",
    border: "0.5px solid var(--border-subtle)",
    color: "var(--fill-primary)",
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
      onClick={onClose}
    >
      <div
        className="w-[400px] rounded-[var(--radius-lg)] p-6"
        style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--border-subtle)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-5 flex items-center justify-between">
          <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {t("conn_wechatTitle")}
          </h3>
          <button onClick={onClose} style={{ color: "var(--fill-tertiary)" }}>
            <X size={ICON_SIZE.md} />
          </button>
        </div>

        {step === "idle" && (
          <div className="flex flex-col items-center gap-4 py-6">
            <div
              className="flex h-16 w-16 items-center justify-center rounded-full"
              style={{ background: "rgba(72,187,120,0.1)" }}
            >
              <QrCode size={28} style={{ color: "var(--green)" }} />
            </div>
            <p className="text-center text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              {t("conn_wechatScan")}
            </p>
            <button
              onClick={startLogin}
              className="rounded-[var(--radius-sm)] px-4 py-2 text-[13px] font-medium transition-colors"
              style={{ background: "var(--tint)", color: "#fff" }}
            >
              {t("conn_getQr")}
            </button>
          </div>
        )}

        {step === "loading" && (
          <div className="flex flex-col items-center gap-3 py-8">
            <SpinnerGap size={24} className="animate-spin" style={{ color: "var(--tint)" }} />
            <p className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              {t("conn_fetchingQr")}
            </p>
          </div>
        )}

        {(step === "scanning" || step === "scanned") && (
          <div className="flex flex-col items-center gap-4 py-2">
            {qrUrl ? (
              <div
                className="rounded-[var(--radius-md)] p-3"
                style={{ background: "#fff" }}
              >
                <img src={qrUrl} alt="WeChat QR Code" className="h-48 w-48" />
              </div>
            ) : (
              <div className="flex h-48 w-48 items-center justify-center rounded-[var(--radius-md)] bg-white">
                <QrCode size={48} style={{ color: "#ccc" }} />
              </div>
            )}
            <div className="flex items-center gap-2">
              {step === "scanned" ? (
                <>
                  <DeviceMobile size={14} style={{ color: "var(--green)" }} />
                  <p className="text-[13px] font-medium" style={{ color: "var(--green)" }}>
                    {t("conn_scannedConfirm")}
                  </p>
                </>
              ) : (
                <>
                  <QrCode size={14} style={{ color: "var(--fill-tertiary)" }} />
                  <p className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
                    {t("conn_scanQr")}
                  </p>
                </>
              )}
            </div>
          </div>
        )}

        {step === "verify_code" && (
          <div className="flex flex-col items-center gap-4 py-4">
            <div
              className="flex h-12 w-12 items-center justify-center rounded-full"
              style={{ background: "rgba(237,137,54,0.1)" }}
            >
              <Key size={22} style={{ color: "var(--yellow)" }} />
            </div>
            <p className="text-center text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              {message}
            </p>
            <input
              value={verifyCode}
              onChange={(e) => setVerifyCode(e.target.value)}
              placeholder={t("conn_enterCode")}
              className="w-32 rounded-[var(--radius-sm)] px-3 py-2 text-center text-[16px] font-mono tracking-wider outline-none"
              style={inputStyle}
              autoFocus
              onKeyDown={(e) => e.key === "Enter" && submitVerifyCode()}
            />
            <button
              onClick={submitVerifyCode}
              disabled={!verifyCode.trim()}
              className="rounded-[var(--radius-sm)] px-4 py-1.5 text-[12px] font-medium transition-colors disabled:opacity-40"
              style={{ background: "var(--tint)", color: "#fff" }}
            >
              {t("conn_submit")}
            </button>
          </div>
        )}

        {step === "confirmed" && (
          <div className="flex flex-col items-center gap-3 py-8">
            <CheckCircle size={32} style={{ color: "var(--green)" }} />
            <p className="text-[14px] font-medium" style={{ color: "var(--green)" }}>
              {message}
            </p>
          </div>
        )}

        {step === "error" && (
          <div className="flex flex-col items-center gap-4 py-6">
            <p className="text-[13px]" style={{ color: "var(--red)" }}>
              {message}
            </p>
            <button
              onClick={startLogin}
              className="rounded-[var(--radius-sm)] px-4 py-1.5 text-[12px] font-medium transition-colors"
              style={{ background: "var(--tint)", color: "#fff" }}
            >
              {t("retry")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function SectionLabel({ label }: { label: string }) {
  return (
    <div
      className="mb-2 text-[11px] font-medium uppercase tracking-wider"
      style={{ color: "var(--fill-quaternary)" }}
    >
      {label}
    </div>
  );
}

function McpDetailModal({
  open,
  serverId,
  onClose,
  onReload,
  onRemove,
}: {
  open: boolean;
  serverId: string;
  onClose: () => void;
  onReload: () => void;
  onRemove: (id: string) => void;
}) {
  const { t } = useTranslation("common");
  const [data, setData] = useState<McpDetailResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [toolSearch, setToolSearch] = useState("");
  const [toolsExpanded, setToolsExpanded] = useState(true);

  useEffect(() => {
    if (!open || !serverId) return;
    setLoading(true);
    setToolSearch("");
    setToolsExpanded(true);
    api.mcpDetail(serverId).then((d) => {
      setData(d);
      setLoading(false);
    });
  }, [open, serverId]);

  if (!open) return null;

  const filteredTools = data
    ? data.tools.filter(
        (t) =>
          !toolSearch ||
          t.name.toLowerCase().includes(toolSearch.toLowerCase()) ||
          (t.description && t.description.toLowerCase().includes(toolSearch.toLowerCase())),
      )
    : [];

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
      onClick={onClose}
    >
      <div
        className="flex max-h-[80vh] w-[480px] flex-col rounded-[var(--radius-lg)]"
        style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--border-subtle)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 pt-5 pb-3">
          <div className="flex items-center gap-2">
            <HardDrives size={16} style={{ color: "var(--fill-secondary)" }} />
            <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              {serverId}
            </h3>
            {data && (
              <span
                className="rounded-full px-1.5 py-0.5 text-[10px]"
                style={{
                  background:
                    data.status === "connected"
                      ? "rgba(72,187,120,0.12)"
                      : "var(--bg-tertiary)",
                  color:
                    data.status === "connected" ? "var(--green)" : "var(--fill-quaternary)",
                }}
              >
                {data.status}
              </span>
            )}
            {data?.config.source === "project" && (
              <span
                className="rounded-full px-1.5 py-0.5 text-[10px]"
                style={{ background: "rgba(99,179,237,0.12)", color: "var(--tint)" }}
              >
                {t("conn_projectConfig")}
              </span>
            )}
          </div>
          <button onClick={onClose} style={{ color: "var(--fill-tertiary)" }}>
            <X size={ICON_SIZE.md} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-5 pb-5">
          {loading ? (
            <div className="flex items-center justify-center py-12">
              <SpinnerGap size={20} className="animate-spin" style={{ color: "var(--tint)" }} />
            </div>
          ) : data ? (
            <div className="flex flex-col gap-4">
              {data.error && (
                <div
                  className="rounded-[var(--radius-sm)] px-3 py-2 text-[12px]"
                  style={{ background: "rgba(229,62,62,0.08)", color: "var(--red)" }}
                >
                  {data.error}
                </div>
              )}

              <div>
                <SectionLabel label={t("conn_config")} />
                <div
                  className="flex flex-col gap-1.5 rounded-[var(--radius-sm)] p-3 text-[12px] font-mono"
                  style={{ background: "var(--bg-primary)", border: "0.5px solid var(--border-subtle)" }}
                >
                  <div className="flex gap-2">
                    <span style={{ color: "var(--fill-quaternary)" }}>{t("conn_command")}</span>
                    <span style={{ color: "var(--fill-primary)" }}>{data.config.command || "—"}</span>
                  </div>
                  {data.config.args.length > 0 && (
                    <div className="flex gap-2">
                      <span style={{ color: "var(--fill-quaternary)" }}>{t("conn_args")}</span>
                      <span style={{ color: "var(--fill-primary)" }}>{data.config.args.join(" ")}</span>
                    </div>
                  )}
                  <div className="flex gap-2">
                    <span style={{ color: "var(--fill-quaternary)" }}>{t("conn_transport")}</span>
                    <span style={{ color: "var(--fill-primary)" }}>{data.config.transport}</span>
                  </div>
                  {data.config.url && (
                    <div className="flex gap-2">
                      <span style={{ color: "var(--fill-quaternary)" }}>URL</span>
                      <span style={{ color: "var(--fill-primary)" }}>{data.config.url}</span>
                    </div>
                  )}
                  {Object.keys(data.config.env).length > 0 && (
                    <div className="mt-1 border-t border-[var(--border-subtle)] pt-1.5">
                      <span style={{ color: "var(--fill-quaternary)" }}>{t("conn_envVars")}</span>
                      {Object.entries(data.config.env).map(([k, v]) => (
                        <div key={k} className="ml-2 flex gap-2">
                          <span style={{ color: "var(--fill-tertiary)" }}>{k}=</span>
                          <span style={{ color: "var(--fill-primary)" }}>{v}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>

              <div>
                <div className="mb-2 flex items-center justify-between">
                  <button
                    className="flex items-center gap-1"
                    onClick={() => setToolsExpanded((v) => !v)}
                    style={{ color: "var(--fill-quaternary)" }}
                  >
                    <span className="text-[11px] font-medium uppercase tracking-wider">
                      {t("conn_tools", { count: data.tools.length })}
                    </span>
                    {toolsExpanded ? (
                      <CaretUp size={12} />
                    ) : (
                      <CaretDown size={12} />
                    )}
                  </button>
                  {toolsExpanded && data.tools.length > 5 && (
                    <div
                      className="flex items-center gap-1 rounded-[var(--radius-xs)] px-2 py-1"
                      style={{ background: "var(--bg-primary)", border: "0.5px solid var(--border-subtle)" }}
                    >
                      <MagnifyingGlass size={10} style={{ color: "var(--fill-quaternary)" }} />
                      <input
                        value={toolSearch}
                        onChange={(e) => setToolSearch(e.target.value)}
                        placeholder={t("conn_searchPlaceholder")}
                        className="w-24 bg-transparent text-[11px] outline-none"
                        style={{ color: "var(--fill-primary)" }}
                      />
                    </div>
                  )}
                </div>
                {toolsExpanded && (
                  <div
                    className="max-h-[240px] overflow-y-auto rounded-[var(--radius-sm)]"
                    style={{ background: "var(--bg-primary)", border: "0.5px solid var(--border-subtle)" }}
                  >
                    {filteredTools.length === 0 ? (
                      <div className="py-6 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
                        {toolSearch ? t("conn_noMatchingTools") : t("conn_noTools")}
                      </div>
                    ) : (
                      filteredTools.map((t, i) => (
                        <div
                          key={t.name}
                          className="flex items-start gap-2 px-3 py-2"
                          style={{
                            borderBottom:
                              i < filteredTools.length - 1
                                ? "0.5px solid var(--border-subtle)"
                                : undefined,
                          }}
                        >
                          <Terminal size={12} className="mt-0.5 shrink-0" style={{ color: "var(--fill-quaternary)" }} />
                          <div>
                            <div className="text-[12px] font-medium" style={{ color: "var(--fill-primary)" }}>
                              {t.name}
                            </div>
                            {t.description && (
                              <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                                {t.description}
                              </div>
                            )}
                          </div>
                        </div>
                      ))
                    )}
                  </div>
                )}
              </div>

              {data.connectedAt && (
                <div className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
                  {t("conn_connectedAt", { time: new Date(data.connectedAt).toLocaleString() })}
                </div>
              )}

              <div className="flex items-center gap-2 border-t border-[var(--border-subtle)] pt-3">
                <button
                  onClick={() => { onReload(); onClose(); }}
                  className="flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
                  style={{ color: "var(--fill-tertiary)" }}
                >
                  <ArrowsClockwise size={12} />
                  {t("conn_reload")}
                </button>
                {data?.config.source !== "project" && (
                  <button
                    onClick={() => { onRemove(serverId); onClose(); }}
                    className="flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--red)" }}
                  >
                    <Trash size={12} />
                    {t("delete")}
                  </button>
                )}
              </div>
            </div>
          ) : (
            <div className="py-8 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
              {t("conn_loadFailed")}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

const EDITABLE_CONFIG_KEYS = ["appId", "appSecret", "verificationToken", "encryptKey", "domain", "replyMode"];

function ChannelDetailModal({
  open,
  channelId,
  onClose,
  onConnect,
  onDisconnect,
  onUpdated,
}: {
  open: boolean;
  channelId: string;
  onClose: () => void;
  onConnect: (id: string) => void;
  onDisconnect: (id: string) => void;
  onUpdated: () => void;
}) {
  const { t } = useTranslation("common");
  const [data, setData] = useState<ChannelDetailResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editValues, setEditValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [saveMsg, setSaveMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [toolSearch, setToolSearch] = useState("");
  const [toolsExpanded, setToolsExpanded] = useState(true);

  useEffect(() => {
    if (!open || !channelId) return;
    setLoading(true);
    setEditing(false);
    setSaveMsg(null);
    setToolSearch("");
    setToolsExpanded(true);
    api.channelsDetail(channelId).then((d) => {
      setData(d);
      setLoading(false);
    });
  }, [open, channelId]);

  const startEdit = () => {
    if (!data) return;
    const vals: Record<string, string> = {};
    for (const k of EDITABLE_CONFIG_KEYS) {
      const v = data.config[k];
      vals[k] = v != null ? String(v) : "";
    }
    setEditValues(vals);
    setEditing(true);
    setSaveMsg(null);
  };

  const handleSave = async () => {
    setSaving(true);
    setSaveMsg(null);
    const config: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(editValues)) {
      if (v.trim()) config[k] = v.trim();
    }
    const result = await api.channelsUpdate(channelId, config);
    setSaving(false);
    if (result.ok) {
      setSaveMsg({ ok: true, text: t("conn_savedReloaded") });
      setEditing(false);
      onUpdated();
      api.channelsDetail(channelId).then(setData);
    } else {
      setSaveMsg({ ok: false, text: result.reloadError ?? t("conn_saveFailed") });
    }
  };

  const handleRestore = async () => {
    setRestoring(true);
    setSaveMsg(null);
    const result = await api.channelsRestore(channelId);
    setRestoring(false);
    if (result.ok) {
      setSaveMsg({ ok: true, text: t("conn_restoredReloaded") });
      setEditing(false);
      onUpdated();
      api.channelsDetail(channelId).then(setData);
    } else {
      setSaveMsg({ ok: false, text: result.reloadError ?? t("conn_restoreFailed") });
    }
  };

  if (!open) return null;

  const connected = data?.status === "connected";
  const statusCfg = data ? (STATUS_CONFIG[data.status] ?? STATUS_CONFIG.available) : null;
  const activeCaps = data
    ? Object.entries(data.capabilities ?? {})
        .filter(([, v]) => v)
        .map(([k]) => k)
    : [];
  const configEntries = data?.config
    ? Object.entries(data.config).filter(
        ([k, v]) => v != null && v !== "" && typeof v !== "object" && k !== "hasBackup",
      )
    : [];

  const inputStyle: React.CSSProperties = {
    background: "var(--bg-primary)",
    border: "0.5px solid var(--border-subtle)",
    color: "var(--fill-primary)",
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
      onClick={onClose}
    >
      <div
        className="flex max-h-[80vh] w-[480px] flex-col rounded-[var(--radius-lg)]"
        style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--border-subtle)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 pt-5 pb-3">
          <div className="flex items-center gap-2">
            <WifiHigh size={16} style={{ color: "var(--fill-secondary)" }} />
            <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              {data?.name ?? channelId}
            </h3>
            {statusCfg && (
              <span
                className="rounded-full px-1.5 py-0.5 text-[10px]"
                style={{ background: statusCfg.bg, color: statusCfg.fg }}
              >
                {t(statusCfg.labelKey)}
              </span>
            )}
            {data?.connectionMode && (
              <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                {data.connectionMode}
              </span>
            )}
          </div>
          <button onClick={onClose} style={{ color: "var(--fill-tertiary)" }}>
            <X size={ICON_SIZE.md} />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto px-5 pb-5">
          {loading ? (
            <div className="flex items-center justify-center py-12">
              <SpinnerGap size={20} className="animate-spin" style={{ color: "var(--tint)" }} />
            </div>
          ) : data ? (
            <div className="flex flex-col gap-4">
              <p className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
                {data.description}
              </p>

              {data.aliases.length > 0 && (
                <div className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
                  {t("conn_aliases", { aliases: data.aliases.join(", ") })}
                </div>
              )}

              {activeCaps.length > 0 && (
                <div>
                  <SectionLabel label={t("conn_capabilities")} />
                  <div className="flex flex-wrap gap-1.5">
                    {activeCaps.map((cap) => (
                      <span
                        key={cap}
                        className="rounded-full px-2 py-0.5 text-[11px]"
                        style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)" }}
                      >
                        {t(CAP_LABELS[cap] ?? cap)}
                      </span>
                    ))}
                  </div>
                </div>
              )}

              <div>
                <div className="mb-2 flex items-center justify-between">
                  <SectionLabel label={t("conn_config")} />
                  {!editing && (
                    <button
                      onClick={startEdit}
                      className="flex items-center gap-1 rounded-[var(--radius-xs)] px-1.5 py-0.5 text-[10px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
                      style={{ color: "var(--fill-tertiary)" }}
                    >
                      <PencilSimple size={10} />
                      {t("conn_edit")}
                    </button>
                  )}
                </div>

                {editing ? (
                  <div
                    className="flex flex-col gap-2 rounded-[var(--radius-sm)] p-3"
                    style={{ background: "var(--bg-primary)", border: "0.5px solid var(--tint)" }}
                  >
                    {EDITABLE_CONFIG_KEYS.map((k) => (
                      <div key={k}>
                        <label className="mb-0.5 block text-[10px] font-medium" style={{ color: "var(--fill-quaternary)" }}>
                          {k}
                        </label>
                        <input
                          value={editValues[k] ?? ""}
                          onChange={(e) => setEditValues((prev) => ({ ...prev, [k]: e.target.value }))}
                          className="w-full rounded-[var(--radius-xs)] px-2 py-1.5 text-[12px] font-mono outline-none"
                          style={inputStyle}
                          placeholder={k.includes("Secret") || k.includes("Key") || k.includes("Token") ? "••••••" : ""}
                        />
                      </div>
                    ))}
                    <div className="mt-1 flex items-center gap-2">
                      <button
                        onClick={handleSave}
                        disabled={saving}
                        className="flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[11px] font-medium transition-colors disabled:opacity-40"
                        style={{ background: "var(--tint)", color: "#fff" }}
                      >
                        <FloppyDisk size={11} />
                        {saving ? t("conn_saving") : t("conn_saveReload")}
                      </button>
                      {data.hasBackup && (
                        <button
                          onClick={handleRestore}
                          disabled={restoring}
                          className="flex items-center gap-1 rounded-[var(--radius-sm)] px-2 py-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-40"
                          style={{ color: "var(--fill-tertiary)" }}
                          title={t("conn_restoreBackupTitle")}
                        >
                          <ArrowCounterClockwise size={11} />
                          {t("conn_restoreBackup")}
                        </button>
                      )}
                      <button
                        onClick={() => { setEditing(false); setSaveMsg(null); }}
                        className="ml-auto text-[11px] transition-colors hover:bg-[var(--bg-hover)]"
                        style={{ color: "var(--fill-quaternary)" }}
                      >
                        {t("cancel")}
                      </button>
                    </div>
                  </div>
                ) : configEntries.length > 0 ? (
                  <div
                    className="flex flex-col gap-1.5 rounded-[var(--radius-sm)] p-3 text-[12px] font-mono"
                    style={{ background: "var(--bg-primary)", border: "0.5px solid var(--border-subtle)" }}
                  >
                    {configEntries.map(([k, v]) => (
                      <div key={k} className="flex gap-2">
                        <span style={{ color: "var(--fill-quaternary)" }}>{k}</span>
                        <span style={{ color: "var(--fill-primary)" }}>{String(v)}</span>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div
                    className="rounded-[var(--radius-sm)] py-4 text-center text-[11px]"
                    style={{ background: "var(--bg-primary)", border: "0.5px solid var(--border-subtle)", color: "var(--fill-quaternary)" }}
                  >
                    {t("conn_notConfigured")}
                  </div>
                )}
              </div>

              {saveMsg && (
                <div
                  className="rounded-[var(--radius-sm)] px-3 py-2 text-[11px]"
                  style={{
                    background: saveMsg.ok ? "rgba(72,187,120,0.08)" : "rgba(229,62,62,0.08)",
                    color: saveMsg.ok ? "var(--green)" : "var(--red)",
                  }}
                >
                  {saveMsg.text}
                </div>
              )}

              {data.tools.length > 0 && (() => {
                const filteredChTools = data.tools.filter(
                  (t) =>
                    !toolSearch ||
                    t.name.toLowerCase().includes(toolSearch.toLowerCase()) ||
                    (t.description && t.description.toLowerCase().includes(toolSearch.toLowerCase())),
                );
                return (
                  <div>
                    <div className="mb-2 flex items-center justify-between">
                      <button
                        className="flex items-center gap-1"
                        onClick={() => setToolsExpanded((v) => !v)}
                        style={{ color: "var(--fill-quaternary)" }}
                      >
                        <span className="text-[11px] font-medium uppercase tracking-wider">
                          {t("conn_tools", { count: data.tools.length })}
                        </span>
                        {toolsExpanded ? (
                          <CaretUp size={12} />
                        ) : (
                          <CaretDown size={12} />
                        )}
                      </button>
                      {toolsExpanded && data.tools.length > 5 && (
                        <div
                          className="flex items-center gap-1 rounded-[var(--radius-xs)] px-2 py-1"
                          style={{ background: "var(--bg-primary)", border: "0.5px solid var(--border-subtle)" }}
                        >
                          <MagnifyingGlass size={10} style={{ color: "var(--fill-quaternary)" }} />
                          <input
                            value={toolSearch}
                            onChange={(e) => setToolSearch(e.target.value)}
                            placeholder={t("conn_searchPlaceholder")}
                            className="w-24 bg-transparent text-[11px] outline-none"
                            style={{ color: "var(--fill-primary)" }}
                          />
                        </div>
                      )}
                    </div>
                    {toolsExpanded && (
                      <div
                        className="max-h-[200px] overflow-y-auto rounded-[var(--radius-sm)]"
                        style={{ background: "var(--bg-primary)", border: "0.5px solid var(--border-subtle)" }}
                      >
                        {filteredChTools.length === 0 ? (
                          <div className="py-6 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
                            {toolSearch ? t("conn_noMatchingTools") : t("conn_noTools")}
                          </div>
                        ) : (
                          filteredChTools.map((t, i) => (
                            <div
                              key={t.name}
                              className="flex items-start gap-2 px-3 py-2"
                              style={{
                                borderBottom:
                                  i < filteredChTools.length - 1
                                    ? "0.5px solid var(--border-subtle)"
                                    : undefined,
                              }}
                            >
                              <Terminal size={12} className="mt-0.5 shrink-0" style={{ color: "var(--fill-quaternary)" }} />
                              <div>
                                <div className="text-[12px] font-medium" style={{ color: "var(--fill-primary)" }}>
                                  {t.name}
                                </div>
                                {t.description && (
                                  <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                                    {t.description}
                                  </div>
                                )}
                              </div>
                            </div>
                          ))
                        )}
                      </div>
                    )}
                  </div>
                );
              })()}

              <div className="flex items-center gap-2 border-t border-[var(--border-subtle)] pt-3">
                {connected ? (
                  <button
                    onClick={() => { onDisconnect(channelId); onClose(); }}
                    className="flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--red)" }}
                  >
                    <LinkBreak size={12} />
                    {t("conn_disconnectConn")}
                  </button>
                ) : configEntries.length > 0 ? (
                  <button
                    onClick={() => { onConnect(channelId); onClose(); }}
                    className="flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors"
                    style={{ background: "var(--tint)", color: "#fff" }}
                  >
                    <Link size={12} />
                    {t("conn_connect")}
                  </button>
                ) : (
                  <button
                    onClick={startEdit}
                    className="flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors"
                    style={{ background: "var(--tint)", color: "#fff" }}
                  >
                    <PencilSimple size={12} />
                    {t("conn_configAndConnect")}
                  </button>
                )}
              </div>
            </div>
          ) : (
            <div className="py-8 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
              {t("conn_loadFailed")}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function EmptyState({
  icon: Icon,
  text,
}: {
  icon: React.ComponentType<{ size?: number; weight?: import("@phosphor-icons/react").IconWeight }>;
  text: string;
}) {
  return (
    <div className="flex flex-col items-center gap-2 py-8" style={{ color: "var(--fill-quaternary)" }}>
      <Icon size={24} />
      <p className="text-[12px]">{text}</p>
    </div>
  );
}

function AddMcpModal({
  open,
  onClose,
  onSubmit,
}: {
  open: boolean;
  onClose: () => void;
  onSubmit: (id: string, command: string, args: string[]) => void;
}) {
  const { t } = useTranslation("common");
  const [id, setId] = useState("");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [submitting, setSubmitting] = useState(false);

  if (!open) return null;

  const handleSubmit = async () => {
    if (!id.trim() || !command.trim()) return;
    setSubmitting(true);
    const argList = args
      .split(",")
      .map((a) => a.trim())
      .filter(Boolean);
    onSubmit(id.trim(), command.trim(), argList);
    setId("");
    setCommand("");
    setArgs("");
    setSubmitting(false);
    onClose();
  };

  const inputStyle: React.CSSProperties = {
    background: "var(--bg-primary)",
    border: "0.5px solid var(--border-subtle)",
    color: "var(--fill-primary)",
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
      onClick={onClose}
    >
      <div
        className="w-[400px] rounded-[var(--radius-lg)] p-5"
        style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--border-subtle)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {t("conn_addMcp")}
          </h3>
          <button onClick={onClose} style={{ color: "var(--fill-tertiary)" }}>
            <X size={ICON_SIZE.md} />
          </button>
        </div>

        <div className="flex flex-col gap-3">
          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              {t("conn_identifier")}
            </label>
            <input
              value={id}
              onChange={(e) => setId(e.target.value)}
              placeholder="my-mcp-server"
              className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
              style={inputStyle}
            />
          </div>
          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              {t("conn_startCommand")}
            </label>
            <input
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder="npx @anthropic/mcp-server"
              className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
              style={inputStyle}
            />
          </div>
          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              {t("conn_argsComma")}
            </label>
            <input
              value={args}
              onChange={(e) => setArgs(e.target.value)}
              placeholder="--port, 3000"
              className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
              style={inputStyle}
            />
          </div>
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <button
            onClick={onClose}
            className="rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] transition-colors"
            style={{ color: "var(--fill-tertiary)" }}
          >
            {t("cancel")}
          </button>
          <button
            onClick={handleSubmit}
            disabled={!id.trim() || !command.trim() || submitting}
            className="rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors disabled:opacity-40"
            style={{ background: "var(--tint)", color: "#fff" }}
          >
            {submitting ? t("conn_adding") : t("conn_add")}
          </button>
        </div>
      </div>
    </div>
  );
}

export function ConnectionsPage() {
  const { t } = useTranslation("common");
  const [mcpServers, setMcpServers] = useState<McpServerStatus[]>([]);
  const [channels, setChannels] = useState<ChannelStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [reloading, setReloading] = useState(false);
  const [showAddModal, setShowAddModal] = useState(false);
  const [showWechatQr, setShowWechatQr] = useState(false);
  const [mcpDetailId, setMcpDetailId] = useState<string | null>(null);
  const [channelDetailId, setChannelDetailId] = useState<string | null>(null);

  const fetchData = useCallback(async () => {
    const [mcp, ch] = await Promise.all([api.getMcpStatus(), api.listChannels()]);
    setMcpServers(mcp);
    setChannels(ch);
    setLoading(false);
  }, []);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  const handleReload = async () => {
    setReloading(true);
    try {
      const servers = await api.reloadMcpServers();
      setMcpServers(servers);
    } catch (e) {
      console.warn("[connections] reload error:", e);
    }
    setReloading(false);
  };

  const handleAddMcp = async (id: string, command: string, args: string[]) => {
    try {
      await api.addMcpServer(id, command, args);
      const servers = await api.getMcpStatus();
      setMcpServers(servers);
    } catch (e) {
      console.warn("[connections] add mcp error:", e);
    }
  };

  const handleRemoveMcp = async (id: string) => {
    try {
      await api.removeMcpServer(id);
      setMcpServers((prev) => prev.filter((s) => s.id !== id));
    } catch (e) {
      console.warn("[connections] remove mcp error:", e);
    }
  };

  const handleConnect = async (ch: ChannelStatus) => {
    if (ch.id === "wechat") {
      setShowWechatQr(true);
      return;
    }
    if (ch.status === "configured") {
      try {
        const result = await api.channelsConnect(ch.id);
        if (result.ok) {
          const updated = await api.listChannels();
          setChannels(updated);
        } else {
          setChannelDetailId(ch.id);
        }
      } catch {
        setChannelDetailId(ch.id);
      }
    } else {
      setChannelDetailId(ch.id);
    }
  };

  const handleDisconnect = async (ch: ChannelStatus) => {
    try {
      await api.channelsDisconnect(ch.id);
      const updated = await api.listChannels();
      setChannels(updated);
    } catch (e) {
      console.warn("[connections] disconnect error:", e);
    }
  };

  const handleWechatSuccess = async () => {
    setShowWechatQr(false);
    const updated = await api.listChannels();
    setChannels(updated);
  };

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center" style={{ color: "var(--fill-quaternary)" }}>
        <ArrowsClockwise size={20} className="animate-spin" />
      </div>
    );
  }

  return (
    <div
      className="flex h-full flex-col overflow-y-auto"
      style={{ background: "var(--bg-primary)" }}
    >
      <div className="mx-auto w-full max-w-[640px] px-6 py-6">
        {/* MCP Servers */}
        <section className="mb-8">
          <div className="mb-3 flex items-center justify-between">
            <div className="flex items-center gap-2">
              <HardDrives size={ICON_SIZE.md} style={{ color: "var(--fill-secondary)" }} />
              <h2
                className="text-[14px] font-semibold tracking-[-0.01em]"
                style={{ color: "var(--fill-primary)" }}
              >
                {t("conn_mcpServers")}
              </h2>
              <span
                className="rounded-full px-1.5 py-0.5 text-[10px] font-medium"
                style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)" }}
              >
                {mcpServers.length}
              </span>
            </div>
            <div className="flex items-center gap-1">
              <button
                onClick={handleReload}
                disabled={reloading}
                className="flex items-center gap-1 rounded-[var(--radius-sm)] px-2 py-1 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--fill-tertiary)" }}
                title={t("conn_reloadAllTitle")}
              >
                <ArrowsClockwise
                  size={12}
                 
                  className={reloading ? "animate-spin" : ""}
                />
                {t("conn_reloadAll")}
              </button>
              <button
                onClick={() => setShowAddModal(true)}
                className="flex items-center gap-1 rounded-[var(--radius-sm)] px-2 py-1 text-[11px] font-medium transition-colors"
                style={{ background: "var(--tint)", color: "#fff" }}
              >
                <Plus size={12} />
                {t("conn_add")}
              </button>
            </div>
          </div>

          {mcpServers.length === 0 ? (
            <EmptyState icon={WifiSlash} text={t("conn_noMcp")} />
          ) : (
            <div className="flex flex-col gap-2">
              {mcpServers.map((s) => (
                <div key={s.id} className="group">
                  <McpCard server={s} onRemove={handleRemoveMcp} onClick={setMcpDetailId} />
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Channels */}
        <section>
          <div className="mb-3 flex items-center gap-2">
            <WifiHigh size={ICON_SIZE.md} style={{ color: "var(--fill-secondary)" }} />
            <h2
              className="text-[14px] font-semibold tracking-[-0.01em]"
              style={{ color: "var(--fill-primary)" }}
            >
              {t("conn_channels")}
            </h2>
            <span
              className="rounded-full px-1.5 py-0.5 text-[10px] font-medium"
              style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)" }}
            >
              {channels.length}
            </span>
          </div>

          {channels.length === 0 ? (
            <EmptyState icon={WifiSlash} text={t("conn_noChannels")} />
          ) : (
            <div className="flex flex-col gap-2">
              {channels.map((ch) => (
                <ChannelCard
                  key={ch.id}
                  channel={ch}
                  onConnect={handleConnect}
                  onDisconnect={handleDisconnect}
                  onClick={setChannelDetailId}
                />
              ))}
            </div>
          )}
        </section>
      </div>

      <AddMcpModal
        open={showAddModal}
        onClose={() => setShowAddModal(false)}
        onSubmit={handleAddMcp}
      />

      <WechatQrModal
        open={showWechatQr}
        onClose={() => setShowWechatQr(false)}
        onSuccess={handleWechatSuccess}
      />

      <McpDetailModal
        open={mcpDetailId !== null}
        serverId={mcpDetailId ?? ""}
        onClose={() => setMcpDetailId(null)}
        onReload={handleReload}
        onRemove={handleRemoveMcp}
      />

      <ChannelDetailModal
        open={channelDetailId !== null}
        channelId={channelDetailId ?? ""}
        onClose={() => setChannelDetailId(null)}
        onConnect={(id) => {
          const ch = channels.find((c) => c.id === id);
          if (ch) handleConnect(ch);
        }}
        onDisconnect={(id) => {
          const ch = channels.find((c) => c.id === id);
          if (ch) handleDisconnect(ch);
        }}
        onUpdated={fetchData}
      />
    </div>
  );
}
