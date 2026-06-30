import {
  AlertTriangle,
  CheckSquare,
  DownloadCloud,
  Package,
  RefreshCw,
  Search,
  Sparkles,
  Square
} from 'lucide-react';
import { FormEvent, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useTaskCompletion } from '../hooks/useTaskCompletion';
import { Modal } from './Modal';
import { useToast } from './Toast';

type CatalogItem = components['schemas']['CatalogItem'];
type CatalogLibraryContextResponse = components['schemas']['CatalogLibraryContextResponse'];
type CatalogResourceRecommendation = components['schemas']['CatalogResourceRecommendation'];
type CatalogRemoteSearchResponse = components['schemas']['CatalogRemoteSearchResponse'];
type CatalogSearchResponse = components['schemas']['CatalogSearchResponse'];
type CatalogStatsResponse = components['schemas']['CatalogStatsResponse'];
type CatalogTransferPlanItem = components['schemas']['CatalogTransferPlanItem'];
type CatalogTransferPlanRequest = components['schemas']['CatalogTransferPlanRequest'];
type CatalogTransferPlanResponse = components['schemas']['CatalogTransferPlanResponse'];
type CatalogTransferTarget = components['schemas']['CatalogTransferTarget'];
type ConfigResponse = components['schemas']['ConfigResponse'];
type TaskRun = components['schemas']['TaskRun'];

type CatalogSource = 'local' | 'remote';
type LinkFilter = string;

type TransferTarget = {
  lib?: string;
  cid?: string;
  label: string;
};

type PendingTransfer = {
  items: CatalogItem[];
  mode: 'single' | 'batch';
  target: TransferTarget;
  plans: CatalogTransferPlanResponse[];
  context?: CatalogLibraryContextResponse | null;
};

type CatalogTransferExecuteRequest = {
  items: CatalogTransferPlanItem[];
  target: CatalogTransferTarget;
};

const localLinkTypeOptions: Array<{ value: LinkFilter; label: string }> = [
  { value: '', label: '全部类型' },
  { value: 'share115', label: '115 秒传' },
  { value: 'magnet', label: '磁力' },
  { value: 'ed2k', label: 'ed2k' }
];

const remoteDiskTypeOptions: Array<{ value: LinkFilter; label: string }> = [
  { value: '', label: '全部网盘' },
  { value: '115', label: '115' },
  { value: 'quark', label: '夸克' },
  { value: 'baidu', label: '百度' },
  { value: 'aliyun', label: '阿里云' },
  { value: 'xunlei', label: '迅雷' },
  { value: 'uc', label: 'UC' },
  { value: '123', label: '123' },
  { value: 'guangya', label: '光亚' }
];

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function parseCidMapValue(value: unknown): Record<string, string> {
  const raw = typeof value === 'string' ? parseJsonObject(value) : value;
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {};
  return Object.fromEntries(
    Object.entries(raw as Record<string, unknown>)
      .map(([key, item]) => [key, typeof item === 'string' ? item : item == null ? '' : String(item)])
      .filter(([key, item]) => key.trim() && item.trim())
  );
}

function parseJsonObject(value: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    return {};
  }
}

function itemToPlanItem(item: CatalogItem): CatalogTransferPlanItem {
  return {
    name: item.name,
    sheet: item.sheet,
    link: item.link,
    link_type: item.link_type,
    is_pkg: item.is_pkg,
    share: item.share,
    rc: item.rc
  };
}

function linkTypeLabel(type: string) {
  if (type === 'share115') return '115 秒传';
  if (type === '115') return '115';
  if (type === 'quark') return '夸克';
  if (type === 'baidu') return '百度';
  if (type === 'aliyun') return '阿里云';
  if (type === 'xunlei') return '迅雷';
  if (type === 'uc') return 'UC';
  if (type === '123') return '123';
  if (type === 'guangya') return '光亚';
  if (type === 'magnet') return '磁力';
  if (type === 'ed2k') return 'ed2k';
  return type || '未知';
}

