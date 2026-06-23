import { useState, useEffect, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle, XCircle, X, Plus, Info } from "@phosphor-icons/react";
import * as api from "../../lib/api";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import { usePermissionStore } from "../../lib/stores/permission-store";
import { SectionTitle } from "./SettingsShared";
import { inputCls as sharedInputCls, inputStyle as sharedInputStyle } from "../common/FormElements";

type DangerousOpsPolicy = "deny" | "allow" | "confirm";
type ExecutionMode = "plan" | "default" | "auto-edit" | "yolo";

const MANAGED_PATTERNS = ["write_file", "edit_file", "apply_patch", "multi_edit", "shell_exec"];

const MODE_TO_PRESET: Record<ExecutionMode, string> = {
  plan: "plan-only",
  default: "suggest",
  "auto-edit": "auto-edit",
  yolo: "full-auto",
};

function dedupe(values: string[]): string[] {
  return Array.from(new Set(values));
}

function stripManaged(values: string[] | undefined): string[] {
  return (values ?? []).filter((item) => !MANAGED_PATTERNS.includes(item));
}

function hasAll(haystack: string[] | undefined, needles: string[]): boolean {
  const set = new Set(haystack ?? []);
  return needles.every((item) => set.has(item));
}

function inferMode(behavior?: api.AgentBehaviorConfig): ExecutionMode {
  const ask = behavior?.toolsAsk ?? behavior?.requireConfirmationFor ?? [];
  const deny = behavior?.toolsDeny ?? [];
  const fileAccess = behavior?.fileAccess ?? "workspace";

  if (hasAll(deny, MANAGED_PATTERNS)) return "plan";

  const writePatternsInAsk = hasAll(ask, MANAGED_PATTERNS);
  const shellInAsk = ask.includes("shell_exec");
  const writeToolsInAsk = hasAll(ask, ["write_file", "edit_file", "apply_patch", "multi_edit"]);

  if (writePatternsInAsk) return "default";
  if (shellInAsk && !writeToolsInAsk && fileAccess === "full") return "auto-edit";
  if (!writePatternsInAsk && !shellInAsk && fileAccess === "full") return "yolo";
  if (shellInAsk && !writeToolsInAsk) return "auto-edit";
  return "default";
}

