import { useEffect, useState, useCallback, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  Plus, Clock, Play, Trash, ClockCounterClockwise,
  ToggleLeft, ToggleRight, Robot, Globe, WarningCircle,
  CaretLeft, SpinnerGap, Lightning, Calendar, FolderOpen,
} from "@phosphor-icons/react";
import { useAutomationStore } from "../../lib/stores/automation-store";
import type { CronJob, CronJobAction, CronJobRun } from "../../lib/transport";
import { cronToHuman } from "./CronScheduleHelper";
import { CronScheduleHelper } from "./CronScheduleHelper";

type ViewMode = "list" | "form" | "history";

export function AutomationView() {
  const jobs = useAutomationStore((s) => s.jobs);
  const loading = useAutomationStore((s) => s.loading);
  const loadJobs = useAutomationStore((s) => s.loadJobs);
  const createJob = useAutomationStore((s) => s.createJob);
  const updateJob = useAutomationStore((s) => s.updateJob);
  const deleteJob = useAutomationStore((s) => s.deleteJob);
  const runNow = useAutomationStore((s) => s.runNow);
  const fetchRuns = useAutomationStore((s) => s.fetchRuns);
  const runs = useAutomationStore((s) => s.runs);
  const selectedJobId = useAutomationStore((s) => s.selectedJobId);

  const [view, setView] = useState<ViewMode>("list");
  const [editingJob, setEditingJob] = useState<CronJob | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<CronJob | null>(null);
  const [viewKey, setViewKey] = useState(0);

  useEffect(() => { loadJobs(); }, [loadJobs]);

  const navigateTo = useCallback((next: ViewMode) => {
    setViewKey((k) => k + 1);
    setView(next);
  }, []);

  const handleCreate = useCallback(() => {
    setEditingJob(null);
    navigateTo("form");
  }, [navigateTo]);

  const handleEdit = useCallback((job: CronJob) => {
    setEditingJob(job);
    navigateTo("form");
  }, [navigateTo]);

  const handleHistory = useCallback((job: CronJob) => {
    fetchRuns(job.id);
    navigateTo("history");
  }, [fetchRuns, navigateTo]);

  const handleFormSubmit = useCallback(async (data: AutoFormData) => {
    if (editingJob) {
      await updateJob(editingJob.id, data as unknown as Partial<CronJob>);
    } else {
      await createJob(data as unknown as Parameters<typeof createJob>[0]);
    }
    navigateTo("list");
  }, [editingJob, updateJob, createJob, navigateTo]);

  const handleRunNow = useCallback(async (job: CronJob) => {
    await runNow(job.id);
  }, [runNow]);

  const handleDeleteConfirm = useCallback(async () => {
    if (deleteTarget) {
      await deleteJob(deleteTarget.id);
      setDeleteTarget(null);
    }
  }, [deleteTarget, deleteJob]);

  return (
    <div className="auto-view flex h-full flex-col" style={{ background: "var(--bg-card)" }}>
      <style>{ANIM_CSS}</style>

      <div className="flex-1 overflow-y-auto" style={{ overscrollBehavior: "contain" }}>
        <ViewTransition key={viewKey} direction={view === "list" ? "back" : "forward"}>
          <div className="mx-auto w-full max-w-[640px] px-6 py-8">
            {/* View header — integrated inside content area like chat welcome */}
            <div className="mb-6 flex items-center justify-between">
              <div className="flex items-center gap-2">
                {view !== "list" ? (
                  <button
                    onClick={() => navigateTo("list")}
                    className="flex items-center gap-1 rounded-[var(--radius-xs)] py-1 pr-1.5 pl-0.5 transition-colors duration-150 hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--fill-tertiary)", cursor: "pointer", background: "none", border: "none" }}
                    aria-label="Back to list"
                  >
                    <CaretLeft size={16} />
                    <span className="text-[12px]">Back</span>
                  </button>
                ) : (
                  <div className="flex items-center gap-2.5">
                    <Lightning size={18} style={{ color: "var(--tint)" }} />
                    <h1 className="text-[20px] font-bold tracking-tight" style={{ color: "var(--fill-primary)" }}>Automations</h1>
                    {jobs.length > 0 && (
                      <span
                        className="rounded-full px-2 py-0.5 text-[11px] font-semibold tabular-nums"
                        style={{ background: "color-mix(in srgb, var(--tint) 8%, transparent)", color: "var(--tint)" }}
                      >
                        {jobs.length}
                      </span>
                    )}
                  </div>
                )}
                {view === "form" && (
                  <span className="text-[15px] font-semibold" style={{ color: "var(--fill-primary)" }}>
                    {editingJob ? "Edit Automation" : "New Automation"}
                  </span>
                )}
                {view === "history" && (
                  <span className="text-[15px] font-semibold" style={{ color: "var(--fill-primary)" }}>Run History</span>
                )}
              </div>
              {view === "list" && jobs.length > 0 && (
                <button
                  onClick={handleCreate}
                  className="flex items-center gap-1.5 rounded-[var(--radius-sm)] px-4 py-2 text-[13px] font-semibold text-white transition-all duration-150 hover:brightness-110 active:scale-[0.97]"
                  style={{ background: "var(--tint)", cursor: "pointer", border: "none" }}
                >
                  <Plus size={14} /> New
                </button>
              )}
            </div>

            {view === "list" && (
              <JobList
                jobs={jobs}
                loading={loading}
                onCreate={handleCreate}
                onEdit={handleEdit}
                onDelete={setDeleteTarget}
                onHistory={handleHistory}
                onToggle={(job) => updateJob(job.id, { enabled: !job.enabled })}
                onRunNow={handleRunNow}
              />
            )}
            {view === "form" && (
              <JobForm
                initial={editingJob}
                onSubmit={handleFormSubmit}
                onCancel={() => navigateTo("list")}
              />
            )}
            {view === "history" && (
              <JobHistory
                jobId={selectedJobId}
                jobName={jobs.find((j) => j.id === selectedJobId)?.name ?? ""}
                runs={runs}
              />
            )}
          </div>
        </ViewTransition>
      </div>

      {/* Delete confirmation portal */}
      {deleteTarget && createPortal(
        <div className="fixed inset-0 z-50 flex items-center justify-center" role="alertdialog" aria-label="Confirm deletion">
          <div
            className="absolute inset-0 av-backdrop-enter"
            style={{ background: "rgba(0,0,0,0.18)", backdropFilter: "blur(6px)" }}
            onClick={() => setDeleteTarget(null)}
          />
          <div
            className="relative rounded-[var(--radius-lg)] px-6 py-5 av-dialog-enter"
            style={{ background: "var(--bg-elevated)", boxShadow: "0 24px 48px -12px rgba(0,0,0,0.15), 0 0 0 0.5px var(--separator)", maxWidth: 360, width: "calc(100% - 40px)" }}
          >
            <div className="mb-3 flex items-center gap-2.5">
              <div className="flex h-8 w-8 items-center justify-center rounded-full" style={{ background: "color-mix(in srgb, var(--red, #E53E3E) 8%, transparent)" }}>
                <WarningCircle size={15} style={{ color: "var(--red, #E53E3E)" }} />
              </div>
              <span className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>Delete Automation</span>
            </div>
            <p className="mb-5 text-[13px] leading-relaxed" style={{ color: "var(--fill-secondary)" }}>
              Are you sure you want to delete <strong>&ldquo;{deleteTarget.name}&rdquo;</strong>? All run history will be lost.
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setDeleteTarget(null)}
                className="rounded-[var(--radius-xs)] px-4 py-2 text-[13px] font-medium transition-colors duration-150 hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--fill-secondary)", cursor: "pointer", background: "none", border: "none" }}
              >
                Cancel
              </button>
              <button
                onClick={handleDeleteConfirm}
                className="rounded-[var(--radius-xs)] px-4 py-2 text-[13px] font-medium text-white transition-all duration-150 hover:brightness-110 active:scale-[0.97]"
                style={{ background: "var(--red, #E53E3E)", cursor: "pointer", border: "none" }}
              >
                Delete
              </button>
            </div>
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}

