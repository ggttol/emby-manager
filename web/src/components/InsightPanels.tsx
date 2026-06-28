import {
  AlertTriangle,
  CheckCircle2,
  Copy,
  FileText,
  ListChecks,
  Play,
  RefreshCw,
  SearchX,
  Trash2,
  Webhook
} from 'lucide-react';
import { FormEvent, ReactNode, useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { ConfirmDanger } from './Modal';
import { TASK_COMPLETED_EVENT, type TaskCompleteDetail } from './TaskCenter';
import { useToast } from './Toast';

type CatalogInsight = components['schemas']['CatalogInsight'];
type CatalogDuplicateGroup = components['schemas']['CatalogDuplicateGroup'];
type CatalogDuplicatesResponse = components['schemas']['CatalogDuplicatesResponse'];
type CleanupSummaryResponse = components['schemas']['CleanupSummaryResponse'];
type DedupAnalysisResponse = components['schemas']['DedupAnalysisResponse'];
type DedupAutoAllResponse = components['schemas']['DedupAutoAllResponse'];
type DedupGroup = components['schemas']['DedupGroup'];
type DedupReviewGroup = components['schemas']['DedupReviewGroup'];
type EmbyLibrary = components['schemas']['EmbyLibrary'];
type GapsScanLibResult = components['schemas']['GapsScanLibResult'];
type GapsSummaryResponse = components['schemas']['GapsSummaryResponse'];
type InsightMeta = components['schemas']['InsightMeta'];
type InsightTodo = components['schemas']['InsightTodo'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];
type ReplaceExecuteResponse = components['schemas']['ReplaceExecuteResponse'];
type StrmReadOnlyOverview = components['schemas']['StrmReadOnlyOverview'];
type TaskRun = components['schemas']['TaskRun'];
type TaskHistorySummary = components['schemas']['TaskHistorySummary'];
type ZhuigengGapRow = components['schemas']['ZhuigengGapRow'];
type ZhuigengGapsSummaryResponse = components['schemas']['ZhuigengGapsSummaryResponse'];
type ZhuigengItem = components['schemas']['ZhuigengItem'];
type ZhuigengScanAiringResponse = components['schemas']['ZhuigengScanAiringResponse'];
type ZhuigengScanAiringRow = components['schemas']['ZhuigengScanAiringRow'];
type ZhuigengStatusResponse = components['schemas']['ZhuigengStatusResponse'];

type EmptyDirCleanupResponse = {
  ok: boolean;
  dry_run: boolean;
  execute: boolean;
  root: string;
  candidate_count: number;
  samples: string[];
  truncated: boolean;
  warnings: string[];
  task?: TaskRun | null;
};

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

function isActiveTask(task?: TaskRun | null) {
  return task ? ['pending', 'running'].includes(task.status) : false;
}

function shouldRefreshCleanup(task: TaskRun) {
  return task.status === 'done' && task.kind.startsWith('cleanup_');
}

function asGapsScanResult(result: unknown): GapsScanLibResult | null {
  if (!result || typeof result !== 'object') return null;
  const candidate = result as Partial<GapsScanLibResult>;
  if (!Array.isArray(candidate.items) || typeof candidate.lib !== 'string') return null;
  return candidate as GapsScanLibResult;
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

function DuplicateGroupList({
  title,
  groups,
  empty
}: {
  title: string;
  groups: CatalogDuplicateGroup[];
  empty: string;
}) {
  return (
    <div className="duplicateGroupColumn">
      <strong>{title}</strong>
      <div className="insightList compact">
        {groups.map((group) => (
          <article key={`${title}-${group.key}`}>
            <span className="badge warn">{count(group.count)}</span>
            <strong>{group.key}</strong>
            <small>
              {(group.link_types || []).join(' / ') || 'unknown'}
              {group.sample_names?.length ? ` · ${(group.sample_names || []).join('、')}` : ''}
              {group.sample_sheets?.length ? ` · ${(group.sample_sheets || []).join('、')}` : ''}
            </small>
          </article>
        ))}
        {groups.length === 0 && <div className="empty inlineEmpty">{empty}</div>}
      </div>
    </div>
  );
}

function CatalogDuplicateDetails({ data }: { data?: CatalogDuplicatesResponse | null }) {
  const distribution = data?.link_type_distribution || [];
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>资源目录重复信号</h2>
        <span className="badge">只读</span>
      </div>
      <div className="miniStats catalogMiniStats">
        <span>重复链接组 <strong>{count(data?.duplicate_link_groups)}</strong></span>
        <span>重复名称组 <strong>{count(data?.duplicate_name_groups)}</strong></span>
        <span>样例上限 <strong>{count(data?.limit)}</strong></span>
      </div>
      <div className="miniStats">
        {distribution.map((item) => (
          <span key={item.link_type}>{item.link_type} <strong>{count(item.count)}</strong></span>
        ))}
        {data && distribution.length === 0 && <span>分布 <strong>0</strong></span>}
      </div>
      <div className="readonlySplit">
        <DuplicateGroupList
          title="重复链接 top 组"
          groups={data?.link_groups || []}
          empty="没有重复链接样例"
        />
        <DuplicateGroupList
          title="重复名称 top 组"
          groups={data?.name_groups || []}
          empty="没有重复名称样例"
        />
      </div>
      <p className="mutedParagraph">仅展示 catalog_items 中 link/name 的重复分组，不生成媒体库删除建议。</p>
    </section>
  );
}

function StrmBlock({
  strm,
  emptyCleanup,
  emptyCleanupTask,
  action
}: {
  strm?: StrmReadOnlyOverview;
  emptyCleanup?: EmptyDirCleanupResponse | null;
  emptyCleanupTask?: TaskRun | null;
  action?: ReactNode;
}) {
  const emptySamples = emptyCleanup?.samples?.length ? emptyCleanup.samples : strm?.empty_directory_samples || [];
  const otherSamples = strm?.other_file_samples || [];
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>strm 只读信号</h2>
        {action}
      </div>
      <div className="miniStats">
        <span>文件 <strong>{count(strm?.files)}</strong></span>
        <span>.strm <strong>{count(strm?.strm_files)}</strong></span>
        <span>字幕 <strong>{count(strm?.subtitle_files)}</strong></span>
        <span>空目录 <strong>{count(strm?.empty_directories)}</strong></span>
        {emptyCleanup && <span>可清理 <strong>{count(emptyCleanup.candidate_count)}</strong></span>}
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
      <div className="sampleColumns">
        <div>
          <strong>空目录样例</strong>
          {emptySamples.map((item) => <code key={`empty-${item}`}>{item}</code>)}
          {strm && emptySamples.length === 0 && <small>暂无</small>}
        </div>
        <div>
          <strong>非 STRM 文件样例</strong>
          {otherSamples.map((item) => <code key={`other-${item}`}>{item}</code>)}
          {strm && otherSamples.length === 0 && <small>暂无</small>}
        </div>
      </div>
      {emptyCleanup?.truncated && <div className="notice warn">空目录候选已截断，仅处理前 {count(emptyCleanup.candidate_count)} 个</div>}
      {emptyCleanup?.warnings?.length ? <WarningList warnings={emptyCleanup.warnings} /> : null}
      {emptyCleanupTask && (
        <div className="notice">
          已创建任务：{emptyCleanupTask.label} · {emptyCleanupTask.status}
        </div>
      )}
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
  variant,
  duplicates,
  emptyCleanup,
  emptyCleanupTask,
  emptyCleanupLoading,
  onExecuteEmptyCleanup
}: {
  title: string;
  subtitle: string;
  notice: string;
  data: CleanupSummaryResponse | null;
  duplicates?: CatalogDuplicatesResponse | null;
  emptyCleanup?: EmptyDirCleanupResponse | null;
  emptyCleanupTask?: TaskRun | null;
  emptyCleanupLoading?: boolean;
  loading: boolean;
  error: string;
  onRefresh: () => void;
  onExecuteEmptyCleanup?: () => void;
  variant: 'cleanup' | 'dedup';
}) {
  const duplicateTotal = Number(data?.catalog?.duplicate_links || 0) + Number(data?.catalog?.duplicate_names || 0);
  const emptyCandidateCount = Number(emptyCleanup?.candidate_count ?? data?.strm?.empty_directories ?? 0);
  const emptyCleanupActive = isActiveTask(emptyCleanupTask);
  const emptyCleanupAction = variant === 'cleanup' ? (
    <button
      className="btn compact"
      onClick={onExecuteEmptyCleanup}
      disabled={emptyCleanupLoading || emptyCleanupActive || emptyCandidateCount === 0}
    >
      <Trash2 size={14} />
      {emptyCleanupLoading ? '提交中' : '清理空 STRM 目录'}
    </button>
  ) : undefined;

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
          <p className="mutedParagraph">只读展示资源目录重复信号；当前页面不会执行替换、删除、Emby 更新或 undo 写入。</p>
        </section>
      ) : (
        <section className="readonlyBlock">
          <h2>清理待办</h2>
          <TodoList items={data?.todos || []} empty="当前只读预检没有生成清理待办" />
        </section>
      )}
      {variant === 'dedup' && <CatalogDuplicateDetails data={duplicates} />}
      <div className="readonlySplit">
        <CatalogBlock catalog={data?.catalog} />
        <StrmBlock
          strm={data?.strm}
          emptyCleanup={variant === 'cleanup' ? emptyCleanup : null}
          emptyCleanupTask={variant === 'cleanup' ? emptyCleanupTask : null}
          action={emptyCleanupAction}
        />
      </div>
      <div className="readonlySplit">
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
  const [emptyCleanup, setEmptyCleanup] = useState<EmptyDirCleanupResponse | null>(null);
  const [emptyCleanupTask, setEmptyCleanupTask] = useState<TaskRun | null>(null);
  const [emptyCleanupLoading, setEmptyCleanupLoading] = useState(false);
  const [confirmEmptyCleanup, setConfirmEmptyCleanup] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [summary, emptyPreview] = await Promise.all([
        api<CleanupSummaryResponse>('/api/v2/cleanup/suggest', { method: 'POST', body: JSON.stringify({}) }),
        api<EmptyDirCleanupResponse>('/api/v2/cleanup/empty-dirs', { method: 'POST', body: JSON.stringify({ execute: false }) })
      ]);
      setData(summary);
      setEmptyCleanup(emptyPreview);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`智能清理预检失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    const onTaskCompleted = (event: Event) => {
      const detail = (event as CustomEvent<TaskCompleteDetail>).detail;
      if (detail?.task && shouldRefreshCleanup(detail.task)) {
        setEmptyCleanupTask(detail.task);
        load();
      }
    };
    window.addEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
    return () => window.removeEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
  }, [load]);

  const executeEmptyCleanup = async () => {
    setEmptyCleanupLoading(true);
    setConfirmEmptyCleanup(false);
    try {
      const result = await api<EmptyDirCleanupResponse>('/api/v2/cleanup/empty-dirs', {
        method: 'POST',
        body: JSON.stringify({ execute: true })
      });
      setEmptyCleanup(result);
      setEmptyCleanupTask(result.task || null);
      toast.push('已创建空 STRM 目录清理任务', 'ok');
    } catch (e) {
      toast.push(`清理空 STRM 目录失败：${errorMessage(e)}`, 'error');
    } finally {
      setEmptyCleanupLoading(false);
    }
  };

  return (
    <>
      {confirmEmptyCleanup && (
        <ConfirmDanger
          title="确认清理空 STRM 目录"
          confirmText="确认清理"
          onCancel={() => setConfirmEmptyCleanup(false)}
          onConfirm={executeEmptyCleanup}
          body={(
            <div className="dangerCopy">
              <p>只删除 strm_root 下当前仍为空的目录，不访问 115/CD 根目录。</p>
              <code>{emptyCleanup?.root || data?.strm?.root || 'strm_root'}</code>
            </div>
          )}
        />
      )}
      <CleanupLayout
        title="智能清理预检"
        subtitle="汇总任务、catalog、strm、autostrm 和日志信号。"
        notice="当前 Rust 版智能清理只读预检，不做评分删除、不移动文件、不调用 Emby/115。"
        data={data}
        emptyCleanup={emptyCleanup}
        emptyCleanupTask={emptyCleanupTask}
        emptyCleanupLoading={emptyCleanupLoading}
        loading={loading}
        error={error}
        onRefresh={load}
        onExecuteEmptyCleanup={() => setConfirmEmptyCleanup(true)}
        variant="cleanup"
      />
    </>
  );
}

function DedupAutoGroups({ groups }: { groups: DedupGroup[] }) {
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>Auto groups</h2>
        <span className="badge warn">{count(groups.length)}</span>
      </div>
      <div className="insightList compact">
        {groups.map((group) => (
          <article key={`auto-${group.tmdb}-${group.keep.folder}`}>
            <span className="badge warn">tmdb:{group.tmdb || 'unknown'}</span>
            <strong>保留 {group.keep.folder}</strong>
            <small>
              {group.keep.lib} · score {count(group.keep.score)} · 删除 {count(group.remove.length)} 个：
              {group.remove.map((row) => row.folder).join('、') || '无'}
            </small>
          </article>
        ))}
        {groups.length === 0 && <div className="empty inlineEmpty">没有可自动处理的重复组</div>}
      </div>
    </section>
  );
}

function DedupReviewGroups({ groups }: { groups: DedupReviewGroup[] }) {
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>Review groups</h2>
        <span className="badge">{count(groups.length)}</span>
      </div>
      <div className="insightList compact">
        {groups.map((group) => (
          <article key={`review-${group.tmdb}-${group.why}`}>
            <span className="badge">tmdb:{group.tmdb || 'unknown'}</span>
            <strong>{group.why}</strong>
            <small>
              {group.rows.map((row) => `${row.lib}/${row.folder} · score ${row.score} · n ${row.n}`).join('；')}
            </small>
          </article>
        ))}
        {groups.length === 0 && <div className="empty inlineEmpty">没有需要人工复核的重复组</div>}
      </div>
    </section>
  );
}

function DedupAutoAllResultBlock({ result }: { result: DedupAutoAllResponse | null }) {
  if (!result) return null;
  return (
    <section className="readonlyBlock">
      <h2>Auto-all 结果</h2>
      <div className="miniStats">
        <span>处理组 <strong>{count(result.total)}</strong></span>
        <span>成功 <strong>{count(result.ok_count)}</strong></span>
        <span>删除 folder <strong>{count(result.total_removed_folders)}</strong></span>
        <span>仍需复核 <strong>{count(result.review_count)}</strong></span>
      </div>
      <div className="insightList compact">
        {result.results.map((item) => (
          <article key={`auto-result-${item.tmdb}-${item.kept}`}>
            <span className={`badge ${item.ok ? 'ok' : 'error'}`}>{item.ok ? 'ok' : 'error'}</span>
            <strong>{item.kept}</strong>
            <small>tmdb:{item.tmdb || 'unknown'} · removed {count(item.removed)}{item.err ? ` · ${item.err}` : ''}</small>
          </article>
        ))}
        {result.results.length === 0 && <div className="empty inlineEmpty">没有执行任何自动组</div>}
      </div>
    </section>
  );
}

function ReplaceResultBlock({ result }: { result: ReplaceExecuteResponse | null }) {
  if (!result) return null;
  return (
    <section className="readonlyBlock">
      <h2>Replace 结果</h2>
      <div className="miniStats">
        <span>状态 <strong>{result.ok ? '完成' : '失败'}</strong></span>
        <span>库 <strong>{result.lib}</strong></span>
        <span>重命名 <strong>{result.renamed ? '是' : '否'}</strong></span>
        <span>通知 Emby <strong>{result.notified ? '是' : '否'}</strong></span>
      </div>
      <div className="insightList compact">
        <article>
          <span className="badge ok">keep</span>
          <strong>{result.kept_as}</strong>
          <small>{result.msg}</small>
        </article>
        <article>
          <span className="badge warn">drop</span>
          <strong>{result.dropped}</strong>
          <small>{result.deleted_from.join('、') || '没有删除路径'} · undo {result.undo_id}</small>
        </article>
        {result.emby_updates.map((item) => (
          <article key={`${item.Path}-${item.UpdateType}`}>
            <span className="badge">{item.UpdateType}</span>
            <strong>{item.Path}</strong>
          </article>
        ))}
      </div>
    </section>
  );
}

export function DedupPanel() {
  const [data, setData] = useState<DedupAnalysisResponse | null>(null);
  const [autoAllResult, setAutoAllResult] = useState<DedupAutoAllResponse | null>(null);
  const [replaceResult, setReplaceResult] = useState<ReplaceExecuteResponse | null>(null);
  const [replaceDraft, setReplaceDraft] = useState({ lib: '', win_folder: '', lose_folder: '', reason: '' });
  const [confirmAction, setConfirmAction] = useState<'auto-all' | 'replace' | null>(null);
  const [acting, setActing] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      setData(await api<DedupAnalysisResponse>('/api/v2/dedup/duplicates'));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`去重列表加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    load();
  }, [load]);

  const autoGroups = data?.dups || [];
  const reviewGroups = data?.review || [];
  const removeTotal = autoGroups.reduce((total, group) => total + group.remove.length, 0);
  const reviewRows = reviewGroups.reduce((total, group) => total + group.rows.length, 0);
  const replaceReady = Boolean(
    replaceDraft.lib.trim() &&
    replaceDraft.win_folder.trim() &&
    replaceDraft.lose_folder.trim()
  );

  const patchReplaceDraft = (patch: Partial<typeof replaceDraft>) => {
    setReplaceDraft((current) => ({ ...current, ...patch }));
  };

  const submitReplace = (event: FormEvent) => {
    event.preventDefault();
    if (!replaceReady) {
      toast.push('先填写 lib、win_folder 和 lose_folder', 'warn');
      return;
    }
    setConfirmAction('replace');
  };

  const executeAutoAll = async () => {
    setActing(true);
    setConfirmAction(null);
    try {
      const result = await api<DedupAutoAllResponse>('/api/v2/dedup/auto-all', {
        method: 'POST',
        body: JSON.stringify({})
      });
      setAutoAllResult(result);
      toast.push(`auto-all 完成：删除 ${result.total_removed_folders} 个 folder`, 'ok');
      await load();
    } catch (e) {
      toast.push(`auto-all 失败：${errorMessage(e)}`, 'error');
    } finally {
      setActing(false);
    }
  };

  const executeReplace = async () => {
    setActing(true);
    setConfirmAction(null);
    try {
      const result = await api<ReplaceExecuteResponse>('/api/v2/dedup/replace', {
        method: 'POST',
        body: JSON.stringify({
          lib: replaceDraft.lib.trim(),
          win_folder: replaceDraft.win_folder.trim(),
          lose_folder: replaceDraft.lose_folder.trim(),
          reason: replaceDraft.reason.trim() || undefined
        })
      });
      setReplaceResult(result);
      toast.push(result.msg || 'replace 执行完成', 'ok');
      await load();
    } catch (e) {
      toast.push(`replace 失败：${errorMessage(e)}`, 'error');
    } finally {
      setActing(false);
    }
  };

  return (
    <>
      {confirmAction === 'auto-all' && (
        <ConfirmDanger
          title="确认自动去重"
          confirmText="确认执行 auto-all"
          onCancel={() => setConfirmAction(null)}
          onConfirm={executeAutoAll}
          body={(
            <div className="dangerCopy">
              <p>将按后端自动组删除 remove folder，并写入 undo 与 Emby 更新。人审组不会自动处理。</p>
              <code>auto groups {count(autoGroups.length)} · remove folders {count(removeTotal)}</code>
            </div>
          )}
        />
      )}
      {confirmAction === 'replace' && (
        <ConfirmDanger
          title="确认替换重复目录"
          confirmText="确认 replace"
          onCancel={() => setConfirmAction(null)}
          onConfirm={executeReplace}
          body={(
            <div className="dangerCopy">
              <p>将删除 lose folder，并把 win folder 作为保留目录完成替换。</p>
              <code>{replaceDraft.lib.trim()} / keep {replaceDraft.win_folder.trim()} / drop {replaceDraft.lose_folder.trim()}</code>
            </div>
          )}
        />
      )}
      <section className="insightPanel">
        <div className="insightToolbar">
          <div>
            <strong>去重闭环</strong>
            <span>读取 strm 重复分组，自动组可一键处理，人审组可手动 replace。</span>
          </div>
          <button className="btn ghost" onClick={load} disabled={loading || acting}>
            <RefreshCw size={16} />
            {loading ? '加载中' : '刷新'}
          </button>
        </div>
        <div className="notice warn scanNotice">去重会删除目录并触发 Emby 更新；auto-all 与 replace 都会先要求确认。</div>
        {error && <div className="notice warn whitespaceNotice">{error}</div>}
        <div className="statGrid">
          <StatCard icon={<ListChecks />} label="Auto groups" value={count(autoGroups.length)} tone={autoGroups.length ? 'warn' : 'ok'} hint={`remove ${count(removeTotal)}`} />
          <StatCard icon={<AlertTriangle />} label="Review groups" value={count(reviewGroups.length)} tone={reviewGroups.length ? 'warn' : 'ok'} hint={`rows ${count(reviewRows)}`} />
          <StatCard icon={<Trash2 />} label="已执行删除" value={count(autoAllResult?.total_removed_folders)} tone={autoAllResult?.total_removed_folders ? 'warn' : 'neutral'} hint={autoAllResult ? `成功 ${count(autoAllResult.ok_count)}` : '等待操作'} />
          <StatCard icon={<CheckCircle2 />} label="Replace" value={replaceResult?.ok ? '完成' : '待命'} tone={replaceResult?.ok ? 'ok' : 'neutral'} hint={replaceResult?.kept_as || '手动 lib/win/lose'} />
        </div>
        <section className="readonlyBlock">
          <div className="sectionTitleRow">
            <h2>执行入口</h2>
            <button className="btn danger compact" onClick={() => setConfirmAction('auto-all')} disabled={loading || acting || autoGroups.length === 0}>
              <Trash2 size={14} />
              {acting ? '执行中' : 'auto-all'}
            </button>
          </div>
          <form className="manageForm" onSubmit={submitReplace}>
            <div className="manageFormHead">
              <strong>Replace</strong>
            </div>
            <label>
              lib
              <input className="input" aria-label="替换 lib" value={replaceDraft.lib} onChange={(event) => patchReplaceDraft({ lib: event.target.value })} placeholder="例如 剧集" />
            </label>
            <label>
              win_folder
              <input className="input" aria-label="替换 win_folder" value={replaceDraft.win_folder} onChange={(event) => patchReplaceDraft({ win_folder: event.target.value })} placeholder="保留目录名" />
            </label>
            <label>
              lose_folder
              <input className="input" aria-label="替换 lose_folder" value={replaceDraft.lose_folder} onChange={(event) => patchReplaceDraft({ lose_folder: event.target.value })} placeholder="删除目录名" />
            </label>
            <label>
              reason
              <input className="input" aria-label="替换原因" value={replaceDraft.reason} onChange={(event) => patchReplaceDraft({ reason: event.target.value })} placeholder="可选" />
            </label>
            <button className="btn danger" type="submit" disabled={acting || !replaceReady}>replace</button>
          </form>
        </section>
        <DedupAutoAllResultBlock result={autoAllResult} />
        <ReplaceResultBlock result={replaceResult} />
        <div className="readonlySplit">
          <DedupAutoGroups groups={autoGroups} />
          <DedupReviewGroups groups={reviewGroups} />
        </div>
      </section>
    </>
  );
}

