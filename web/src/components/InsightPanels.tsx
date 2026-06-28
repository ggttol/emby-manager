import {
  AlertTriangle,
  CheckCircle2,
  FileText,
  ListChecks,
  RefreshCw,
  SearchX,
  Webhook
} from 'lucide-react';
import { ReactNode, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useToast } from './Toast';

type CatalogInsight = components['schemas']['CatalogInsight'];
type CleanupSummaryResponse = components['schemas']['CleanupSummaryResponse'];
type GapsSummaryResponse = components['schemas']['GapsSummaryResponse'];
type InsightMeta = components['schemas']['InsightMeta'];
type InsightTodo = components['schemas']['InsightTodo'];
type StrmReadOnlyOverview = components['schemas']['StrmReadOnlyOverview'];
type TaskHistorySummary = components['schemas']['TaskHistorySummary'];

type Tone = 'neutral' | 'ok' | 'warn' | 'error';

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function count(value: number | null | undefined) {
  return Number(value || 0).toLocaleString('zh-CN');
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

function todoTone(severity: string): Tone {
  if (severity === 'high') return 'error';
  if (severity === 'medium') return 'warn';
  return 'neutral';
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
  tone?: Tone;
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

function WarningList({ warnings }: { warnings: string[] }) {
  if (!warnings.length) return null;
  return (
    <div className="notice warn whitespaceNotice">
      {warnings.map((warning) => <div key={warning}>{warning}</div>)}
    </div>
  );
}

function TodoList({ items, empty }: { items: InsightTodo[]; empty: string }) {
  if (!items.length) return <div className="empty inlineEmpty">{empty}</div>;
  return (
    <div className="insightList">
      {items.map((todo, index) => (
        <article className={todoTone(todo.severity)} key={`${todo.area}-${todo.source}-${index}`}>
          <span className={`badge ${todoTone(todo.severity)}`}>{todo.severity}</span>
          <strong>{todo.message}</strong>
          <small>{todo.area} · {todo.source} · {count(todo.count)}</small>
        </article>
      ))}
    </div>
  );
}

function MetaBlock({ meta }: { meta?: InsightMeta }) {
  if (!meta) return null;
  return (
    <section className="readonlyBlock">
      <h2>覆盖范围</h2>
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
      <h2>任务信号</h2>
      <div className="miniStats">
        <span>总数 <strong>{count(task.total)}</strong></span>
        <span>运行中 <strong>{count(task.running)}</strong></span>
        <span>失败 <strong>{count(task.error)}</strong></span>
        <span>中断 <strong>{count(task.interrupted)}</strong></span>
      </div>
      <div className="insightList">
        {task.recent_issues.map((issue) => (
          <article key={issue.id}>
            <span className={`badge ${issue.status}`}>{issue.status}</span>
            <strong>{issue.label || issue.kind}</strong>
            <small>{issue.message || '无错误消息'} · {dateText(issue.updated_at)}</small>
          </article>
        ))}
        {task.recent_issues.length === 0 && <div className="empty inlineEmpty">最近没有失败或中断任务</div>}
      </div>
    </section>
  );
}

function CatalogBlock({ catalog }: { catalog?: CatalogInsight }) {
  return (
    <section className="readonlyBlock">
      <h2>Catalog 概览</h2>
      <div className="miniStats catalogMiniStats">
        <span>总数 <strong>{count(catalog?.total)}</strong></span>
        <span>115 <strong>{count(catalog?.share115)}</strong></span>
        <span>磁力 <strong>{count(catalog?.magnet)}</strong></span>
        <span>ED2K <strong>{count(catalog?.ed2k)}</strong></span>
        <span>整包 <strong>{count(catalog?.packages)}</strong></span>
        <span>重名 <strong>{count(catalog?.duplicate_names)}</strong></span>
      </div>
    </section>
  );
}

function StrmBlock({ strm }: { strm?: StrmReadOnlyOverview }) {
  return (
    <section className="readonlyBlock">
      <h2>strm 只读信号</h2>
      <div className="miniStats">
        <span>文件 <strong>{count(strm?.files)}</strong></span>
        <span>.strm <strong>{count(strm?.strm_files)}</strong></span>
        <span>字幕 <strong>{count(strm?.subtitle_files)}</strong></span>
        <span>空目录 <strong>{count(strm?.empty_directories)}</strong></span>
      </div>
      <div className="insightList compact">
        {(strm?.samples || []).map((sample) => (
          <article key={`${sample.kind}-${sample.rel_path}`}>
            <span className="badge">{sample.kind}</span>
            <strong>{sample.rel_path}</strong>
          </article>
        ))}
        {strm && strm.samples.length === 0 && <div className="empty inlineEmpty">没有样例</div>}
        {!strm && <div className="empty inlineEmpty">等待 strm 数据</div>}
      </div>
    </section>
  );
}

function CleanupLayout({
  title,
  subtitle,
  notice,
  data,
  loading,
  error,
  onRefresh,
  variant
}: {
  title: string;
  subtitle: string;
  notice: string;
  data: CleanupSummaryResponse | null;
  loading: boolean;
  error: string;
  onRefresh: () => void;
  variant: 'cleanup' | 'dedup';
}) {
  const duplicateTotal = Number(data?.catalog?.duplicate_links || 0) + Number(data?.catalog?.duplicate_names || 0);

  return (
    <section className="insightPanel">
      <div className="insightToolbar">
        <div>
          <strong>{title}</strong>
          <span>{subtitle}</span>
        </div>
        <button className="btn ghost" onClick={onRefresh} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>
      <div className="notice warn scanNotice">{notice}</div>
      {error && <div className="notice warn whitespaceNotice">{error}</div>}
      <div className="statGrid">
        <StatCard icon={<CheckCircle2 />} label="业务状态" value={data?.complete_business_port ? '完整' : '只读'} tone={data?.complete_business_port ? 'ok' : 'warn'} hint="Rust preview" />
        <StatCard icon={<ListChecks />} label={variant === 'dedup' ? '重复信号' : '待处理'} value={count(variant === 'dedup' ? duplicateTotal : data?.todos.length)} tone={(variant === 'dedup' ? duplicateTotal : data?.todos.length) ? 'warn' : 'ok'} hint="不执行写操作" />
        <StatCard icon={<FileText />} label="strm / 字幕" value={`${count(data?.strm?.strm_files)} / ${count(data?.strm?.subtitle_files)}`} hint={data?.strm?.root || '等待数据'} />
        <StatCard icon={<AlertTriangle />} label="异常任务" value={count(taskProblemCount(data?.task_history))} tone={taskProblemCount(data?.task_history) ? 'warn' : 'ok'} hint={`运行中 ${count(data?.task_history?.running)}`} />
      </div>
      <WarningList warnings={data?.warnings || []} />
      {variant === 'dedup' ? (
        <section className="readonlyBlock">
          <h2>去重信号</h2>
          <div className="miniStats">
            <span>重复链接 <strong>{count(data?.catalog?.duplicate_links)}</strong></span>
            <span>重复名称 <strong>{count(data?.catalog?.duplicate_names)}</strong></span>
            <span>资源总量 <strong>{count(data?.catalog?.total)}</strong></span>
            <span>整包 <strong>{count(data?.catalog?.packages)}</strong></span>
          </div>
          <p className="mutedParagraph">v2 尚未暴露去重分析明细和替换/删除执行接口；这里先用 catalog 聚合值做风险提示。</p>
        </section>
      ) : (
        <section className="readonlyBlock">
          <h2>清理待办</h2>
          <TodoList items={data?.todos || []} empty="当前只读预检没有生成清理待办" />
        </section>
      )}
      <div className="readonlySplit">
        <CatalogBlock catalog={data?.catalog} />
        <section className="readonlyBlock">
          <h2>运行健康</h2>
          <div className="miniStats">
            <span>定时 <strong>{count(data?.schedules?.enabled)} / {count(data?.schedules?.total)}</strong></span>
            <span>定时错误 <strong>{count(data?.schedules?.last_errors)}</strong></span>
            <span>7 天错误 <strong>{count(data?.logs?.errors_7d)}</strong></span>
            <span>7 天警告 <strong>{count(data?.logs?.warnings_7d)}</strong></span>
          </div>
        </section>
      </div>
      <TaskHistory task={data?.task_history} />
      <MetaBlock meta={data?.meta} />
    </section>
  );
}

export function CleanupPanel() {
  const [data, setData] = useState<CleanupSummaryResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      setData(await api<CleanupSummaryResponse>('/api/v2/cleanup/suggest', { method: 'POST', body: JSON.stringify({}) }));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`智能清理预检失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  return (
    <CleanupLayout
      title="智能清理预检"
      subtitle="汇总任务、catalog、strm、autostrm 和日志信号。"
      notice="当前 Rust 版智能清理只读预检，不做评分删除、不移动文件、不调用 Emby/115。"
      data={data}
      loading={loading}
      error={error}
      onRefresh={load}
      variant="cleanup"
    />
  );
}

export function DedupPanel() {
  const [data, setData] = useState<CleanupSummaryResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      setData(await api<CleanupSummaryResponse>('/api/v2/cleanup/suggest', { method: 'POST', body: JSON.stringify({}) }));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`去重预检失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  return (
    <CleanupLayout
      title="去重预检"
      subtitle="先展示 catalog 聚合重复信号，替换和删除执行仍待移植。"
      notice="当前 Rust 版没有 dedup 写接口，不会执行替换、删除、Emby 更新或 undo 写入。"
      data={data}
      loading={loading}
      error={error}
      onRefresh={load}
      variant="dedup"
    />
  );
}

export function ZhuigengGapsPanel({ mode }: { mode: 'zhuigeng' | 'gaps' }) {
  const [data, setData] = useState<GapsSummaryResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const isZhuigeng = mode === 'zhuigeng';
  const title = isZhuigeng ? '追更只读预检' : '缺集只读预检';
  const subtitle = isZhuigeng
    ? '聚合 autostrm unmatched、任务异常和 strm 信号，真实在更剧扫描尚未 port。'
    : '聚合 catalog、strm 和任务异常，真实 TMDb 季集对照尚未 port。';
  const notice = isZhuigeng
    ? '当前 Rust 版没有独立追更扫描接口；定时 kind 仍是 dry-run，这里不会遍历在更剧或转存缺集。'
    : '当前 Rust 版不会连接 TMDb/Emby 推断真实缺集集号，只展示可用的只读预检信号。';

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      setData(await api<GapsSummaryResponse>('/api/v2/gaps/scan', { method: 'POST', body: JSON.stringify({}) }));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`${title}失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, [mode]);

  const topLibraries = useMemo(() => (data?.autostrm?.libraries || []).slice(0, 10), [data]);

  return (
    <section className="insightPanel">
      <div className="insightToolbar">
        <div>
          <strong>{title}</strong>
          <span>{subtitle}</span>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>
      <div className="notice warn scanNotice">{notice}</div>
      {error && <div className="notice warn whitespaceNotice">{error}</div>}
      <div className="statGrid">
        <StatCard icon={<CheckCircle2 />} label="业务状态" value={data?.complete_business_port ? '完整' : '只读'} tone={data?.complete_business_port ? 'ok' : 'warn'} hint="v2 preview" />
        <StatCard icon={<ListChecks />} label="待处理" value={count(data?.todos.length)} tone={data?.todos.length ? 'warn' : 'ok'} hint="只读预检" />
        <StatCard icon={isZhuigeng ? <Webhook /> : <SearchX />} label={isZhuigeng ? 'unmatched' : 'strm 文件'} value={isZhuigeng ? count(data?.autostrm?.unmatched?.total) : count(data?.strm?.strm_files)} tone={(isZhuigeng ? data?.autostrm?.unmatched?.total : data?.strm?.strm_files) ? 'warn' : 'neutral'} hint={isZhuigeng ? `${count(data?.autostrm?.seen?.total)} seen` : data?.strm?.root} />
        <StatCard icon={<AlertTriangle />} label="异常任务" value={count(taskProblemCount(data?.task_history))} tone={taskProblemCount(data?.task_history) ? 'warn' : 'ok'} hint={dateText(data?.task_history?.last_updated_at)} />
      </div>
      <WarningList warnings={data?.warnings || []} />
      <section className="readonlyBlock">
        <h2>待处理信号</h2>
        <TodoList items={data?.todos || []} empty={isZhuigeng ? '当前没有追更预检信号' : '当前没有缺集预检信号'} />
      </section>
      <div className="readonlySplit">
        <StrmBlock strm={data?.strm} />
        <section className="readonlyBlock">
          <h2>Autostrm 库分布</h2>
          <div className="insightList compact">
            {topLibraries.map((item) => (
              <article key={item.lib}>
                <span className="badge">{item.lib}</span>
                <strong>seen {count(item.seen)}</strong>
                <small>unmatched {count(item.unmatched)}</small>
              </article>
            ))}
            {data && topLibraries.length === 0 && <div className="empty inlineEmpty">暂无 autostrm 库数据</div>}
            {!data && <div className="empty inlineEmpty">等待 autostrm 数据</div>}
          </div>
        </section>
      </div>
      <div className="readonlySplit">
        <CatalogBlock catalog={data?.catalog} />
        <TaskHistory task={data?.task_history} />
      </div>
      <MetaBlock meta={data?.meta} />
      {isZhuigeng && (
        <section className="readonlyBlock">
          <h2>定时入口</h2>
          <p className="mutedParagraph">`zhuigeng_scan_airing` 已能在定时页创建和立即运行，但当前 worker 仍返回 preview dry-run。</p>
        </section>
      )}
    </section>
  );
}