function actionLabel(item: CatalogItem) {
  if (item.link_type === 'magnet' || item.link_type === 'ed2k') return '离线';
  if (item.transfer || item.link_type === 'share115') return '转存';
  return '预检';
}

function statText(stats: CatalogStatsResponse | null) {
  if (!stats) return '正在读取 catalog 状态';
  if (!stats.available) return '资源库未导入';
  return `库内 ${stats.total.toLocaleString('zh-CN')} 条 · 整包 ${stats.packages.toLocaleString('zh-CN')}`;
}

function taskSummary(task: TaskRun) {
  return task.label || task.kind || task.id;
}

function planActionLabel(action: CatalogTransferPlanResponse['action']) {
  if (action === 'save_share') return '115 秒传';
  if (action === 'offline_download') return '离线下载';
  return '不支持';
}

function recommendationClass(level?: string | null) {
  if (level === 'best' || level === 'good' || level === 'warn' || level === 'skip') return level;
  return 'neutral';
}

function isSmartSelectable(item: CatalogItem) {
  const recommendation = item.recommendation;
  return (
    item.link_type === 'share115'
    && !recommendation?.already_have
    && (recommendation?.level === 'best' || recommendation?.level === 'good')
  );
}

function compactText(values: Array<string | number>, fallback = '无') {
  return values.length ? values.join('、') : fallback;
}

function itemTypeLabel(type: string) {
  if (type.toLowerCase() === 'series') return '剧集';
  if (type.toLowerCase() === 'movie') return '电影';
  return type || '条目';
}

function RecommendationCell({ recommendation }: { recommendation?: CatalogResourceRecommendation | null }) {
  if (!recommendation) {
    return <span className="catalogRecommendation mutedText">待判断</span>;
  }
  const level = recommendationClass(recommendation.level);
  return (
    <div className="catalogRecommendation">
      <span className={`catalogRecommendationBadge ${level}`}>
        {recommendation.action}
        <small>{recommendation.score}</small>
      </span>
      {recommendation.episode_ranges.length > 0 && (
        <span className="catalogRecommendationRanges">{recommendation.episode_ranges.join('、')}</span>
      )}
      <ul className="catalogRecommendationReasons">
        {recommendation.reasons.slice(0, 2).map((reason) => (
          <li key={reason}>{reason}</li>
        ))}
      </ul>
    </div>
  );
}

