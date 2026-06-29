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
type CleanupCandidate = components['schemas']['CleanupCandidate'];
type CleanupSuggestRequest = components['schemas']['CleanupSuggestRequest'];
type CleanupSummaryResponse = components['schemas']['CleanupSummaryResponse'];
type DedupAnalysisResponse = components['schemas']['DedupAnalysisResponse'];
type DedupAutoAllResponse = components['schemas']['DedupAutoAllResponse'];
type DedupExecuteRequest = components['schemas']['DedupExecuteRequest'];
type EmptyFolderCandidate = components['schemas']['EmptyFolderCandidate'];
type EmptyFolderCleanupRequest = components['schemas']['EmptyFolderCleanupRequest'];
type EmptyFolderCleanupTaskResult = components['schemas']['EmptyFolderCleanupTaskResult'];
type DedupGroup = components['schemas']['DedupGroup'];
type DedupReviewGroup = components['schemas']['DedupReviewGroup'];
type DedupRow = components['schemas']['DedupRow'];
type EmbyLibrary = components['schemas']['EmbyLibrary'];
type GapsScanLibResult = components['schemas']['GapsScanLibResult'];
type InsightMeta = components['schemas']['InsightMeta'];
type InsightTodo = components['schemas']['InsightTodo'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];
type ManageDeleteBatchRequest = components['schemas']['ManageDeleteBatchRequest'];
type ManageDeleteRequest = components['schemas']['ManageDeleteRequest'];
type ManageMoveBatchRequest = components['schemas']['ManageMoveBatchRequest'];
type ReplaceExecuteResponse = components['schemas']['ReplaceExecuteResponse'];
type SeriesGapsResponse = components['schemas']['SeriesGapsResponse'];
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

type DedupExecuteBatchGroup = {
  tmdb?: string | null;
  remove: DedupExecuteRequest['remove'];
};

type DedupExecuteBatchRequest = {
  groups: DedupExecuteBatchGroup[];
};

type DedupExecuteBatchItemResult = {
  tmdb?: string | null;
  ok: boolean;
  removed: number;
  err?: string | null;
};

type DedupExecuteBatchResult = {
  results: DedupExecuteBatchItemResult[];
  ok_count: number;
  total: number;
};

type Tone = 'neutral' | 'ok' | 'warn' | 'error';

const CLEANUP_DIMENSIONS = [
  { value: 'rating', label: '评分' },
  { value: 'idle', label: '闲置' },
  { value: 'size', label: '体积' },
  { value: 'meta', label: '元数据' }
];

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
  return task.status === 'done' && (task.kind.startsWith('cleanup_') || task.kind === 'manage_delete_batch_execute');
}

function isEmptyFolderCleanupResult(result: unknown): result is EmptyFolderCleanupTaskResult {
  return Boolean(result && typeof result === 'object' && Array.isArray((result as Partial<EmptyFolderCleanupTaskResult>).items));
}

function isDedupExecuteBatchResult(result: unknown): result is DedupExecuteBatchResult {
  return Boolean(result && typeof result === 'object' && Array.isArray((result as Partial<DedupExecuteBatchResult>).results));
}

function candidateKey(candidate: CleanupCandidate) {
  return `${candidate.lib}\u0000${candidate.item_id}\u0000${candidate.path || candidate.name}`;
}

function emptyFolderKey(candidate: EmptyFolderCandidate) {
  return candidate.folder;
}

function folderFromCandidate(candidate: CleanupCandidate) {
  const explicitFolder = (candidate as CleanupCandidate & { folder?: string | null }).folder?.trim();
  if (explicitFolder) return explicitFolder;
  const path = candidate.path?.trim();
  if (path) {
    const normalized = path.replace(/\\/g, '/').replace(/\/+$/, '');
    const parts = normalized.split('/').filter(Boolean);
    const filename = parts.pop();
    if (filename?.includes('.') && parts.length > 0) return parts[parts.length - 1];
    if (filename) return filename;
  }
  return candidate.name;
}

function deleteRequestFromCandidate(candidate: CleanupCandidate): ManageDeleteRequest {
  return {
    lib: candidate.lib,
    folder: folderFromCandidate(candidate),
    item_id: candidate.item_id || null,
    reason: `智能清理 score ${candidate.score}`
  };
}

function deleteRequestFromEmptyFolder(lib: string, candidate: EmptyFolderCandidate): ManageDeleteRequest {
  return {
    lib,
    folder: candidate.folder,
    item_id: null,
    reason: '115 empty-folders 扫描候选'
  };
}

function zhuigengArchiveKey(item: ZhuigengItem) {
  return `${item.lib}\u0000${item.folder || item.name}\u0000${item.id || item.tmdb || ''}`;
}

function dedupRowKey(tmdb: string | null | undefined, row: DedupRow) {
  return `${tmdb || 'unknown'}\u0000${row.lib}\u0000${row.folder}`;
}

function asGapsScanResult(result: unknown): GapsScanLibResult | null {
  if (!result || typeof result !== 'object') return null;
  const candidate = result as Partial<GapsScanLibResult>;
  if (!Array.isArray(candidate.items) || typeof candidate.lib !== 'string') return null;
  return candidate as GapsScanLibResult;
}

function asZhuigengScanAiringResult(result: unknown): ZhuigengScanAiringResponse | null {
  if (!result || typeof result !== 'object') return null;
  const candidate = result as Partial<ZhuigengScanAiringResponse>;
  if (!Array.isArray(candidate.results)) return null;
  return candidate as ZhuigengScanAiringResponse;
}

