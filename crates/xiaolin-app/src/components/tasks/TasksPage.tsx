import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  Clock,
  Plus,
  Trash,
  PencilSimple,
  X,
  CaretDown,
  CaretRight,
  ArrowsClockwise,
  Play,
  Pause,
  CheckCircle,
  XCircle,
  Timer,
} from "@phosphor-icons/react";
import { ICON_SIZE } from "../../lib/ui-tokens";
import * as api from "../../lib/api";
import type { CronJob, CronJobAction, CronJobRun } from "../../lib/transport";

const SCHEDULE_PRESETS: { labelKey: string; value: string }[] = [
  { labelKey: "tasks_preset_hourly", value: "0 * * * *" },
  { labelKey: "tasks_preset_daily9", value: "0 9 * * *" },
  { labelKey: "tasks_preset_daily18", value: "0 18 * * *" },
  { labelKey: "tasks_preset_mon9", value: "0 9 * * 1" },
  { labelKey: "tasks_preset_fri18", value: "0 18 * * 5" },
  { labelKey: "tasks_preset_monthly", value: "0 9 1 * *" },
];

const DAY_KEYS = [
  "tasks_day_sun",
  "tasks_day_mon",
  "tasks_day_tue",
  "tasks_day_wed",
  "tasks_day_thu",
  "tasks_day_fri",
  "tasks_day_sat",
] as const;

const STATUS_MAP: Record<string, { color: string; bg: string; labelKey: string }> = {
  idle: { color: "var(--fill-tertiary)", bg: "var(--bg-tertiary)", labelKey: "tasks_status_idle" },
  running: { color: "var(--blue)", bg: "rgba(59,130,246,0.12)", labelKey: "tasks_status_running" },
  failed: { color: "var(--red)", bg: "rgba(239,68,68,0.12)", labelKey: "tasks_status_failed" },
  disabled: { color: "var(--fill-quaternary)", bg: "var(--bg-tertiary)", labelKey: "tasks_status_disabled" },
};

function cronToHuman(expr: string, t: TFunction<"common">): string {
  const parts = expr.trim().split(/\s+/);
  if (parts.length !== 5) return expr;
  const [min, hour, dom, _mon, dow] = parts;

  const preset = SCHEDULE_PRESETS.find((p) => p.value === expr);
  if (preset) return t(preset.labelKey);

  if (min === "0" && hour === "*" && dom === "*" && dow === "*") return t("tasks_cronHourly");
  if (min.startsWith("*/")) return t("tasks_cronEveryNMin", { n: min.slice(2) });
  if (hour.startsWith("*/") && min === "0") return t("tasks_cronEveryNHour", { n: hour.slice(2) });
  if (dom === "*" && dow === "*" && hour !== "*" && min !== "*") {
    return t("tasks_cronDaily", { hour, min: min.padStart(2, "0") });
  }
  if (dow !== "*" && dom === "*") {
    const dayIdx = parseInt(dow);
    const dayStr =
      dayIdx >= 0 && dayIdx <= 6 ? t(DAY_KEYS[dayIdx]) : dow;
    return t("tasks_cronWeekly", { day: dayStr, hour, min: min.padStart(2, "0") });
  }
  return t("tasks_cronCustom", { expr });
}

function JobStatusBadge({ status }: { status: string }) {
  const { t } = useTranslation("common");
  const c = STATUS_MAP[status] ?? STATUS_MAP.idle;

  return (
    <span
      className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium"
      style={{ color: c.color, background: c.bg }}
    >
      {status === "running" && <ArrowsClockwise size={10} className="animate-spin" />}
      {t(c.labelKey)}
    </span>
  );
}

