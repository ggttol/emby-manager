import { CalendarClock, Pencil, Play, Plus, RefreshCw, Save, Trash2 } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useTaskCompletion } from '../hooks/useTaskCompletion';
import { ConfirmDanger } from './Modal';
import { useToast } from './Toast';

type ScheduleJob = components['schemas']['ScheduleJob'];
type ScheduleRequest = components['schemas']['ScheduleRequest'];
type RunScheduleResponse = components['schemas']['RunScheduleResponse'];
type TaskRun = components['schemas']['TaskRun'];

type ScheduleMode = 'daily' | 'weekly' | 'monthly';

type ScheduleSpec = {
  mode: ScheduleMode;
  hour: number;
  minute: number;
  weekday: number;
  day: number;
};

type Draft = {
  id?: string;
  name: string;
  kind: string;
  enabled: boolean;
  schedule: ScheduleSpec;
  paramsJson: string;
};

const kindOptions = [
  { kind: 'scan_all', label: '扫全库', desc: '逐库生成缺失 STRM、清孤儿并刷新变更库' },
  { kind: 'zhuigeng_scan_airing', label: '追更扫描', desc: '对所有在更剧用剧名扫对应库，拿新集' },
  { kind: 'fix_posters_all', label: '海报修复', desc: '对所有无海报项跑保守自动匹配' },
  { kind: 'refresh_no_rating_all', label: '刷新无评分', desc: '对所有无评分剧调 Emby Refresh 重拉 TMDb' },
  { kind: 'monitor_incremental', label: '增量补扫', desc: 'autostrm webhook 兜底，只扫 mtime 变新的 top 目录' }
];

const weekdayLabels = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function asObject(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function numberValue(value: unknown, fallback: number) {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function stringValue(value: unknown, fallback: string) {
  return typeof value === 'string' && value ? value : fallback;
}

function normalizeSchedule(value: unknown): ScheduleSpec {
  const obj = asObject(value);
  const mode = stringValue(obj.mode, 'daily');
  return {
    mode: mode === 'weekly' || mode === 'monthly' ? mode : 'daily',
    hour: numberValue(obj.hour, 3),
    minute: numberValue(obj.minute, 0),
    weekday: numberValue(obj.weekday, 0),
    day: numberValue(obj.day, 1)
  };
}

function draftFromJob(job?: ScheduleJob): Draft {
  return {
    id: job?.id,
    name: job?.name || '',
    kind: job?.kind || kindOptions[0].kind,
    enabled: job?.enabled ?? true,
    schedule: normalizeSchedule(job?.schedule),
    paramsJson: JSON.stringify(asObject(job?.params), null, 2)
  };
}

function validateDraft(draft: Draft): ScheduleRequest {
  const hour = Number(draft.schedule.hour);
  const minute = Number(draft.schedule.minute);
  const weekday = Number(draft.schedule.weekday);
  const day = Number(draft.schedule.day);
  if (!Number.isInteger(hour) || hour < 0 || hour > 23) throw new Error('小时必须是 0 到 23');
  if (!Number.isInteger(minute) || minute < 0 || minute > 59) throw new Error('分钟必须是 0 到 59');
  if (draft.schedule.mode === 'weekly' && (!Number.isInteger(weekday) || weekday < 0 || weekday > 6)) {
    throw new Error('星期必须是周一到周日');
  }
  if (draft.schedule.mode === 'monthly' && (!Number.isInteger(day) || day < 1 || day > 31)) {
    throw new Error('日期必须是 1 到 31');
  }
  let params: unknown;
  try {
    params = draft.paramsJson.trim() ? JSON.parse(draft.paramsJson) : {};
  } catch {
    throw new Error('参数 JSON 不是合法 JSON');
  }
  if (!params || typeof params !== 'object' || Array.isArray(params)) throw new Error('参数 JSON 必须是对象');
  return {
    name: draft.name.trim() || kindLabel(draft.kind),
    kind: draft.kind,
    enabled: draft.enabled,
    params,
    schedule: {
      mode: draft.schedule.mode,
      hour,
      minute,
      weekday,
      day
    }
  };
}

function kindLabel(kind: string) {
  return kindOptions.find((item) => item.kind === kind)?.label || kind;
}

function kindDesc(kind: string) {
  return kindOptions.find((item) => item.kind === kind)?.desc || '';
}

function scheduleText(schedule: unknown) {
  const spec = normalizeSchedule(schedule);
  const time = `${String(spec.hour).padStart(2, '0')}:${String(spec.minute).padStart(2, '0')}`;
  if (spec.mode === 'daily') return `每天 ${time}`;
  if (spec.mode === 'weekly') return `每周 ${weekdayLabels[spec.weekday] || '周一'} ${time}`;
  return `每月 ${spec.day} 日 ${time}`;
}

function dateText(value?: string | null) {
  if (!value) return '未运行';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  });
}