/* ─── View Transition Wrapper ─── */

function ViewTransition({ children, direction }: { children: ReactNode; direction: "forward" | "back" }) {
  return (
    <div className={direction === "forward" ? "av-slide-in-right" : "av-fade-in"}>
      {children}
    </div>
  );
}

/* ─── Job List ─── */

function JobList({
  jobs, loading, onCreate, onEdit, onDelete, onHistory, onToggle, onRunNow,
}: {
  jobs: CronJob[];
  loading: boolean;
  onCreate: () => void;
  onEdit: (j: CronJob) => void;
  onDelete: (j: CronJob) => void;
  onHistory: (j: CronJob) => void;
  onToggle: (j: CronJob) => void;
  onRunNow: (j: CronJob) => void;
}) {
  if (loading) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 py-20 av-fade-in">
        <SpinnerGap size={22} className="animate-spin" style={{ color: "var(--fill-quaternary)" }} />
        <p className="text-[13px]" style={{ color: "var(--fill-quaternary)" }}>Loading automations…</p>
      </div>
    );
  }

  if (jobs.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-5 py-20 av-fade-in">
        <div
          className="av-float flex h-14 w-14 items-center justify-center rounded-[14px]"
          style={{ background: "color-mix(in srgb, var(--tint) 6%, transparent)" }}
        >
          <Calendar size={28} style={{ color: "var(--tint)", opacity: 0.8 }} />
        </div>
        <div className="text-center">
          <p className="text-[16px] font-semibold" style={{ color: "var(--fill-primary)" }}>No automations yet</p>
          <p className="mt-2 text-[13px] leading-relaxed" style={{ color: "var(--fill-quaternary)", maxWidth: 320 }}>
            Schedule recurring tasks to run automatically — like daily reports, health checks, or data syncs.
          </p>
        </div>
        <button
          onClick={onCreate}
          className="mt-2 flex items-center gap-1.5 rounded-[var(--radius-sm)] px-5 py-2.5 text-[13px] font-semibold text-white transition-all duration-150 hover:brightness-110 active:scale-[0.97]"
          style={{ background: "var(--tint)", cursor: "pointer", border: "none" }}
        >
          <Plus size={14} /> Create automation
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {jobs.map((job, idx) => (
        <div
          key={job.id}
          className="av-stagger group flex items-center gap-3.5 rounded-[var(--radius-sm)] px-4 py-3.5 transition-all duration-200"
          style={{
            "--stagger-i": idx,
            cursor: "pointer",
            background: "var(--bg-elevated)",
            border: "0.5px solid var(--separator)",
            boxShadow: "0 1px 2px rgba(0,0,0,0.03)",
          } as React.CSSProperties}
          onClick={() => onEdit(job)}
          role="button"
          tabIndex={0}
          onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onEdit(job); } }}
        >
          <div
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[11px] transition-transform duration-200 group-hover:scale-[1.04]"
            style={{ background: job.action.type === "agent_chat" ? "color-mix(in srgb, var(--tint) 8%, transparent)" : "color-mix(in srgb, var(--orange, #ED8936) 8%, transparent)" }}
          >
            {job.action.type === "agent_chat" ? (
              <Robot size={18} style={{ color: "var(--tint)" }} />
            ) : (
              <Globe size={18} style={{ color: "var(--orange, #ED8936)" }} />
            )}
          </div>

          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="truncate text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>{job.name}</span>
              <StatusPill status={job.enabled ? (job.status === "failed" ? "failed" : "active") : "paused"} />
            </div>
            <div className="mt-1 flex items-center gap-3 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              <span className="flex items-center gap-1">
                <Clock size={10} />
                {cronToHuman(job.schedule)}
              </span>
              {job.last_run && (
                <span>Last run {relativeTime(job.last_run)}</span>
              )}
            </div>
          </div>

          <div className="flex shrink-0 items-center gap-1">
            <IconBtn
              icon={<Play size={13} style={{ color: "var(--tint)" }} />}
              title="Run now"
              onClick={(e) => { e.stopPropagation(); onRunNow(job); }}
              className="opacity-60 group-hover:opacity-100"
            />
            <IconBtn
              icon={job.enabled ? <ToggleRight size={15} style={{ color: "var(--green, #38A169)" }} /> : <ToggleLeft size={15} style={{ color: "var(--fill-quaternary)" }} />}
              title={job.enabled ? "Pause" : "Enable"}
              onClick={(e) => { e.stopPropagation(); onToggle(job); }}
              className="opacity-60 group-hover:opacity-100"
            />
            <IconBtn
              icon={<ClockCounterClockwise size={13} style={{ color: "var(--fill-tertiary)" }} />}
              title="Run history"
              onClick={(e) => { e.stopPropagation(); onHistory(job); }}
              className="opacity-0 group-hover:opacity-100"
            />
            <IconBtn
              icon={<Trash size={13} style={{ color: "var(--red, #E53E3E)" }} />}
              title="Delete"
              onClick={(e) => { e.stopPropagation(); onDelete(job); }}
              className="opacity-0 group-hover:opacity-100"
            />
          </div>
        </div>
      ))}
    </div>
  );
}