function RunHistory({ jobId }: { jobId: string }) {
  const { t } = useTranslation("common");
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
        <ArrowsClockwise size={12} className="animate-spin" />
        <span className="text-[11px]">{t("tasks_loadingRuns")}</span>
      </div>
    );
  }

  if (runs.length === 0) {
    return (
      <p className="py-3 text-center text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        {t("tasks_noRuns")}
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
              <CheckCircle size={12} style={{ color: "var(--green)" }} />
            ) : run.status === "running" ? (
              <ArrowsClockwise size={12} className="animate-spin" style={{ color: "var(--blue)" }} />
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
  const { t } = useTranslation("common");
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
          {expanded ? <CaretDown /> : <CaretRight />}
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
              {cronToHuman(job.schedule, t)}
            </span>
            <span className="font-mono" style={{ color: "var(--fill-quaternary)" }}>
              {job.schedule}
            </span>
            {(job.run_count ?? 0) > 0 && (
              <span>
                {t("tasks_runs", { count: job.run_count })}
                {(job.error_count ?? 0) > 0 && (
                  <span style={{ color: "var(--red)" }}>
                    {t("tasks_failures", { count: job.error_count })}
                  </span>
                )}
              </span>
            )}
          </div>
          {job.next_run && (
            <p className="mt-0.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              {t("tasks_nextRun", { time: new Date(job.next_run).toLocaleString() })}
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
            title={job.enabled ? t("tasks_disable") : t("tasks_enable")}
          >
            {job.enabled ? <Play /> : <Pause />}
          </button>
          <button
            onClick={() => onEdit(job)}
            className="flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)" }}
            title={t("tasks_edit")}
          >
            <PencilSimple />
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
                {t("confirm")}
              </button>
              <button
                onClick={() => setConfirming(false)}
                className="rounded-[var(--radius-xs)] px-2 py-1 text-[11px]"
                style={{ color: "var(--fill-tertiary)" }}
              >
                {t("cancel")}
              </button>
            </div>
          ) : (
            <button
              onClick={() => setConfirming(true)}
              className="flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] transition-colors hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-tertiary)" }}
              title={t("delete")}
            >
              <Trash />
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
            {t("tasks_runHistory")}
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
  const { t } = useTranslation("common");
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
            {editingJob ? t("tasks_edit") : t("tasks_create")}
          </h3>
          <button onClick={onClose} style={{ color: "var(--fill-tertiary)" }}>
            <X size={ICON_SIZE.md} />
          </button>
        </div>

        <div className="flex flex-col gap-3">
          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              {t("tasks_name")}
            </label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("tasks_namePlaceholder")}
              className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
              style={inputStyle}
            />
          </div>

          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              {t("tasks_schedule")}
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
                {t("tasks_presets")}
              </button>
              <button
                onClick={() => setScheduleMode("custom")}
                className="rounded-[var(--radius-sm)] px-3 py-1 text-[11px] font-medium transition-colors"
                style={{
                  background: scheduleMode === "custom" ? "var(--tint)" : "var(--bg-tertiary)",
                  color: scheduleMode === "custom" ? "#fff" : "var(--fill-tertiary)",
                }}
              >
                {t("tasks_custom")}
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
                    {t(p.labelKey)}
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
                    → {cronToHuman(customSchedule, t)}
                  </p>
                )}
              </div>
            )}
          </div>

          <div>
            <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              {t("tasks_actionType")}
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
                {t("tasks_agentChat")}
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
                  placeholder={t("tasks_agentIdPlaceholder")}
                  className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none"
                  style={inputStyle}
                />
                <textarea
                  value={message}
                  onChange={(e) => setMessage(e.target.value)}
                  placeholder={t("tasks_promptPlaceholder")}
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
              {t("tasks_enabled")}
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
            {t("cancel")}
          </button>
          <button
            onClick={handleSubmit}
            disabled={!name.trim() || !schedule.trim() || submitting}
            className="rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors disabled:opacity-40"
            style={{ background: "var(--tint)", color: "#fff" }}
          >
            {submitting ? t("tasks_saving") : editingJob ? t("tasks_save") : t("tasks_createBtn")}
          </button>
        </div>
      </div>
    </div>
  );
}

export function TasksPage() {
  const { t } = useTranslation("common");
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
        <ArrowsClockwise size={20} className="animate-spin" />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto" style={{ background: "var(--bg-primary)" }}>
      <div className="mx-auto w-full max-w-[640px] px-6 py-6">
        <div className="mb-4 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Clock size={ICON_SIZE.md} style={{ color: "var(--fill-secondary)" }} />
            <h2 className="text-[14px] font-semibold tracking-[-0.01em]" style={{ color: "var(--fill-primary)" }}>
              {t("tasks_title")}
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
            <Plus size={12} />
            {t("tasks_new")}
          </button>
        </div>

        {jobs.length === 0 ? (
          <div className="flex flex-col items-center gap-3 py-16" style={{ color: "var(--fill-quaternary)" }}>
            <Clock size={32} />
            <p className="text-[13px]">{t("tasks_empty")}</p>
            <button
              onClick={handleCreate}
              className="flex items-center gap-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors"
              style={{ background: "var(--tint)", color: "#fff" }}
            >
              <Plus size={12} />
              {t("tasks_createFirst")}
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