function statusClass(status?: string | null) {
  if (!status) return 'pending';
  if (status === 'done') return 'done';
  if (status === 'running') return 'running';
  if (status === 'error') return 'error';
  return status;
}

export function SchedulesPanel() {
  const [jobs, setJobs] = useState<ScheduleJob[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ScheduleJob | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [runningId, setRunningId] = useState<string | null>(null);
  const [trackedTaskIds, setTrackedTaskIds] = useState<string[]>([]);
  const [error, setError] = useState('');
  const toast = useToast();

  const sortedJobs = useMemo(
    () => [...jobs].sort((a, b) => a.name.localeCompare(b.name, 'zh-CN')),
    [jobs]
  );

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      const data = await api<ScheduleJob[]>('/api/v2/schedules');
      setJobs(data);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`定时任务加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const trackTask = (task: TaskRun) => {
    setTrackedTaskIds((prev) => (prev.includes(task.id) ? prev : [task.id, ...prev].slice(0, 20)));
  };

  useTaskCompletion(trackedTaskIds, (task) => {
    void load();
    toast.push(
      task.status === 'done' ? `定时任务完成：${task.label || task.kind}` : `定时任务结束：${task.label || task.kind} · ${task.status}`,
      task.status === 'done' ? 'ok' : 'warn'
    );
  });

  const patchDraft = (patch: Partial<Draft>) => {
    setDraft((prev) => (prev ? { ...prev, ...patch } : prev));
  };

  const patchSchedule = (patch: Partial<ScheduleSpec>) => {
    setDraft((prev) => (prev ? { ...prev, schedule: { ...prev.schedule, ...patch } } : prev));
  };

  const saveDraft = async () => {
    if (!draft) return;
    let payload: ScheduleRequest;
    try {
      payload = validateDraft(draft);
    } catch (e) {
      toast.push(errorMessage(e), 'warn');
      return;
    }
    setSaving(true);
    try {
      const data = await api<ScheduleJob>(draft.id ? `/api/v2/schedules/${draft.id}` : '/api/v2/schedules', {
        method: draft.id ? 'PUT' : 'POST',
        body: JSON.stringify(payload)
      });
      setJobs((prev) => {
        if (!draft.id) return [data, ...prev];
        return prev.map((job) => (job.id === data.id ? data : job));
      });
      setDraft(null);
      toast.push(draft.id ? '定时任务已保存' : '定时任务已创建', 'ok');
    } catch (e) {
      toast.push(`保存定时任务失败：${errorMessage(e)}`, 'error');
    } finally {
      setSaving(false);
    }
  };

  const toggleEnabled = async (job: ScheduleJob) => {
    const payload = validateDraft({ ...draftFromJob(job), enabled: !job.enabled });
    try {
      const data = await api<ScheduleJob>(`/api/v2/schedules/${job.id}`, {
        method: 'PUT',
        body: JSON.stringify(payload)
      });
      setJobs((prev) => prev.map((item) => (item.id === data.id ? data : item)));
      toast.push(data.enabled ? '已启用' : '已停用', 'ok');
    } catch (e) {
      toast.push(`更新启停失败：${errorMessage(e)}`, 'error');
    }
  };

  const runNow = async (job: ScheduleJob) => {
    setRunningId(job.id);
    try {
      const data = await api<RunScheduleResponse>(`/api/v2/schedules/${job.id}/run`, { method: 'POST' });
      trackTask(data.task);
      toast.push(`已创建任务：${data.task.label}`, 'ok');
      await load();
    } catch (e) {
      toast.push(`立即运行失败：${errorMessage(e)}`, 'error');
    } finally {
      setRunningId(null);
    }
  };

  const deleteJob = async () => {
    if (!deleteTarget) return;
    try {
      await api(`/api/v2/schedules/${deleteTarget.id}`, { method: 'DELETE' });
      setJobs((prev) => prev.filter((job) => job.id !== deleteTarget.id));
      setDeleteTarget(null);
      toast.push('定时任务已删除', 'ok');
    } catch (e) {
      toast.push(`删除失败：${errorMessage(e)}`, 'error');
    }
  };

  return (
    <section className="schedulesPanel">
      <div className="schedulesToolbar">
        <button className="btn ghost" onClick={load} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
        <button className="btn" onClick={() => setDraft(draftFromJob())}>
          <Plus size={16} />
          新建定时
        </button>
      </div>

      {error && <div className="notice warn">{error}</div>}

      {draft && (
        <section className="scheduleEditor">
          <div className="scheduleEditorHead">
            <h2>{draft.id ? '编辑定时任务' : '新建定时任务'}</h2>
            <button className="btn ghost compact" onClick={() => setDraft(null)}>取消</button>
          </div>
          <div className="scheduleFormGrid">
            <label>
              <span>任务类型</span>
              <select className="input" aria-label="定时任务类型" value={draft.kind} onChange={(event) => patchDraft({ kind: event.target.value })}>
                {kindOptions.map((item) => <option key={item.kind} value={item.kind}>{item.label}</option>)}
              </select>
              <small>{kindDesc(draft.kind)}</small>
            </label>
            <label>
              <span>名称</span>
              <input className="input" aria-label="定时任务名称" value={draft.name} onChange={(event) => patchDraft({ name: event.target.value })} placeholder={kindLabel(draft.kind)} />
            </label>
            <label>
              <span>触发模式</span>
              <select className="input" aria-label="触发模式" value={draft.schedule.mode} onChange={(event) => patchSchedule({ mode: event.target.value as ScheduleMode })}>
                <option value="daily">每日</option>
                <option value="weekly">每周</option>
                <option value="monthly">每月</option>
              </select>
            </label>
            {draft.schedule.mode === 'weekly' && (
              <label>
                <span>星期</span>
                <select className="input" aria-label="星期" value={draft.schedule.weekday} onChange={(event) => patchSchedule({ weekday: Number(event.target.value) })}>
                  {weekdayLabels.map((label, index) => <option value={index} key={label}>{label}</option>)}
                </select>
              </label>
            )}
            {draft.schedule.mode === 'monthly' && (
              <label>
                <span>日期</span>
                <select className="input" aria-label="日期" value={draft.schedule.day} onChange={(event) => patchSchedule({ day: Number(event.target.value) })}>
                  {Array.from({ length: 31 }, (_, index) => index + 1).map((day) => <option key={day} value={day}>{day} 日</option>)}
                </select>
              </label>
            )}
            <label>
              <span>小时</span>
              <input className="input" aria-label="小时" inputMode="numeric" value={draft.schedule.hour} onChange={(event) => patchSchedule({ hour: Number(event.target.value) })} />
            </label>
            <label>
              <span>分钟</span>
              <input className="input" aria-label="分钟" inputMode="numeric" value={draft.schedule.minute} onChange={(event) => patchSchedule({ minute: Number(event.target.value) })} />
            </label>
            <label className="switchRow scheduleSwitch">
              <input type="checkbox" checked={draft.enabled} onChange={(event) => patchDraft({ enabled: event.target.checked })} />
              <span>启用</span>
            </label>
          </div>
          <label className="scheduleParams">
            <span>参数 JSON</span>
            <textarea className="input" aria-label="参数 JSON" value={draft.paramsJson} onChange={(event) => patchDraft({ paramsJson: event.target.value })} />
          </label>
          <button className="btn" onClick={saveDraft} disabled={saving}>
            <Save size={16} />
            {saving ? '保存中' : '保存定时'}
          </button>
        </section>
      )}

      <div className="scheduleList">
        {sortedJobs.map((job) => (
          <article className={`scheduleItem ${job.enabled ? '' : 'disabled'}`} key={job.id}>
            <div className="scheduleItemMain">
              <CalendarClock size={20} />
              <div>
                <strong>{job.name}</strong>
                <span>{kindLabel(job.kind)} · {scheduleText(job.schedule)}</span>
              </div>
            </div>
            <div className="scheduleMeta">
              <span className={`badge ${statusClass(job.last_status)}`}>{job.last_status || '未运行'}</span>
              <span>最近 {dateText(job.last_run_at)}</span>
              {job.last_error && <span className="scheduleError">{job.last_error}</span>}
            </div>
            <div className="scheduleActions">
              <button className="btn ghost compact" onClick={() => toggleEnabled(job)}>{job.enabled ? '停用' : '启用'}</button>
              <button className="btn ghost compact" onClick={() => runNow(job)} disabled={!job.enabled || runningId === job.id}>
                <Play size={14} />
                立即运行
              </button>
              <button className="btn ghost compact" onClick={() => setDraft(draftFromJob(job))}>
                <Pencil size={14} />
                编辑
              </button>
              <button className="btn ghost compact dangerText" onClick={() => setDeleteTarget(job)}>
                <Trash2 size={14} />
                删除
              </button>
            </div>
          </article>
        ))}
        {!loading && sortedJobs.length === 0 && (
          <div className="empty">还没有定时任务</div>
        )}
      </div>

      {deleteTarget && (
        <ConfirmDanger
          title="删除定时任务"
          body={<p>删除「{deleteTarget.name}」后不会再自动运行。</p>}
          confirmText="删除"
          onCancel={() => setDeleteTarget(null)}
          onConfirm={deleteJob}
        />
      )}
    </section>
  );
}
