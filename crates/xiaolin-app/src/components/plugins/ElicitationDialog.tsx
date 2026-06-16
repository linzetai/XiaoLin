import { useState, useEffect, useCallback, useRef } from "react";
import { X, PaperPlaneTilt } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import * as wsClient from "../../lib/ws-client";
import * as transport from "../../lib/transport";

interface ElicitationRequest {
  elicitationId: string;
  serverId: string;
  serverName: string;
  message: string;
  requestedSchema: {
    type?: string;
    properties?: Record<
      string,
      {
        type?: string;
        description?: string;
        enum?: string[];
        oneOf?: { const: string; title?: string }[];
        default?: unknown;
      }
    >;
    required?: string[];
  };
}

export function ElicitationDialog() {
  const { t } = useTranslation("plugins");
  const [request, setRequest] = useState<ElicitationRequest | null>(null);
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [submitting, setSubmitting] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const unsub = wsClient.on("mcp.elicitation.request", (msg: unknown) => {
      const d = ((msg as Record<string, unknown>)?.data ?? msg) as ElicitationRequest;
      setRequest(d);
      const initial: Record<string, unknown> = {};
      const props = d.requestedSchema?.properties;
      if (props) {
        for (const [key, schema] of Object.entries(props)) {
          if (schema.default !== undefined) {
            initial[key] = schema.default;
          } else if (schema.type === "boolean") {
            initial[key] = false;
          } else if (schema.type === "number") {
            initial[key] = 0;
          } else {
            initial[key] = "";
          }
        }
      }
      setValues(initial);
      setSubmitting(false);
    });

    const unsubTimeout = wsClient.on("mcp.elicitation.timeout", (msg: unknown) => {
      const d = ((msg as Record<string, unknown>)?.data ?? msg) as { elicitationId: string };
      setRequest((prev) => (prev?.elicitationId === d.elicitationId ? null : prev));
    });

    return () => {
      unsub();
      unsubTimeout();
    };
  }, []);

  const handleSubmit = useCallback(async () => {
    if (!request) return;
    setSubmitting(true);
    try {
      await transport.mcpElicitationReply(request.elicitationId, "accept", values);
    } catch (e) {
      console.error("elicitation reply failed:", e);
    }
    setRequest(null);
    setSubmitting(false);
  }, [request, values]);

  const handleDecline = useCallback(async () => {
    if (!request) return;
    setSubmitting(true);
    try {
      await transport.mcpElicitationReply(request.elicitationId, "decline");
    } catch (e) {
      console.error("elicitation decline failed:", e);
    }
    setRequest(null);
    setSubmitting(false);
  }, [request]);

  if (!request) return null;

  const properties = request.requestedSchema?.properties ?? {};
  const required = new Set(request.requestedSchema?.required ?? []);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.45)", backdropFilter: "blur(4px)" }}
      onClick={(e) => {
        if (e.target === e.currentTarget) handleDecline();
      }}
    >
      <div
        ref={dialogRef}
        className="relative flex max-h-[80vh] w-full max-w-md flex-col overflow-hidden rounded-xl shadow-2xl"
        style={{
          background: "var(--bg-primary)",
          border: "1px solid var(--border-primary)",
        }}
      >
        {/* Header */}
        <div
          className="flex items-center justify-between px-5 py-4"
          style={{ borderBottom: "1px solid var(--border-secondary)" }}
        >
          <div className="flex-1 min-w-0">
            <h3
              className="text-[14px] font-semibold truncate"
              style={{ color: "var(--fill-primary)" }}
            >
              {request.serverName}
            </h3>
            <p
              className="mt-0.5 text-[12px]"
              style={{ color: "var(--fill-tertiary)" }}
            >
              {t("elicitation.title")}
            </p>
          </div>
          <button
            onClick={handleDecline}
            className="ml-3 flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors hover:opacity-70"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <X size={16} />
          </button>
        </div>

        {/* Message */}
        {request.message && (
          <div className="px-5 pt-4">
            <p
              className="text-[13px] leading-relaxed"
              style={{ color: "var(--fill-secondary)" }}
            >
              {request.message}
            </p>
          </div>
        )}

        {/* Form */}
        <div className="flex-1 overflow-y-auto px-5 py-4">
          <div className="flex flex-col gap-3">
            {Object.entries(properties).map(([key, schema]) => (
              <FieldRenderer
                key={key}
                name={key}
                schema={schema}
                required={required.has(key)}
                value={values[key]}
                onChange={(v) =>
                  setValues((prev) => ({ ...prev, [key]: v }))
                }
              />
            ))}
          </div>
        </div>

        {/* Footer */}
        <div
          className="flex items-center justify-between px-5 py-3"
          style={{ borderTop: "1px solid var(--border-secondary)" }}
        >
          <span
            className="text-[11px]"
            style={{ color: "var(--fill-quaternary)" }}
          >
            {t("elicitation.timeout_hint", { minutes: 5 })}
          </span>
          <div className="flex gap-2">
            <button
              onClick={handleDecline}
              disabled={submitting}
              className="rounded-md px-3 py-1.5 text-[12px] font-medium transition-colors hover:opacity-80"
              style={{
                background: "var(--bg-tertiary)",
                color: "var(--fill-secondary)",
              }}
            >
              {t("elicitation.cancel")}
            </button>
            <button
              onClick={handleSubmit}
              disabled={submitting}
              className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[12px] font-medium transition-colors hover:opacity-90"
              style={{ background: "var(--tint)", color: "#fff" }}
            >
              <PaperPlaneTilt size={13} weight="bold" />
              {t("elicitation.submit")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function FieldRenderer({
  name,
  schema,
  required,
  value,
  onChange,
}: {
  name: string;
  schema: {
    type?: string;
    description?: string;
    enum?: string[];
    oneOf?: { const: string; title?: string }[];
    default?: unknown;
  };
  required: boolean;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const label = (
    <label
      className="mb-1 block text-[12px] font-medium"
      style={{ color: "var(--fill-secondary)" }}
    >
      {name}
      {required && (
        <span className="ml-0.5" style={{ color: "var(--red)" }}>
          *
        </span>
      )}
      {schema.description && (
        <span
          className="ml-1.5 font-normal"
          style={{ color: "var(--fill-quaternary)" }}
        >
          {schema.description}
        </span>
      )}
    </label>
  );

  const inputStyle: React.CSSProperties = {
    background: "var(--bg-secondary)",
    border: "1px solid var(--border-primary)",
    color: "var(--fill-primary)",
    borderRadius: "var(--radius-xs)",
  };

  if (schema.enum || schema.oneOf) {
    const options = schema.oneOf
      ? schema.oneOf.map((o) => ({ value: o.const, label: o.title ?? o.const }))
      : (schema.enum ?? []).map((e) => ({ value: e, label: e }));

    return (
      <div>
        {label}
        <select
          value={String(value ?? "")}
          onChange={(e) => onChange(e.target.value)}
          className="w-full px-2.5 py-1.5 text-[13px]"
          style={inputStyle}
        >
          <option value="">—</option>
          {options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </div>
    );
  }

  if (schema.type === "boolean") {
    return (
      <div className="flex items-center gap-2">
        <input
          type="checkbox"
          checked={Boolean(value)}
          onChange={(e) => onChange(e.target.checked)}
          className="h-4 w-4 rounded"
          style={{ accentColor: "var(--tint)" }}
        />
        {label}
      </div>
    );
  }

  if (schema.type === "number") {
    return (
      <div>
        {label}
        <input
          type="number"
          value={String(value ?? "")}
          onChange={(e) => onChange(Number(e.target.value))}
          className="w-full px-2.5 py-1.5 text-[13px]"
          style={inputStyle}
        />
      </div>
    );
  }

  return (
    <div>
      {label}
      <input
        type="text"
        value={String(value ?? "")}
        onChange={(e) => onChange(e.target.value)}
        className="w-full px-2.5 py-1.5 text-[13px]"
        style={inputStyle}
      />
    </div>
  );
}
