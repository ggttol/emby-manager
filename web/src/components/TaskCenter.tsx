import { Bell, RefreshCw, XCircle } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { Drawer } from './Drawer';
import { useToast } from './Toast';

type TaskRun = components['schemas']['TaskRun'];
type TaskListResponse = components['schemas']['TaskListResponse'];

const active = new Set(['pending', 'running']);
type TaskFilter = 'all' | 'active' | 'done' | 'issue';

const statusLabels: Record<string, string> = {
  pending: '排队',
  running: '运行中',
  done: '完成',
  error: '失败',
  cancelled: '已取消',
  interrupted: '已中断'
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function statusLabel(status: string) {
  return statusLabels[status] || status;
}

function formatTime(value?: string | null) {
  if (!value) return '未记录';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  });
}

function resultPreview(result: unknown) {
  if (result === null || result === undefined) return '';
  if (typeof result === 'string') return result;
  try {
    return JSON.stringify(result);
  } catch {
    return String(result);
  }
}

export function TaskCenter() {
  const [open, setOpen] = useState(false);
  const [tasks, setTasks] = useState<TaskRun[]>([]);
  const [activeCount, setActiveCount] = useState(0);
  const [loadError, setLoadError] = useState('');
  const [cancellingId, setCancellingId] = useState<string | null>(null);
  const [filter, setFilter] = useState<TaskFilter>('all');
  const toast = useToast();

  const load = async (options: { silent?: boolean } = {}) => {
    try {
      const data = await api<TaskListResponse>('/api/v2/tasks?limit=50');
      setTasks(data.tasks);
      setActiveCount(data.active_count);
      setLoadError('');
    } catch (e) {
      const message = errorMessage(e);
      setLoadError(message);
      if (!options.silent) {
        toast.push(`任务中心加载失败：${message}`, 'error');
      }
    }
  };

  useEffect(() => {
    load({ silent: true });
    const timer = window.setInterval(() => {
      load({ silent: true });
    }, activeCount > 0 ? 900 : 5000);
    return () => window.clearInterval(timer);
  }, [activeCount]);

  const pct = useMemo(() => {
    const running = tasks.filter((task) => active.has(task.status));
    const total = running.reduce((sum, task) => sum + (task.total || 0), 0);
    const progress = running.reduce((sum, task) => sum + (task.progress || 0), 0);
    return total > 0 ? Math.min(100, Math.round((progress / total) * 100)) : activeCount ? 5 : 0;
  }, [activeCount, tasks]);

  const counts = useMemo(() => ({
    all: tasks.length,
    active: tasks.filter((task) => active.has(task.status)).length,
    done: tasks.filter((task) => task.status === 'done').length,
    issue: tasks.filter((task) => ['error', 'cancelled', 'interrupted'].includes(task.status)).length
  }), [tasks]);

  const visibleTasks = useMemo(() => tasks.filter((task) => {
    if (filter === 'active') return active.has(task.status);
    if (filter === 'done') return task.status === 'done';
    if (filter === 'issue') return ['error', 'cancelled', 'interrupted'].includes(task.status);
    return true;
  }), [filter, tasks]);

  const cancel = async (task: TaskRun) => {
    setCancellingId(task.id);
    try {
      const res = await api<components['schemas']['TaskCancelResponse']>(`/api/v2/tasks/${task.id}/cancel`, { method: 'POST' });
      toast.push(res.ok ? `已请求取消：${task.label || task.kind}` : '任务已结束或不存在', res.ok ? 'warn' : 'info');
      await load({ silent: true });
    } catch (e) {
      toast.push(`取消任务失败：${errorMessage(e)}`, 'error');
    } finally {
      setCancellingId(null);
    }
  };

  return (
    <>
      <button
        className="bell"
        onClick={() => {
          setOpen(true);
          load();
        }}
        aria-label="任务中心"
      >
        <Bell size={18} />
        {activeCount > 0 && <span>{activeCount}</span>}
      </button>
      {activeCount > 0 && <div className="globalProgress"><i style={{ width: `${pct}%` }} /></div>}
      {open && (
        <Drawer title="任务中心" onClose={() => setOpen(false)}>
          <div className="drawerToolbar">
            <span>{activeCount ? `${activeCount} 个进行中` : '无进行中任务'} · 共 {tasks.length} 条</span>
            <button className="iconBtn" onClick={() => load()} aria-label="刷新"><RefreshCw size={16} /></button>
          </div>
          <div className="taskFilters" role="group" aria-label="任务过滤">
            {([
              ['all', '全部', counts.all],
              ['active', '进行中', counts.active],
              ['done', '完成', counts.done],
              ['issue', '异常', counts.issue]
            ] as const).map(([key, label, count]) => (
              <button key={key} className={filter === key ? 'active' : ''} onClick={() => setFilter(key)}>
                {label}<span>{count}</span>
              </button>
            ))}
          </div>
          {loadError && <div className="taskError">加载失败：{loadError}</div>}
          <div className="taskList">
            {visibleTasks.length === 0 && <p className="empty">{tasks.length === 0 ? '没有任务' : '当前过滤下没有任务'}</p>}
            {visibleTasks.map((task) => {
              const taskPct = task.total ? Math.min(100, Math.round((task.progress / task.total) * 100)) : 0;
              const canCancel = active.has(task.status) && !task.cancel_requested;
              const preview = resultPreview(task.result);
              return (
                <article className="taskCard" key={task.id}>
                  <div>
                    <strong>{task.label || task.kind}</strong>
                    <span className={`badge ${task.status}`}>{statusLabel(task.status)}</span>
                  </div>
                  <dl className="taskMeta">
                    <div><dt>类型</dt><dd>{task.kind}</dd></div>
                    <div><dt>更新</dt><dd>{formatTime(task.updated_at)}</dd></div>
                    {task.started_at && <div><dt>开始</dt><dd>{formatTime(task.started_at)}</dd></div>}
                    {task.ended_at && <div><dt>结束</dt><dd>{formatTime(task.ended_at)}</dd></div>}
                  </dl>
                  <p>{task.status_text || task.kind}</p>
                  {active.has(task.status) && (
                    <>
                      <div className="miniProgress"><i style={{ width: `${task.total ? taskPct : 5}%` }} /></div>
                      <small>{task.progress}/{task.total || '?'} · {taskPct}%</small>
                      <button className="btn ghost compact" onClick={() => cancel(task)} disabled={!canCancel || cancellingId === task.id}>
                        <XCircle size={14} /> {task.cancel_requested || cancellingId === task.id ? '取消中' : '取消'}
                      </button>
                    </>
                  )}
                  {preview && <p className="resultText">{preview}</p>}
                  {task.error && <p className="errorText">{task.error}</p>}
                </article>
              );
            })}
          </div>
        </Drawer>
      )}
    </>
  );
}