function asZhuigengGapsSummaryResult(result: unknown): ZhuigengGapsSummaryResponse | null {
  if (!result || typeof result !== 'object') return null;
  const candidate = result as Partial<ZhuigengGapsSummaryResponse>;
  if (!Array.isArray(candidate.items)) return null;
  return candidate as ZhuigengGapsSummaryResponse;
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

function EmptyFolderCleanupBlock({
  lib,
  task,
  result,
  selectedKeys,
  loading,
  onScan,
  onToggle,
  onToggleAll,
  onDeleteSelected
}: {
  lib: string;
  task?: TaskRun | null;
  result?: EmptyFolderCleanupTaskResult | null;
  selectedKeys: Set<string>;
  loading?: boolean;
  onScan?: () => void;
  onToggle?: (candidate: EmptyFolderCandidate) => void;
  onToggleAll?: () => void;
  onDeleteSelected?: () => void;
}) {
  const items = result?.items || [];
  const selectedCount = items.filter((item) => selectedKeys.has(emptyFolderKey(item))).length;
  const allSelected = items.length > 0 && selectedCount === items.length;
  const active = isActiveTask(task);
  return (
    <section className="readonlyBlock">
      <div className="sectionTitleRow">
        <h2>115 empty-folders</h2>
        <div className="inlineActions compactActions">
          <button className="btn ghost compact" onClick={onScan} disabled={!lib || loading || active}>
            <SearchX size={14} />
            {loading || active ? '扫描中' : '扫描'}
          </button>
          <button className="btn ghost compact" onClick={onToggleAll} disabled={items.length === 0}>
            <ListChecks size={14} />
            {allSelected ? '取消全选' : '全选'}
          </button>
          <button className="btn danger compact" onClick={onDeleteSelected} disabled={selectedCount === 0}>
            <Trash2 size={14} />
            删除选中 {count(selectedCount)}
          </button>
        </div>
      </div>
      <div className="miniStats">
        <span>库 <strong>{result?.lib || lib || '未选择'}</strong></span>
        <span>候选 <strong>{count(items.length)}</strong></span>
        <span>已扫 <strong>{count(result?.total_scanned)}</strong></span>
        <span>大小 KB <strong>{count(result?.total_size_kb)}</strong></span>
      </div>
      {task && <div className="notice">扫描任务：{task.label} · {task.status}</div>}
      {result?.truncated && <div className="notice warn">候选已截断，请缩小库或提高后端 limit。</div>}
      {result?.warnings?.length ? <WarningList warnings={result.warnings} /> : null}
      <div className="insightList compact cleanupCandidateList">
        {items.map((item) => {
          const key = emptyFolderKey(item);
          const checked = selectedKeys.has(key);
          return (
            <article key={`empty-folder-${key}`}>
              <input
                type="checkbox"
                aria-label={`选择 115 空文件夹：${item.folder}`}
                checked={checked}
                onChange={() => onToggle?.(item)}
              />
              <div className="cleanupCandidateBody">
                <strong>{item.folder}</strong>
                <small>{result?.lib || lib} · other_files {count(item.other_files)} · {count(item.size_kb)} KB</small>
              </div>
            </article>
          );
        })}
        {result && items.length === 0 && <div className="empty inlineEmpty">本次扫描没有 115 空文件夹候选</div>}
        {!result && <div className="empty inlineEmpty">选择媒体库后扫描 115 挂载目录，结果会从任务中心回填。</div>}
      </div>
    </section>
  );
}

function CleanupLayout({
  title,
  subtitle,
  notice,
  data,
  libraries,
  selectedLib,
  top,
  minScore,
  dimensions,
  selectedCandidateKeys,
  deleteTask,
  refreshNoRatingTask,
  loading,
  error,
  actionLoading,
  onRefresh,
  onSubmitSuggest,
  onLibChange,
  onTopChange,
  onMinScoreChange,
  onToggleDimension,
  onToggleCandidate,
  onToggleAllCandidates,
  onRequestDeleteSelected,
  onRefreshNoRating,
  variant,
  duplicates,
  emptyCleanup,
  emptyCleanupTask,
  emptyCleanupLoading,
  emptyCleanupLib,
  emptyFolderLib,
  emptyFolderTask,
  emptyFolderResult,
  emptyFolderSelectedKeys,
  emptyFolderLoading,
  onExecuteEmptyCleanup,
  onEmptyCleanupLibChange,
  onEmptyFolderLibChange,
  onScanEmptyFolders,
  onToggleEmptyFolder,
  onToggleAllEmptyFolders,
  onRequestDeleteEmptyFolders
}: {
  title: string;
  subtitle: string;
  notice: string;
  data: CleanupSummaryResponse | null;
  libraries?: EmbyLibrary[];
  selectedLib?: string;
  top?: number;
  minScore?: number;
  dimensions?: string[];
  selectedCandidateKeys?: Set<string>;
  deleteTask?: TaskRun | null;
  refreshNoRatingTask?: TaskRun | null;
  duplicates?: CatalogDuplicatesResponse | null;
  emptyCleanup?: EmptyDirCleanupResponse | null;
  emptyCleanupTask?: TaskRun | null;
  emptyCleanupLoading?: boolean;
  emptyCleanupLib?: string;
  emptyFolderLib?: string;
  emptyFolderTask?: TaskRun | null;
  emptyFolderResult?: EmptyFolderCleanupTaskResult | null;
  emptyFolderSelectedKeys?: Set<string>;
  emptyFolderLoading?: boolean;
  loading: boolean;
  error: string;
  actionLoading?: string | null;
  onRefresh: () => void;
  onSubmitSuggest?: (event: FormEvent<HTMLFormElement>) => void;
  onLibChange?: (value: string) => void;
  onTopChange?: (value: number) => void;
  onMinScoreChange?: (value: number) => void;
  onToggleDimension?: (dimension: string) => void;
  onToggleCandidate?: (candidate: CleanupCandidate) => void;
  onToggleAllCandidates?: () => void;
  onRequestDeleteSelected?: () => void;
  onRefreshNoRating?: () => void;
  onExecuteEmptyCleanup?: () => void;
  onEmptyCleanupLibChange?: (value: string) => void;
  onEmptyFolderLibChange?: (value: string) => void;
  onScanEmptyFolders?: () => void;
  onToggleEmptyFolder?: (candidate: EmptyFolderCandidate) => void;
  onToggleAllEmptyFolders?: () => void;
  onRequestDeleteEmptyFolders?: () => void;
  variant: 'cleanup' | 'dedup';
}) {
  const duplicateTotal = Number(data?.catalog?.duplicate_links || 0) + Number(data?.catalog?.duplicate_names || 0);
  const emptyCandidateCount = Number(emptyCleanup?.candidate_count ?? data?.strm?.empty_directories ?? 0);
  const emptyCleanupActive = isActiveTask(emptyCleanupTask);
  const candidates = data?.cleanup_candidates || [];
  const selectedCount = candidates.filter((candidate) => selectedCandidateKeys?.has(candidateKey(candidate))).length;
  const allCandidatesSelected = candidates.length > 0 && selectedCount === candidates.length;
  const deleteActive = isActiveTask(deleteTask);
  const refreshNoRatingActive = isActiveTask(refreshNoRatingTask);
  const emptyFolderActive = isActiveTask(emptyFolderTask);
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
        <button className="btn ghost" onClick={() => onRefresh()} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>
      <div className="notice warn scanNotice">{notice}</div>
      {error && <div className="notice warn whitespaceNotice">{error}</div>}
      {variant === 'cleanup' && onSubmitSuggest && (
        <form className="cleanupControls" onSubmit={onSubmitSuggest}>
          <label>
            媒体库
            <select className="input" aria-label="智能清理媒体库" value={selectedLib || ''} onChange={(event) => onLibChange?.(event.target.value)}>
              <option value="">全部媒体库</option>
              {(libraries || []).map((library) => (
                <option key={`${library.id || library.name}-${library.type}`} value={library.name}>{library.name}</option>
              ))}
            </select>
          </label>
          <label>
            top
            <input className="input compactInput" aria-label="智能清理 top" type="number" min={1} max={500} value={top ?? 50} onChange={(event) => onTopChange?.(Number(event.target.value))} />
          </label>
          <label>
            min_score
            <input className="input compactInput" aria-label="智能清理 min_score" type="number" min={0} max={100} step={0.5} value={minScore ?? 10} onChange={(event) => onMinScoreChange?.(Number(event.target.value))} />
          </label>
          <div className="cleanupDimensionGroup" role="group" aria-label="智能清理 dimensions">
            {CLEANUP_DIMENSIONS.map((dimension) => (
              <label className="switchRow" key={dimension.value}>
                <input
                  type="checkbox"
                  checked={(dimensions || []).includes(dimension.value)}
                  onChange={() => onToggleDimension?.(dimension.value)}
                />
                {dimension.label}
              </label>
            ))}
          </div>
          <label>
            空目录 lib
            <select className="input" aria-label="空目录清理 lib" value={emptyCleanupLib || ''} onChange={(event) => onEmptyCleanupLibChange?.(event.target.value)}>
              <option value="">全部 / 后端默认</option>
              {(libraries || []).map((library) => (
                <option key={`empty-${library.id || library.name}-${library.type}`} value={library.name}>{library.name}</option>
              ))}
            </select>
          </label>
          <label>
            115 空文件夹 lib
            <select className="input" aria-label="115 empty-folders lib" value={emptyFolderLib || ''} onChange={(event) => onEmptyFolderLibChange?.(event.target.value)}>
              <option value="">请选择库</option>
              {(libraries || []).map((library) => (
                <option key={`empty-folder-${library.id || library.name}-${library.type}`} value={library.name}>{library.name}</option>
              ))}
            </select>
          </label>
          <button className="btn" disabled={loading || actionLoading === 'suggest'}>
            <SearchX size={16} />
            {loading || actionLoading === 'suggest' ? '分析中' : '生成建议'}
          </button>
          <button className="btn ghost" type="button" onClick={onRefreshNoRating} disabled={Boolean(actionLoading) || refreshNoRatingActive}>
            <RefreshCw size={16} />
            {actionLoading === 'refresh-no-rating' ? '提交中' : '刷新无评分'}
          </button>
          <button className="btn ghost" type="button" onClick={onScanEmptyFolders} disabled={!emptyFolderLib || Boolean(actionLoading) || emptyFolderActive}>
            <SearchX size={16} />
            {actionLoading === 'empty-folders' || emptyFolderActive ? '扫描中' : '扫描 115 空文件夹'}
          </button>
        </form>
      )}
      <div className="statGrid">
        <StatCard icon={<CheckCircle2 />} label="业务状态" value={data?.complete_business_port ? '完整' : '只读'} tone={data?.complete_business_port ? 'ok' : 'warn'} hint="Rust preview" />
        <StatCard icon={<ListChecks />} label={variant === 'dedup' ? '重复信号' : '清理候选'} value={count(variant === 'dedup' ? duplicateTotal : candidates.length)} tone={(variant === 'dedup' ? duplicateTotal : candidates.length) ? 'warn' : 'ok'} hint={variant === 'dedup' ? '不执行写操作' : `${count(selectedCount)} 已选`} />
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
          <div className="sectionTitleRow">
            <h2>清理候选</h2>
            <div className="inlineActions compactActions">
              <button className="btn ghost compact" onClick={onToggleAllCandidates} disabled={candidates.length === 0}>
                <ListChecks size={14} />
                {allCandidatesSelected ? '取消全选' : '全选候选'}
              </button>
              <button className="btn danger compact" onClick={onRequestDeleteSelected} disabled={selectedCount === 0 || Boolean(actionLoading) || deleteActive}>
                <Trash2 size={14} />
                删除选中 {count(selectedCount)}
              </button>
            </div>
          </div>
          <div className="insightList compact cleanupCandidateList">
            {candidates.map((candidate) => {
              const key = candidateKey(candidate);
              const checked = Boolean(selectedCandidateKeys?.has(key));
              return (
                <article key={key}>
                  <input
                    type="checkbox"
                    aria-label={`选择清理候选：${candidate.name}`}
                    checked={checked}
                    onChange={() => onToggleCandidate?.(candidate)}
                  />
                  <div className="cleanupCandidateBody">
                    <strong>{candidate.name}</strong>
                    <small>{candidate.lib} · score {candidate.score} · {folderFromCandidate(candidate)}</small>
                    {candidate.reasons.length > 0 && <small>{candidate.reasons.join('；')}</small>}
                    <div className="cleanupDimensions">
                      {Object.entries(candidate.dimensions || {}).map(([name, score]) => (
                        <span key={`${key}-${name}`} className={score.warning ? 'warn' : ''}>
                          {name}: {score.score}{score.value ? ` · ${score.value}` : ''}{score.warning ? ` · ${score.warning}` : ''}
                        </span>
                      ))}
                    </div>
                  </div>
                </article>
              );
            })}
            {data && candidates.length === 0 && <div className="empty inlineEmpty">当前条件没有清理候选</div>}
            {!data && <div className="empty inlineEmpty">等待清理建议</div>}
          </div>
          {deleteTask && <div className="notice">已创建任务：{deleteTask.label} · {deleteTask.status}</div>}
          {refreshNoRatingTask && <div className="notice">无评分刷新任务：{refreshNoRatingTask.label} · {refreshNoRatingTask.status}</div>}
          {data?.todos?.length ? <TodoList items={data.todos} empty="" /> : null}
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
      {variant === 'cleanup' && (
        <EmptyFolderCleanupBlock
          lib={emptyFolderLib || ''}
          task={emptyFolderTask}
          result={emptyFolderResult}
          selectedKeys={emptyFolderSelectedKeys || new Set()}
          loading={emptyFolderLoading}
          onScan={onScanEmptyFolders}
          onToggle={onToggleEmptyFolder}
          onToggleAll={onToggleAllEmptyFolders}
          onDeleteSelected={onRequestDeleteEmptyFolders}
        />
      )}
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
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [selectedLib, setSelectedLib] = useState('');
  const [emptyCleanupLib, setEmptyCleanupLib] = useState('');
  const [emptyFolderLib, setEmptyFolderLib] = useState('');
  const [top, setTop] = useState(50);
  const [minScore, setMinScore] = useState(10);
  const [dimensions, setDimensions] = useState<string[]>(CLEANUP_DIMENSIONS.map((dimension) => dimension.value));
  const [selectedCandidateKeys, setSelectedCandidateKeys] = useState<Set<string>>(new Set());
  const [selectedEmptyFolderKeys, setSelectedEmptyFolderKeys] = useState<Set<string>>(new Set());
  const [pendingDelete, setPendingDelete] = useState<ManageDeleteBatchRequest | null>(null);
  const [pendingDeleteKind, setPendingDeleteKind] = useState<'cleanup' | 'empty-folders'>('cleanup');
  const [deleteTask, setDeleteTask] = useState<TaskRun | null>(null);
  const [refreshNoRatingTask, setRefreshNoRatingTask] = useState<TaskRun | null>(null);
  const [emptyCleanup, setEmptyCleanup] = useState<EmptyDirCleanupResponse | null>(null);
  const [emptyCleanupTask, setEmptyCleanupTask] = useState<TaskRun | null>(null);
  const [emptyFolderTask, setEmptyFolderTask] = useState<TaskRun | null>(null);
  const [emptyFolderResult, setEmptyFolderResult] = useState<EmptyFolderCleanupTaskResult | null>(null);
  const [emptyCleanupLoading, setEmptyCleanupLoading] = useState(false);
  const [confirmEmptyCleanup, setConfirmEmptyCleanup] = useState(false);
  const [confirmDeleteSelected, setConfirmDeleteSelected] = useState(false);
  const [loading, setLoading] = useState(true);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [error, setError] = useState('');
  const toast = useToast();

  const emptyCleanupPayload = useCallback((execute: boolean) => ({
    execute,
    lib: emptyCleanupLib || null
  }), [emptyCleanupLib]);

  const suggestPayload = useCallback((): CleanupSuggestRequest => ({
    lib: selectedLib || null,
    top,
    min_score: minScore,
    dimensions
  }), [dimensions, minScore, selectedLib, top]);

  const load = useCallback(async (payload?: CleanupSuggestRequest) => {
    setLoading(true);
    setError('');
    try {
      const [libs, summary, emptyPreview] = await Promise.all([
        api<LibrariesResponse>('/api/v2/libraries'),
        api<CleanupSummaryResponse>('/api/v2/cleanup/suggest', { method: 'POST', body: JSON.stringify(payload || suggestPayload()) }),
        api<EmptyDirCleanupResponse>('/api/v2/cleanup/empty-dirs', { method: 'POST', body: JSON.stringify(emptyCleanupPayload(false)) })
      ]);
      const nextLibraries = libs.libraries || [];
      setLibraries(nextLibraries);
      if (!selectedLib && nextLibraries[0]?.name) {
        setSelectedLib(nextLibraries[0].name);
      }
      if (!emptyFolderLib && nextLibraries[0]?.name) {
        setEmptyFolderLib(nextLibraries[0].name);
      }
      setData(summary);
      setEmptyCleanup(emptyPreview);
      setSelectedCandidateKeys((previous) => {
        const available = new Set((summary.cleanup_candidates || []).map(candidateKey));
        const next = new Set([...previous].filter((key) => available.has(key)));
        return next;
      });
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`智能清理预检失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [emptyCleanupPayload, emptyFolderLib, selectedLib, suggestPayload, toast]);

  useEffect(() => {
    load();
  }, []);

  useEffect(() => {
    const onTaskCompleted = (event: Event) => {
      const detail = (event as CustomEvent<TaskCompleteDetail>).detail;
      if (detail?.task && shouldRefreshCleanup(detail.task)) {
        if (detail.task.kind === 'manage_delete_batch_execute') {
          setDeleteTask(detail.task);
        } else if (detail.task.kind === 'cleanup_empty_folders') {
          setEmptyFolderTask(detail.task);
          const result = detail.task.result;
          if (isEmptyFolderCleanupResult(result)) {
            setEmptyFolderResult(result);
            setSelectedEmptyFolderKeys((previous) => {
              const available = new Set(result.items.map(emptyFolderKey));
              return new Set([...previous].filter((key) => available.has(key)));
            });
          }
        } else {
          setEmptyCleanupTask(detail.task);
        }
        load();
      }
    };
    window.addEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
    return () => window.removeEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
  }, [load]);

  const submitSuggest = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setActionLoading('suggest');
    try {
      await load(suggestPayload());
    } finally {
      setActionLoading(null);
    }
  };

  const toggleDimension = (dimension: string) => {
    setDimensions((current) => {
      if (current.includes(dimension)) {
        const next = current.filter((item) => item !== dimension);
        return next.length ? next : current;
      }
      return [...current, dimension];
    });
  };

  const toggleCandidate = (candidate: CleanupCandidate) => {
    const key = candidateKey(candidate);
    setSelectedCandidateKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const toggleAllCandidates = () => {
    const candidates = data?.cleanup_candidates || [];
    const allKeys = candidates.map(candidateKey);
    setSelectedCandidateKeys((current) => {
      if (allKeys.length > 0 && allKeys.every((key) => current.has(key))) return new Set();
      return new Set(allKeys);
    });
  };

  const requestDeleteSelected = () => {
    const selected = (data?.cleanup_candidates || []).filter((candidate) => selectedCandidateKeys.has(candidateKey(candidate)));
    if (selected.length === 0) {
      toast.push('先选择要删除的清理候选', 'warn');
      return;
    }
    const payload: ManageDeleteBatchRequest = {
      items: selected.map(deleteRequestFromCandidate),
      reason: `智能清理 min_score ${minScore}`
    };
    setPendingDelete(payload);
    setPendingDeleteKind('cleanup');
    setConfirmDeleteSelected(true);
  };

  const scanEmptyFolders = async () => {
    const lib = emptyFolderLib.trim();
    if (!lib) {
      toast.push('先选择要扫描的 115 媒体库', 'warn');
      return;
    }
    const payload: EmptyFolderCleanupRequest = { lib };
    setActionLoading('empty-folders');
    setError('');
    try {
      const task = await api<TaskRun>('/api/v2/cleanup/empty-folders', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setEmptyFolderTask(task);
      setEmptyFolderResult(null);
      setSelectedEmptyFolderKeys(new Set());
      toast.push(`已创建 115 空文件夹扫描任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`115 空文件夹扫描失败：${message}`, 'error');
    } finally {
      setActionLoading(null);
    }
  };

  const toggleEmptyFolder = (candidate: EmptyFolderCandidate) => {
    const key = emptyFolderKey(candidate);
    setSelectedEmptyFolderKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const toggleAllEmptyFolders = () => {
    const items = emptyFolderResult?.items || [];
    const allKeys = items.map(emptyFolderKey);
    setSelectedEmptyFolderKeys((current) => {
      if (allKeys.length > 0 && allKeys.every((key) => current.has(key))) return new Set();
      return new Set(allKeys);
    });
  };

  const requestDeleteEmptyFolders = () => {
    const result = emptyFolderResult;
    if (!result) {
      toast.push('先完成一次 115 空文件夹扫描', 'warn');
      return;
    }
    const selected = result.items.filter((candidate) => selectedEmptyFolderKeys.has(emptyFolderKey(candidate)));
    if (selected.length === 0) {
      toast.push('先选择要删除的 115 空文件夹候选', 'warn');
      return;
    }
    const payload: ManageDeleteBatchRequest = {
      items: selected.map((candidate) => deleteRequestFromEmptyFolder(result.lib || emptyFolderLib, candidate)),
      reason: `115 empty-folders 扫描 ${result.lib || emptyFolderLib}`
    };
    setPendingDelete(payload);
    setPendingDeleteKind('empty-folders');
    setConfirmDeleteSelected(true);
  };

  const executeDeleteSelected = async () => {
    if (!pendingDelete) return;
    setConfirmDeleteSelected(false);
    setActionLoading('delete-selected');
    setError('');
    try {
      const task = await api<TaskRun>('/api/v2/manage/delete/batch/execute', {
        method: 'POST',
        body: JSON.stringify(pendingDelete)
      });
      setDeleteTask(task);
      toast.push(`已创建${pendingDeleteKind === 'empty-folders' ? '115 空文件夹' : '智能清理'}删除任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`智能清理删除失败：${message}`, 'error');
    } finally {
      setActionLoading(null);
    }
  };

  const changeEmptyFolderLib = (value: string) => {
    setEmptyFolderLib(value);
    setEmptyFolderResult(null);
    setSelectedEmptyFolderKeys(new Set());
  };

  const refreshNoRating = async () => {
    if (!selectedLib) {
      toast.push('先选择要刷新的库', 'warn');
      return;
    }
    setActionLoading('refresh-no-rating');
    setError('');
    try {
      const task = await api<TaskRun>('/api/v2/cleanup/refresh-no-rating', {
        method: 'POST',
        body: JSON.stringify({ lib: selectedLib })
      });
      setRefreshNoRatingTask(task);
      toast.push(`已创建无评分刷新任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`刷新无评分失败：${message}`, 'error');
    } finally {
      setActionLoading(null);
    }
  };

  const executeEmptyCleanup = async () => {
    setEmptyCleanupLoading(true);
    setConfirmEmptyCleanup(false);
    try {
      const result = await api<EmptyDirCleanupResponse>('/api/v2/cleanup/empty-dirs', {
        method: 'POST',
        body: JSON.stringify(emptyCleanupPayload(true))
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
      {confirmDeleteSelected && pendingDelete && (
        <ConfirmDanger
          title={pendingDeleteKind === 'empty-folders' ? '确认删除 115 空文件夹候选' : '确认删除智能清理候选'}
          confirmText="确认删除选中"
          onCancel={() => setConfirmDeleteSelected(false)}
          onConfirm={executeDeleteSelected}
          body={(
            <div className="dangerCopy">
              <p>{pendingDeleteKind === 'empty-folders' ? '将把选中的 115 空文件夹候选交给批量真实删除任务逐项处理。' : '将创建批量真实删除任务，按 Emby ItemId 和目录逐项处理。'}</p>
              <code>{pendingDelete.items.map((item) => `${item.lib}/${item.folder}${item.item_id ? `/${item.item_id}` : ''}`).join('\n')}</code>
            </div>
          )}
        />
      )}
      <CleanupLayout
        title="智能清理预检"
        subtitle="按评分、闲置、体积和元数据维度生成可执行候选。"
        notice="评分删除已接入批量任务；size / idle 仍受挂载状态、播放记录和媒体元数据完整度影响，请先复核候选。"
        data={data}
        libraries={libraries}
        selectedLib={selectedLib}
        top={top}
        minScore={minScore}
        dimensions={dimensions}
        selectedCandidateKeys={selectedCandidateKeys}
        deleteTask={deleteTask}
        refreshNoRatingTask={refreshNoRatingTask}
        emptyCleanup={emptyCleanup}
        emptyCleanupTask={emptyCleanupTask}
        emptyCleanupLoading={emptyCleanupLoading}
        emptyCleanupLib={emptyCleanupLib}
        emptyFolderLib={emptyFolderLib}
        emptyFolderTask={emptyFolderTask}
        emptyFolderResult={emptyFolderResult}
        emptyFolderSelectedKeys={selectedEmptyFolderKeys}
        emptyFolderLoading={actionLoading === 'empty-folders'}
        loading={loading}
        error={error}
        actionLoading={actionLoading}
        onRefresh={load}
        onSubmitSuggest={submitSuggest}
        onLibChange={setSelectedLib}
        onTopChange={setTop}
        onMinScoreChange={setMinScore}
        onToggleDimension={toggleDimension}
        onToggleCandidate={toggleCandidate}
        onToggleAllCandidates={toggleAllCandidates}
        onRequestDeleteSelected={requestDeleteSelected}
        onRefreshNoRating={refreshNoRating}
        onExecuteEmptyCleanup={() => setConfirmEmptyCleanup(true)}
        onEmptyCleanupLibChange={setEmptyCleanupLib}
        onEmptyFolderLibChange={changeEmptyFolderLib}
        onScanEmptyFolders={scanEmptyFolders}
        onToggleEmptyFolder={toggleEmptyFolder}
        onToggleAllEmptyFolders={toggleAllEmptyFolders}
        onRequestDeleteEmptyFolders={requestDeleteEmptyFolders}
        variant="cleanup"
      />
    </>
  );
}

function DedupAutoGroups({
  groups,
  selectedKeys,
  onToggle
}: {
  groups: DedupGroup[];
  selectedKeys: Set<string>;
  onToggle: (tmdb: string, row: DedupRow) => void;
}) {
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
            <div className="dedupRemoveList">
              {group.remove.map((row) => {
                const key = dedupRowKey(group.tmdb, row);
                return (
                  <label className="switchRow dedupPickRow" key={key}>
                    <input
                      type="checkbox"
                      aria-label={`选择去重删除：${row.lib}/${row.folder}`}
                      checked={selectedKeys.has(key)}
                      onChange={() => onToggle(group.tmdb, row)}
                    />
                    <span>{row.lib}/{row.folder} · score {count(row.score)} · n {count(row.n)}</span>
                  </label>
                );
              })}
            </div>
          </article>
        ))}
        {groups.length === 0 && <div className="empty inlineEmpty">没有可自动处理的重复组</div>}
      </div>
    </section>
  );
}

function DedupReviewGroups({
  groups,
  selectedKeys,
  onToggle
}: {
  groups: DedupReviewGroup[];
  selectedKeys: Set<string>;
  onToggle: (tmdb: string, row: DedupRow) => void;
}) {
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
            <div className="dedupRemoveList">
              {group.rows.map((row) => {
                const key = dedupRowKey(group.tmdb, row);
                return (
                  <label className="switchRow dedupPickRow" key={key}>
                    <input
                      type="checkbox"
                      aria-label={`选择去重删除：${row.lib}/${row.folder}`}
                      checked={selectedKeys.has(key)}
                      onChange={() => onToggle(group.tmdb, row)}
                    />
                    <span>{row.lib}/{row.folder} · score {count(row.score)} · n {count(row.n)}</span>
                  </label>
                );
              })}
            </div>
          </article>
        ))}
        {groups.length === 0 && <div className="empty inlineEmpty">没有需要人工复核的重复组</div>}
      </div>
    </section>
  );
}

