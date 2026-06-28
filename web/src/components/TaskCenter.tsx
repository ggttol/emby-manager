import { Bell, ChevronDown, ChevronUp, Copy, RefreshCw, Search, XCircle } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { Drawer } from './Drawer';
import { useToast } from './Toast';

type TaskRun = components['schemas']['TaskRun'];
type TaskListResponse = components['schemas']['TaskListResponse'];

const active = new Set(['pending', 'running']);
const terminal = new Set(['done', 'error', 'cancelled', 'interrupted']);
type TaskFilter = 'all' | 'active' | 'done' | 'issue';

export const TASK_COMPLETED_EVENT = 'emby-manager:task-completed';

export type TaskCompleteDetail = {
  task: TaskRun;
  previousTask: TaskRun;
  previousStatus: string;
};

type TaskCenterProps = {
  onTaskComplete?: (detail: TaskCompleteDetail) => void;
};

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

function formatDuration(start?: string | null, end?: string | null) {
  if (!start || !end) return '未记录';
  const started = new Date(start).getTime();
  const ended = new Date(end).getTime();
  if (Number.isNaN(started) || Number.isNaN(ended) || ended < started) return '未记录';
  const seconds = Math.max(0, Math.round((ended - started) / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes < 60) return rest ? `${minutes} 分 ${rest} 秒` : `${minutes} 分`;
  const hours = Math.floor(minutes / 60);
  const minuteRest = minutes % 60;
  return minuteRest ? `${hours} 小时 ${minuteRest} 分` : `${hours} 小时`;
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

function prettyJson(value: unknown) {
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function hasJsonPayload(value: unknown) {
  if (value === null || value === undefined) return false;
  if (typeof value === 'object' && !Array.isArray(value)) return Object.keys(value).length > 0;
  if (Array.isArray(value)) return value.length > 0;
  return true;
}

function searchText(value: unknown) {
  return resultPreview(value).toLocaleLowerCase('zh-CN');
}

function taskMatchesQuery(task: TaskRun, tokens: string[]) {
  if (tokens.length === 0) return true;
  const haystack = [
    task.id,
    task.kind,
    task.label,
    task.status,
    task.status_text,
    task.source,
    task.error,
    searchText(task.params),
    searchText(task.result)
  ].filter(Boolean).join('\n').toLocaleLowerCase('zh-CN');
  return tokens.every((token) => haystack.includes(token));
}

function isCompletedTransition(previous?: TaskRun, next?: TaskRun) {
  if (!previous || !next) return false;
  return active.has(previous.status) && terminal.has(next.status);
}

export function TaskCenter({ onTaskComplete }: TaskCenterProps = {}) {
  const [open, setOpen] = useState(false);
  const [tasks, setTasks] = useState<TaskRun[]>([]);
  const [activeCount, setActiveCount] = useState(0);
  const [loadError, setLoadError] = useState('');
  const [cancellingId, setCancellingId] = useState<string | null>(null);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => new Set());
  const [filter, setFilter] = useState<TaskFilter>('all');
  const [query, setQuery] = useState('');
  const knownTasksRef = useRef<Map<string, TaskRun>>(new Map());
  const emittedIdsRef = useRef<Set<string>>(new Set());
  const toast = useToast();

  const emitCompletedTasks = useCallback((nextTasks: TaskRun[]) => {
    const knownTasks = knownTasksRef.current;
    const completed = nextTasks
      .map((task) => ({ task, previousTask: knownTasks.get(task.id) }))
      .filter(({ task, previousTask }) => (
        isCompletedTransition(previousTask, task) && !emittedIdsRef.current.has(task.id)
      ));

    knownTasksRef.current = new Map(nextTasks.map((task) => [task.id, task]));

    for (const { task, previousTask } of completed) {
      if (!previousTask) continue;
      emittedIdsRef.current.add(task.id);
      const detail: TaskCompleteDetail = {
        task,
        previousTask,
        previousStatus: previousTask.status
      };
      onTaskComplete?.(detail);
      window.dispatchEvent(new CustomEvent<TaskCompleteDetail>(TASK_COMPLETED_EVENT, { detail }));
    }
  }, [onTaskComplete]);

  const load = useCallback(async (options: { silent?: boolean } = {}) => {
    try {
      const data = await api<TaskListResponse>('/api/v2/tasks?limit=50');
      emitCompletedTasks(data.tasks);
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
  }, [emitCompletedTasks, toast]);

  useEffect(() => {
    load({ silent: true });
    const timer = window.setInterval(() => {
      load({ silent: true });
    }, activeCount > 0 ? 900 : 5000);
    return () => window.clearInterval(timer);
  }, [activeCount, load]);

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

  const visibleTasks = useMemo(() => {
    const tokens = query.trim().toLocaleLowerCase('zh-CN').split(/\s+/).filter(Boolean);
    return tasks.filter((task) => {
      if (filter === 'active' && !active.has(task.status)) return false;
      if (filter === 'done' && task.status !== 'done') return false;
      if (filter === 'issue' && !['error', 'cancelled', 'interrupted'].includes(task.status)) return false;
      return taskMatchesQuery(task, tokens);
    });
  }, [filter, query, tasks]);

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

  const toggleExpanded = (taskId: string) => {
    setExpandedIds((current) => {
      const next = new Set(current);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return next;
    });
  };

  const copyTaskId = async (task: TaskRun) => {
    try {
      await navigator.clipboard.writeText(task.id);
      toast.push('任务 ID 已复制', 'ok');
    } catch (e) {
      toast.push(`复制失败：${errorMessage(e)}`, 'error');
    }
  };

  const expandVisible = () => {
    setExpandedIds((current) => {
      const next = new Set(current);
      visibleTasks.forEach((task) => next.add(task.id));
      return next;
    });
  };

  const collapseVisible = () => {
    setExpandedIds((current) => {
      const next = new Set(current);
      visibleTasks.forEach((task) => next.delete(task.id));
      return next;
    });
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
          <div className="taskSearchBar">
            <Search size={15} />
            <input
              className="input"
              aria-label="任务搜索"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="搜索名称、类型、ID、参数或错误"
            />
            {query && (
              <button className="iconBtn" onClick={() => setQuery('')} aria-label="清空任务搜索">
                <XCircle size={15} />
              </button>
            )}
          </div>
          <div className="taskBulkActions">
            <span>显示 {visibleTasks.length} / {tasks.length}</span>
            <div>
              <button className="btn ghost compact" onClick={expandVisible} disabled={visibleTasks.length === 0}>
                <ChevronDown size={14} />
                展开可见
              </button>
              <button className="btn ghost compact" onClick={collapseVisible} disabled={visibleTasks.length === 0}>
                <ChevronUp size={14} />
                收起可见
              </button>
            </div>
          </div>
          {loadError && <div className="taskError">加载失败：{loadError}</div>}
          <div className="taskList">
            {visibleTasks.length === 0 && <p className="empty">{tasks.length === 0 ? '没有任务' : '当前过滤下没有任务'}</p>}
            {visibleTasks.map((task) => {
              const taskPct = task.total ? Math.min(100, Math.round((task.progress / task.total) * 100)) : 0;
              const canCancel = active.has(task.status) && !task.cancel_requested;
              const preview = resultPreview(task.result);
              const expanded = expandedIds.has(task.id);
              const taskName = task.label || task.kind;
              const duration = formatDuration(task.started_at || task.queued_at, task.ended_at || task.updated_at);
              const showParams = hasJsonPayload(task.params);
              const showResult = hasJsonPayload(task.result);
              return (
                <article className="taskCard" key={task.id}>
                  <div>
                    <strong>{taskName}</strong>
                    <span className={`badge ${task.status}`}>{statusLabel(task.status)}</span>
                    <button
                      className="iconBtn taskDetailToggle"
                      onClick={() => toggleExpanded(task.id)}
                      title={expanded ? '收起详情' : '展开详情'}
                      aria-label={`${expanded ? '收起' : '展开'}任务详情：${taskName}`}
                    >
                      {expanded ? <ChevronUp size={15} /> : <ChevronDown size={15} />}
                    </button>
                  </div>
                  <dl className="taskMeta">
                    <div><dt>类型</dt><dd>{task.kind}</dd></div>
                    <div><dt>来源</dt><dd>{task.source || 'manual'}</dd></div>
                    <div><dt>排队</dt><dd>{formatTime(task.queued_at)}</dd></div>
                    <div><dt>耗时</dt><dd>{duration}</dd></div>
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
                  {expanded && (
                    <div className="taskDetails">
                      <div className="taskIdLine">
                        <span>{task.id}</span>
                        <button
                          className="iconBtn"
                          onClick={() => copyTaskId(task)}
                          title="复制任务 ID"
                          aria-label={`复制任务 ID：${taskName}`}
                        >
                          <Copy size={14} />
                        </button>
                      </div>
                      {showParams && (
                        <section>
                          <h4>参数</h4>
                          <pre>{prettyJson(task.params)}</pre>
                        </section>
                      )}
                      {showResult && (
                        <section>
                          <h4>结果</h4>
                          <pre>{prettyJson(task.result)}</pre>
                        </section>
                      )}
                      {task.error && (
                        <section>
                          <h4>错误</h4>
                          <pre>{task.error}</pre>
                        </section>
                      )}
                    </div>
                  )}
                </article>
              );
            })}
          </div>
        </Drawer>
      )}
    </>
  );
}