function episodeSummaryText(episode?: ZhuigengItem['last_episode_to_air']) {
  if (!episode) return '';
  const season = episode.season_number ?? '?';
  const ep = episode.episode_number ?? '?';
  return [
    `S${String(season).padStart(2, '0')}E${String(ep).padStart(2, '0')}`,
    episode.name,
    episode.air_date
  ].filter(Boolean).join(' · ');
}

function zhuigengItemTone(item: ZhuigengItem): Tone {
  if (item.err) return 'error';
  if (item.behind > 0) return 'warn';
  if (item.continuing) return 'ok';
  return 'neutral';
}

function CopyTextBlock({
  title,
  text,
  empty,
  onCopy
}: {
  title: string;
  text: string;
  empty: string;
  onCopy: (text: string, label: string) => void;
}) {
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>{title}</h2>
        <button className="btn ghost compact" onClick={() => onCopy(text, title)} disabled={!text.trim()}>
          <Copy size={14} />
          复制
        </button>
      </div>
      <div className="notice whitespaceNotice">{text.trim() || empty}</div>
    </section>
  );
}

function ZhuigengItemList({ items }: { items: ZhuigengItem[] }) {
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>追更列表</h2>
        <span className="badge">{count(items.length)}</span>
      </div>
      <div className="gapResultList">
        {items.map((item) => {
          const tone = zhuigengItemTone(item);
          const lastEpisode = episodeSummaryText(item.last_episode_to_air);
          const nextEpisode = episodeSummaryText(item.next_episode_to_air);
          return (
            <article key={`${item.lib}-${item.id || item.folder}-${item.tmdb}`} className={tone === 'error' ? 'error' : ''}>
              <div>
                <strong>{item.name}</strong>
                <span className={`badge ${tone}`}>{item.continuing ? 'continuing' : item.ended ? 'ended' : item.state}</span>
                {item.behind > 0 && <span className="badge warn">behind {count(item.behind)}</span>}
                {item.tmdb && <span className="badge">tmdb:{item.tmdb}</span>}
              </div>
              <p>{item.resource_hint || item.behind_hint || item.state}</p>
              <small>
                {item.lib} · local {count(item.local_count)}
                {item.local_latest_episode ? ` · latest ${item.local_latest_episode}` : ''}
                {lastEpisode ? ` · last ${lastEpisode}` : ''}
                {nextEpisode ? ` · next ${nextEpisode}` : ''}
              </small>
              {item.err && <p className="errorText">{item.err}</p>}
            </article>
          );
        })}
        {items.length === 0 && <div className="empty inlineEmpty">没有追更条目</div>}
      </div>
    </section>
  );
}

