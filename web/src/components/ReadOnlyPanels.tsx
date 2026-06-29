import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  Copy,
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
type DashboardTodoResponse = components['schemas']['DashboardTodoResponse'];
type GapsSummaryResponse = components['schemas']['GapsSummaryResponse'];
type InsightMeta = components['schemas']['InsightMeta'];
type InsightTodo = components['schemas']['InsightTodo'];
type PathStatus = components['schemas']['PathStatus'];
type StrmListResponse = components['schemas']['StrmListResponse'];
type StrmOverview = components['schemas']['StrmOverview'];
type SystemSummary = components['schemas']['SystemSummary'];
type TaskHistorySummary = components['schemas']['TaskHistorySummary'];
type DockerContainerSummary = components['schemas']['DockerContainerSummary'];
type EmbyHealthSummary = components['schemas']['EmbyHealthSummary'];
type HostMetrics = components['schemas']['HostMetrics'];
type LoadAverage = components['schemas']['LoadAverage'];
type MemorySummary = components['schemas']['MemorySummary'];

type DashboardTodoParity = {
  noposter: number;
  dups_auto: number;
  dups_review: number;
  airing_count: number;
  noposter_by_lib: Record<string, number>;
  errors: string[];
};

const EMPTY_DASHBOARD_TODO: DashboardTodoParity = {
  noposter: 0,
  dups_auto: 0,
  dups_review: 0,
  airing_count: 0,
  noposter_by_lib: {},
  errors: []
};

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

function boolText(value: boolean | null | undefined) {
  if (value == null) return '未知';
  return value ? '是' : '否';
}

function statusTone(
  status: string | null | undefined,
  ok = false
): 'neutral' | 'ok' | 'warn' | 'error' {
  if (ok || status === 'ok' || status === 'online') return 'ok';
  if (!status || status === 'checking') return 'neutral';
  if (status === 'offline' || status === 'unavailable' || status === 'parse_error') return 'error';
  return 'warn';
}

function badgeClass(tone: 'neutral' | 'ok' | 'warn' | 'error') {
  if (tone === 'ok') return 'badge done';
  if (tone === 'warn') return 'badge warn';
  if (tone === 'error') return 'badge error';
  return 'badge';
}

function memoryUsedBytes(memory?: MemorySummary | null) {
  if (!memory) return null;
  return Math.max(0, memory.total_bytes - memory.available_bytes);
}

