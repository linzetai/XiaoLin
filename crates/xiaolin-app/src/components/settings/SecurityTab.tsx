import { useState, useEffect, useCallback } from "react";
import { CheckCircle, XCircle, X, Plus } from "lucide-react";
import * as api from "../../lib/api";
import { useAgentStore } from "../../lib/stores";
import { SectionTitle } from "./SettingsShared";
import { ICON } from "../../lib/ui-tokens";
import { inputCls as sharedInputCls, inputStyle as sharedInputStyle } from "../common/FormElements";

type DangerousOpsPolicy = "deny" | "allow" | "confirm";
type ExecutionMode = "plan" | "default" | "auto-edit" | "yolo";

const POLICY_OPTIONS: { value: DangerousOpsPolicy; label: string; desc: string }[] = [
  { value: "deny", label: "拒绝", desc: "直接阻止所有危险操作" },
  { value: "confirm", label: "确认", desc: "暂停并弹窗询问用户是否继续（推荐）" },
  { value: "allow", label: "允许", desc: "不做任何检查，直接执行" },
];

const MANAGED_PATTERNS = ["write_file", "edit_file", "apply_patch", "multi_edit", "shell_exec"];

const EXECUTION_MODE_OPTIONS: { value: ExecutionMode; label: string; desc: string }[] = [
  { value: "plan", label: "Plan（只读）", desc: "禁止写文件与 shell，仅允许工作区内读取，适合分析与规划" },
  { value: "default", label: "Default（确认）", desc: "写文件与 shell 需确认，仅访问工作区内文件" },
  { value: "auto-edit", label: "Auto-Edit", desc: "文件编辑自动通过且可访问全文件系统，shell 仍需确认" },
  { value: "yolo", label: "YOLO", desc: "所有操作自动通过，可访问全文件系统，仅限可信环境" },
];

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
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const [allowedHosts, setAllowedHosts] = useState<string[]>([]);
  const [newHost, setNewHost] = useState("");
  const [opsPolicy, setOpsPolicy] = useState<DangerousOpsPolicy>("confirm");
  const [executionMode, setExecutionMode] = useState<ExecutionMode>("default");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [modeSaving, setModeSaving] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: "ok" | "err" } | null>(null);

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
      showToast("已保存，立即生效", "ok");
    } catch {
      showToast("保存失败", "err");
    } finally {
      setSaving(false);
    }
  }, [showToast]);

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
      showToast("执行模式已更新", "ok");
    } catch {
      showToast("执行模式更新失败", "err");
    } finally {
      setModeSaving(false);
    }
  }, [activeAgentId, showToast]);

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
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中...</span>
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
            animation: "fade-in var(--duration-fast) var(--ease-out)",
          }}
        >
          {toast.type === "ok" ? <CheckCircle {...ICON.sm} /> : <XCircle {...ICON.sm} />}
          {toast.msg}
        </div>
      )}

      <div>
        <SectionTitle>执行模式</SectionTitle>
        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          统一控制 Agent 的工具权限与文件系统访问范围。决定写文件、shell 执行的审批策略，以及是否允许访问工作区外的文件。
        </p>
        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {EXECUTION_MODE_OPTIONS.map((opt, idx) => (
            <button
              key={opt.value}
              onClick={() => handleExecutionModeChange(opt.value)}
              disabled={modeSaving || !activeAgentId}
              className="flex w-full cursor-pointer items-center gap-3 px-4 py-3 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:cursor-not-allowed disabled:opacity-50"
              style={idx < EXECUTION_MODE_OPTIONS.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
            >
              <span
                className="flex h-[20px] w-[20px] shrink-0 items-center justify-center rounded-full transition-all duration-150"
                style={{
                  border: executionMode === opt.value ? "none" : "2px solid var(--fill-quaternary)",
                  background: executionMode === opt.value ? "var(--tint)" : "transparent",
                }}
              >
                {executionMode === opt.value && (
                  <CheckCircle size={16} strokeWidth={2.5} style={{ color: "#fff" }} />
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
        <SectionTitle>危险操作保护</SectionTitle>
        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          控制 Shell 中执行 rm、rmdir、chmod 等危险命令时的行为策略。
        </p>
        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {POLICY_OPTIONS.map((opt, idx) => (
            <button
              key={opt.value}
              onClick={() => handlePolicyChange(opt.value)}
              disabled={saving}
              className="flex w-full cursor-pointer items-center gap-3 px-4 py-3 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:cursor-not-allowed disabled:opacity-50"
              style={idx < POLICY_OPTIONS.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
            >
              <span
                className="flex h-[20px] w-[20px] shrink-0 items-center justify-center rounded-full transition-all duration-150"
                style={{
                  border: opsPolicy === opt.value ? "none" : "2px solid var(--fill-quaternary)",
                  background: opsPolicy === opt.value ? "var(--tint)" : "transparent",
                }}
              >
                {opsPolicy === opt.value && (
                  <CheckCircle size={16} strokeWidth={2.5} style={{ color: "#fff" }} />
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
        <SectionTitle>SSRF 白名单</SectionTitle>
        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          允许 http_fetch / web_fetch 访问的内网主机。默认情况下，解析到私有 IP (localhost, 10.x, 192.168.x) 的 URL 会被 SSRF 保护拦截。
          将主机名或 host:port 加入白名单后可绕过此限制，适用于本地 SearXNG、内部 API 等场景。
        </p>

        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {allowedHosts.length === 0 ? (
            <div className="px-4 py-4 text-center text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              暂无白名单主机 — 所有指向私有 IP 的请求将被拦截
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
                  title="移除"
                >
                  <X {...ICON.sm} style={{ color: "var(--red)" }} />
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
            placeholder="例: localhost:8888 或 searxng.internal"
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
            <Plus {...ICON.sm} />
            添加
          </button>
        </div>
      </div>

      <div>
        <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
          配置保存到 ~/.xiaolin/config/default.json 的 security.ssrfAllowedHosts 字段，保存后立即生效。
        </p>
      </div>
    </div>
  );
}