function DedupExecuteBatchTaskBlock({ task }: { task: TaskRun | null }) {
  if (!task) return null;
  const result = isDedupExecuteBatchResult(task.result) ? task.result : null;
  return (
    <section className="readonlyBlock">
      <h2>批量去重任务</h2>
      <div className="miniStats">
        <span>状态 <strong>{task.status}</strong></span>
        <span>进度 <strong>{count(task.progress)} / {count(task.total)}</strong></span>
        <span>成功组 <strong>{count(result?.ok_count)}</strong></span>
        <span>总组 <strong>{count(result?.total ?? task.total)}</strong></span>
      </div>
      {task.status_text && <p className="mutedParagraph">{task.status_text}</p>}
      <div className="insightList compact">
        {(result?.results || []).map((item, index) => (
          <article key={`dedup-batch-${item.tmdb || 'unknown'}-${index}`}>
            <span className={`badge ${item.ok ? 'ok' : 'error'}`}>{item.ok ? 'ok' : 'error'}</span>
            <strong>tmdb:{item.tmdb || 'unknown'}</strong>
            <small>removed {count(item.removed)}{item.err ? ` · ${item.err}` : ''}</small>
          </article>
        ))}
        {!result && <div className="empty inlineEmpty">等待任务完成后展示每个 tmdb 分组结果</div>}
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
  const [executeBatchTask, setExecuteBatchTask] = useState<TaskRun | null>(null);
  const [replaceResult, setReplaceResult] = useState<ReplaceExecuteResponse | null>(null);
  const [replaceDraft, setReplaceDraft] = useState({ lib: '', win_folder: '', lose_folder: '', reason: '' });
  const [selectedDedupKeys, setSelectedDedupKeys] = useState<Set<string>>(new Set());
  const [pendingExecuteBatch, setPendingExecuteBatch] = useState<DedupExecuteBatchRequest | null>(null);
  const [confirmAction, setConfirmAction] = useState<'auto-all' | 'replace' | 'execute' | null>(null);
  const [acting, setActing] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const toast = useToast();

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const next = await api<DedupAnalysisResponse>('/api/v2/dedup/duplicates');
      setData(next);
      setSelectedDedupKeys((previous) => {
        const available = new Set([
          ...next.dups.flatMap((group) => group.remove.map((row) => dedupRowKey(group.tmdb, row))),
          ...next.review.flatMap((group) => group.rows.map((row) => dedupRowKey(group.tmdb, row)))
        ]);
        return new Set([...previous].filter((key) => available.has(key)));
      });
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

  useEffect(() => {
    const onTaskCompleted = (event: Event) => {
      const detail = (event as CustomEvent<TaskCompleteDetail>).detail;
      if (detail?.task?.kind !== 'dedup_exec_batch') return;
      setExecuteBatchTask(detail.task);
      if (detail.task.status === 'done') {
        setSelectedDedupKeys(new Set());
        load();
      }
    };
    window.addEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
    return () => window.removeEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
  }, [load]);

  const autoGroups = data?.dups || [];
  const reviewGroups = data?.review || [];
  const manualRows = useMemo(() => [
    ...autoGroups.flatMap((group) => group.remove.map((row) => ({ tmdb: group.tmdb, row }))),
    ...reviewGroups.flatMap((group) => group.rows.map((row) => ({ tmdb: group.tmdb, row })))
  ], [autoGroups, reviewGroups]);
  const selectedManualRows = manualRows.filter((item) => selectedDedupKeys.has(dedupRowKey(item.tmdb, item.row)));
  const removeTotal = autoGroups.reduce((total, group) => total + group.remove.length, 0);
  const reviewRows = reviewGroups.reduce((total, group) => total + group.rows.length, 0);
  const executeBatchResult = isDedupExecuteBatchResult(executeBatchTask?.result) ? executeBatchTask.result : null;
  const manualDedupActive = isActiveTask(executeBatchTask);
  const replaceReady = Boolean(
    replaceDraft.lib.trim() &&
    replaceDraft.win_folder.trim() &&
    replaceDraft.lose_folder.trim()
  );

  const patchReplaceDraft = (patch: Partial<typeof replaceDraft>) => {
    setReplaceDraft((current) => ({ ...current, ...patch }));
  };

  const toggleDedupRow = (tmdb: string, row: DedupRow) => {
    const key = dedupRowKey(tmdb, row);
    setSelectedDedupKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const clearDedupSelection = () => {
    setSelectedDedupKeys(new Set());
  };

  const requestExecuteSelected = () => {
    if (selectedManualRows.length === 0) {
      toast.push('先选择要人工删除的重复目录', 'warn');
      return;
    }
    const groupsByTmdb = new Map<string, DedupExecuteBatchGroup>();
    const seen = new Set<string>();
    for (const { tmdb, row } of selectedManualRows) {
      const groupKey = tmdb || '';
      const rowKey = `${groupKey}\u0000${row.lib}\u0000${row.folder}`;
      if (seen.has(rowKey)) continue;
      seen.add(rowKey);
      const group = groupsByTmdb.get(groupKey) || {
        tmdb: tmdb || null,
        remove: []
      };
      group.remove.push({ lib: row.lib, folder: row.folder });
      groupsByTmdb.set(groupKey, group);
    }
    const groups = [...groupsByTmdb.values()].filter((group) => group.remove.length > 0);
    if (groups.length === 0) {
      toast.push('没有可提交的去重分组', 'warn');
      return;
    }
    setPendingExecuteBatch({ groups });
    setConfirmAction('execute');
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

  const executeManualDedup = async () => {
    if (!pendingExecuteBatch) return;
    setActing(true);
    setConfirmAction(null);
    try {
      const task = await api<TaskRun>('/api/v2/dedup/execute-batch', {
        method: 'POST',
        body: JSON.stringify(pendingExecuteBatch)
      });
      setExecuteBatchTask(task);
      toast.push(`已创建批量去重任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      toast.push(`创建批量去重任务失败：${errorMessage(e)}`, 'error');
    } finally {
      setActing(false);
      setPendingExecuteBatch(null);
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
      {confirmAction === 'execute' && pendingExecuteBatch && (
        <ConfirmDanger
          title="确认人工去重删除"
          confirmText="确认删除选中重复目录"
          onCancel={() => setConfirmAction(null)}
          onConfirm={executeManualDedup}
          body={(
            <div className="dangerCopy">
              <p>将按 tmdb 分组创建批量去重任务，并写入 undo 与 Emby 更新。</p>
              <code>{pendingExecuteBatch.groups.map((group) => {
                const folders = group.remove.map((item) => `${item.lib}/${item.folder}`).join(', ');
                return `tmdb:${group.tmdb || 'unknown'} -> ${folders}`;
              }).join('\n')}</code>
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
          <StatCard icon={<Trash2 />} label="批量去重" value={executeBatchTask?.status || '待命'} tone={executeBatchTask?.status === 'done' ? 'ok' : manualDedupActive ? 'warn' : 'neutral'} hint={executeBatchResult ? `成功 ${count(executeBatchResult.ok_count)} / ${count(executeBatchResult.total)}` : `已选 ${count(selectedManualRows.length)}`} />
          <StatCard icon={<CheckCircle2 />} label="Replace" value={replaceResult?.ok ? '完成' : '待命'} tone={replaceResult?.ok ? 'ok' : 'neutral'} hint={replaceResult?.kept_as || '手动 lib/win/lose'} />
        </div>
        <section className="readonlyBlock">
          <div className="sectionTitleRow">
            <h2>执行入口</h2>
            <div className="inlineActions compactActions">
              <button className="btn ghost compact" onClick={clearDedupSelection} disabled={selectedManualRows.length === 0 || acting}>
                <ListChecks size={14} />
                清空选择
              </button>
              <button className="btn danger compact" onClick={requestExecuteSelected} disabled={loading || acting || manualDedupActive || selectedManualRows.length === 0}>
                <Trash2 size={14} />
                {manualDedupActive ? '任务中' : `人工删除 ${count(selectedManualRows.length)}`}
              </button>
              <button className="btn danger compact" onClick={() => setConfirmAction('auto-all')} disabled={loading || acting || autoGroups.length === 0}>
                <Trash2 size={14} />
                {acting ? '执行中' : 'auto-all'}
              </button>
            </div>
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
        <DedupExecuteBatchTaskBlock task={executeBatchTask} />
        <DedupAutoAllResultBlock result={autoAllResult} />
        <ReplaceResultBlock result={replaceResult} />
        <div className="readonlySplit">
          <DedupAutoGroups groups={autoGroups} selectedKeys={selectedDedupKeys} onToggle={toggleDedupRow} />
          <DedupReviewGroups groups={reviewGroups} selectedKeys={selectedDedupKeys} onToggle={toggleDedupRow} />
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

function ZhuigengItemList({
  items,
  selectedArchiveKeys,
  onToggleArchive
}: {
  items: ZhuigengItem[];
  selectedArchiveKeys?: Set<string>;
  onToggleArchive?: (item: ZhuigengItem) => void;
}) {
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
          const archiveable = Boolean(item.ended && item.folder && item.id);
          const archiveKey = zhuigengArchiveKey(item);
          return (
            <article key={`${item.lib}-${item.id || item.folder}-${item.tmdb}`} className={tone === 'error' ? 'error' : ''}>
              <div>
                {archiveable && (
                  <input
                    type="checkbox"
                    aria-label={`选择归档：${item.name}`}
                    checked={Boolean(selectedArchiveKeys?.has(archiveKey))}
                    onChange={() => onToggleArchive?.(item)}
                  />
                )}
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

function SeriesGapsDetailBlock({ detail }: { detail: SeriesGapsResponse }) {
  const seasonRows = detail.seasons || [];
  return (
    <div className="taskInlineStatus">
      <div>
        <strong>{detail.id || '单剧缺集'}</strong>
        <span className="badge">{detail.mode}</span>
      </div>
      <div className="miniStats">
        <span>已有 <strong>{count(detail.have)}</strong></span>
        <span>缺口 <strong>{count(detail.gaps)}</strong></span>
        <span>本地最大 <strong>{count(detail.max_ep)}</strong></span>
        <span>TMDb 最大 <strong>{count(detail.tmdb_max)}</strong></span>
      </div>
      {detail.mode === 'absolute' ? (
        <p>{detail.gap_list.length ? `E${detail.gap_list.join(',')}` : '没有缺集'}</p>
      ) : (
        <div className="gapResultList">
          {seasonRows.map((season) => (
            <article key={`${season.season ?? 'none'}-${season.lo}-${season.hi}`}>
              <div>
                <strong>S{String(season.season ?? 0).padStart(2, '0')}</strong>
                <span className="badge">已有 {count(season.count)}</span>
                <span className={season.gapcount ? 'badge warn' : 'badge done'}>缺 {count(season.gapcount)}</span>
              </div>
              <p>{season.gaps.length ? `E${season.gaps.join(',')}` : '齐全'}</p>
              <small>E{season.lo} - E{season.hi}</small>
            </article>
          ))}
          {seasonRows.length === 0 && <div className="empty inlineEmpty">没有季集详情</div>}
        </div>
      )}
      {detail.noidx > 0 && <small>{count(detail.noidx)} 集缺少 IndexNumber，未参与缺集判定</small>}
    </div>
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
  const [zhuigeng, setZhuigeng] = useState<ZhuigengStatusResponse | null>(null);
  const [airingResult, setAiringResult] = useState<ZhuigengScanAiringResponse | null>(null);
  const [zhuigengGapResult, setZhuigengGapResult] = useState<ZhuigengGapsSummaryResponse | null>(null);
  const [airingTask, setAiringTask] = useState<TaskRun | null>(null);
  const [zhuigengGapTask, setZhuigengGapTask] = useState<TaskRun | null>(null);
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [selectedLib, setSelectedLib] = useState('');
  const [libraryError, setLibraryError] = useState('');
  const [scanTask, setScanTask] = useState<TaskRun | null>(null);
  const [scanResult, setScanResult] = useState<GapsScanLibResult | null>(null);
  const [seriesId, setSeriesId] = useState('');
  const [seriesDetail, setSeriesDetail] = useState<SeriesGapsResponse | null>(null);
  const [seriesDetailLoading, setSeriesDetailLoading] = useState(false);
  const [archiveTargetLib, setArchiveTargetLib] = useState('');
  const [selectedArchiveKeys, setSelectedArchiveKeys] = useState<Set<string>>(new Set());
  const [archiveTask, setArchiveTask] = useState<TaskRun | null>(null);
  const [confirmArchive, setConfirmArchive] = useState(false);
  const [startingScan, setStartingScan] = useState(false);
  const [zhuigengAction, setZhuigengAction] = useState<'scan-airing' | 'gaps-summary' | 'archive' | ''>('');
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
    setLibraryError('');
    try {
      const res = await api<LibrariesResponse>('/api/v2/libraries');
      const tv = res.libraries.filter((library) => library.type === 'tvshows');
      setLibraries(tv);
      if (isZhuigeng) {
        setArchiveTargetLib((current) => {
          if (current && tv.some((library) => library.name === current)) return current;
          return tv.find((library) => /完结/.test(library.name))?.name || tv[0]?.name || '';
        });
      } else {
        setSelectedLib((current) => {
          if (current && tv.some((library) => library.name === current)) return current;
          return tv[0]?.name || '';
        });
      }
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
        const [status] = await Promise.all([
          api<ZhuigengStatusResponse>('/api/v2/zhuigeng'),
          loadLibraries()
        ]);
        setZhuigeng(status);
        setSelectedArchiveKeys((previous) => {
          const available = new Set((status.items || []).filter((item) => item.ended && item.folder && item.id).map(zhuigengArchiveKey));
          return new Set([...previous].filter((key) => available.has(key)));
        });
      } else {
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

  useEffect(() => {
    const onTaskCompleted = (event: Event) => {
      const detail = (event as CustomEvent<TaskCompleteDetail>).detail;
      const task = detail?.task;
      if (!task) return;
      if (task.kind === 'zhuigeng_scan_airing') {
        setAiringTask(task);
        const result = asZhuigengScanAiringResult(task.result);
        if (result) {
          setAiringResult(result);
          toast.push(`在更扫描完成：${result.total} 个`, 'ok');
        }
        load();
      } else if (task.kind === 'zhuigeng_gaps_summary') {
        setZhuigengGapTask(task);
        const result = asZhuigengGapsSummaryResult(task.result);
        if (result) {
          setZhuigengGapResult(result);
          toast.push(`缺集汇总完成：${result.total} 条`, 'ok');
        }
      }
    };
    window.addEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
    return () => window.removeEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
  }, [load, toast]);

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

  const loadSeriesDetail = async (event?: FormEvent<HTMLFormElement>) => {
    event?.preventDefault();
    const id = seriesId.trim();
    if (!id) {
      toast.push('先填写 Emby Series Id', 'warn');
      return;
    }
    setSeriesDetailLoading(true);
    setSeriesDetail(null);
    try {
      const params = new URLSearchParams({ id });
      const detail = await api<SeriesGapsResponse>(`/api/v2/gaps/series?${params.toString()}`);
      setSeriesDetail(detail);
      toast.push(`已读取单剧缺集：${id}`, 'ok');
    } catch (e) {
      toast.push(`单剧缺集查询失败：${errorMessage(e)}`, 'error');
    } finally {
      setSeriesDetailLoading(false);
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
      const task = await api<TaskRun>('/api/v2/zhuigeng/scan-airing', { method: 'POST' });
      setAiringTask(task);
      setAiringResult(null);
      toast.push(`已启动在更扫描：${task.label || task.kind}`, 'ok');
    } catch (e) {
      toast.push(`启动在更扫描失败：${errorMessage(e)}`, 'error');
    } finally {
      setZhuigengAction('');
    }
  };

  const runGapsSummary = async () => {
    setZhuigengAction('gaps-summary');
    try {
      const task = await api<TaskRun>('/api/v2/zhuigeng/gaps-summary', { method: 'POST' });
      setZhuigengGapTask(task);
      setZhuigengGapResult(null);
      toast.push(`已启动缺集汇总：${task.label || task.kind}`, 'ok');
    } catch (e) {
      toast.push(`启动缺集汇总失败：${errorMessage(e)}`, 'error');
    } finally {
      setZhuigengAction('');
    }
  };

  const archiveableItems = (zhuigeng?.items || []).filter((item) => item.ended && item.folder && item.id);
  const selectedArchiveItems = archiveableItems.filter((item) => selectedArchiveKeys.has(zhuigengArchiveKey(item)));

  const toggleArchiveItem = (item: ZhuigengItem) => {
    const key = zhuigengArchiveKey(item);
    setSelectedArchiveKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const toggleAllArchiveItems = () => {
    const keys = archiveableItems.map(zhuigengArchiveKey);
    setSelectedArchiveKeys((current) => {
      if (keys.length > 0 && keys.every((key) => current.has(key))) return new Set();
      return new Set(keys);
    });
  };

  const requestArchiveSelected = () => {
    if (!archiveTargetLib) {
      toast.push('先选择归档目标库', 'warn');
      return;
    }
    if (selectedArchiveItems.length === 0) {
      toast.push('先选择要归档的完结剧', 'warn');
      return;
    }
    setConfirmArchive(true);
  };

  const executeArchiveSelected = async () => {
    setConfirmArchive(false);
    setZhuigengAction('archive');
    try {
      const byLib = selectedArchiveItems.reduce<Record<string, ZhuigengItem[]>>((acc, item) => {
        (acc[item.lib] ||= []).push(item);
        return acc;
      }, {});
      const tasks: TaskRun[] = [];
      for (const [fromLib, group] of Object.entries(byLib)) {
        const payload: ManageMoveBatchRequest = {
          from_lib: fromLib,
          to_lib: archiveTargetLib,
          items: group.map((item) => ({
            folder: item.folder || '',
            item_id: item.id || null,
            to_folder: null
          })),
          on_conflict: 'smart',
          reason: '追更完结剧智能归档'
        };
        const task = await api<TaskRun>('/api/v2/manage/move/batch/execute', {
          method: 'POST',
          body: JSON.stringify(payload)
        });
        tasks.push(task);
      }
      setArchiveTask(tasks[tasks.length - 1] || null);
      setSelectedArchiveKeys(new Set());
      toast.push(`已创建 ${tasks.length} 个归档任务`, 'ok');
    } catch (e) {
      toast.push(`归档任务创建失败：${errorMessage(e)}`, 'error');
    } finally {
      setZhuigengAction('');
    }
  };

  if (isZhuigeng) {
    const items = zhuigeng?.items || [];
    const behind = items.reduce((total, item) => total + item.behind, 0);
    const errors = items.filter((item) => item.err).length;
    const actionRunning = Boolean(zhuigengAction) || isActiveTask(airingTask) || isActiveTask(zhuigengGapTask);
    return (
      <section className="insightPanel">
        {confirmArchive && (
          <ConfirmDanger
            title="确认智能归档完结剧"
            confirmText="确认归档"
            onCancel={() => setConfirmArchive(false)}
            onConfirm={executeArchiveSelected}
            body={(
              <div className="dangerCopy">
                <p>将把选中的完结剧按来源库分组，移动到目标库，并启用 smart 冲突处理。</p>
                <code>{selectedArchiveItems.map((item) => `${item.lib}/${item.folder} -> ${archiveTargetLib}`).join('\n')}</code>
              </div>
            )}
          />
        )}
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
              <select className="input compactSelect" aria-label="归档目标库" value={archiveTargetLib} onChange={(event) => setArchiveTargetLib(event.target.value)}>
                {libraries.length === 0 && <option value="">无目标库</option>}
                {libraries.map((library) => (
                  <option key={library.id || library.name} value={library.name}>{library.name}</option>
                ))}
              </select>
              <button className="btn ghost compact" onClick={toggleAllArchiveItems} disabled={archiveableItems.length === 0 || actionRunning}>
                <ListChecks size={14} />
                {selectedArchiveItems.length === archiveableItems.length && archiveableItems.length > 0 ? '取消全选' : '全选完结'}
              </button>
              <button className="btn danger compact" onClick={requestArchiveSelected} disabled={selectedArchiveItems.length === 0 || !archiveTargetLib || actionRunning}>
                <Trash2 size={14} />
                {zhuigengAction === 'archive' ? '提交中' : `智能归档 ${count(selectedArchiveItems.length)}`}
              </button>
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
          <p className="mutedParagraph">scan-airing 汇总在更剧状态；gaps-summary 输出 continuing 且 behind 的求资源清单；智能归档只处理已完结且有 folder/id 的条目。</p>
          {airingTask && <div className="notice">在更扫描任务：{airingTask.label} · {airingTask.status}</div>}
          {zhuigengGapTask && <div className="notice">缺集汇总任务：{zhuigengGapTask.label} · {zhuigengGapTask.status}</div>}
          {archiveTask && <div className="notice">归档任务：{archiveTask.label} · {archiveTask.status}</div>}
        </section>
        <CopyTextBlock
          title="追更 copy_text"
          text={zhuigeng?.copy_text || ''}
          empty="当前没有可复制的追更求资源文本"
          onCopy={copyText}
        />
        <ZhuigengScanAiringBlock result={airingResult} onCopy={copyText} />
        <ZhuigengGapsSummaryBlock result={zhuigengGapResult} onCopy={copyText} />
        <ZhuigengItemList items={items} selectedArchiveKeys={selectedArchiveKeys} onToggleArchive={toggleArchiveItem} />
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
      {!isZhuigeng && (
        <section className="readonlyBlock">
          <div className="sectionTitleRow">
            <h2>单剧详情</h2>
            <button className="btn ghost compact" onClick={() => setSeriesDetail(null)} disabled={!seriesDetail}>
              清空
            </button>
          </div>
          <form className="gapScanControls" onSubmit={loadSeriesDetail}>
            <input
              className="input"
              aria-label="Emby Series Id"
              value={seriesId}
              onChange={(event) => setSeriesId(event.target.value)}
              placeholder="series id"
            />
            <button className="btn" type="submit" disabled={seriesDetailLoading || !seriesId.trim()}>
              <SearchX size={16} />
              {seriesDetailLoading ? '查询中' : '查询缺集'}
            </button>
          </form>
          {seriesDetail && <SeriesGapsDetailBlock detail={seriesDetail} />}
        </section>
      )}
      {scanResult && <GapsScanResultBlock result={scanResult} onCopy={copyText} />}
      <div className="statGrid">
        <StatCard icon={<CheckCircle2 />} label="业务状态" value="真实扫描" tone="ok" hint="/api/v2/gaps/scan-lib" />
        <StatCard icon={<ListChecks />} label="剧集库" value={count(libraries.length)} tone={libraries.length ? 'ok' : 'warn'} hint={selectedLib || '未选择'} />
        <StatCard icon={<SearchX />} label="有缺/落后" value={count(scanResult?.total)} tone={scanResult?.total ? 'warn' : 'ok'} hint={`已扫 ${count(scanResult?.analyzed)}`} />
        <StatCard icon={<AlertTriangle />} label="任务状态" value={scanTask?.status || '未运行'} tone={scanTask?.status === 'error' ? 'warn' : 'neutral'} hint={scanTask?.status_text || '等待全库扫描'} />
      </div>
    </section>
  );
}
