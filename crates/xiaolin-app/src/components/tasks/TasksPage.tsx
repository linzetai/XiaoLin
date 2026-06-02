import { useCallback, useEffect, useState } from "react";
import {
  Clock,
  Plus,
  Trash2,
  Pencil,
  X,
  ChevronDown,
  ChevronRight,
  RefreshCw,
  Play,
  Pause,
  CheckCircle2,
  XCircle,
  Timer,
} from "lucide-react";
import { ICON } from "../../lib/ui-tokens";
import * as api from "../../lib/api";
import type { CronJob, CronJobAction, CronJobRun } from "../../lib/transport";

const SCHEDULE_PRESETS: { label: string; value: string }[] = [
  { label: "每小时", value: "0 * * * *" },
  { label: "每天 9:00", value: "0 9 * * *" },
  { label: "每天 18:00", value: "0 18 * * *" },
  { label: "每周一 9:00", value: "0 9 * * 1" },
  { label: "每周五 18:00", value: "0 18 * * 5" },
  { label: "每月 1 日 9:00", value: "0 9 1 * *" },
];

function cronToHuman(expr: string): string {
  const parts = expr.trim().split(/\s+/);
  if (parts.length !== 5) return expr;
  const [min, hour, dom, _mon, dow] = parts;

  const preset = SCHEDULE_PRESETS.find((p) => p.value === expr);
  if (preset) return preset.label;

  if (min === "0" && hour === "*" && dom === "*" && dow === "*") return "每小时整点";
  if (min.startsWith("*/")) return `每 ${min.slice(2)} 分钟`;
  if (hour.startsWith("*/") && min === "0") return `每 ${hour.slice(2)} 小时`;
  if (dom === "*" && dow === "*" && hour !== "*" && min !== "*") return `每天 ${hour}:${min.padStart(2, "0")}`;
  if (dow !== "*" && dom === "*") {
    const dayNames = ["日", "一", "二", "三", "四", "五", "六"];
    const dayIdx = parseInt(dow);
    const dayStr = dayIdx >= 0 && dayIdx <= 6 ? `周${dayNames[dayIdx]}` : `周${dow}`;
    return `每${dayStr} ${hour}:${min.padStart(2, "0")}`;
  }
  return expr;
}

function JobStatusBadge({ status }: { status: string }) {
  const config: Record<string, { color: string; bg: string; label: string }> = {
    idle: { color: "var(--fill-tertiary)", bg: "var(--bg-tertiary)", label: "空闲" },
    running: { color: "var(--blue)", bg: "rgba(59,130,246,0.12)", label: "运行中" },
    failed: { color: "var(--red)", bg: "rgba(239,68,68,0.12)", label: "失败" },
    disabled: { color: "var(--fill-quaternary)", bg: "var(--bg-tertiary)", label: "已禁用" },
  };
  const c = config[status] ?? config.idle;

  return (
    <span
      className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium"
      style={{ color: c.color, background: c.bg }}
    >
      {status === "running" && <RefreshCw size={10} className="animate-spin" />}
      {c.label}
    </span>
  );
}

