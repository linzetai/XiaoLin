import { useState, useEffect, useCallback, useMemo } from "react";
import { ChevronDown, ChevronRight, AlertTriangle, Plus, Pencil, Clock, RefreshCw } from "lucide-react";
import { useAgentStore } from "../../lib/agent-store";
import { useGatewayStore } from "../../lib/store";
import * as api from "../../lib/api";

function parseUtc(ts: string): Date {
  if (!ts || ts.endsWith("Z") || /[+-]\d{2}:\d{2}$/.test(ts)) return new Date(ts);
  return new Date(ts.replace(" ", "T") + "Z");
}
import type { CronJob, CronJobAction, CronJobRun, NotifyChannel } from "../../lib/transport";
import { FormModal, ListContainer, SectionHeader, Toggle } from "./common";


type ScheduleMode = "every_n_min" | "every_n_hour" | "daily" | "weekly" | "custom";

function scheduleToMode(schedule: string): { mode: ScheduleMode; minutes?: number; hours?: number; atHour?: number; atMin?: number; weekdays?: number[] } {
  const parts = schedule.trim().split(/\s+/);
  if (parts.length !== 6) return { mode: "custom" };
  const [sec, min, hour, _dom, _mon, dow] = parts;
  if (sec !== "0") return { mode: "custom" };
  const mInterval = min.match(/^\*\/(\d+)$/);
  if (mInterval && hour === "*" && dow === "*") return { mode: "every_n_min", minutes: parseInt(mInterval[1]) };
  const hInterval = hour.match(/^\*\/(\d+)$/);
  if (min === "0" && hInterval && dow === "*") return { mode: "every_n_hour", hours: parseInt(hInterval[1]) };
  if (/^\d+$/.test(min) && /^\d+$/.test(hour) && dow === "*") return { mode: "daily", atHour: parseInt(hour), atMin: parseInt(min) };
  if (/^\d+$/.test(min) && /^\d+$/.test(hour) && /^[\d,\-]+$/.test(dow)) {
    const days = dow.split(",").flatMap(s => { const r = s.split("-"); return r.length === 2 ? Array.from({ length: parseInt(r[1]) - parseInt(r[0]) + 1 }, (_, i) => parseInt(r[0]) + i) : [parseInt(s)]; });
    return { mode: "weekly", atHour: parseInt(hour), atMin: parseInt(min), weekdays: days };
  }
  return { mode: "custom" };
}

function modeToSchedule(mode: ScheduleMode, opts: { minutes?: number; hours?: number; atHour?: number; atMin?: number; weekdays?: number[]; custom?: string }): string {
  switch (mode) {
    case "every_n_min": return `0 */${opts.minutes ?? 5} * * * *`;
    case "every_n_hour": return `0 0 */${opts.hours ?? 1} * * *`;
    case "daily": return `0 ${opts.atMin ?? 0} ${opts.atHour ?? 9} * * *`;
    case "weekly": return `0 ${opts.atMin ?? 0} ${opts.atHour ?? 9} * * ${(opts.weekdays ?? [1, 2, 3, 4, 5]).join(",")}`;
    case "custom": return opts.custom ?? "0 */5 * * * *";
  }
}

const WEEKDAY_NAMES = ["日", "一", "二", "三", "四", "五", "六"];