/* ─── Job Form ─── */

interface AutoFormData {
  name: string;
  schedule: string;
  action: CronJobAction;
  enabled: boolean;
  work_dir?: string | null;
}

function JobForm({
  initial, onSubmit, onCancel,
}: {
  initial: CronJob | null;
  onSubmit: (data: AutoFormData) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [schedule, setSchedule] = useState(initial?.schedule ?? "0 9 * * *");
  const [actionType, setActionType] = useState<"agent_chat" | "webhook">(
    initial?.action.type === "webhook" ? "webhook" : "agent_chat",
  );
  const [message, setMessage] = useState(
    initial?.action.type === "agent_chat" ? (initial.action as { message?: string }).message ?? "" : "",
  );
  const [webhookUrl, setWebhookUrl] = useState(
    initial?.action.type === "webhook" ? (initial.action as { url?: string }).url ?? "" : "",
  );
  const [workDir, setWorkDir] = useState(initial?.work_dir ?? "");
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [submitting, setSubmitting] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => { nameRef.current?.focus(); }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    try {
      const action: CronJobAction =
        actionType === "agent_chat"
          ? { type: "agent_chat", agent_id: "main", message }
          : { type: "webhook", url: webhookUrl };
      await onSubmit({ name, schedule, action, enabled, work_dir: workDir || null });
    } finally {
      setSubmitting(false);
    }
  };

  const inputCls = "w-full px-3 py-2.5 text-[13px] outline-none transition-all duration-150 rounded-[var(--radius-xs)] focus:ring-2 focus:ring-[var(--tint)] focus:ring-opacity-25";
  const inputStyle: React.CSSProperties = {
    background: "var(--bg-card)",
    border: "1px solid var(--separator)",
    color: "var(--fill-primary)",
  };

  return (
    <form onSubmit={handleSubmit} className="flex flex-col gap-6">
      <fieldset disabled={submitting} className="flex flex-col gap-6">
        <FormSection title="Basic info">
          <label className="flex flex-col gap-1.5">
            <span className="text-[12px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Name</span>
            <input ref={nameRef} type="text" value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Daily health check" required className={inputCls} style={inputStyle} />
          </label>
          <label className="flex items-center gap-2.5 cursor-pointer select-none">
            <button type="button" role="switch" aria-checked={enabled} onClick={() => setEnabled(!enabled)}
              className="relative inline-flex h-[22px] w-[40px] shrink-0 items-center rounded-full transition-colors duration-200"
              style={{ background: enabled ? "var(--tint)" : "var(--bg-tertiary)", cursor: "pointer", border: "none" }}
            >
              <span className="inline-block h-4 w-4 rounded-full bg-white shadow-sm transition-transform duration-200" style={{ transform: enabled ? "translateX(20px)" : "translateX(3px)" }} />
            </button>
            <span className="text-[13px]" style={{ color: enabled ? "var(--fill-primary)" : "var(--fill-quaternary)" }}>
              {enabled ? "Enabled" : "Paused"}
            </span>
          </label>
        </FormSection>

        <FormSection title="Schedule">
          <CronScheduleHelper value={schedule} onChange={setSchedule} />
        </FormSection>

        <FormSection title="Action">
          <div className="flex gap-2">
            {(["agent_chat", "webhook"] as const).map((t) => {
              const active = actionType === t;
              const tint = t === "agent_chat" ? "var(--tint)" : "var(--orange, #ED8936)";
              return (
                <button key={t} type="button" onClick={() => setActionType(t)}
                  className="flex items-center gap-1.5 rounded-[var(--radius-xs)] px-3.5 py-2 text-[12px] font-medium transition-all duration-150 active:scale-[0.97]"
                  style={{ background: active ? `color-mix(in srgb, ${tint} 10%, transparent)` : "var(--bg-card)", color: active ? tint : "var(--fill-secondary)", border: `1px solid ${active ? tint : "var(--separator)"}`, cursor: "pointer" }}
                >
                  {t === "agent_chat" ? <Robot size={13} /> : <Globe size={13} />}
                  {t === "agent_chat" ? "Agent Chat" : "Webhook"}
                </button>
              );
            })}
          </div>

          {actionType === "agent_chat" ? (
            <label className="flex flex-col gap-1.5">
              <span className="text-[12px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Prompt</span>
              <textarea value={message} onChange={(e) => setMessage(e.target.value)} placeholder="Describe the task the agent should perform..." required rows={4} className={`${inputCls} resize-none`} style={inputStyle} />
            </label>
          ) : (
            <label className="flex flex-col gap-1.5">
              <span className="text-[12px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Webhook URL</span>
              <input type="url" value={webhookUrl} onChange={(e) => setWebhookUrl(e.target.value)} placeholder="https://example.com/webhook" required className={inputCls} style={inputStyle} />
            </label>
          )}
        </FormSection>

        {actionType === "agent_chat" && (
          <FormSection title="Workspace">
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={workDir}
                onChange={(e) => setWorkDir(e.target.value)}
                placeholder="Working directory (optional)"
                className={`${inputCls} flex-1`}
                style={inputStyle}
              />
              <button
                type="button"
                onClick={async () => {
                  try {
                    const { open: tauriOpen } = await import("@tauri-apps/plugin-dialog");
                    const selected = await tauriOpen({ directory: true, multiple: false, defaultPath: workDir || undefined }) as string | null;
                    if (selected) setWorkDir(selected);
                  } catch {
                    const selected = prompt("Enter working directory:", workDir);
                    if (selected) setWorkDir(selected);
                  }
                }}
                className="flex shrink-0 items-center justify-center rounded-[var(--radius-xs)] p-2.5 transition-colors duration-150 hover:bg-[var(--bg-hover)]"
                style={{ background: "var(--bg-card)", border: "1px solid var(--separator)", cursor: "pointer", color: "var(--fill-tertiary)" }}
                title="Browse directory"
              >
                <FolderOpen size={14} />
              </button>
            </div>
            <p className="text-[11px] leading-relaxed" style={{ color: "var(--fill-quaternary)" }}>
              The directory where the agent will execute commands. Leave empty for default.
            </p>
          </FormSection>
        )}
      </fieldset>

      <div className="flex justify-end gap-2 pt-1">
        <button type="button" onClick={onCancel} disabled={submitting}
          className="rounded-[var(--radius-xs)] px-4 py-2 text-[13px] font-medium transition-colors duration-150 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-secondary)", cursor: "pointer", background: "none", border: "none" }}
        >Cancel</button>
        <button type="submit" disabled={submitting}
          className="flex items-center gap-1.5 rounded-[var(--radius-xs)] px-5 py-2 text-[13px] font-semibold text-white transition-all duration-150 disabled:opacity-50 hover:brightness-110 active:scale-[0.97]"
          style={{ background: "var(--tint)", cursor: "pointer", border: "none" }}
        >
          {submitting && <SpinnerGap size={13} className="animate-spin" />}
          {initial ? "Save changes" : "Create"}
        </button>
      </div>
    </form>
  );
}

function FormSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-3">
      <span className="text-[11px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>{title}</span>
      <div
        className="flex flex-col gap-3.5 rounded-[var(--radius-sm)] p-4"
        style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)" }}
      >
        {children}
      </div>
    </div>
  );
}

/* ─── Job History ─── */

function JobHistory({ jobId, jobName, runs }: { jobId: string | null; jobName: string; runs: CronJobRun[] }) {
  if (!jobId) return null;

  return (
    <div>
      <div className="mb-5 flex items-center gap-2">
        <span className="text-[11px] font-semibold uppercase tracking-widest" style={{ color: "var(--fill-quaternary)" }}>History</span>
        <span style={{ color: "var(--separator)" }}>|</span>
        <span className="truncate text-[13px] font-medium" style={{ color: "var(--fill-secondary)" }}>{jobName}</span>
      </div>
      {runs.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-3 py-16 av-fade-in">
          <div className="av-float flex h-12 w-12 items-center justify-center rounded-[12px]" style={{ background: "var(--bg-primary)" }}>
            <Play size={20} style={{ color: "var(--fill-quaternary)" }} />
          </div>
          <p className="text-[13px]" style={{ color: "var(--fill-quaternary)" }}>No runs yet</p>
          <p className="text-[11px] text-center leading-relaxed" style={{ color: "var(--fill-quaternary)", maxWidth: 260 }}>
            This automation hasn&apos;t triggered yet. Runs will appear here once it executes.
          </p>
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {runs.map((run, idx) => (
            <div
              key={run.id}
              className="av-stagger flex items-center gap-3 rounded-[var(--radius-xs)] px-3.5 py-3 transition-colors duration-150 hover:bg-[var(--bg-hover)]"
              style={{ "--stagger-i": idx } as React.CSSProperties}
            >
              <RunDot status={run.status} />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 text-[12px]" style={{ color: "var(--fill-primary)" }}>
                  <span className="tabular-nums">{new Date(run.started_at).toLocaleString()}</span>
                  {run.ended_at && <span className="tabular-nums" style={{ color: "var(--fill-quaternary)" }}>{fmtDuration(new Date(run.started_at), new Date(run.ended_at))}</span>}
                </div>
                {run.error && <div className="mt-0.5 truncate text-[11px]" style={{ color: "var(--red, #E53E3E)" }}>{run.error}</div>}
                {run.output && !run.error && <div className="mt-0.5 truncate text-[11px]" style={{ color: "var(--fill-quaternary)" }}>{run.output}</div>}
              </div>
              <RunPill status={run.status} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/* ─── Shared micro-components ─── */

function IconBtn({ icon, title, onClick, className = "" }: { icon: React.ReactNode; title: string; onClick: (e: React.MouseEvent) => void; className?: string }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-[var(--radius-xs)] p-1.5 transition-all duration-200 hover:bg-[var(--bg-hover)] active:scale-90 ${className}`}
      title={title}
      aria-label={title}
      style={{ cursor: "pointer", background: "none", border: "none" }}
    >
      {icon}
    </button>
  );
}

function StatusPill({ status }: { status: "active" | "failed" | "paused" }) {
  const c = {
    active: { bg: "color-mix(in srgb, var(--green, #38A169) 10%, transparent)", fg: "var(--green, #38A169)" },
    failed: { bg: "color-mix(in srgb, var(--red, #E53E3E) 10%, transparent)", fg: "var(--red, #E53E3E)" },
    paused: { bg: "var(--bg-tertiary)", fg: "var(--fill-quaternary)" },
  }[status];
  return (
    <span
      className="shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide"
      style={{ background: c.bg, color: c.fg }}
    >
      {status}
    </span>
  );
}

function RunDot({ status }: { status: string }) {
  const isRunning = status === "running";
  const color = status === "completed" || status === "ok" ? "var(--green, #38A169)" : isRunning ? "var(--orange, #ED8936)" : status === "failed" ? "var(--red, #E53E3E)" : "var(--fill-quaternary)";
  return (
    <span className="relative flex h-2 w-2 shrink-0">
      {isRunning && <span className="absolute inline-flex h-full w-full animate-ping rounded-full opacity-40" style={{ background: color }} />}
      <span className="relative inline-flex h-2 w-2 rounded-full" style={{ background: color }} />
    </span>
  );
}

function RunPill({ status }: { status: string }) {
  const m: Record<string, { bg: string; fg: string }> = {
    completed: { bg: "color-mix(in srgb, var(--green, #38A169) 8%, transparent)", fg: "var(--green, #38A169)" },
    ok: { bg: "color-mix(in srgb, var(--green, #38A169) 8%, transparent)", fg: "var(--green, #38A169)" },
    running: { bg: "color-mix(in srgb, var(--orange, #ED8936) 8%, transparent)", fg: "var(--orange, #ED8936)" },
    failed: { bg: "color-mix(in srgb, var(--red, #E53E3E) 8%, transparent)", fg: "var(--red, #E53E3E)" },
  };
  const c = m[status] ?? { bg: "var(--bg-tertiary)", fg: "var(--fill-quaternary)" };
  return <span className="shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide" style={{ background: c.bg, color: c.fg }}>{status === "completed" ? "ok" : status}</span>;
}

function relativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function fmtDuration(start: Date, end: Date): string {
  const ms = end.getTime() - start.getTime();
  if (ms < 1000) return `${ms}ms`;
  const secs = Math.round(ms / 1000);
  if (secs < 60) return `${secs}s`;
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

/* ─── Animation CSS ─── */

const ANIM_CSS = `
@media (prefers-reduced-motion: no-preference) {
  .av-fade-in {
    animation: avFadeIn 220ms cubic-bezier(0.16, 1, 0.3, 1) both;
  }
  .av-slide-in-right {
    animation: avSlideInRight 280ms cubic-bezier(0.16, 1, 0.3, 1) both;
  }
  .av-backdrop-enter {
    animation: avFadeIn 200ms ease-out both;
  }
  .av-dialog-enter {
    animation: avDialogIn 240ms cubic-bezier(0.16, 1, 0.3, 1) both;
  }
  .av-float {
    animation: avFloat 4s ease-in-out infinite;
  }
  .av-stagger {
    animation: avFadeUp 260ms cubic-bezier(0.16, 1, 0.3, 1) both;
    animation-delay: calc(var(--stagger-i, 0) * 35ms);
  }
}

@keyframes avFadeIn {
  from { opacity: 0; }
  to   { opacity: 1; }
}
@keyframes avSlideInRight {
  from { opacity: 0; transform: translateX(10px); }
  to   { opacity: 1; transform: translateX(0); }
}
@keyframes avDialogIn {
  from { opacity: 0; transform: scale(0.96) translateY(4px); }
  to   { opacity: 1; transform: scale(1) translateY(0); }
}
@keyframes avFloat {
  0%, 100% { transform: translateY(0); }
  50%      { transform: translateY(-5px); }
}
@keyframes avFadeUp {
  from { opacity: 0; transform: translateY(4px); }
  to   { opacity: 1; transform: translateY(0); }
}
`;
