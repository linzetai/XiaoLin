import { useState, useCallback, useMemo, useRef, useEffect } from "react";
import { X, Plus, Trash, Terminal, Globe, Lightning, ArrowsClockwise } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { usePluginStore } from "../../lib/stores/plugin-store";
import { ICON_SIZE, BTN_PRIMARY_SM } from "../../lib/ui-tokens";

type TransportType = "stdio" | "sse" | "streamable_http" | "websocket";

interface KvEntry {
  key: string;
  value: string;
}

interface AddServerModalProps {
  open: boolean;
  onClose: () => void;
  prefill?: {
    id?: string;
    command?: string;
    args?: string[];
    transport?: TransportType;
    url?: string;
  };
}

const TRANSPORT_OPTIONS: { value: TransportType; label: string; icon: typeof Terminal }[] = [
  { value: "stdio", label: "Stdio", icon: Terminal },
  { value: "sse", label: "SSE", icon: Globe },
  { value: "streamable_http", label: "Streamable HTTP", icon: Lightning },
  { value: "websocket", label: "WebSocket", icon: ArrowsClockwise },
];

export function AddServerModal({ open, onClose, prefill }: AddServerModalProps) {
  const { t } = useTranslation("plugins");
  const addPlugin = usePluginStore((s) => s.addPlugin);
  const plugins = usePluginStore((s) => s.plugins);

  const [transport, setTransport] = useState<TransportType>(prefill?.transport ?? "stdio");
  const [serverId, setServerId] = useState(prefill?.id ?? "");
  const [command, setCommand] = useState(prefill?.command ?? "");
  const [args, setArgs] = useState(prefill?.args?.join(" ") ?? "");
  const [url, setUrl] = useState(prefill?.url ?? "");
  const [envEntries, setEnvEntries] = useState<KvEntry[]>([]);
  const [bearerTokenEnvVar, setBearerTokenEnvVar] = useState("");
  const [headerEntries, setHeaderEntries] = useState<KvEntry[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const idInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open && idInputRef.current) {
      setTimeout(() => idInputRef.current?.focus(), 100);
    }
  }, [open]);

  useEffect(() => {
    if (open && prefill) {
      setServerId(prefill.id ?? "");
      setCommand(prefill.command ?? "");
      setArgs(prefill.args?.join(" ") ?? "");
      setTransport(prefill.transport ?? "stdio");
      setUrl(prefill.url ?? "");
    }
  }, [open, prefill]);

  const idError = useMemo(() => {
    if (!serverId) return null;
    if (serverId.includes("__")) return t("add_modal.id_error_double_underscore");
    return null;
  }, [serverId, t]);

  const idExists = useMemo(
    () => plugins.some((p) => p.id === serverId),
    [plugins, serverId],
  );

  const isStdio = transport === "stdio";
  const canSubmit = useMemo(() => {
    if (!serverId.trim() || idError || idExists) return false;
    if (isStdio && !command.trim()) return false;
    if (!isStdio && !url.trim()) return false;
    return true;
  }, [serverId, idError, idExists, isStdio, command, url]);

  const handleClose = useCallback(() => {
    if (submitting) return;
    setServerId("");
    setCommand("");
    setArgs("");
    setUrl("");
    setEnvEntries([]);
    setBearerTokenEnvVar("");
    setHeaderEntries([]);
    setError(null);
    setTransport("stdio");
    onClose();
  }, [submitting, onClose]);

  const handleSubmit = useCallback(async () => {
    if (!canSubmit || submitting) return;
    setSubmitting(true);
    setError(null);

    const env: Record<string, string> = {};
    for (const e of envEntries) {
      if (e.key.trim()) env[e.key.trim()] = e.value;
    }

    const httpHeaders: Record<string, string> = {};
    for (const h of headerEntries) {
      if (h.key.trim()) httpHeaders[h.key.trim()] = h.value;
    }

    const ok = await addPlugin({
      id: serverId.trim(),
      ...(isStdio
        ? {
            command: command.trim(),
            args: args.trim() ? args.trim().split(/\s+/) : [],
            transport: "stdio",
          }
        : {
            transport,
            url: url.trim(),
            ...(bearerTokenEnvVar.trim() ? { bearer_token_env_var: bearerTokenEnvVar.trim() } : {}),
            ...(Object.keys(httpHeaders).length > 0 ? { http_headers: httpHeaders } : {}),
          }),
      ...(Object.keys(env).length > 0 ? { env } : {}),
    });

    setSubmitting(false);
    if (ok) {
      handleClose();
    } else {
      setError(t("add_modal.submit_error"));
    }
  }, [canSubmit, submitting, addPlugin, serverId, isStdio, command, args, transport, url, envEntries, bearerTokenEnvVar, headerEntries, handleClose, t]);

  const addEnvRow = useCallback(() => {
    setEnvEntries((prev) => [...prev, { key: "", value: "" }]);
  }, []);

  const removeEnvRow = useCallback((index: number) => {
    setEnvEntries((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const updateEnv = useCallback((index: number, field: "key" | "value", val: string) => {
    setEnvEntries((prev) =>
      prev.map((e, i) => (i === index ? { ...e, [field]: val } : e)),
    );
  }, []);

  const addHeaderRow = useCallback(() => {
    setHeaderEntries((prev) => [...prev, { key: "", value: "" }]);
  }, []);

  const removeHeaderRow = useCallback((index: number) => {
    setHeaderEntries((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const updateHeader = useCallback((index: number, field: "key" | "value", val: string) => {
    setHeaderEntries((prev) =>
      prev.map((e, i) => (i === index ? { ...e, [field]: val } : e)),
    );
  }, []);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.45)" }}
      onClick={(e) => { if (e.target === e.currentTarget) handleClose(); }}
    >
      <div
        className="w-[480px] max-h-[85vh] flex flex-col rounded-xl overflow-hidden shadow-2xl"
        style={{ background: "var(--bg-card)", border: "0.5px solid var(--separator)" }}
      >
        {/* Header */}
        <div
          className="flex items-center justify-between px-5 py-3.5 shrink-0"
          style={{ borderBottom: "0.5px solid var(--separator)" }}
        >
          <h2 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {t("add_modal.title")}
          </h2>
          <button
            onClick={handleClose}
            className="flex items-center justify-center w-6 h-6 rounded-md transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)", background: "transparent", border: "none", cursor: "pointer" }}
          >
            <X size={ICON_SIZE.sm} />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4 flex flex-col gap-4">
          {/* Transport Selector */}
          <div className="flex flex-col gap-1.5">
            <label className="text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
              {t("add_modal.transport_label")}
            </label>
            <div className="flex gap-1.5">
              {TRANSPORT_OPTIONS.map((opt) => {
                const Icon = opt.icon;
                const active = transport === opt.value;
                return (
                  <button
                    key={opt.value}
                    onClick={() => setTransport(opt.value)}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[12px] font-medium transition-colors"
                    style={{
                      background: active ? "var(--tint)" : "var(--bg-tertiary)",
                      color: active ? "#fff" : "var(--fill-secondary)",
                      border: "none",
                      cursor: "pointer",
                    }}
                  >
                    <Icon size={ICON_SIZE.xs} weight={active ? "bold" : "regular"} />
                    {opt.label}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Server ID */}
          <FieldGroup label={t("add_modal.id_label")} required>
            <input
              ref={idInputRef}
              type="text"
              value={serverId}
              onChange={(e) => setServerId(e.target.value)}
              placeholder="my-mcp-server"
              className="modal-input"
            />
            {idError && <span className="text-[11px]" style={{ color: "var(--red)" }}>{idError}</span>}
            {!idError && idExists && (
              <span className="text-[11px]" style={{ color: "var(--orange)" }}>
                {t("add_modal.id_exists_hint")}
              </span>
            )}
          </FieldGroup>

          {/* Stdio fields */}
          {isStdio && (
            <>
              <FieldGroup label={t("add_modal.command_label")} required>
                <input
                  type="text"
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  placeholder="npx"
                  className="modal-input"
                />
              </FieldGroup>
              <FieldGroup label={t("add_modal.args_label")}>
                <input
                  type="text"
                  value={args}
                  onChange={(e) => setArgs(e.target.value)}
                  placeholder="-y @modelcontextprotocol/server-filesystem ."
                  className="modal-input"
                />
              </FieldGroup>
            </>
          )}

          {/* URL field for SSE / StreamableHTTP */}
          {!isStdio && (
            <FieldGroup label={t("add_modal.url_label")} required>
              <input
                type="url"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://api.example.com/mcp"
                className="modal-input"
              />
            </FieldGroup>
          )}

          {/* Bearer Token (HTTP only) */}
          {!isStdio && (
            <FieldGroup label={t("add_modal.bearer_label")}>
              <input
                type="text"
                value={bearerTokenEnvVar}
                onChange={(e) => setBearerTokenEnvVar(e.target.value)}
                placeholder="MY_MCP_TOKEN"
                className="modal-input"
                style={{ fontFamily: "var(--font-mono, monospace)", fontSize: "12px" }}
              />
              <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                {t("add_modal.bearer_hint")}
              </span>
            </FieldGroup>
          )}

          {/* Custom HTTP Headers (HTTP only) */}
          {!isStdio && (
            <div className="flex flex-col gap-1.5">
              <div className="flex items-center justify-between">
                <label className="text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
                  {t("add_modal.headers_label")}
                </label>
                <button
                  onClick={addHeaderRow}
                  className="flex items-center gap-1 text-[11px] font-medium transition-colors hover:opacity-80"
                  style={{ color: "var(--tint)", background: "transparent", border: "none", cursor: "pointer" }}
                >
                  <Plus size={ICON_SIZE.xs} />
                  {t("add_modal.headers_add")}
                </button>
              </div>
              {headerEntries.map((entry, i) => (
                <div key={i} className="flex gap-1.5 items-center">
                  <input
                    type="text"
                    value={entry.key}
                    onChange={(e) => updateHeader(i, "key", e.target.value)}
                    placeholder="X-Custom-Header"
                    className="modal-input flex-1"
                    style={{ fontFamily: "var(--font-mono, monospace)", fontSize: "12px" }}
                  />
                  <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>:</span>
                  <input
                    type="text"
                    value={entry.value}
                    onChange={(e) => updateHeader(i, "value", e.target.value)}
                    placeholder="value or $ENV_VAR"
                    className="modal-input flex-[2]"
                    style={{ fontFamily: "var(--font-mono, monospace)", fontSize: "12px" }}
                  />
                  <button
                    onClick={() => removeHeaderRow(i)}
                    className="flex items-center justify-center w-6 h-6 rounded-md transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--red)", background: "transparent", border: "none", cursor: "pointer", flexShrink: 0 }}
                  >
                    <Trash size={ICON_SIZE.xs} />
                  </button>
                </div>
              ))}
              {headerEntries.length > 0 && (
                <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                  {t("add_modal.headers_hint")}
                </span>
              )}
            </div>
          )}

          {/* Environment Variables */}
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between">
              <label className="text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
                {t("add_modal.env_label")}
              </label>
              <button
                onClick={addEnvRow}
                className="flex items-center gap-1 text-[11px] font-medium transition-colors hover:opacity-80"
                style={{ color: "var(--tint)", background: "transparent", border: "none", cursor: "pointer" }}
              >
                <Plus size={ICON_SIZE.xs} />
                {t("add_modal.env_add")}
              </button>
            </div>
            {envEntries.map((entry, i) => (
              <div key={i} className="flex gap-1.5 items-center">
                <input
                  type="text"
                  value={entry.key}
                  onChange={(e) => updateEnv(i, "key", e.target.value)}
                  placeholder="KEY"
                  className="modal-input flex-1"
                  style={{ fontFamily: "var(--font-mono, monospace)", fontSize: "12px" }}
                />
                <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>=</span>
                <input
                  type="text"
                  value={entry.value}
                  onChange={(e) => updateEnv(i, "value", e.target.value)}
                  placeholder="value"
                  className="modal-input flex-[2]"
                  style={{ fontFamily: "var(--font-mono, monospace)", fontSize: "12px" }}
                />
                <button
                  onClick={() => removeEnvRow(i)}
                  className="flex items-center justify-center w-6 h-6 rounded-md transition-colors hover:bg-[var(--bg-hover)]"
                  style={{ color: "var(--red)", background: "transparent", border: "none", cursor: "pointer", flexShrink: 0 }}
                >
                  <Trash size={ICON_SIZE.xs} />
                </button>
              </div>
            ))}
          </div>

          {/* Error */}
          {error && (
            <div className="text-[12px] px-3 py-2 rounded-md" style={{ color: "var(--red)", background: "var(--red-bg, rgba(239,68,68,0.08))" }}>
              {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div
          className="flex items-center justify-end gap-2 px-5 py-3 shrink-0"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          <button
            onClick={handleClose}
            className="px-3 py-1.5 rounded-md text-[12px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-secondary)", background: "transparent", border: "none", cursor: "pointer" }}
          >
            {t("add_modal.cancel")}
          </button>
          <button
            onClick={handleSubmit}
            disabled={!canSubmit || submitting}
            className={BTN_PRIMARY_SM}
            style={{ opacity: (!canSubmit || submitting) ? 0.5 : 1, cursor: (!canSubmit || submitting) ? "not-allowed" : "pointer" }}
          >
            {submitting ? t("add_modal.submitting") : t("add_modal.submit")}
          </button>
        </div>
      </div>

      <style>{`
        .modal-input {
          width: 100%;
          padding: 6px 10px;
          border-radius: var(--radius-xs, 6px);
          border: 0.5px solid var(--separator);
          background: var(--bg-tertiary);
          color: var(--fill-primary);
          font-size: 13px;
          outline: none;
          transition: border-color 150ms;
        }
        .modal-input:focus {
          border-color: var(--tint);
        }
        .modal-input::placeholder {
          color: var(--fill-quaternary);
        }
      `}</style>
    </div>
  );
}

function FieldGroup({ label, required, children }: { label: string; required?: boolean; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
        {label}
        {required && <span style={{ color: "var(--red)", marginLeft: 2 }}>*</span>}
      </label>
      {children}
    </div>
  );
}