function SchedulePicker({ schedule, onChange }: { schedule: string; onChange: (s: string) => void }) {
  const parsed = useMemo(() => scheduleToMode(schedule), [schedule]);
  const [mode, setMode] = useState<ScheduleMode>(parsed.mode);
  const [minutes, setMinutes] = useState(parsed.minutes ?? 5);
  const [hours, setHours] = useState(parsed.hours ?? 1);
  const [atHour, setAtHour] = useState(parsed.atHour ?? 9);
  const [atMin, setAtMin] = useState(parsed.atMin ?? 0);
  const [weekdays, setWeekdays] = useState<number[]>(parsed.weekdays ?? [1, 2, 3, 4, 5]);
  const [custom, setCustom] = useState(schedule);

  const emit = useCallback((m: ScheduleMode, o: Parameters<typeof modeToSchedule>[1]) => {
    onChange(modeToSchedule(m, o));
  }, [onChange]);

  const selectCls = "select-premium";
  const selectStyle = {} as React.CSSProperties;
  const inlineCls = "rounded-[6px] px-2.5 py-1.5 text-[13px] outline-none text-center transition-colors focus:outline-none";
  const inlineStyle = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };
  const labelStyle = { color: "var(--fill-tertiary)" };

  return (
    <div className="space-y-2">
      <label className="mb-1 block text-[11px] font-medium" style={labelStyle}>执行频率</label>
      <div className="relative">
        <select value={mode} onChange={(e) => { const m = e.target.value as ScheduleMode; setMode(m); emit(m, { minutes, hours, atHour, atMin, weekdays, custom }); }} className={selectCls} style={selectStyle}>
          <option value="every_n_min">每隔 N 分钟</option>
          <option value="every_n_hour">每隔 N 小时</option>
          <option value="daily">每天定时</option>
          <option value="weekly">每周定时</option>
          <option value="custom">自定义 Cron</option>
        </select>
        <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={labelStyle} />
      </div>

      {mode === "every_n_min" && (
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
          <span>每</span>
          <input type="number" min={1} max={59} value={minutes} onChange={(e) => { const v = Math.max(1, Math.min(59, parseInt(e.target.value) || 1)); setMinutes(v); emit(mode, { minutes: v }); }} className={inlineCls + " w-16"} style={inlineStyle} />
          <span>分钟</span>
        </div>
      )}
      {mode === "every_n_hour" && (
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
          <span>每</span>
          <input type="number" min={1} max={23} value={hours} onChange={(e) => { const v = Math.max(1, Math.min(23, parseInt(e.target.value) || 1)); setHours(v); emit(mode, { hours: v }); }} className={inlineCls + " w-16"} style={inlineStyle} />
          <span>小时</span>
        </div>
      )}
      {(mode === "daily" || mode === "weekly") && (
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
          <span>时间</span>
          <input type="number" min={0} max={23} value={atHour} onChange={(e) => { const v = Math.max(0, Math.min(23, parseInt(e.target.value) || 0)); setAtHour(v); emit(mode, { atHour: v, atMin, weekdays }); }} className={inlineCls + " w-14"} style={inlineStyle} />
          <span>:</span>
          <input type="number" min={0} max={59} value={atMin} onChange={(e) => { const v = Math.max(0, Math.min(59, parseInt(e.target.value) || 0)); setAtMin(v); emit(mode, { atHour, atMin: v, weekdays }); }} className={inlineCls + " w-14"} style={inlineStyle} />
        </div>
      )}
      {mode === "weekly" && (
        <div className="flex flex-wrap gap-1">
          {WEEKDAY_NAMES.map((name, i) => (
            <button key={i} onClick={() => { const next = weekdays.includes(i) ? weekdays.filter(d => d !== i) : [...weekdays, i].sort(); setWeekdays(next); emit(mode, { atHour, atMin, weekdays: next }); }}
              className="cursor-pointer rounded-[4px] px-2 py-1 text-[11px] font-medium transition-colors"
              style={{ background: weekdays.includes(i) ? "var(--fill-primary)" : "var(--bg-base)", color: weekdays.includes(i) ? "var(--fill-inverse)" : "var(--fill-tertiary)", border: "0.5px solid var(--separator-opaque)" }}
            >{name}</button>
          ))}
        </div>
      )}
      {mode === "custom" && (
        <div>
          <input value={custom} onChange={(e) => { setCustom(e.target.value); onChange(e.target.value); }} placeholder="0 */5 * * * *" className={"w-full rounded-[6px] px-3 py-2 font-mono text-[13px] outline-none transition-colors focus:outline-none"} style={inlineStyle} />
          <p className="mt-1 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>6 字段: 秒 分 时 日 月 周</p>
        </div>
      )}
    </div>
  );
}

/* ━━━ Run Log viewer ━━━ */

