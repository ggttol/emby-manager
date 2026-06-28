import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  Database,
  FileText,
  Gauge,
  HardDrive,
  ListChecks,
  RefreshCw,
  Server,
  Subtitles,
  Webhook
} from 'lucide-react';
import { FormEvent, ReactNode, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useToast } from './Toast';

type AutostrmStatusResponse = components['schemas']['AutostrmStatusResponse'];
type CleanupSummaryResponse = components['schemas']['CleanupSummaryResponse'];
type GapsSummaryResponse = components['schemas']['GapsSummaryResponse'];
type InsightMeta = components['schemas']['InsightMeta'];
type InsightTodo = components['schemas']['InsightTodo'];
type PathStatus = components['schemas']['PathStatus'];
type StrmListResponse = components['schemas']['StrmListResponse'];
type StrmOverview = components['schemas']['StrmOverview'];
type SystemSummary = components['schemas']['SystemSummary'];
type TaskHistorySummary = components['schemas']['TaskHistorySummary'];

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function count(value: number | null | undefined) {
  return Number(value || 0).toLocaleString('zh-CN');
}

function percent(value: number | null | undefined) {
  if (value == null || !Number.isFinite(value)) return '未知';
  return `${value.toFixed(1)}%`;
}

function bytes(value: number | null | undefined) {
  const size = Number(value || 0);
  if (!Number.isFinite(size) || size <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let next = size;
  let index = 0;
  while (next >= 1024 && index < units.length - 1) {
    next /= 1024;
    index += 1;
  }
  return `${next >= 10 || index === 0 ? next.toFixed(0) : next.toFixed(1)} ${units[index]}`;
}

function dateText(value?: string | null) {
  if (!value) return '无记录';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  });
}

function todoTone(severity: string) {
  if (severity === 'high') return 'error';
  if (severity === 'medium') return 'warn';
  return 'info';
}

function taskProblemCount(task?: TaskHistorySummary) {
  if (!task) return 0;
  return task.error + task.cancelled + task.interrupted + task.stale_running;
}

function StatCard({
  icon,
  label,
  value,
  hint,
  tone = 'neutral'
}: {
  icon: ReactNode;
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  tone?: 'neutral' | 'ok' | 'warn' | 'error';
}) {
  return (
    <article className={`statCard ${tone}`}>
      <div>{icon}</div>
      <span>{label}</span>
      <strong>{value}</strong>
      {hint && <small>{hint}</small>}
    </article>
  );
}

function TodoList({ items, empty }: { items: InsightTodo[]; empty: string }) {
  if (!items.length) return <div className="empty inlineEmpty">{empty}</div>;
  return (
    <div className="todoList">
      {items.map((todo, index) => (
        <article className={`todoItem ${todoTone(todo.severity)}`} key={`${todo.area}-${todo.source}-${index}`}>
          <span className="badge">{todo.severity}</span>
          <strong>{todo.message}</strong>
          <small>{todo.area} · {todo.source} · {count(todo.count)}</small>
        </article>
      ))}
    </div>
  );
}

function WarningList({ warnings }: { warnings: string[] }) {
  if (!warnings.length) return null;
  return (
    <div className="notice warn whitespaceNotice">
      {warnings.map((warning) => <div key={warning}>{warning}</div>)}
    </div>
  );
}

function MetaBlock({ meta }: { meta?: InsightMeta }) {
  if (!meta) return null;
  return (
    <section className="readonlyBlock">
      <h2>只读覆盖范围</h2>
      <div className="metaColumns">
        <div>
          <strong>数据源</strong>
          <ul>{meta.source.map((item) => <li key={item}>{item}</li>)}</ul>
        </div>
        <div>
          <strong>已覆盖</strong>
          <ul>{meta.coverage.map((item) => <li key={item}>{item}</li>)}</ul>
        </div>
        <div>
          <strong>限制</strong>
          <ul>{meta.limitations.map((item) => <li key={item}>{item}</li>)}</ul>
        </div>
      </div>
    </section>
  );
}