export function SecurityTab() {
  const { t } = useTranslation("settings");
  const activeAgentId = useChatMetaStore((s) => s.activeAgentId);

  const policyOptions = useMemo(() => [
    { value: "deny" as const, label: t("policy_deny"), desc: t("policy_denyDesc") },
    { value: "confirm" as const, label: t("policy_confirm"), desc: t("policy_confirmDesc") },
    { value: "allow" as const, label: t("policy_allow"), desc: t("policy_allowDesc") },
  ], [t]);

  const executionModeOptions = useMemo(() => [
    { value: "plan" as const, label: t("executionMode_plan"), desc: t("executionMode_planDesc") },
    { value: "default" as const, label: t("executionMode_default"), desc: t("executionMode_defaultDesc") },
    { value: "auto-edit" as const, label: t("executionMode_autoEdit"), desc: t("executionMode_autoEditDesc") },
    { value: "yolo" as const, label: t("executionMode_yolo"), desc: t("executionMode_yoloDesc") },
  ], [t]);
  const [allowedHosts, setAllowedHosts] = useState<string[]>([]);
  const [newHost, setNewHost] = useState("");
  const [opsPolicy, setOpsPolicy] = useState<DangerousOpsPolicy>("confirm");
  const [executionMode, setExecutionMode] = useState<ExecutionMode>("default");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [modeSaving, setModeSaving] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: "ok" | "err" } | null>(null);
  const sessionPresetIds = usePermissionStore((s) => s.sessionPresetIds);
  const sessionOverrideCount = useMemo(
    () => Object.keys(sessionPresetIds).filter(
      (sid) => sessionPresetIds[sid] !== MODE_TO_PRESET[executionMode]
    ).length,
    [sessionPresetIds, executionMode],
  );

  const showToast = useCallback((msg: string, type: "ok" | "err") => {
    setToast({ msg, type });
    setTimeout(() => setToast(null), 2500);
  }, []);

  useEffect(() => {
    api.getConfig("security").then((data) => {
      const cfg = data as { key?: string; value?: Record<string, unknown> } | null;
      const val = (cfg?.value ?? cfg) as Record<string, unknown> | null;
      if (val?.ssrfAllowedHosts && Array.isArray(val.ssrfAllowedHosts)) {
        setAllowedHosts(val.ssrfAllowedHosts as string[]);
      }
      if (val?.dangerousOpsPolicy && typeof val.dangerousOpsPolicy === "string") {
        setOpsPolicy(val.dangerousOpsPolicy as DangerousOpsPolicy);
      }
    }).catch(() => {}).finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    if (!activeAgentId) return;
    api.getAgent(activeAgentId)
      .then((agent) => setExecutionMode(inferMode(agent?.behavior)))
      .catch(() => {
        setExecutionMode("default");
      });
  }, [activeAgentId]);

  const persistSecurity = useCallback(async (patch: Record<string, unknown>) => {
    setSaving(true);
    try {
      await api.setConfig("security", patch);
      showToast(t("savedOk"), "ok");
    } catch {
      showToast(t("saveFailed"), "err");
    } finally {
      setSaving(false);
    }
  }, [showToast, t]);

  const persistHosts = useCallback(async (hosts: string[]) => {
    setAllowedHosts(hosts);
    await persistSecurity({ ssrfAllowedHosts: hosts });
  }, [persistSecurity]);

  const handlePolicyChange = useCallback(async (policy: DangerousOpsPolicy) => {
    setOpsPolicy(policy);
    await persistSecurity({ dangerousOpsPolicy: policy });
  }, [persistSecurity]);

  const handleExecutionModeChange = useCallback(async (mode: ExecutionMode) => {
    if (!activeAgentId) return;
    setExecutionMode(mode);
    setModeSaving(true);
    try {
      const agent = await api.getAgent(activeAgentId);
      if (!agent) throw new Error("agent not found");

      const behavior = agent.behavior ?? {};
      const cleanAsk = stripManaged(behavior.toolsAsk ?? behavior.requireConfirmationFor);
      const cleanDeny = stripManaged(behavior.toolsDeny);

      let nextAsk = cleanAsk;
      let nextDeny = cleanDeny;
      let nextFileAccess: api.FileAccessMode;

      if (mode === "plan") {
        nextDeny = dedupe([...cleanDeny, ...MANAGED_PATTERNS]);
        nextFileAccess = "workspace";
      } else if (mode === "default") {
        nextAsk = dedupe([...cleanAsk, ...MANAGED_PATTERNS]);
        nextFileAccess = "workspace";
      } else if (mode === "auto-edit") {
        nextAsk = dedupe([...cleanAsk, "shell_exec"]);
        nextFileAccess = "full";
      } else {
        // YOLO: no restrictions
        nextFileAccess = "full";
      }

      const ok = await api.updateAgent(activeAgentId, {
        ...agent,
        behavior: {
          ...behavior,
          fileAccess: nextFileAccess,
          toolsAsk: nextAsk,
          requireConfirmationFor: nextAsk,
          toolsDeny: nextDeny,
        },
      });

      if (!ok) throw new Error("update failed");
      showToast(t("executionModeUpdated"), "ok");
    } catch {
      showToast(t("executionModeUpdateFailed"), "err");
    } finally {
      setModeSaving(false);
    }
  }, [activeAgentId, showToast, t]);

  const handleAdd = () => {
    const trimmed = newHost.trim();
    if (!trimmed || allowedHosts.includes(trimmed)) return;
    const updated = [...allowedHosts, trimmed];
    setNewHost("");
    persistHosts(updated);
  };

  const handleRemove = (host: string) => {
    persistHosts(allowedHosts.filter((h) => h !== host));
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleAdd();
    }
  };

  const inputCls = sharedInputCls + " font-mono";
  const inputStyle = sharedInputStyle;

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>{t("loading")}</span>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {toast && (
        <div
          className="flex items-center gap-2 rounded-[var(--radius-xs)] px-3 py-2 text-[12px] font-medium"
          style={{
            background: toast.type === "ok" ? "color-mix(in srgb, var(--green) 15%, transparent)" : "color-mix(in srgb, var(--red) 15%, transparent)",
            color: toast.type === "ok" ? "var(--green)" : "var(--red)",
          }}
        >
          {toast.type === "ok" ? <CheckCircle  /> : <XCircle  />}
          {toast.msg}
        </div>
      )}

      <div>
        <SectionTitle>{t("executionMode")}</SectionTitle>
        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("executionModeDesc")}
        </p>
        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {executionModeOptions.map((opt, idx) => (
            <button
              key={opt.value}
              onClick={() => handleExecutionModeChange(opt.value)}
              disabled={modeSaving || !activeAgentId}
              className="flex w-full cursor-pointer items-center gap-3 px-4 py-3 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:cursor-not-allowed disabled:opacity-50"
              style={idx < executionModeOptions.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
            >
              <span
                className="flex h-[20px] w-[20px] shrink-0 items-center justify-center rounded-full transition-all duration-150"
                style={{
                  border: executionMode === opt.value ? "none" : "2px solid var(--fill-quaternary)",
                  background: executionMode === opt.value ? "var(--tint)" : "transparent",
                }}
              >
                {executionMode === opt.value && (
                  <CheckCircle size={16} weight="bold" style={{ color: "#fff" }} />
                )}
              </span>
              <div>
                <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{opt.label}</div>
                <div className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{opt.desc}</div>
              </div>
            </button>
          ))}
        </div>
        {sessionOverrideCount > 0 && (
          <div
            className="mt-2 flex items-start gap-2 rounded-[var(--radius-xs)] px-3 py-2 text-[11px]"
            style={{
              background: "color-mix(in srgb, var(--tint) 8%, transparent)",
              color: "var(--tint)",
            }}
          >
            <Info className="mt-px shrink-0" />
            <span>
              {t("sessionOverrideHint", { count: sessionOverrideCount })}
            </span>
          </div>
        )}
      </div>

      <div>
        <SectionTitle>{t("dangerousOps")}</SectionTitle>
        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("dangerousOpsDesc")}
        </p>
        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {policyOptions.map((opt, idx) => (
            <button
              key={opt.value}
              onClick={() => handlePolicyChange(opt.value)}
              disabled={saving}
              className="flex w-full cursor-pointer items-center gap-3 px-4 py-3 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:cursor-not-allowed disabled:opacity-50"
              style={idx < policyOptions.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
            >
              <span
                className="flex h-[20px] w-[20px] shrink-0 items-center justify-center rounded-full transition-all duration-150"
                style={{
                  border: opsPolicy === opt.value ? "none" : "2px solid var(--fill-quaternary)",
                  background: opsPolicy === opt.value ? "var(--tint)" : "transparent",
                }}
              >
                {opsPolicy === opt.value && (
                  <CheckCircle size={16} weight="bold" style={{ color: "#fff" }} />
                )}
              </span>
              <div>
                <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{opt.label}</div>
                <div className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{opt.desc}</div>
              </div>
            </button>
          ))}
        </div>
      </div>

      <div>
        <SectionTitle>{t("ssrfWhitelist")}</SectionTitle>
        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("ssrfWhitelistDesc")}
        </p>

        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {allowedHosts.length === 0 ? (
            <div className="px-4 py-4 text-center text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              {t("ssrfEmpty")}
            </div>
          ) : (
            allowedHosts.map((host, idx) => (
              <div
                key={host}
                className="group flex items-center justify-between px-4 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                style={idx < allowedHosts.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
              >
                <span className="text-[13px] font-mono" style={{ color: "var(--fill-primary)" }}>{host}</span>
                <button
                  onClick={() => handleRemove(host)}
                  disabled={saving}
                  className="flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded-full opacity-0 transition-all duration-100 hover:bg-[var(--bg-hover)] group-hover:opacity-100"
                  title={t("remove")}
                >
                  <X  style={{ color: "var(--red)" }} />
                </button>
              </div>
            ))
          )}
        </div>

        <div className="mt-3 flex items-center gap-2">
          <input
            value={newHost}
            onChange={(e) => setNewHost(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t("ssrfPlaceholder")}
            className={inputCls}
            style={inputStyle}
            disabled={saving}
          />
          <button
            onClick={handleAdd}
            disabled={saving || !newHost.trim()}
            className="flex shrink-0 cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-3 py-2 text-[12px] font-medium text-white transition-colors hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
            style={{ background: "var(--tint)" }}
          >
            <Plus  />
            {t("add")}
          </button>
        </div>
      </div>

      <div>
        <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
          {t("ssrfConfigHint")}
        </p>
      </div>
    </div>
  );
}