function RunLogList({ jobId }: { jobId: string }) {
  const [runs, setRuns] = useState<CronJobRun[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<number | null>(null);

  useEffect(() => {
    setLoading(true);
    api.listCronRuns(jobId, 20).then(setRuns).finally(() => setLoading(false));
  }, [jobId]);

  if (loading) return <p className="py-3 text-center text-[11px]" style={{ color: "var(--fill-quaternary)" }}>加载中...</p>;
  if (runs.length === 0) return <p className="py-3 text-center text-[11px]" style={{ color: "var(--fill-quaternary)" }}>暂无执行记录</p>;

  return (
    <div className="space-y-1">
      {runs.map((run) => (
        <div key={run.id} className="rounded-[6px] text-[11px]" style={{ background: "var(--bg-base)", border: "0.5px solid var(--separator-opaque)" }}>
          <div className="flex cursor-pointer items-center justify-between gap-2 px-2.5 py-1.5" onClick={() => setExpanded(expanded === run.id ? null : run.id)}>
            <div className="flex items-center gap-2">
              <span className="inline-block h-[6px] w-[6px] rounded-full" style={{ background: run.status === "ok" ? "var(--green, #48bb78)" : run.status === "running" ? "var(--blue, #4299e1)" : "var(--red, #e53e3e)" }} />
              <span style={{ color: "var(--fill-secondary)" }}>{parseUtc(run.started_at).toLocaleString("zh-CN")}</span>
            </div>
            <span style={{ color: "var(--fill-quaternary)" }}>{run.status === "ok" ? "成功" : run.status === "running" ? "运行中" : "失败"}</span>
          </div>
          {expanded === run.id && (
            <div className="border-t px-2.5 py-2" style={{ borderColor: "var(--separator-opaque)" }}>
              {run.output && (
                <div className="mb-1">
                  <span className="font-medium" style={{ color: "var(--fill-tertiary)" }}>Agent 回复:</span>
                  <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-[4px] p-2 text-[11px]" style={{ background: "var(--bg-tertiary)", color: "var(--fill-secondary)" }}>{run.output}</pre>
                </div>
              )}
              {run.error && (
                <div>
                  <span className="font-medium" style={{ color: "var(--red, #e53e3e)" }}>错误:</span>
                  <pre className="mt-1 max-h-24 overflow-auto whitespace-pre-wrap rounded-[4px] p-2 text-[11px]" style={{ background: "var(--bg-tertiary)", color: "var(--red, #e53e3e)" }}>{run.error}</pre>
                </div>
              )}
              {run.ended_at && (
                <p className="mt-1" style={{ color: "var(--fill-quaternary)" }}>
                  耗时: {Math.round((parseUtc(run.ended_at).getTime() - parseUtc(run.started_at).getTime()) / 1000)}s
                </p>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

/* ━━━ Cron Job Form ━━━ */

const EMPTY_CRON_JOB: Partial<CronJob> & { schedule: string; action: CronJobAction } = {
  name: "",
  schedule: "0 */5 * * * *",
  action: { type: "agent_chat", agent_id: "", message: "" },
  enabled: true,
  notify_channels: [],
};

function CronJobForm({
  job,
  agentId,
  isNew,
  onSave,
  onCancel,
  onDelete,
  saving,
}: {
  job: Partial<CronJob> & { schedule: string; action: CronJobAction };
  agentId: string;
  isNew: boolean;
  onSave: (j: Partial<CronJob> & { schedule: string; action: CronJobAction }) => void;
  onCancel: () => void;
  onDelete?: () => void;
  saving: boolean;
}) {
  const [form, setForm] = useState({ ...job });
  const [actionType, setActionType] = useState<"agent_chat" | "webhook">(
    job.action?.type === "webhook" ? "webhook" : "agent_chat",
  );
  const [showLogs, setShowLogs] = useState(!isNew);
  const [showNotifyChannels, setShowNotifyChannels] = useState(false);
  const [notifyChannels, setNotifyChannels] = useState<NotifyChannel[]>(job.notify_channels || []);
  const [newChannel, setNewChannel] = useState<NotifyChannel>({ channel_id: "", target_id: "", target_type: "p2p" });
  const [duplicateWarning, setDuplicateWarning] = useState(false);

  const inputCls = "w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:outline-none";
  const inputStyle = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };
  const labelCls = "mb-1 block text-[11px] font-medium";
  const labelStyle = { color: "var(--fill-tertiary)" };

  const handleSubmit = () => {
    const action: CronJobAction = actionType === "webhook"
      ? { type: "webhook", url: form.action?.url ?? "", method: form.action?.method ?? "POST", body: form.action?.body }
      : { type: "agent_chat", agent_id: agentId, message: form.action?.message ?? "" };
    onSave({ ...form, action, notify_channels: notifyChannels });
  };

  return (
    <div className="space-y-3">
      <div>
        <label className={labelCls} style={labelStyle}>任务名称</label>
        <input
          value={form.name ?? ""}
          onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
          placeholder="每日汇报"
          className={inputCls}
          style={inputStyle}
        />
      </div>

      <SchedulePicker schedule={form.schedule} onChange={(s) => setForm((f) => ({ ...f, schedule: s }))} />

      <div>
        <label className={labelCls} style={labelStyle}>动作类型</label>
        <div className="relative">
          <select
            value={actionType}
            onChange={(e) => setActionType(e.target.value as "agent_chat" | "webhook")}
            className="select-premium"
          >
            <option value="agent_chat">Agent 对话</option>
            <option value="webhook">Webhook</option>
          </select>
          <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={labelStyle} />
        </div>
      </div>

      {actionType === "agent_chat" ? (
        <div>
          <label className={labelCls} style={labelStyle}>消息内容</label>
          <textarea
            value={form.action?.message ?? ""}
            onChange={(e) =>
              setForm((f) => ({ ...f, action: { ...f.action, message: e.target.value } }))
            }
            placeholder="请生成今日工作汇报"
            rows={3}
            className={inputCls + " resize-none"}
            style={inputStyle}
          />
        </div>
      ) : (
        <>
          <div>
            <label className={labelCls} style={labelStyle}>Webhook URL</label>
            <input
              value={form.action?.url ?? ""}
              onChange={(e) =>
                setForm((f) => ({ ...f, action: { ...f.action, url: e.target.value } }))
              }
              placeholder="https://example.com/webhook"
              className={inputCls + " font-mono"}
              style={inputStyle}
            />
          </div>
          <div>
            <label className={labelCls} style={labelStyle}>HTTP 方法</label>
            <div className="relative">
              <select
                value={form.action?.method ?? "POST"}
                onChange={(e) =>
                  setForm((f) => ({ ...f, action: { ...f.action, method: e.target.value } }))
                }
                className="select-premium select-mono"
              >
                <option value="POST">POST</option>
                <option value="GET">GET</option>
                <option value="PUT">PUT</option>
                <option value="DELETE">DELETE</option>
              </select>
              <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={labelStyle} />
            </div>
          </div>
        </>
      )}

      <div className="flex items-center gap-2 pt-1">
        <label className={labelCls} style={labelStyle}>启用</label>
        <Toggle
          checked={form.enabled !== false}
          onChange={(v) => setForm((f) => ({ ...f, enabled: v }))}
        />
      </div>

      {/* 通知渠道配置 - 独立段落 */}
      <div className="pt-1">
        <button
          type="button"
          onClick={() => setShowNotifyChannels(!showNotifyChannels)}
          className="flex w-full cursor-pointer items-center gap-1 text-[11px] font-medium transition-colors"
          style={{ color: "var(--fill-tertiary)" }}
        >
          {showNotifyChannels ? <ChevronDown size={10} strokeWidth={2} /> : <ChevronRight size={10} strokeWidth={2} />}
          通知渠道 {notifyChannels.length > 0 && <span className="rounded-full px-1.5 text-[10px]" style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}>{notifyChannels.length}</span>}
        </button>
      </div>

      {showNotifyChannels && (
        <div className="rounded-[6px] p-3 space-y-3" style={{ background: "var(--bg-tertiary)", border: "0.5px solid var(--separator-opaque)" }}>
          {notifyChannels.length > 0 && (
            <div className="space-y-1.5">
              {notifyChannels.map((channel, index) => (
                <div key={index} className="flex items-center gap-2 rounded-[4px] px-2.5 py-2" style={{ background: "var(--bg-base)", border: "0.5px solid var(--separator-opaque)" }}>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 text-[12px] font-medium" style={{ color: "var(--fill-primary)" }}>
                      <span>
                        {channel.channel_id === "feishu" ? "飞书" :
                         channel.channel_id === "lark" ? "Lark" :
                         channel.channel_id === "slack" ? "Slack" :
                         channel.channel_id === "discord" ? "Discord" :
                         channel.channel_id === "matrix" ? "Matrix" :
                         channel.channel_id === "msteams" ? "Teams" :
                         channel.channel_id === "whatsapp" ? "WhatsApp" :
                         channel.channel_id}
                      </span>
                      <span className="rounded-[3px] px-1 text-[9px]" style={{ background: "var(--bg-tertiary)", color: "var(--fill-quaternary)" }}>
                        {channel.target_type === "p2p" ? "私聊" : "群组"}
                      </span>
                    </div>
                    <div className="mt-0.5 truncate font-mono text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                      {channel.target_id}
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={() => setNotifyChannels(notifyChannels.filter((_, i) => i !== index))}
                    className="shrink-0 cursor-pointer rounded-[4px] px-1.5 py-0.5 text-[10px] transition-colors hover:opacity-80"
                    style={{ color: "var(--red, #e53e3e)" }}
                  >
                    移除
                  </button>
                </div>
              ))}
            </div>
          )}

          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <div className="relative flex-1">
                <select
                  value={newChannel.channel_id}
                  onChange={(e) => setNewChannel({...newChannel, channel_id: e.target.value})}
                  className="select-premium text-[11px]"
                >
                  <option value="">渠道类型</option>
                  <option value="feishu">飞书</option>
                  <option value="lark">Lark</option>
                  <option value="slack">Slack</option>
                  <option value="discord">Discord</option>
                  <option value="matrix">Matrix</option>
                  <option value="msteams">Teams</option>
                  <option value="whatsapp">WhatsApp</option>
                  <option value="telegram">Telegram</option>
                </select>
                <ChevronDown size={9} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2" style={labelStyle} />
              </div>
              <div className="relative" style={{ width: "80px" }}>
                <select
                  value={newChannel.target_type}
                  onChange={(e) => setNewChannel({...newChannel, target_type: e.target.value as "p2p" | "group"})}
                  className="select-premium text-[11px]"
                >
                  <option value="p2p">私聊</option>
                  <option value="group">群组</option>
                </select>
                <ChevronDown size={9} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2" style={labelStyle} />
              </div>
            </div>
            <div className="flex items-center gap-2">
              <input
                value={newChannel.target_id}
                onChange={(e) => setNewChannel({...newChannel, target_id: e.target.value})}
                placeholder="目标 ID（chat_id / open_id）"
                className={inputCls + " flex-1 text-[11px]"}
                style={inputStyle}
              />
              <button
                type="button"
                onClick={() => {
                  if (newChannel.channel_id && newChannel.target_id) {
                    const exists = notifyChannels.some(
                      ch => ch.channel_id === newChannel.channel_id && 
                            ch.target_id === newChannel.target_id &&
                            ch.target_type === newChannel.target_type
                    );
                    if (!exists) {
                      setNotifyChannels([...notifyChannels, { ...newChannel }]);
                      setNewChannel({ channel_id: "", target_id: "", target_type: "p2p" });
                      setDuplicateWarning(false);
                    } else {
                      setDuplicateWarning(true);
                      setTimeout(() => setDuplicateWarning(false), 3000);
                    }
                  }
                }}
                disabled={!newChannel.channel_id || !newChannel.target_id}
                className="shrink-0 cursor-pointer rounded-[6px] px-3 py-2 text-[11px] font-medium transition-colors hover:opacity-90 disabled:opacity-40"
                style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
              >
                <Plus size={11} strokeWidth={2} />
              </button>
            </div>
            {duplicateWarning && (
              <p className="text-[10px]" style={{ color: "var(--red, #e53e3e)" }}>此渠道配置已存在</p>
            )}
          </div>
          <p className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
            任务完成或失败时发送通知
          </p>
        </div>
      )}

      {!isNew && job.id && (
        <div className="pt-1">
          <button
            onClick={() => setShowLogs(!showLogs)}
            className="flex w-full cursor-pointer items-center gap-1 text-[11px] font-medium transition-colors"
            style={{ color: "var(--fill-tertiary)" }}
          >
            {showLogs ? <ChevronDown size={10} strokeWidth={2} /> : <ChevronRight size={10} strokeWidth={2} />}
            执行记录 {(job as CronJob).run_count > 0 && `(${(job as CronJob).run_count})`}
          </button>
          {showLogs && <div className="mt-2"><RunLogList jobId={job.id!} /></div>}
        </div>
      )}

      <div className="flex items-center justify-between pt-1">
        <div>
          {!isNew && onDelete && (
            <button
              onClick={onDelete}
              disabled={saving}
              className="rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors hover:opacity-80"
              style={{ color: "var(--red, #e53e3e)" }}
            >
              删除任务
            </button>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={onCancel}
            disabled={saving}
            className="cursor-pointer rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors"
            style={{ color: "var(--fill-secondary)" }}
          >
            取消
          </button>
          <button
            onClick={handleSubmit}
            disabled={saving || !form.name || !form.schedule}
            className="cursor-pointer rounded-[6px] px-4 py-1.5 text-[12px] font-medium transition-colors hover:opacity-90 disabled:opacity-50"
            style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
          >
            {saving ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function CronTab() {
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const gatewayReady = useGatewayStore((s) => s.connected);

  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [loading, setLoading] = useState(false);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const loadJobs = useCallback(async () => {
    if (!gatewayReady) return;
    setLoading(true);
    const list = await api.listCronJobs(activeAgentId);
    setJobs(list);
    setLoading(false);
  }, [activeAgentId, gatewayReady]);

  useEffect(() => {
    loadJobs();
  }, [loadJobs]);

  const handleCreate = useCallback(async (job: Partial<CronJob> & { schedule: string; action: CronJobAction }) => {
    setSaving(true);
    try {
      await api.upsertCronJob({
        id: "",
        name: job.name ?? "Unnamed",
        schedule: job.schedule,
        action: job.action,
        enabled: job.enabled !== false,
        status: "idle",
        run_count: 0,
        error_count: 0,
        created_at: new Date().toISOString(),
        last_run: null,
        next_run: null,
        last_error: null,
        notify_channels: job.notify_channels ?? [],
      } as CronJob & { schedule: string; action: CronJobAction });
      setAdding(false);
      await loadJobs();
    } catch (e) {
      console.error("[cron] create failed:", e);
    }
    setSaving(false);
  }, [loadJobs]);

  const handleUpdate = useCallback(async (job: Partial<CronJob> & { schedule: string; action: CronJobAction }) => {
    if (!editing) return;
    setSaving(true);
    try {
      const existing = jobs.find((j) => j.id === editing);
      if (existing) {
        await api.upsertCronJob({
          ...existing,
          ...job,
          notify_channels: job.notify_channels ?? existing.notify_channels,
        } as CronJob & { schedule: string; action: CronJobAction });
      }
      setEditing(null);
      await loadJobs();
    } catch (e) {
      console.error("[cron] update failed:", e);
    }
    setSaving(false);
  }, [editing, jobs, loadJobs]);

  const handleDelete = useCallback(async (jobId: string) => {
    setSaving(true);
    try {
      await api.deleteCronJob(jobId);
      setEditing(null);
      await loadJobs();
    } catch (e) {
      console.error("[cron] delete failed:", e);
    }
    setSaving(false);
  }, [loadJobs]);

  const handleToggle = useCallback(async (jobId: string, enabled: boolean) => {
    const job = jobs.find((j) => j.id === jobId);
    if (!job) return;
    setJobs((prev) => prev.map((j) => j.id === jobId ? { ...j, enabled } : j));
    try {
      await api.upsertCronJob({ ...job, enabled } as CronJob & { schedule: string; action: CronJobAction });
    } catch {
      setJobs((prev) => prev.map((j) => j.id === jobId ? { ...j, enabled: !enabled } : j));
    }
  }, [jobs]);

  const editingJob = editing ? jobs.find((j) => j.id === editing) : null;

  const formatStatus = (job: CronJob) => {
    if (job.status === "running") return "运行中";
    if (job.status === "failed") return "失败";
    if (!job.enabled) return "已禁用";
    return "空闲";
  };

  const statusColor = (job: CronJob) => {
    if (job.status === "running") return "var(--blue, #4299e1)";
    if (job.status === "failed") return "var(--red, #e53e3e)";
    if (!job.enabled) return "var(--fill-quaternary)";
    return "var(--green, #48bb78)";
  };

  return (
    <div className="space-y-4 p-4">
      <div className="flex items-center justify-between">
        <SectionHeader count={jobs.filter((j) => j.enabled).length} total={jobs.length}>
          定时任务
        </SectionHeader>
        <div className="flex items-center gap-1">
          <button
            onClick={loadJobs}
            disabled={loading}
            className="cursor-pointer rounded-[var(--radius-xs)] p-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-40"
            title="刷新"
          >
            <RefreshCw size={13} strokeWidth={1.5} className={loading ? "animate-spin" : ""} style={{ color: "var(--fill-tertiary)" }} />
          </button>
          {!adding && (
            <button
              onClick={() => { setAdding(true); setEditing(null); }}
              className="flex cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] p-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <Plus size={11} strokeWidth={2} /> 新增
            </button>
          )}
        </div>
      </div>

      <FormModal open={adding} onClose={() => setAdding(false)} title="新增定时任务">
        <CronJobForm
          job={{ ...EMPTY_CRON_JOB, action: { ...EMPTY_CRON_JOB.action, agent_id: activeAgentId } }}
          agentId={activeAgentId}
          isNew
          onSave={handleCreate}
          onCancel={() => setAdding(false)}
          saving={saving}
        />
      </FormModal>

      {editingJob && (
        <FormModal open onClose={() => setEditing(null)} title={`编辑 — ${editingJob.name}`}>
          <CronJobForm
            job={editingJob}
            agentId={activeAgentId}
            isNew={false}
            onSave={handleUpdate}
            onCancel={() => setEditing(null)}
            onDelete={() => handleDelete(editingJob.id)}
            saving={saving}
          />
        </FormModal>
      )}

      {jobs.length === 0 ? (
        <ListContainer>
          <div className="px-3 py-6 text-center">
            <Clock size={18} strokeWidth={1.5} className="mx-auto mb-2" style={{ color: "var(--fill-quaternary)" }} />
            <p className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              暂无定时任务
            </p>
            <p className="mt-1 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              添加定时任务，Agent 将按设定时间自动执行
            </p>
          </div>
        </ListContainer>
      ) : (
        <ListContainer>
          {jobs.map((job, i) => (
            <div
              key={job.id}
              className="group cursor-pointer px-3 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{ borderBottom: i < jobs.length - 1 ? "0.5px solid var(--separator)" : undefined }}
              onClick={() => { setEditing(job.id); setAdding(false); }}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex min-w-0 flex-1 items-center gap-2">
                  <Clock size={13} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                  <span className="truncate text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
                    {job.name}
                  </span>
                  <span
                    className="inline-block h-[6px] w-[6px] shrink-0 rounded-full"
                    style={{ background: statusColor(job) }}
                    title={formatStatus(job)}
                  />
                </div>
                <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
                  <Toggle
                    checked={job.enabled}
                    onChange={(v) => { handleToggle(job.id, v); }}
                  />
                  <Pencil size={12} strokeWidth={1.5} className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100" style={{ color: "var(--fill-quaternary)" }} />
                </div>
              </div>
              <div className="mt-1 flex items-center gap-3 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
                <span className="font-mono">{job.schedule}</span>
                <span>·</span>
                <span>{job.action.type === "agent_chat" ? "Agent 对话" : "Webhook"}</span>
                {job.run_count > 0 && (
                  <>
                    <span>·</span>
                    <span>已执行 {job.run_count} 次</span>
                  </>
                )}
              </div>
              {job.last_error && (
                <div className="mt-1 flex items-center gap-1 text-[10px]" style={{ color: "var(--red, #e53e3e)" }}>
                  <AlertTriangle size={10} strokeWidth={1.5} />
                  <span className="truncate">{job.last_error}</span>
                </div>
              )}
              {job.next_run && (
                <div className="mt-0.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                  下次执行: {parseUtc(job.next_run).toLocaleString("zh-CN")}
                </div>
              )}
            </div>
          ))}
        </ListContainer>
      )}
    </div>
  );
}