function RunHistory({ jobId }: { jobId: string }) {
  const [runs, setRuns] = useState<CronJobRun[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api.listCronRuns(jobId, 20).then((r) => {
      setRuns(r);
      setLoading(false);
    });
  }, [jobId]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 py-3" style={{ color: "var(--fill-quaternary)" }}>
        <RefreshCw size={12} className="animate-spin" />
        <span className="text-[11px]">加载运行记录...</span>
      </div>
    );
  }

  if (runs.length === 0) {
    return (
      <p className="py-3 text-center text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        暂无运行记录
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-1">
      {runs.map((run) => {
        const ok = run.status === "ok" || run.status === "completed";
        const duration =
          run.started_at && run.ended_at
            ? `${Math.round((new Date(run.ended_at).getTime() - new Date(run.started_at).getTime()) / 1000)}s`
            : "—";
        return (
          <div
            key={run.id}
            className="flex items-center gap-2 rounded-[var(--radius-xs)] px-3 py-1.5 text-[11px]"
            style={{ background: "var(--bg-primary)" }}
          >
            {ok ? (
              <CheckCircle2 size={12} style={{ color: "var(--green)" }} />
            ) : run.status === "running" ? (
              <RefreshCw size={12} className="animate-spin" style={{ color: "var(--blue)" }} />
            ) : (
              <XCircle size={12} style={{ color: "var(--red)" }} />
            )}
            <span style={{ color: "var(--fill-tertiary)" }}>
              {new Date(run.started_at).toLocaleString()}
            </span>
            <span className="flex items-center gap-0.5" style={{ color: "var(--fill-quaternary)" }}>
              <Timer size={10} />
              {duration}
            </span>
            {run.error && (
              <span className="ml-auto truncate max-w-[200px]" style={{ color: "var(--red)" }}>
                {run.error}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}

function JobCard({
  job,
  onEdit,
  onDelete,
  onToggle,
}: {
  job: CronJob;
  onEdit: (job: CronJob) => void;
  onDelete: (id: string) => void;
  onToggle: (job: CronJob) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const status = job.status ?? (job.enabled ? "idle" : "disabled");

  return (
    <div
      className="rounded-[var(--radius-md)] transition-colors duration-150"
      style={{
        background: "var(--bg-secondary)",
        border: "0.5px solid var(--border-subtle)",
      }}
    >
      <div
        className="group flex cursor-pointer items-center gap-3 px-4 py-3"
        onClick={() => setExpanded(!expanded)}
      >
        <span style={{ color: "var(--fill-quaternary)" }}>
          {expanded ? <ChevronDown {...ICON.sm} /> : <ChevronRight {...ICON.sm} />}
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
              {job.name}
            </span>
            <JobStatusBadge status={status} />
          </div>
          <div className="mt-0.5 flex items-center gap-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            <span className="flex items-center gap-1">
              <Clock size={10} />
              {cronToHuman(job.schedule)}
            </span>
            <span className="font-mono" style={{ color: "var(--fill-quaternary)" }}>
              {job.schedule}
            </span>
            {(job.run_count ?? 0) > 0 && (
              <span>
                {job.run_count} 次运行
                {(job.error_count ?? 0) > 0 && (
                  <span style={{ color: "var(--red)" }}> · {job.error_count} 次失败</span>
                )}
              </span>
            )}
          </div>
          {job.next_run && (
            <p className="mt-0.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              下次: {new Date(job.next_run).toLocaleString()}
            </p>
          )}
        </div>

        <div
          className="flex items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            onClick={() => onToggle(job)}
            className="flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: job.enabled ? "var(--green)" : "var(--fill-quaternary)" }}
            title={job.enabled ? "禁用" : "启用"}
          >
            {job.enabled ? <Play {...ICON.sm} /> : <Pause {...ICON.sm} />}
          </button>
          <button
            onClick={() => onEdit(job)}
            className="flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)" }}
            title="编辑"
          >
            <Pencil {...ICON.sm} />
          </button>
          {confirming ? (
            <div className="flex items-center gap-1">
              <button
                onClick={() => {
                  onDelete(job.id);
                  setConfirming(false);
                }}
                className="rounded-[var(--radius-xs)] px-2 py-1 text-[11px] font-medium"
                style={{ background: "var(--red)", color: "#fff" }}
              >
                确认
              </button>
              <button
                onClick={() => setConfirming(false)}
                className="rounded-[var(--radius-xs)] px-2 py-1 text-[11px]"
                style={{ color: "var(--fill-tertiary)" }}
              >
                取消
              </button>
            </div>
          ) : (
            <button
              onClick={() => setConfirming(true)}
              className="flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] transition-colors hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-tertiary)" }}
              title="删除"
            >
              <Trash2 {...ICON.sm} />
            </button>
          )}
        </div>
      </div>

      {expanded && (
        <div
          className="border-t px-4 py-3"
          style={{ borderColor: "var(--border-subtle)" }}
        >
          <p className="mb-2 text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
            运行历史
          </p>
          <RunHistory jobId={job.id} />
        </div>
      )}
    </div>
  );
}

function CronJobModal({
  open,
  onClose,
  onSubmit,
  editingJob,
}: {
  open: boolean;
  onClose: () => void;
  onSubmit: (job: Partial<CronJob> & { name: string; schedule: string; action: CronJobAction }) => void;
  editingJob: CronJob | null;
}) {
  const [name, setName] = useState("");
  const [scheduleMode, setScheduleMode] = useState<"preset" | "custom">("preset");
  const [presetIdx, setPresetIdx] = useState(0);
  const [customSchedule, setCustomSchedule] = useState("");
  const [actionType, setActionType] = useState<"agent_chat" | "webhook">("agent_chat");
  const [agentId, setAgentId] = useState("");
  const [message, setMessage] = useState("");
  const [webhookUrl, setWebhookUrl] = useState("");
  const [webhookMethod, setWebhookMethod] = useState("POST");
  const [enabled, setEnabled] = useState(true);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (editingJob) {
      setName(editingJob.name);
      setEnabled(editingJob.enabled);
      const presetMatch = SCHEDULE_PRESETS.findIndex((p) => p.value === editingJob.schedule);
      if (presetMatch >= 0) {
        setScheduleMode("preset");
        setPresetIdx(presetMatch);
      } else {
        setScheduleMode("custom");
        setCustomSchedule(editingJob.schedule);
      }
      setActionType(editingJob.action.type);
      if (editingJob.action.type === "agent_chat") {
        setAgentId(editingJob.action.agent_id ?? "");
        setMessage(editingJob.action.message ?? "");
      } else {
        setWebhookUrl(editingJob.action.url ?? "");
        setWebhookMethod(editingJob.action.method ?? "POST");
      }
    } else {
      setName("");
      setScheduleMode("preset");
      setPresetIdx(0);
      setCustomSchedule("");
      setActionType("agent_chat");
      setAgentId("");
      setMessage("");
      setWebhookUrl("");
      setWebhookMethod("POST");
      setEnabled(true);
    }
  }, [editingJob, open]);

  if (!open) return null;

  const schedule = scheduleMode === "preset" ? SCHEDULE_PRESETS[presetIdx].value : customSchedule;

  const handleSubmit = () => {
    if (!name.trim() || !schedule.trim()) return;
    setSubmitting(true);
    const action: CronJobAction =
      actionType === "agent_chat"
        ? { type: "agent_chat", agent_id: agentId || undefined, message }
        : { type: "webhook", url: webhookUrl, method: webhookMethod };

    onSubmit({
      ...(editingJob ? { id: editingJob.id } : {}),
      name: name.trim(),
      schedule: schedule.trim(),
      action,
      enabled,
    });
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
        className="w-[440px] max-h-[80vh] overflow-y-auto rounded-[var(--radius-lg)] p-5"
        style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--border-subtle)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {editingJob ? "编辑任务" : "创建定时任务"}
          </h3>
          <button onClick={onClose} style={{ color: "var(--fill-tertiary)" }}>
            <X {...ICON.md} />
          </button>
        </div>

        <div className="flex flex-col gap-3">
          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              任务名称
            </label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="每日日报总结"
              className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
              style={inputStyle}
            />
          </div>

          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              执行计划
            </label>
            <div className="mb-2 flex gap-2">
              <button
                onClick={() => setScheduleMode("preset")}
                className="rounded-[var(--radius-sm)] px-3 py-1 text-[11px] font-medium transition-colors"
                style={{
                  background: scheduleMode === "preset" ? "var(--tint)" : "var(--bg-tertiary)",
                  color: scheduleMode === "preset" ? "#fff" : "var(--fill-tertiary)",
                }}
              >
                常用预设
              </button>
              <button
                onClick={() => setScheduleMode("custom")}
                className="rounded-[var(--radius-sm)] px-3 py-1 text-[11px] font-medium transition-colors"
                style={{
                  background: scheduleMode === "custom" ? "var(--tint)" : "var(--bg-tertiary)",
                  color: scheduleMode === "custom" ? "#fff" : "var(--fill-tertiary)",
                }}
              >
                自定义
              </button>
            </div>
            {scheduleMode === "preset" ? (
              <div className="flex flex-wrap gap-1.5">
                {SCHEDULE_PRESETS.map((p, i) => (
                  <button
                    key={p.value}
                    onClick={() => setPresetIdx(i)}
                    className="rounded-full px-2.5 py-1 text-[11px] transition-colors"
                    style={{
                      background: presetIdx === i ? "var(--tint-bg)" : "var(--bg-tertiary)",
                      color: presetIdx === i ? "var(--tint)" : "var(--fill-tertiary)",
                      border: presetIdx === i ? "0.5px solid var(--tint)" : "0.5px solid transparent",
                    }}
                  >
                    {p.label}
                  </button>
                ))}
              </div>
            ) : (
              <div>
                <input
                  value={customSchedule}
                  onChange={(e) => setCustomSchedule(e.target.value)}
                  placeholder="0 */2 * * *"
                  className="w-full rounded-[var(--radius-sm)] px-3 py-2 font-mono text-[13px] outline-none"
                  style={inputStyle}
                />
                {customSchedule && (
                  <p className="mt-1 text-[11px]" style={{ color: "var(--tint)" }}>
                    → {cronToHuman(customSchedule)}
                  </p>
                )}
              </div>
            )}
          </div>

          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              动作类型
            </label>
            <div className="mb-2 flex gap-2">
              <button
                onClick={() => setActionType("agent_chat")}
                className="rounded-[var(--radius-sm)] px-3 py-1 text-[11px] font-medium transition-colors"
                style={{
                  background: actionType === "agent_chat" ? "var(--tint)" : "var(--bg-tertiary)",
                  color: actionType === "agent_chat" ? "#fff" : "var(--fill-tertiary)",
                }}
              >
                Agent 对话
              </button>
              <button
                onClick={() => setActionType("webhook")}
                className="rounded-[var(--radius-sm)] px-3 py-1 text-[11px] font-medium transition-colors"
                style={{
                  background: actionType === "webhook" ? "var(--tint)" : "var(--bg-tertiary)",
                  color: actionType === "webhook" ? "#fff" : "var(--fill-tertiary)",
                }}
              >
                Webhook
              </button>
            </div>

            {actionType === "agent_chat" ? (
              <div className="flex flex-col gap-2">
                <input
                  value={agentId}
                  onChange={(e) => setAgentId(e.target.value)}
                  placeholder="Agent ID（留空使用默认）"
                  className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
                  style={inputStyle}
                />
                <textarea
                  value={message}
                  onChange={(e) => setMessage(e.target.value)}
                  placeholder="发送给 Agent 的提示词"
                  rows={3}
                  className="w-full resize-none rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
                  style={inputStyle}
                />
              </div>
            ) : (
              <div className="flex flex-col gap-2">
                <input
                  value={webhookUrl}
                  onChange={(e) => setWebhookUrl(e.target.value)}
                  placeholder="https://example.com/webhook"
                  className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
                  style={inputStyle}
                />
                <select
                  value={webhookMethod}
                  onChange={(e) => setWebhookMethod(e.target.value)}
                  className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
                  style={inputStyle}
                >
                  <option value="POST">POST</option>
                  <option value="GET">GET</option>
                  <option value="PUT">PUT</option>
                </select>
              </div>
            )}
          </div>

          <div className="flex items-center gap-2">
            <label className="text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              启用
            </label>
            <button
              onClick={() => setEnabled(!enabled)}
              className="relative h-5 w-9 rounded-full transition-colors"
              style={{ background: enabled ? "var(--tint)" : "var(--bg-tertiary)" }}
            >
              <span
                className="absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform"
                style={{ transform: enabled ? "translateX(18px)" : "translateX(2px)" }}
              />
            </button>
          </div>
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <button
            onClick={onClose}
            className="rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] transition-colors"
            style={{ color: "var(--fill-tertiary)" }}
          >
            取消
          </button>
          <button
            onClick={handleSubmit}
            disabled={!name.trim() || !schedule.trim() || submitting}
            className="rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors disabled:opacity-40"
            style={{ background: "var(--tint)", color: "#fff" }}
          >
            {submitting ? "保存中..." : editingJob ? "保存" : "创建"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function TasksPage() {
  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [loading, setLoading] = useState(true);
  const [showModal, setShowModal] = useState(false);
  const [editingJob, setEditingJob] = useState<CronJob | null>(null);

  const fetchJobs = useCallback(async () => {
    const result = await api.listCronJobs();
    setJobs(result);
    setLoading(false);
  }, []);

  useEffect(() => {
    fetchJobs();
  }, [fetchJobs]);

  const handleCreate = () => {
    setEditingJob(null);
    setShowModal(true);
  };

  const handleEdit = (job: CronJob) => {
    setEditingJob(job);
    setShowModal(true);
  };

  const handleSubmit = async (
    job: Partial<CronJob> & { name: string; schedule: string; action: CronJobAction },
  ) => {
    try {
      await api.upsertCronJob(job);
      await fetchJobs();
    } catch (e) {
      console.warn("[tasks] upsert error:", e);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await api.deleteCronJob(id);
      setJobs((prev) => prev.filter((j) => j.id !== id));
    } catch (e) {
      console.warn("[tasks] delete error:", e);
    }
  };

  const handleToggle = async (job: CronJob) => {
    try {
      await api.upsertCronJob({
        ...job,
        enabled: !job.enabled,
      });
      await fetchJobs();
    } catch (e) {
      console.warn("[tasks] toggle error:", e);
    }
  };

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center" style={{ color: "var(--fill-quaternary)" }}>
        <RefreshCw size={20} className="animate-spin" />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto" style={{ background: "var(--bg-primary)" }}>
      <div className="mx-auto w-full max-w-[640px] px-6 py-6">
        <div className="mb-4 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Clock {...ICON.md} style={{ color: "var(--fill-secondary)" }} />
            <h2 className="text-[14px] font-semibold tracking-[-0.01em]" style={{ color: "var(--fill-primary)" }}>
              定时任务
            </h2>
            <span
              className="rounded-full px-1.5 py-0.5 text-[10px] font-medium"
              style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)" }}
            >
              {jobs.length}
            </span>
          </div>
          <button
            onClick={handleCreate}
            className="flex items-center gap-1 rounded-[var(--radius-sm)] px-2 py-1 text-[11px] font-medium transition-colors"
            style={{ background: "var(--tint)", color: "#fff" }}
          >
            <Plus size={12} strokeWidth={2} />
            新建
          </button>
        </div>

        {jobs.length === 0 ? (
          <div className="flex flex-col items-center gap-3 py-16" style={{ color: "var(--fill-quaternary)" }}>
            <Clock size={32} strokeWidth={1.2} />
            <p className="text-[13px]">暂无定时任务</p>
            <button
              onClick={handleCreate}
              className="flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors"
              style={{ background: "var(--tint)", color: "#fff" }}
            >
              <Plus size={12} strokeWidth={2} />
              创建第一个任务
            </button>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {jobs.map((job) => (
              <JobCard
                key={job.id}
                job={job}
                onEdit={handleEdit}
                onDelete={handleDelete}
                onToggle={handleToggle}
              />
            ))}
          </div>
        )}
      </div>

      <CronJobModal
        open={showModal}
        onClose={() => {
          setShowModal(false);
          setEditingJob(null);
        }}
        onSubmit={handleSubmit}
        editingJob={editingJob}
      />
    </div>
  );
}