function ZhuigengScanAiringBlock({
  result,
  onCopy
}: {
  result: ZhuigengScanAiringResponse | null;
  onCopy: (text: string, label: string) => void;
}) {
  if (!result) return null;
  const rows = result.results || [];
  const behind = rows.reduce((total, row) => total + row.behind, 0);
  const errors = rows.filter((row) => !row.ok || row.err).length;
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>在更扫描结果</h2>
        <button className="btn ghost compact" onClick={() => onCopy(result.copy_text, '在更求资源文本')} disabled={!result.copy_text.trim()}>
          <Copy size={14} />
          复制
        </button>
      </div>
      <div className="miniStats">
        <span>在更 <strong>{count(result.total)}</strong></span>
        <span>落后 <strong>{count(behind)}</strong></span>
        <span>错误 <strong>{count(errors)}</strong></span>
        <span>可复制 <strong>{result.copy_text.trim() ? '有' : '无'}</strong></span>
      </div>
      {result.note && <p className="mutedParagraph">{result.note}</p>}
      <div className="gapResultList">
        {rows.map((row: ZhuigengScanAiringRow) => (
          <article key={`airing-${row.lib}-${row.id || row.tmdb}-${row.name}`} className={row.err ? 'error' : ''}>
            <div>
              <strong>{row.name}</strong>
              <span className={`badge ${row.behind ? 'warn' : 'ok'}`}>{row.status}</span>
              {row.tmdb && <span className="badge">tmdb:{row.tmdb}</span>}
            </div>
            <p>{row.hint || row.err || '当前不落后'}</p>
            <small>{row.lib} · behind {count(row.behind)}</small>
          </article>
        ))}
        {rows.length === 0 && <div className="empty inlineEmpty">没有在更扫描结果</div>}
      </div>
    </section>
  );
}