function loadAverageText(load?: LoadAverage | null) {
  if (!load) return '无 load average';
  return `${load.one.toFixed(2)} / ${load.five.toFixed(2)} / ${load.fifteen.toFixed(2)}`;
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

function buildSystemReport(system: SystemSummary | null) {
  if (!system) return '';
  const memory = system.host.memory;
  const load = system.host.load_average;
  const lines = [
    'Emby Manager System Report',
    `generated_at: ${new Date().toISOString()}`,
    `ok: ${system.ok}`,
    `version: ${system.version}`,
    `rust_version: ${system.rust_version}`,
    '',
    '[Emby]',
    `configured: ${system.emby.configured}`,
    `status: ${system.emby.status}`,
    `online: ${system.emby.online}`,
    `http_status: ${system.emby.http_status ?? 'unknown'}`,
    `server_name: ${system.emby.server_name || 'unknown'}`,
    `version: ${system.emby.version || 'unknown'}`,
    `base_url: ${system.emby.base_url || 'unknown'}`,
    '',
    '[Database]',
    `status: ${system.database.status}`,
    `current_database: ${system.database.current_database || 'unknown'}`,
    `pool_size: ${system.database.pool_size}`,
    `idle_connections: ${system.database.idle_connections}`,
    `url: ${system.database.url || 'unknown'}`,
    '',
    '[Docker]',
    `configured: ${system.docker.configured}`,
    `available: ${system.docker.available}`,
    `status: ${system.docker.status}`,
    `docker_bin: ${system.docker.docker_bin}`,
    `containers: ${system.docker.running}/${system.docker.total} running`,
    ...system.docker.containers.map(
      (container) =>
        `- ${container.name} | ${container.image} | ${container.state} | ${container.status} | ${container.ports || 'no ports'}`
    ),
    '',
    '[Host]',
    `os: ${system.host.os}`,
    `arch: ${system.host.arch}`,
    `process_id: ${system.host.process_id}`,
    `memory: ${memory ? `${bytes(memoryUsedBytes(memory))} used / ${bytes(memory.total_bytes)} total (${percent(memory.used_percent)})` : 'unknown'}`,
    `load_average: ${load ? loadAverageText(load) : 'unknown'}`,
    '',
    '[Paths]',
    ...system.configured_roots.map((path) => {
      const disk = path.disk
        ? ` | disk=${path.disk.mount_point} ${bytes(path.disk.available_bytes)} available ${percent(path.disk.used_percent)} used`
        : '';
      return `- ${path.key}: ${path.path} | exists=${path.exists} | readable=${path.readable ?? 'unknown'} | writable=${path.writable_hint ?? 'unknown'}${disk}`;
    }),
    '',
    '[Warnings]',
    ...(system.warnings.length ? system.warnings.map((warning) => `- ${warning}`) : ['none'])
  ];
  return lines.join('\n');
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
  const [dashboardTodo, setDashboardTodo] = useState<DashboardTodoParity>(EMPTY_DASHBOARD_TODO);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = async () => {
    setLoading(true);
    setError('');
    setDashboardTodo(EMPTY_DASHBOARD_TODO);
    try {
      const [systemData, cleanupData, gapsData, autostrmData, todoData] = await Promise.all([
        api<SystemSummary>('/api/v2/system/summary'),
        api<CleanupSummaryResponse>('/api/v2/cleanup/suggest', { method: 'POST', body: JSON.stringify({}) }),
        api<GapsSummaryResponse>('/api/v2/gaps/scan', { method: 'POST', body: JSON.stringify({}) }),
        api<AutostrmStatusResponse>('/api/v2/autostrm/status'),
        api<DashboardTodoResponse>('/api/v2/dashboard/todo')
      ]);
      setSystem(systemData);
      setCleanup(cleanupData);
      setGaps(gapsData);
      setAutostrm(autostrmData);
      setDashboardTodo({
        noposter: todoData.noposter || 0,
        dups_auto: todoData.dups_auto || 0,
        dups_review: todoData.dups_review || 0,
        airing_count: todoData.airing_count || 0,
        noposter_by_lib: todoData.noposter_by_lib || {},
        errors: [todoData.noposter_err, todoData.dups_err, todoData.airing_err].filter(Boolean) as string[]
      });
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
    ...(autostrm?.warnings || []),
    ...dashboardTodo.errors
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
        <StatCard icon={<AlertTriangle />} label="无海报" value={count(dashboardTodo.noposter)} tone={dashboardTodo.noposter ? 'warn' : 'ok'} hint="旧版 dash/todo" />
        <StatCard icon={<CheckCircle2 />} label="自动去重" value={count(dashboardTodo.dups_auto)} tone={dashboardTodo.dups_auto ? 'warn' : 'ok'} hint="可自动处理组" />
        <StatCard icon={<AlertTriangle />} label="人工重复" value={count(dashboardTodo.dups_review)} tone={dashboardTodo.dups_review ? 'warn' : 'ok'} hint="需人工 review" />
        <StatCard icon={<Webhook />} label="在更剧" value={count(dashboardTodo.airing_count)} tone={dashboardTodo.airing_count ? 'warn' : 'ok'} hint="追更 continuing" />
        <StatCard icon={<FileText />} label="strm / 字幕" value={`${count(cleanup?.strm?.strm_files)} / ${count(cleanup?.strm?.subtitle_files)}`} hint={cleanup?.strm?.root || system?.strm_root} />
        <StatCard icon={<Webhook />} label="Autostrm unmatched" value={count(autostrm?.unmatched?.total)} tone={autostrm?.unmatched?.total ? 'warn' : 'ok'} hint={`${count(autostrm?.seen?.total)} seen`} />
      </div>
      <WarningList warnings={warnings} />
      <DashboardTodoParityBlock todo={dashboardTodo} />
      <section className="readonlyBlock">
        <h2>待处理信号</h2>
        <TodoList items={todos} empty="当前只读预检没有发现待处理信号" />
      </section>
      <TaskHistory task={cleanup?.task_history} />
    </section>
  );
}

function DashboardTodoParityBlock({ todo }: { todo: DashboardTodoParity }) {
  const libs = Object.entries(todo.noposter_by_lib).sort((left, right) => right[1] - left[1]);
  if (!todo.noposter && !todo.dups_auto && !todo.dups_review && !todo.airing_count && libs.length === 0) {
    return null;
  }
  return (
    <section className="readonlyBlock">
      <h2>旧版待办计数</h2>
      <div className="miniStats">
        <span><strong>{count(todo.noposter)}</strong>无海报</span>
        <span><strong>{count(todo.dups_auto)}</strong>自动去重</span>
        <span><strong>{count(todo.dups_review)}</strong>人工重复</span>
        <span><strong>{count(todo.airing_count)}</strong>在更剧</span>
      </div>
      {libs.length > 0 && (
        <div className="libraryBars">
          {libs.slice(0, 6).map(([lib, total]) => (
            <article key={lib}>
              <strong>{lib}</strong>
              <span>无海报 {count(total)}</span>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

export function SystemPanel() {
  const [system, setSystem] = useState<SystemSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();
  const report = useMemo(() => buildSystemReport(system), [system]);

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

  const copyReport = async () => {
    if (!system) {
      toast.push('系统报告还没加载完成', 'warn');
      return;
    }
    try {
      await navigator.clipboard.writeText(report);
      toast.push('系统报告已复制', 'ok');
    } catch (e) {
      toast.push(`复制失败：${errorMessage(e)}`, 'error');
    }
  };

  const roots = system?.configured_roots || [];
  const healthyRoots = roots.filter((path) => path.exists && path.warnings.length === 0).length;
  const memory = system?.host?.memory;
  const loadAverage = system?.host?.load_average;
  const embyTone = system?.emby ? statusTone(system.emby.status, system.emby.online) : 'neutral';
  const dockerTone = system?.docker ? statusTone(system.docker.status, system.docker.status === 'ok') : 'neutral';
  const pathTone = system && roots.length > 0 && healthyRoots === roots.length ? 'ok' : 'warn';

  return (
    <section className="readonlyPanel">
      <div className="readonlyToolbar">
        <div>
          <strong>系统健康</strong>
          <span>数据库、路径、磁盘和主机负载的实时只读状态。</span>
        </div>
        <div className="readonlyToolbarActions">
          <button className="btn ghost" onClick={copyReport} disabled={!system}>
            <Copy size={16} />
            复制系统报告
          </button>
          <button className="btn ghost" onClick={load} disabled={loading}>
            <RefreshCw size={16} />
            {loading ? '加载中' : '刷新'}
          </button>
        </div>
      </div>
      {error && <div className="notice warn">{error}</div>}
      <div className="statGrid">
        <StatCard icon={<CheckCircle2 />} label="整体" value={system?.ok ? '正常' : '需检查'} tone={system?.ok ? 'ok' : 'warn'} hint={system?.version || '等待数据'} />
        <StatCard icon={<Server />} label="Emby" value={system?.emby?.online ? '在线' : system?.emby?.configured ? '离线' : '未配置'} tone={embyTone} hint={system?.emby?.version || system?.emby?.server_name || '无版本'} />
        <StatCard icon={<Server />} label="Docker" value={system?.docker ? `${count(system.docker.running)} / ${count(system.docker.total)}` : '未知'} tone={dockerTone} hint={system?.docker?.status || '等待数据'} />
        <StatCard icon={<HardDrive />} label="路径" value={`${count(healthyRoots)} / ${count(roots.length)}`} tone={pathTone} hint="已配置根路径" />
        <StatCard icon={<Database />} label="Postgres" value={system?.database?.status || '未知'} tone={system?.database?.status === 'ok' ? 'ok' : 'warn'} hint={`${system?.database?.pool_size || 0} pool / ${system?.database?.idle_connections || 0} idle`} />
        <StatCard icon={<Gauge />} label="内存" value={percent(memory?.used_percent)} hint={memory ? `${bytes(memory.available_bytes)} 可用 / ${bytes(memory.total_bytes)}` : '无主机数据'} />
        <StatCard icon={<Activity />} label="负载" value={loadAverage ? loadAverage.one.toFixed(2) : '未知'} hint={loadAverage ? `${loadAverage.five.toFixed(2)} / ${loadAverage.fifteen.toFixed(2)}` : `${system?.host?.os || ''} ${system?.host?.arch || ''}`} />
        <StatCard icon={<FileText />} label="进程" value={system?.host?.process_id ? String(system.host.process_id) : '未知'} hint={`${system?.host?.os || 'unknown'} / ${system?.host?.arch || 'unknown'}`} />
      </div>
      <WarningList warnings={system?.warnings || []} />
      <div className="systemDetailGrid">
        <SystemEmbyBlock emby={system?.emby} />
        <SystemDockerBlock docker={system?.docker} />
      </div>
      <SystemHostBlock host={system?.host} />
      <section className="readonlyBlock">
        <h2>路径与磁盘</h2>
        <DiskMountList paths={roots} />
        <div className="pathGrid">
          {roots.map((path) => <PathCard key={path.key} path={path} />)}
          {system && roots.length === 0 && <div className="empty inlineEmpty">没有配置根路径</div>}
        </div>
      </section>
      {system?.database?.warning && <div className="notice warn">{system.database.warning}</div>}
    </section>
  );
}

function SystemEmbyBlock({ emby }: { emby?: EmbyHealthSummary }) {
  if (!emby) {
    return (
      <section className="readonlyBlock">
        <h2>Emby</h2>
        <div className="empty inlineEmpty">等待 Emby 状态</div>
      </section>
    );
  }
  const tone = statusTone(emby.status, emby.online);
  return (
    <section className="readonlyBlock">
      <div className="systemBlockHead">
        <h2>Emby</h2>
        <span className={badgeClass(tone)}>{emby.online ? 'online' : emby.status}</span>
      </div>
      <div className="systemKeyValueGrid">
        <span><strong>版本</strong>{emby.version || '未知'}</span>
        <span><strong>服务器</strong>{emby.server_name || '未返回'}</span>
        <span><strong>HTTP</strong>{emby.http_status ? `HTTP ${emby.http_status}` : '无响应'}</span>
        <span><strong>系统</strong>{emby.operating_system || '未知'}</span>
        <span><strong>已配置</strong>{boolText(emby.configured)}</span>
        <span><strong>Server ID</strong>{emby.server_id || '未返回'}</span>
      </div>
      <code className="systemCodeLine">{emby.base_url || '未配置 base_url'}</code>
      {emby.warning && <div className="notice warn">{emby.warning}</div>}
    </section>
  );
}

function SystemDockerBlock({ docker }: { docker?: SystemSummary['docker'] }) {
  if (!docker) {
    return (
      <section className="readonlyBlock">
        <h2>Docker</h2>
        <div className="empty inlineEmpty">等待 Docker 状态</div>
      </section>
    );
  }
  const tone = statusTone(docker.status, docker.status === 'ok');
  return (
    <section className="readonlyBlock">
      <div className="systemBlockHead">
        <h2>Docker</h2>
        <span className={badgeClass(tone)}>{docker.status}</span>
      </div>
      <div className="miniStats systemMiniStats">
        <span><strong>{count(docker.total)}</strong>容器总数</span>
        <span><strong>{count(docker.running)}</strong>运行中</span>
        <span><strong>{boolText(docker.available)}</strong>CLI 可用</span>
        <span><strong>{boolText(docker.configured)}</strong>已配置</span>
      </div>
      <code className="systemCodeLine">{docker.docker_bin || '未配置 docker_bin'}</code>
      {docker.warning && <div className="notice warn">{docker.warning}</div>}
      <strong className="systemSubhead">容器列表</strong>
      <DockerContainerList containers={docker.containers} />
    </section>
  );
}

function SystemHostBlock({ host }: { host?: HostMetrics }) {
  if (!host) {
    return (
      <section className="readonlyBlock">
        <h2>主机指标</h2>
        <div className="empty inlineEmpty">等待主机指标</div>
      </section>
    );
  }
  const memory = host.memory;
  return (
    <section className="readonlyBlock">
      <h2>主机指标</h2>
      <div className="systemKeyValueGrid">
        <span><strong>OS / Arch</strong>{host.os} / {host.arch}</span>
        <span><strong>进程 PID</strong>{host.process_id}</span>
        <span><strong>内存总量</strong>{memory ? bytes(memory.total_bytes) : '未知'}</span>
        <span><strong>内存已用</strong>{memory ? `${bytes(memoryUsedBytes(memory))} (${percent(memory.used_percent)})` : '未知'}</span>
        <span><strong>内存可用</strong>{memory ? bytes(memory.available_bytes) : '未知'}</span>
        <span><strong>Load 1 / 5 / 15</strong>{loadAverageText(host.load_average)}</span>
      </div>
    </section>
  );
}

function DockerContainerList({ containers }: { containers: DockerContainerSummary[] }) {
  if (!containers.length) return <div className="empty inlineEmpty">没有 Docker 容器数据</div>;
  return (
    <div className="containerList">
      {containers.map((container) => {
        const running = container.state.toLowerCase() === 'running';
        return (
          <article key={container.id || container.name} className="containerItem">
            <div>
              <strong>{container.name}</strong>
              <span className={badgeClass(running ? 'ok' : 'warn')}>{container.state || 'unknown'}</span>
            </div>
            <code>{container.image}</code>
            <small>{container.status || '无状态'} · {container.ports || '无端口映射'}</small>
            <small>{container.id}</small>
          </article>
        );
      })}
    </div>
  );
}

function DiskMountList({ paths }: { paths: PathStatus[] }) {
  const mounts = new Map<string, NonNullable<PathStatus['disk']>>();
  paths.forEach((path) => {
    if (!path.disk) return;
    mounts.set(`${path.disk.filesystem}:${path.disk.mount_point}`, path.disk);
  });
  const disks = Array.from(mounts.values());
  if (!disks.length) return <div className="empty inlineEmpty">没有磁盘挂载数据</div>;
  return (
    <div className="miniStats systemMiniStats">
      {disks.map((disk) => (
        <span key={`${disk.filesystem}:${disk.mount_point}`}>
          <strong>{disk.mount_point}</strong>
          {bytes(disk.available_bytes)} 可用 / {bytes(disk.total_bytes)} · {percent(disk.used_percent)} 已用
        </span>
      ))}
    </div>
  );
}

function PathCard({ path }: { path: PathStatus }) {
  const tone = path.exists && path.warnings.length === 0 ? 'ok' : path.exists ? 'warn' : 'error';
  return (
    <article className={`pathCard ${tone}`}>
      <div>
        <HardDrive size={17} />
        <strong>{path.label}</strong>
        <span className={badgeClass(tone)}>{path.exists ? '存在' : '缺失'}</span>
      </div>
      <code title={path.path}>{path.path}</code>
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
        <StatCard icon={<CheckCircle2 />} label="覆盖率" value={percent(overview?.subtitle_coverage_percent)} tone={overview?.strm_without_subtitles ? 'warn' : 'ok'} hint={`${count(overview?.strm_with_subtitles)} 有字幕 / ${count(overview?.strm_without_subtitles)} 缺字幕`} />
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
  const subtitleExtensions = overview.subtitle_extensions || [];
  const subtitleLanguages = overview.subtitle_languages || [];
  const libraryCoverage = overview.library_coverage || [];
  const missingSubtitleSamples = overview.missing_subtitle_samples || [];
  const samples = overview.samples || [];
  return (
    <div className="readonlySplit">
      <section className="readonlyBlock">
        <h2>字幕扩展</h2>
        <div className="extensionList">
          {subtitleExtensions.map((item) => (
            <span key={item.extension}><strong>.{item.extension}</strong>{count(item.count)}</span>
          ))}
          {subtitleExtensions.length === 0 && <div className="empty inlineEmpty">没有发现外挂字幕扩展</div>}
        </div>
      </section>
      <section className="readonlyBlock">
        <h2>语言</h2>
        <div className="extensionList">
          {subtitleLanguages.map((item) => (
            <span key={item.language}><strong>{item.language}</strong>{count(item.count)}</span>
          ))}
          {subtitleLanguages.length === 0 && <div className="empty inlineEmpty">没有语言标签</div>}
        </div>
      </section>
      <section className="readonlyBlock">
        <h2>按库覆盖</h2>
        <div className="coverageList">
          {libraryCoverage.map((item) => (
            <article key={item.library}>
              <strong>{item.library}</strong>
              <span>{percent(item.coverage_percent)}</span>
              <small>{count(item.with_subtitles)} / {count(item.strm_files)} 有字幕，缺 {count(item.missing_subtitles)}</small>
            </article>
          ))}
          {libraryCoverage.length === 0 && <div className="empty inlineEmpty">没有 .strm 可统计</div>}
        </div>
      </section>
      <section className="readonlyBlock">
        <h2>样例</h2>
        <div className="sampleList">
          {samples.map((sample) => (
            <article key={`${sample.kind}-${sample.rel_path}`}>
              <span className="badge">{sample.kind}</span>
              <strong>{sample.rel_path}</strong>
              <small>.{sample.extension || 'unknown'} · {bytes(sample.size)}</small>
            </article>
          ))}
          {samples.length === 0 && <div className="empty inlineEmpty">没有样例</div>}
        </div>
      </section>
      <section className="readonlyBlock">
        <h2>缺字幕样例</h2>
        <div className="sampleList">
          {missingSubtitleSamples.map((sample) => (
            <article key={sample}>
              <span className="badge warn">missing</span>
              <strong>{sample}</strong>
            </article>
          ))}
          {missingSubtitleSamples.length === 0 && <div className="empty inlineEmpty">没有缺字幕样例</div>}
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
          <span>展示 seen/unmatched 表；自动处理由 webhook 入口触发。</span>
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
        <StatCard icon={<CheckCircle2 />} label="业务 port" value={status?.complete_business_port ? '完整' : '只读'} tone={status?.complete_business_port ? 'ok' : 'warn'} hint="状态只读，webhook 写入" />
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