function CatalogLibraryContextPanel({ context }: { context: CatalogLibraryContextResponse }) {
  const summary = context.summary;
  const episodeText = summary.episode_ranges.length
    ? summary.episode_ranges.join('、')
    : summary.max_episode > 0
      ? `到 E${summary.max_episode}`
      : '无';
  const missingText = summary.missing_ranges.length ? summary.missing_ranges.join('、') : '无';

  return (
    <div className={`catalogLibraryContext ${context.ok ? '' : 'warn'}`}>
      <div className="catalogContextHead">
        <div>
          <span>本库情况</span>
          <strong>{summary.note}</strong>
          <small>
            {context.total_matches.toLocaleString('zh-CN')} 个匹配
            {context.truncated ? ' · 结果已截断' : ''}
          </small>
        </div>
        <div className="catalogContextBadges">
          <span className={`badge ${summary.matched ? 'done' : 'pending'}`}>{summary.matched ? '已入库' : '未入库'}</span>
          {summary.duplicate && <span className="badge warn">重复 {summary.duplicate_groups}</span>}
          {summary.missing_ranges.length > 0 && <span className="badge warn">缺集 {summary.missing_ranges.length}</span>}
        </div>
      </div>

      <div className="catalogContextSummary">
        <div><span>媒体库</span><strong>{compactText(summary.libraries)}</strong></div>
        <div><span>年份</span><strong>{compactText(summary.years)}</strong></div>
        <div><span>TMDb</span><strong>{compactText(summary.tmdb_ids)}</strong></div>
        <div><span>已有集数</span><strong>{episodeText}</strong></div>
        <div><span>缺口</span><strong>{missingText}</strong></div>
      </div>

      {context.warnings.length > 0 && (
        <div className="catalogContextWarning">
          {context.warnings.map((warning) => (
            <span key={warning}>{warning}</span>
          ))}
        </div>
      )}

      {context.items.length > 0 && (
        <div className="catalogContextList">
          {context.items.slice(0, 4).map((item, index) => (
            <div className="catalogContextItem" key={item.id || `${item.name}-${index}`}>
              <strong>{item.name}</strong>
              <span>
                {itemTypeLabel(item.item_type)}
                {item.library ? ` · ${item.library}` : ''}
                {item.year ? ` · ${item.year}` : ''}
                {item.tmdb ? ` · TMDb ${item.tmdb}` : ''}
              </span>
              <small>
                {item.episode_ranges.length ? `已有 ${item.episode_ranges.join('、')}` : '未读取到集数'}
                {item.missing_ranges.length ? ` · 缺 ${item.missing_ranges.join('、')}` : ''}
                {item.duplicate ? ' · 重复条目' : ''}
                {!item.has_primary_image ? ' · 无主海报' : ''}
                {item.error ? ` · ${item.error}` : ''}
              </small>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function numberField(value: unknown) {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function resultTargetLib(result: unknown) {
  if (!isRecord(result) || !isRecord(result.target)) return null;
  const lib = result.target.lib;
  return typeof lib === 'string' && lib.trim() ? lib.trim() : null;
}

export function CatalogPanel() {
  const [stats, setStats] = useState<CatalogStatsResponse | null>(null);
  const [cidMap, setCidMap] = useState<Record<string, string>>({});
  const [targetLib, setTargetLib] = useState('');
  const [customCid, setCustomCid] = useState('');
  const [source, setSource] = useState<CatalogSource>('remote');
  const [query, setQuery] = useState('');
  const [linkFilter, setLinkFilter] = useState<LinkFilter>('');
  const [items, setItems] = useState<CatalogItem[]>([]);
  const [resultTotal, setResultTotal] = useState(0);
  const [remoteDiskTypes, setRemoteDiskTypes] = useState<Array<{ disk_type: string; count: number }>>([]);
  const [libraryContext, setLibraryContext] = useState<CatalogLibraryContextResponse | null>(null);
  const [selected, setSelected] = useState<Set<number>>(() => new Set());
  const [searched, setSearched] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [loadingMeta, setLoadingMeta] = useState(true);
  const [searching, setSearching] = useState(false);
  const [transferring, setTransferring] = useState(false);
  const [progressText, setProgressText] = useState('');
  const [error, setError] = useState('');
  const [pending, setPending] = useState<PendingTransfer | null>(null);
  const [packageAck, setPackageAck] = useState('');
  const [autoScanAfterTransfer, setAutoScanAfterTransfer] = useState(true);
  const [trackedTaskIds, setTrackedTaskIds] = useState<string[]>([]);
  const [completedTasks, setCompletedTasks] = useState<TaskRun[]>([]);
  const toast = useToast();

  const cidEntries = useMemo(() => Object.entries(cidMap).sort(([a], [b]) => a.localeCompare(b, 'zh-CN')), [cidMap]);
  const selectedItems = useMemo(() => items.filter((_, index) => selected.has(index)), [items, selected]);
  const smartSelectableCount = useMemo(() => items.filter(isSmartSelectable).length, [items]);
  const allSelected = items.length > 0 && selected.size === items.length;
  const filterOptions = source === 'remote' ? remoteDiskTypeOptions : localLinkTypeOptions;

  const trackTask = (task: TaskRun) => {
    setTrackedTaskIds((prev) => (prev.includes(task.id) ? prev : [task.id, ...prev].slice(0, 20)));
  };

  useTaskCompletion(trackedTaskIds, (task) => {
    setCompletedTasks((prev) => [task, ...prev.filter((item) => item.id !== task.id)].slice(0, 8));
    const result = isRecord(task.result) ? task.result : {};
    const succeeded = numberField(result.succeeded);
    const failed = numberField(result.failed);
    toast.push(
      task.status === 'done'
        ? `任务完成：${taskSummary(task)}${succeeded != null ? ` · 成功 ${succeeded}${failed ? ` / 失败 ${failed}` : ''}` : ''}`
        : `任务结束：${taskSummary(task)} · ${task.status}`,
      task.status === 'done' && !failed ? 'ok' : 'warn'
    );
    if (task.kind === 'catalog_transfer_execute' && task.status === 'done' && autoScanAfterTransfer) {
      const lib = resultTargetLib(task.result);
      if (lib) {
        api<TaskRun>('/api/v2/libraries/scan', {
          method: 'POST',
          body: JSON.stringify({ lib })
        })
          .then((scanTask) => {
            trackTask(scanTask);
            toast.push(`目录转存完成，已创建扫库任务：${taskSummary(scanTask)}`, 'ok');
          })
          .catch((e) => toast.push(`目录转存完成，但自动扫库失败：${errorMessage(e)}`, 'error'));
      }
    }
  });

  const loadMeta = async () => {
    setLoadingMeta(true);
    setError('');
    try {
      const [statsData, configData] = await Promise.all([
        api<CatalogStatsResponse>('/api/v2/catalog/stats'),
        api<ConfigResponse>('/api/v2/config')
      ]);
      const nextCidMap = parseCidMapValue(configData.settings.c115_cid_map);
      const keys = Object.keys(nextCidMap).sort((a, b) => a.localeCompare(b, 'zh-CN'));
      setStats(statsData);
      setCidMap(nextCidMap);
      setTargetLib((prev) => (prev && nextCidMap[prev] ? prev : keys[0] || ''));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`资源库配置加载失败：${message}`, 'error');
    } finally {
      setLoadingMeta(false);
    }
  };

  useEffect(() => {
    loadMeta();
  }, []);

  const search = async () => {
    const q = query.trim();
    if (!q) {
      toast.push('先输入片名或关键词', 'warn');
      return;
    }
    setSearching(true);
    setError('');
    try {
      const params = new URLSearchParams({ q, limit: '80' });
      if (source === 'remote') {
        if (linkFilter) params.set('disk_type', linkFilter);
        const data = await api<CatalogRemoteSearchResponse>(`/api/v2/catalog/remote-search?${params.toString()}`);
        setItems(data.items);
        setResultTotal(data.total);
        setRemoteDiskTypes(data.disk_types);
        setLibraryContext(data.context ?? null);
        setTruncated(data.truncated);
      } else {
        if (linkFilter) params.set('link_type', linkFilter);
        const data = await api<CatalogSearchResponse>(`/api/v2/catalog/search?${params.toString()}`);
        setItems(data.items);
        setResultTotal(data.total);
        setRemoteDiskTypes([]);
        setLibraryContext(null);
        setTruncated(data.truncated);
      }
      setSelected(new Set());
      setSearched(true);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`搜索失败：${message}`, 'error');
    } finally {
      setSearching(false);
    }
  };

  const submitSearch = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    search();
  };

  const changeSource = (next: CatalogSource) => {
    setSource(next);
    setLinkFilter('');
    setRemoteDiskTypes([]);
    setLibraryContext(null);
  };

  const parseTarget = (): TransferTarget | null => {
    const cid = customCid.trim();
    if (cid) {
      if (!/^[1-9]\d*$/.test(cid)) {
        toast.push('自定义 cid 必须是正整数，0 根目录不允许', 'warn');
        return null;
      }
      return { cid, label: `cid ${cid}` };
    }
    if (targetLib) return { lib: targetLib, label: `库「${targetLib}」` };
    toast.push('先选择目标库，或填写自定义 cid', 'warn');
    return null;
  };

  const planTransferItem = async (item: CatalogItem, target: TransferTarget): Promise<CatalogTransferPlanResponse> => {
    const request: CatalogTransferPlanRequest = {
      item: itemToPlanItem(item),
      ...(target.cid ? { cid: target.cid } : { lib: target.lib })
    };
    return api<CatalogTransferPlanResponse>('/api/v2/catalog/transfer-plan', {
      method: 'POST',
      body: JSON.stringify(request)
    });
  };

  const requestTransfer = async (transferItems: CatalogItem[], mode: PendingTransfer['mode']) => {
    if (!transferItems.length || transferring) return;
    const target = parseTarget();
    if (!target) return;
    setTransferring(true);
    setProgressText('正在预检目录转存计划...');
    try {
      const plans = await Promise.all(transferItems.map((item) => planTransferItem(item, target)));
      setPackageAck('');
      setPending({ items: transferItems, mode, target, plans, context: source === 'remote' ? libraryContext : null });
      setError('');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`目录转存预检失败：${message}`, 'error');
    } finally {
      setTransferring(false);
      setProgressText('');
    }
  };

  const createTransferBatchTask = async (transferItems: CatalogItem[], target: TransferTarget): Promise<TaskRun> => {
    const request: CatalogTransferExecuteRequest = {
      items: transferItems.map(itemToPlanItem),
      target: target.cid ? { cid: target.cid } : { lib: target.lib }
    };
    return api<TaskRun>('/api/v2/catalog/transfer/execute', {
      method: 'POST',
      body: JSON.stringify(request)
    });
  };

  const runTransfer = async (transfer: PendingTransfer) => {
    setPending(null);
    setTransferring(true);
    setProgressText('正在创建目录转存任务...');
    try {
      const task = await createTransferBatchTask(transfer.items, transfer.target);
      trackTask(task);
      if (transfer.mode === 'batch') setSelected(new Set());
      const prefix = transfer.mode === 'batch' ? '批量任务已交给任务中心，可在任务中心取消' : '任务已交给任务中心，可在任务中心取消';
      toast.push(`${prefix}：${taskSummary(task)}`, 'ok');
      setError('');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`目录转存任务创建失败：${message}`, 'error');
    } finally {
      setTransferring(false);
      setProgressText('');
    }
  };

  const toggleAll = () => {
    setSelected(allSelected ? new Set() : new Set(items.map((_, index) => index)));
  };

  const toggleOne = (index: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  const selectSmart = () => {
    const next = new Set<number>();
    items.forEach((item, index) => {
      if (isSmartSelectable(item)) next.add(index);
    });
    if (!next.size) {
      toast.push('当前结果里没有可智能选择的 115 推荐项', 'warn');
      return;
    }
    setSelected(next);
    toast.push(`已智能选择 ${next.size} 条推荐资源`, 'ok');
  };

  const packageCount = pending?.plans.filter((plan) => plan.is_pkg).length || 0;
  const offlineCount = pending?.plans.filter((plan) => plan.action === 'offline_download').length || 0;
  const shareCount = pending?.plans.filter((plan) => plan.action === 'save_share').length || 0;
  const unsupportedCount = pending?.plans.filter((plan) => plan.action === 'unsupported').length || 0;
  const confirmDisabled = transferring || (packageCount > 0 && packageAck.trim() !== '整包');

  return (
    <section className="catalogPanel">
      <div className="catalogMeta">
        <div>
          <strong>资源搜索</strong>
          <span>{source === 'remote' ? 'TG Resource API · 115 结果可直接转存到目标库' : statText(stats)}</span>
        </div>
        <button className="btn ghost" onClick={loadMeta} disabled={loadingMeta}>
          <RefreshCw size={16} />
          {loadingMeta ? '读取中' : '刷新状态'}
        </button>
      </div>

      <form className="catalogSearchBar" onSubmit={submitSearch}>
        <label>
          <span>关键词</span>
          <input
            className="input"
            aria-label="资源关键词"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="片名、年份、季数或多个关键词"
          />
        </label>
        <label>
          <span>数据源</span>
          <select
            className="input"
            aria-label="资源数据源"
            value={source}
            onChange={(event) => changeSource(event.target.value as CatalogSource)}
          >
            <option value="remote">TG Resource API</option>
            <option value="local">本地 catalog</option>
          </select>
        </label>
        <label>
          <span>{source === 'remote' ? '网盘类型' : '链接类型'}</span>
          <select
            className="input"
            aria-label={source === 'remote' ? '网盘类型' : '链接类型'}
            value={linkFilter}
            onChange={(event) => setLinkFilter(event.target.value as LinkFilter)}
          >
            {filterOptions.map((option) => (
              <option key={option.value || 'all'} value={option.value}>{option.label}</option>
            ))}
          </select>
        </label>
        <button className="btn" disabled={searching}>
          <Search size={16} />
          {searching ? '搜索中' : '搜索'}
        </button>
      </form>

      <div className="catalogTargetBar">
        <label>
          <span>目标库</span>
          <select
            className="input"
            aria-label="目标库"
            value={targetLib}
            onChange={(event) => setTargetLib(event.target.value)}
            disabled={cidEntries.length === 0}
          >
            {cidEntries.length === 0 && <option value="">未配置库 cid</option>}
            {cidEntries.map(([name, cid]) => (
              <option key={name} value={name}>{name} · cid {cid}</option>
            ))}
          </select>
        </label>
        <label>
          <span>自定义 cid</span>
          <input
            className="input"
            aria-label="自定义 cid"
            inputMode="numeric"
            value={customCid}
            onChange={(event) => setCustomCid(event.target.value)}
            placeholder="填了就优先使用"
          />
        </label>
        <p>自定义 cid 优先；磁力和 ed2k 会创建 115 离线下载任务。</p>
        <label className="switchRow catalogAutoScan">
          <input
            type="checkbox"
            aria-label="转存完成后自动扫库"
            checked={autoScanAfterTransfer}
            onChange={(event) => setAutoScanAfterTransfer(event.target.checked)}
          />
          <span>转存完成后自动扫库</span>
        </label>
      </div>

      {source === 'remote' && remoteDiskTypes.length > 0 && (
        <div className="catalogRemoteTypes" aria-label="远程网盘类型分布">
          {remoteDiskTypes.map((item) => (
            <span className="badge" key={item.disk_type}>{linkTypeLabel(item.disk_type)} {item.count.toLocaleString('zh-CN')}</span>
          ))}
        </div>
      )}

      {source === 'remote' && libraryContext && <CatalogLibraryContextPanel context={libraryContext} />}

      {error && <div className="notice warn whitespaceNotice">{error}</div>}

      <div className="catalogResultHead">
        <button className="btn ghost compact" onClick={toggleAll} disabled={!items.length}>
          {allSelected ? <CheckSquare size={15} /> : <Square size={15} />}
          全选
        </button>
        <span>
          {searched
            ? `${items.length}${resultTotal && resultTotal !== items.length ? ` / ${resultTotal.toLocaleString('zh-CN')}` : ''} 条结果${truncated ? ' · 还有更多，请缩小关键词或翻页后续再补' : ''}`
            : '等待搜索'}
        </span>
        <div className="catalogResultActions">
          <button
            className="btn ghost compact"
            onClick={selectSmart}
            disabled={!smartSelectableCount || transferring}
          >
            <Sparkles size={15} />
            智能选择{smartSelectableCount ? ` ${smartSelectableCount}` : ''}
          </button>
          <button
            className="btn compact"
            onClick={() => requestTransfer(selectedItems, 'batch')}
            disabled={!selectedItems.length || transferring}
          >
            <DownloadCloud size={15} />
            转存选中
          </button>
        </div>
      </div>

      <div className="catalogTableWrap">
        <table className="dataTable catalogTable">
          <thead>
            <tr>
              <th>选择</th>
              <th>资源</th>
              <th>建议</th>
              <th>类型</th>
              <th>来源</th>
              <th>链接</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {items.map((item, index) => (
              <tr key={`${item.link}-${index}`}>
                <td>
                  <input
                    type="checkbox"
                    aria-label={`选择 ${item.name}`}
                    checked={selected.has(index)}
                    onChange={() => toggleOne(index)}
                  />
                </td>
                <td>
                  <strong className="catalogName">{item.name}</strong>
                  {item.is_pkg && (
                    <span className="badge packageBadge">
                      <Package size={12} />
                      整包
                    </span>
                  )}
                </td>
                <td><RecommendationCell recommendation={item.recommendation} /></td>
                <td><span className={`badge linkType ${item.link_type}`}>{linkTypeLabel(item.link_type)}</span></td>
                <td>{item.sheet}</td>
                <td><span className="catalogLink" title={item.link}>{item.link}</span></td>
                <td>
                  <button className="btn ghost compact" onClick={() => requestTransfer([item], 'single')} disabled={transferring}>
                    <DownloadCloud size={14} />
                    {actionLabel(item)}
                  </button>
                </td>
              </tr>
            ))}
            {!items.length && (
              <tr>
                <td colSpan={7} className="emptyCell">
                  {searched ? '没有搜到资源，换几个关键词试试' : '输入关键词后搜索资源目录'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {transferring && progressText && <div className="notice catalogProgress">{progressText}</div>}

      {completedTasks.length > 0 && (
        <div className="notice catalogProgress">
          {completedTasks.map((task) => (
            <div key={task.id}>
              {taskSummary(task)} · {task.status}
            </div>
          ))}
        </div>
      )}

      {pending && (
        <Modal title={pending.mode === 'batch' ? '批量转存确认' : `${actionLabel(pending.items[0])}确认`} onClose={() => setPending(null)}>
          <div className="modalBody catalogConfirm">
            <p>
              确认后会创建一个可取消任务，交给任务中心处理 <strong>{pending.items.length}</strong> 条到 <strong>{pending.target.label}</strong>
            </p>
            {pending.context && (
              <div className="notice catalogPlanContext">
                <strong>本库情况：{pending.context.summary.note}</strong>
                {pending.context.summary.duplicate && <span>已有重复组 {pending.context.summary.duplicate_groups} 个</span>}
                {pending.context.summary.missing_ranges.length > 0 && <span>缺口 {pending.context.summary.missing_ranges.join('、')}</span>}
                {pending.context.warnings.map((warning) => (
                  <span key={warning}>{warning}</span>
                ))}
              </div>
            )}
            <dl>
              <div><dt>115 秒传</dt><dd>{shareCount}</dd></div>
              <div><dt>离线下载</dt><dd>{offlineCount}</dd></div>
              <div><dt>整包</dt><dd>{packageCount}</dd></div>
              <div><dt>不支持</dt><dd>{unsupportedCount}</dd></div>
            </dl>
            {unsupportedCount > 0 && (
              <div className="notice warn catalogPlanWarning">
                {pending.plans.filter((plan) => plan.action === 'unsupported').map((plan, index) => (
                  <div key={`${plan.unsupported?.link || plan.label || index}`}>
                    {plan.label || plan.unsupported?.link || `第 ${index + 1} 项`}：{plan.unsupported?.reason || '后端标记为不支持'}
                  </div>
                ))}
              </div>
            )}
            {packageCount > 0 && (
              <label className="packageAck">
                <span>
                  <AlertTriangle size={15} />
                  含整包合集，输入“整包”确认
                </span>
                <input className="input" value={packageAck} onChange={(event) => setPackageAck(event.target.value)} />
              </label>
            )}
            <ul>
              {pending.items.slice(0, 5).map((item, index) => (
                <li key={item.link}>{item.name} · {planActionLabel(pending.plans[index]?.action || 'unsupported')}</li>
              ))}
              {pending.items.length > 5 && <li>还有 {pending.items.length - 5} 条...</li>}
            </ul>
          </div>
          <footer className="modalActions">
            <button className="btn ghost" onClick={() => setPending(null)}>取消</button>
            <button className={packageCount ? 'btn danger' : 'btn'} onClick={() => runTransfer(pending)} disabled={confirmDisabled}>
              {pending.mode === 'batch' ? '开始提交' : actionLabel(pending.items[0])}
            </button>
          </footer>
        </Modal>
      )}
    </section>
  );
}