function ZhuigengGapsSummaryBlock({
  result,
  onCopy
}: {
  result: ZhuigengGapsSummaryResponse | null;
  onCopy: (text: string, label: string) => void;
}) {
  if (!result) return null;
  const rows = result.items || [];
  const behind = rows.reduce((total, row) => total + row.behind, 0);
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>追更缺集汇总</h2>
        <button className="btn ghost compact" onClick={() => onCopy(result.copy_text, '追更缺集求资源文本')} disabled={!result.copy_text.trim()}>
          <Copy size={14} />
          复制
        </button>
      </div>
      <div className="miniStats">
        <span>条目 <strong>{count(result.total)}</strong></span>
        <span>落后集数 <strong>{count(behind)}</strong></span>
        <span>可复制 <strong>{result.copy_text.trim() ? '有' : '无'}</strong></span>
        <span>状态 <strong>{result.ok ? 'ok' : '异常'}</strong></span>
      </div>
      <div className="gapResultList">
        {rows.map((row: ZhuigengGapRow) => (
          <article key={`gap-${row.lib}-${row.id || row.tmdb}-${row.name}`}>
            <div>
              <strong>{row.name}</strong>
              <span className="badge warn">behind {count(row.behind)}</span>
              {row.tmdb && <span className="badge">tmdb:{row.tmdb}</span>}
            </div>
            <p>{row.fmt}</p>
            <small>{row.lib}</small>
          </article>
        ))}
        {rows.length === 0 && <div className="empty inlineEmpty">没有追更缺集</div>}
      </div>
    </section>
  );
}