function TaskHistory({ task }: { task?: TaskHistorySummary }) {
  if (!task) return null;
  return (
    <section className="readonlyBlock">
      <h2>任务历史</h2>
      <div className="miniStats">
        <span>总数 <strong>{count(task.total)}</strong></span>
        <span>运行中 <strong>{count(task.running)}</strong></span>
        <span>失败 <strong>{count(task.error)}</strong></span>
        <span>中断 <strong>{count(task.interrupted)}</strong></span>
      </div>
      {task.recent_issues.length > 0 && (
        <div className="issueList">
          {task.recent_issues.map((issue) => (
            <article key={issue.id}>
              <strong>{issue.label || issue.kind}</strong>
              <span className={`badge ${issue.status}`}>{issue.status}</span>
              <p>{issue.message}</p>
              <small>{dateText(issue.updated_at)}</small>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

export function DashboardPanel() {
  const [system, setSystem] = useState<SystemSummary | null>(null);
  const [cleanup, setCleanup] = useState<CleanupSummaryResponse | null>(null);
  const [gaps, setGaps] = useState<GapsSummaryResponse | null>(null);
  const [autostrm, setAutostrm] = useState<AutostrmStatusResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      const [systemData, cleanupData, gapsData, autostrmData] = await Promise.all([
        api<SystemSummary>('/api/v2/system/summary'),
        api<CleanupSummaryResponse>('/api/v2/cleanup/suggest', { method: 'POST', body: JSON.stringify({}) }),
        api<GapsSummaryResponse>('/api/v2/gaps/scan', { method: 'POST', body: JSON.stringify({}) }),
        api<AutostrmStatusResponse>('/api/v2/autostrm/status')
      ]);
      setSystem(systemData);
      setCleanup(cleanupData);
      setGaps(gapsData);
      setAutostrm(autostrmData);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`仪表盘加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const todos = useMemo(() => {
    const merged = [...(cleanup?.todos || []), ...(gaps?.todos || []), ...(autostrm?.todos || [])];
    return merged.slice(0, 10);
  }, [cleanup, gaps, autostrm]);
  const warnings = [
    ...(system?.warnings || []),
    ...(cleanup?.warnings || []),
    ...(gaps?.warnings || []),
    ...(autostrm?.warnings || [])
  ];

  return (
    <section className="readonlyPanel">
      <div className="readonlyToolbar">
        <div>
          <strong>Rust Preview 总览</strong>
          <span>这里聚合当前 v2 只读预检数据，尚未替代未 port 的危险写操作。</span>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>
      {error && <div className="notice warn">{error}</div>}
      <div className="statGrid">
        <StatCard icon={<Server />} label="服务状态" value={system?.ok ? '正常' : '需检查'} tone={system?.ok ? 'ok' : 'warn'} hint={system?.version || '等待数据'} />
        <StatCard icon={<Database />} label="数据库" value={system?.database?.status || '未知'} tone={system?.database?.status === 'ok' ? 'ok' : 'warn'} hint={system?.database?.current_database || system?.database?.url} />
        <StatCard icon={<ListChecks />} label="待办" value={count(todos.length)} tone={todos.length ? 'warn' : 'ok'} hint="只读预检聚合" />
        <StatCard icon={<Activity />} label="异常任务" value={count(taskProblemCount(cleanup?.task_history))} tone={taskProblemCount(cleanup?.task_history) ? 'warn' : 'ok'} hint={`运行中 ${count(cleanup?.task_history?.running)}`} />
        <StatCard icon={<FileText />} label="strm / 字幕" value={`${count(cleanup?.strm?.strm_files)} / ${count(cleanup?.strm?.subtitle_files)}`} hint={cleanup?.strm?.root || system?.strm_root} />
        <StatCard icon={<Webhook />} label="Autostrm unmatched" value={count(autostrm?.unmatched?.total)} tone={autostrm?.unmatched?.total ? 'warn' : 'ok'} hint={`${count(autostrm?.seen?.total)} seen`} />
      </div>
      <WarningList warnings={warnings} />
      <section className="readonlyBlock">
        <h2>待处理信号</h2>
        <TodoList items={todos} empty="当前只读预检没有发现待处理信号" />
      </section>
      <TaskHistory task={cleanup?.task_history} />
    </section>
  );
}

export function SystemPanel() {
  const [system, setSystem] = useState<SystemSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      setSystem(await api<SystemSummary>('/api/v2/system/summary'));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`系统状态加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  return (
    <section className="readonlyPanel">
      <div className="readonlyToolbar">
        <div>
          <strong>系统健康</strong>
          <span>数据库、路径、磁盘和主机负载的实时只读状态。</span>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>
      {error && <div className="notice warn">{error}</div>}
      <div className="statGrid">
        <StatCard icon={<CheckCircle2 />} label="整体" value={system?.ok ? '正常' : '需检查'} tone={system?.ok ? 'ok' : 'warn'} hint={system?.version || '等待数据'} />
        <StatCard icon={<Database />} label="Postgres" value={system?.database?.status || '未知'} tone={system?.database?.status === 'ok' ? 'ok' : 'warn'} hint={`${system?.database?.pool_size || 0} pool / ${system?.database?.idle_connections || 0} idle`} />
        <StatCard icon={<Gauge />} label="内存" value={percent(system?.host?.memory?.used_percent)} hint={system?.host?.memory ? `${bytes(system.host.memory.available_bytes)} 可用` : '无主机数据'} />
        <StatCard icon={<Activity />} label="负载" value={system?.host?.load_average ? system.host.load_average.one.toFixed(2) : '未知'} hint={system?.host?.load_average ? `${system.host.load_average.five.toFixed(2)} / ${system.host.load_average.fifteen.toFixed(2)}` : `${system?.host?.os || ''} ${system?.host?.arch || ''}`} />
      </div>
      <WarningList warnings={system?.warnings || []} />
      <section className="readonlyBlock">
        <h2>路径与磁盘</h2>
        <div className="pathGrid">
          {(system?.configured_roots || []).map((path) => <PathCard key={path.key} path={path} />)}
        </div>
      </section>
      {system?.database?.warning && <div className="notice warn">{system.database.warning}</div>}
    </section>
  );
}

function PathCard({ path }: { path: PathStatus }) {
  const tone = path.exists && path.warnings.length === 0 ? 'ok' : path.exists ? 'warn' : 'error';
  return (
    <article className={`pathCard ${tone}`}>
      <div>
        <HardDrive size={17} />
        <strong>{path.label}</strong>
        <span className={`badge ${tone}`}>{path.exists ? '存在' : '缺失'}</span>
      </div>
      <code>{path.path}</code>
      <small>{path.expected_kind} · readable {String(path.readable ?? 'unknown')} · writable {String(path.writable_hint ?? 'unknown')}</small>
      {path.disk && (
        <small>{path.disk.mount_point} · {bytes(path.disk.available_bytes)} 可用 · {percent(path.disk.used_percent)} 已用</small>
      )}
      {path.warnings.map((warning) => <p key={warning}>{warning}</p>)}
    </article>
  );
}

export function SubtitlesPanel() {
  const [lib, setLib] = useState('');
  const [response, setResponse] = useState<StrmListResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const overview = response?.overview || null;

  const load = async (nextLib = lib) => {
    setLoading(true);
    setError('');
    try {
      const params = new URLSearchParams({ overview: 'true', overview_depth: '8', sample_limit: '40', limit: '1' });
      if (nextLib.trim()) params.set('lib', nextLib.trim());
      setResponse(await api<StrmListResponse>(`/api/v2/libraries/strm?${params.toString()}`));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`字幕概览加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load('');
  }, []);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    load(lib);
  };

  return (
    <section className="readonlyPanel">
      <div className="readonlyToolbar">
        <div>
          <strong>外挂字幕概览</strong>
          <span>只统计文件名、扩展名和大小，不读取 .strm 内容。</span>
        </div>
        <button className="btn ghost" onClick={() => load(lib)} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>
      <form className="readonlyFilter" onSubmit={submit}>
        <label>
          <span>库名</span>
          <input className="input" aria-label="字幕库名" value={lib} onChange={(event) => setLib(event.target.value)} placeholder="留空统计全部 strm 根" />
        </label>
        <button className="btn" disabled={loading}>查看概览</button>
      </form>
      {error && <div className="notice warn">{error}</div>}
      <div className="statGrid">
        <StatCard icon={<FileText />} label=".strm" value={count(overview?.strm_files)} hint={bytes(overview?.strm_bytes)} />
        <StatCard icon={<Subtitles />} label="字幕文件" value={count(overview?.subtitle_files)} tone={overview?.subtitle_files ? 'ok' : 'warn'} hint={bytes(overview?.subtitle_bytes)} />
        <StatCard icon={<HardDrive />} label="其他文件" value={count(overview?.other_files)} hint={`${count(overview?.directories)} 目录`} />
        <StatCard icon={<AlertTriangle />} label="截断" value={overview?.truncated ? '是' : '否'} tone={overview?.truncated ? 'warn' : 'ok'} hint={`上限 ${count(overview?.entry_limit)}`} />
      </div>
      <WarningList warnings={overview?.warnings || []} />
      <SubtitleDetails overview={overview} />
    </section>
  );
}

function SubtitleDetails({ overview }: { overview: StrmOverview | null }) {
  if (!overview) return <div className="empty inlineEmpty">等待字幕统计数据</div>;
  return (
    <div className="readonlySplit">
      <section className="readonlyBlock">
        <h2>字幕扩展</h2>
        <div className="extensionList">
          {overview.subtitle_extensions.map((item) => (
            <span key={item.extension}><strong>.{item.extension}</strong>{count(item.count)}</span>
          ))}
          {overview.subtitle_extensions.length === 0 && <div className="empty inlineEmpty">没有发现外挂字幕扩展</div>}
        </div>
      </section>
      <section className="readonlyBlock">
        <h2>样例</h2>
        <div className="sampleList">
          {overview.samples.map((sample) => (
            <article key={`${sample.kind}-${sample.rel_path}`}>
              <span className="badge">{sample.kind}</span>
              <strong>{sample.rel_path}</strong>
              <small>.{sample.extension || 'unknown'} · {bytes(sample.size)}</small>
            </article>
          ))}
          {overview.samples.length === 0 && <div className="empty inlineEmpty">没有样例</div>}
        </div>
      </section>
    </div>
  );
}

export function AutostrmPanel() {
  const [status, setStatus] = useState<AutostrmStatusResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      setStatus(await api<AutostrmStatusResponse>('/api/v2/autostrm/status'));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`Autostrm 状态加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  return (
    <section className="readonlyPanel">
      <div className="readonlyToolbar">
        <div>
          <strong>Autostrm 状态</strong>
          <span>只读展示 seen/unmatched 表，不接收 webhook，不触发自动匹配。</span>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>
      {error && <div className="notice warn">{error}</div>}
      <div className="statGrid">
        <StatCard icon={<Webhook />} label="seen" value={count(status?.seen?.total)} hint={`${count(status?.seen?.libraries)} 个库 · ${dateText(status?.seen?.last_seen_at)}`} />
        <StatCard icon={<AlertTriangle />} label="unmatched" value={count(status?.unmatched?.total)} tone={status?.unmatched?.total ? 'warn' : 'ok'} hint={`${count(status?.unmatched?.without_emby_id)} 缺 Emby ID`} />
        <StatCard icon={<ListChecks />} label="库分布" value={count(status?.libraries?.length)} hint="最多 20 个库" />
        <StatCard icon={<CheckCircle2 />} label="业务 port" value={status?.complete_business_port ? '完整' : '只读'} tone={status?.complete_business_port ? 'ok' : 'warn'} hint="webhook worker 尚未接入" />
      </div>
      <WarningList warnings={status?.warnings || []} />
      <section className="readonlyBlock">
        <h2>库分布</h2>
        <div className="libraryBars">
          {(status?.libraries || []).map((item) => (
            <article key={item.lib}>
              <strong>{item.lib}</strong>
              <span>seen {count(item.seen)}</span>
              <span>unmatched {count(item.unmatched)}</span>
            </article>
          ))}
          {status && (status.libraries || []).length === 0 && <div className="empty inlineEmpty">暂无库级 autostrm 数据</div>}
        </div>
      </section>
      <section className="readonlyBlock">
        <h2>待处理信号</h2>
        <TodoList items={status?.todos || []} empty="当前没有 autostrm 待处理信号" />
      </section>
      <MetaBlock meta={status?.meta} />
    </section>
  );
}