function GapsScanResultBlock({
  result,
  onCopy
}: {
  result: GapsScanLibResult;
  onCopy: (text: string, label: string) => void;
}) {
  const rows = result.items || [];
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>全库缺集报告</h2>
        {result.copy_text && (
          <button className="btn ghost compact" onClick={() => onCopy(result.copy_text, `${rows.length} 行求资源文本`)}>
            <Copy size={14} />
            复制全部
          </button>
        )}
      </div>
      <div className="miniStats">
        <span>库 <strong>{result.lib}</strong></span>
        <span>已扫 <strong>{count(result.analyzed)}</strong></span>
        <span>有缺/落后 <strong>{count(result.total)}</strong></span>
        <span>错误 <strong>{count(rows.filter((row) => row.err).length)}</strong></span>
      </div>
      {rows.length === 0 ? (
        <div className="empty inlineEmpty">全部齐全，没有缺集或落后 TMDb 的项目</div>
      ) : (
        <div className="gapResultList">
          {rows.map((row) => (
            <article key={`${row.id || row.name}-${row.fmt || row.err || ''}`} className={row.err ? 'error' : ''}>
              <div>
                <strong>{row.name}</strong>
                {row.tmdb && <span className="badge">tmdb:{row.tmdb}</span>}
                {!row.err && <span className="badge warn">score {row.score}</span>}
              </div>
              {row.err ? (
                <p>{row.err}</p>
              ) : (
                <p>{row.fmt}</p>
              )}
              {!row.err && (
                <small>缺 {count(row.gap_count)} · 落后 {count(row.behind)}</small>
              )}
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

export function ZhuigengGapsPanel({ mode }: { mode: 'zhuigeng' | 'gaps' }) {
  const [data, setData] = useState<GapsSummaryResponse | null>(null);
  const [zhuigeng, setZhuigeng] = useState<ZhuigengStatusResponse | null>(null);
  const [airingResult, setAiringResult] = useState<ZhuigengScanAiringResponse | null>(null);
  const [zhuigengGapResult, setZhuigengGapResult] = useState<ZhuigengGapsSummaryResponse | null>(null);
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [selectedLib, setSelectedLib] = useState('');
  const [libraryError, setLibraryError] = useState('');
  const [scanTask, setScanTask] = useState<TaskRun | null>(null);
  const [scanResult, setScanResult] = useState<GapsScanLibResult | null>(null);
  const [startingScan, setStartingScan] = useState(false);
  const [zhuigengAction, setZhuigengAction] = useState<'scan-airing' | 'gaps-summary' | ''>('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const isZhuigeng = mode === 'zhuigeng';
  const title = isZhuigeng ? '追更检查' : '缺集扫描';
  const subtitle = isZhuigeng
    ? '读取 Emby/TMDb 在更状态，生成落后提示和可复制求资源文本。'
    : '按剧集库读取 Emby Series/Episodes，输出缺集和落后 TMDb 的求资源清单。';
  const notice = isZhuigeng
    ? '追更检查只汇总状态和求资源文本，不写 STRM、不调用 115 转存。'
    : '全库扫描只读 Emby 元数据，不修改媒体文件、不写 STRM、不调用 115。';

  const loadLibraries = async () => {
    if (isZhuigeng) return;
    setLibraryError('');
    try {
      const res = await api<LibrariesResponse>('/api/v2/libraries');
      const tv = res.libraries.filter((library) => library.type === 'tvshows');
      setLibraries(tv);
      setSelectedLib((current) => {
        if (current && tv.some((library) => library.name === current)) return current;
        return tv[0]?.name || '';
      });
    } catch (e) {
      const message = errorMessage(e);
      setLibraryError(message);
      toast.push(`剧集库加载失败：${message}`, 'error');
    }
  };

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      if (isZhuigeng) {
        setZhuigeng(await api<ZhuigengStatusResponse>('/api/v2/zhuigeng'));
      } else {
        setData(await api<GapsSummaryResponse>('/api/v2/gaps/scan', { method: 'POST', body: JSON.stringify({}) }));
        await loadLibraries();
      }
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

  useEffect(() => {
    if (!scanTask?.id || !isActiveTask(scanTask)) return;
    let disposed = false;
    let timer = 0;

    const poll = async () => {
      try {
        const next = await api<TaskRun>(`/api/v2/tasks/${scanTask.id}`);
        if (disposed) return;
        setScanTask(next);
        if (next.status === 'done') {
          const result = asGapsScanResult(next.result);
          if (result) setScanResult(result);
          return;
        }
        if (['error', 'cancelled', 'interrupted'].includes(next.status)) return;
        timer = window.setTimeout(poll, 1200);
      } catch (e) {
        if (!disposed) {
          toast.push(`缺集任务轮询失败：${errorMessage(e)}`, 'error');
        }
      }
    };

    timer = window.setTimeout(poll, 900);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
    };
  }, [scanTask?.id, scanTask?.status]);

  const topLibraries = useMemo(() => (data?.autostrm?.libraries || []).slice(0, 10), [data]);
  const scanPct = scanTask?.total ? Math.min(100, Math.round((scanTask.progress / scanTask.total) * 100)) : 0;
  const canStartScan = !isZhuigeng && selectedLib && !startingScan && !isActiveTask(scanTask);

  const startScan = async () => {
    if (!selectedLib) {
      toast.push('先选择剧集库', 'warn');
      return;
    }
    setStartingScan(true);
    setScanResult(null);
    try {
      const task = await api<TaskRun>('/api/v2/gaps/scan-lib', {
        method: 'POST',
        body: JSON.stringify({ lib: selectedLib })
      });
      setScanTask(task);
      toast.push(`已启动缺集扫描：${selectedLib}`, 'ok');
    } catch (e) {
      toast.push(`启动缺集扫描失败：${errorMessage(e)}`, 'error');
    } finally {
      setStartingScan(false);
    }
  };

  const copyText = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast.push(`已复制 ${label}`, 'ok');
    } catch (e) {
      toast.push(`复制失败：${errorMessage(e)}`, 'error');
    }
  };

  const runScanAiring = async () => {
    setZhuigengAction('scan-airing');
    try {
      const result = await api<ZhuigengScanAiringResponse>('/api/v2/zhuigeng/scan-airing', { method: 'POST' });
      setAiringResult(result);
      toast.push(`在更扫描完成：${result.total} 个`, 'ok');
    } catch (e) {
      toast.push(`在更扫描失败：${errorMessage(e)}`, 'error');
    } finally {
      setZhuigengAction('');
    }
  };

  const runGapsSummary = async () => {
    setZhuigengAction('gaps-summary');
    try {
      const result = await api<ZhuigengGapsSummaryResponse>('/api/v2/zhuigeng/gaps-summary', { method: 'POST' });
      setZhuigengGapResult(result);
      toast.push(`缺集汇总完成：${result.total} 条`, 'ok');
    } catch (e) {
      toast.push(`缺集汇总失败：${errorMessage(e)}`, 'error');
    } finally {
      setZhuigengAction('');
    }
  };

  if (isZhuigeng) {
    const items = zhuigeng?.items || [];
    const behind = items.reduce((total, item) => total + item.behind, 0);
    const errors = items.filter((item) => item.err).length;
    const actionRunning = Boolean(zhuigengAction);
    return (
      <section className="insightPanel">
        <div className="insightToolbar">
          <div>
            <strong>{title}</strong>
            <span>{subtitle}</span>
          </div>
          <button className="btn ghost" onClick={load} disabled={loading || actionRunning}>
            <RefreshCw size={16} />
            {loading ? '加载中' : '刷新'}
          </button>
        </div>
        <div className="notice warn scanNotice">{notice}</div>
        {error && <div className="notice warn whitespaceNotice">{error}</div>}
        <div className="statGrid">
          <StatCard icon={<ListChecks />} label="总数" value={count(zhuigeng?.total)} hint={`错误 ${count(errors)}`} />
          <StatCard icon={<Webhook />} label="continuing" value={count(zhuigeng?.continuing)} tone={zhuigeng?.continuing ? 'ok' : 'neutral'} hint="TMDb 在更" />
          <StatCard icon={<CheckCircle2 />} label="ended" value={count(zhuigeng?.ended)} hint="TMDb 已完结" />
          <StatCard icon={<AlertTriangle />} label="behind" value={count(behind)} tone={behind ? 'warn' : 'ok'} hint="落后集数" />
        </div>
        <section className="readonlyBlock">
          <div className="sectionTitleRow">
            <h2>操作</h2>
            <div className="inlineActions">
              <button className="btn compact" onClick={runScanAiring} disabled={loading || actionRunning}>
                <Play size={14} />
                {zhuigengAction === 'scan-airing' ? '扫描中' : 'scan-airing'}
              </button>
              <button className="btn ghost compact" onClick={runGapsSummary} disabled={loading || actionRunning}>
                <FileText size={14} />
                {zhuigengAction === 'gaps-summary' ? '汇总中' : 'gaps-summary'}
              </button>
            </div>
          </div>
          <p className="mutedParagraph">scan-airing 汇总在更剧状态；gaps-summary 只输出 continuing 且 behind 的求资源清单。</p>
        </section>
        <CopyTextBlock
          title="追更 copy_text"
          text={zhuigeng?.copy_text || ''}
          empty="当前没有可复制的追更求资源文本"
          onCopy={copyText}
        />
        <ZhuigengScanAiringBlock result={airingResult} onCopy={copyText} />
        <ZhuigengGapsSummaryBlock result={zhuigengGapResult} onCopy={copyText} />
        <ZhuigengItemList items={items} />
      </section>
    );
  }

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
      {!isZhuigeng && (
        <section className="readonlyBlock">
          <div className="sectionTitleRow">
            <h2>全库扫描</h2>
            <button className="btn ghost compact" onClick={loadLibraries} disabled={loading}>
              <RefreshCw size={14} />
              刷新库
            </button>
          </div>
          <div className="gapScanControls">
            <select value={selectedLib} onChange={(event) => setSelectedLib(event.target.value)} aria-label="选择剧集库">
              {libraries.length === 0 && <option value="">无剧集库</option>}
              {libraries.map((library) => (
                <option value={library.name} key={library.id || library.name}>{library.name}</option>
              ))}
            </select>
            <button className="btn" onClick={startScan} disabled={!canStartScan}>
              <Play size={16} />
              {startingScan ? '启动中' : '全库扫描'}
            </button>
          </div>
          {libraryError && <div className="notice warn whitespaceNotice">{libraryError}</div>}
          {scanTask && (
            <div className="taskInlineStatus">
              <div>
                <strong>{scanTask.label || scanTask.kind}</strong>
                <span className={`badge ${scanTask.status}`}>{scanTask.status}</span>
              </div>
              <p>{scanTask.status_text || scanTask.kind}</p>
              {isActiveTask(scanTask) && (
                <>
                  <div className="miniProgress"><i style={{ width: `${scanTask.total ? scanPct : 5}%` }} /></div>
                  <small>{scanTask.progress}/{scanTask.total || '?'} · {scanPct}%</small>
                </>
              )}
              {scanTask.error && <p className="errorText">{scanTask.error}</p>}
            </div>
          )}
        </section>
      )}
      {scanResult && <GapsScanResultBlock result={scanResult} onCopy={copyText} />}
      <div className="statGrid">
        <StatCard icon={<CheckCircle2 />} label="业务状态" value={isZhuigeng ? (data?.complete_business_port ? '完整' : '只读') : '扫描可用'} tone={isZhuigeng && !data?.complete_business_port ? 'warn' : 'ok'} hint="v2 preview" />
        <StatCard icon={<ListChecks />} label="待处理" value={count(data?.todos.length)} tone={data?.todos.length ? 'warn' : 'ok'} hint={isZhuigeng ? '只读预检' : '预检信号'} />
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
