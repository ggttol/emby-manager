import {
  AlertTriangle,
  CheckSquare,
  DownloadCloud,
  Package,
  RefreshCw,
  Search,
  Square
} from 'lucide-react';
import { FormEvent, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { Modal } from './Modal';
import { useToast } from './Toast';

type CatalogItem = components['schemas']['CatalogItem'];
type CatalogSearchResponse = components['schemas']['CatalogSearchResponse'];
type CatalogStatsResponse = components['schemas']['CatalogStatsResponse'];
type CatalogTransferPlanItem = components['schemas']['CatalogTransferPlanItem'];
type CatalogTransferPlanRequest = components['schemas']['CatalogTransferPlanRequest'];
type CatalogTransferPlanResponse = components['schemas']['CatalogTransferPlanResponse'];
type CatalogTransferTarget = components['schemas']['CatalogTransferTarget'];
type ConfigResponse = components['schemas']['ConfigResponse'];
type TaskRun = components['schemas']['TaskRun'];

type LinkFilter = '' | 'share115' | 'magnet' | 'ed2k';

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
};

type CatalogTransferExecuteRequest = {
  items: CatalogTransferPlanItem[];
  target: CatalogTransferTarget;
};

const linkTypeOptions: Array<{ value: LinkFilter; label: string }> = [
  { value: '', label: '全部类型' },
  { value: 'share115', label: '115 秒传' },
  { value: 'magnet', label: '磁力' },
  { value: 'ed2k', label: 'ed2k' }
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

export function CatalogPanel() {
  const [stats, setStats] = useState<CatalogStatsResponse | null>(null);
  const [cidMap, setCidMap] = useState<Record<string, string>>({});
  const [targetLib, setTargetLib] = useState('');
  const [customCid, setCustomCid] = useState('');
  const [query, setQuery] = useState('');
  const [linkFilter, setLinkFilter] = useState<LinkFilter>('');
  const [items, setItems] = useState<CatalogItem[]>([]);
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
  const toast = useToast();

  const cidEntries = useMemo(() => Object.entries(cidMap).sort(([a], [b]) => a.localeCompare(b, 'zh-CN')), [cidMap]);
  const selectedItems = useMemo(() => items.filter((_, index) => selected.has(index)), [items, selected]);
  const allSelected = items.length > 0 && selected.size === items.length;

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
      if (linkFilter) params.set('link_type', linkFilter);
      const data = await api<CatalogSearchResponse>(`/api/v2/catalog/search?${params.toString()}`);
      setItems(data.items);
      setSelected(new Set());
      setSearched(true);
      setTruncated(data.truncated);
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
      setPending({ items: transferItems, mode, target, plans });
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

  const packageCount = pending?.plans.filter((plan) => plan.is_pkg).length || 0;
  const offlineCount = pending?.plans.filter((plan) => plan.action === 'offline_download').length || 0;
  const shareCount = pending?.plans.filter((plan) => plan.action === 'save_share').length || 0;
  const unsupportedCount = pending?.plans.filter((plan) => plan.action === 'unsupported').length || 0;
  const confirmDisabled = transferring || (packageCount > 0 && packageAck.trim() !== '整包');

  return (
    <section className="catalogPanel">
      <div className="catalogMeta">
        <div>
          <strong>115 资源目录</strong>
          <span>{statText(stats)}</span>
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
          <span>链接类型</span>
          <select
            className="input"
            aria-label="链接类型"
            value={linkFilter}
            onChange={(event) => setLinkFilter(event.target.value as LinkFilter)}
          >
            {linkTypeOptions.map((option) => (
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
      </div>

      {error && <div className="notice warn whitespaceNotice">{error}</div>}

      <div className="catalogResultHead">
        <button className="btn ghost compact" onClick={toggleAll} disabled={!items.length}>
          {allSelected ? <CheckSquare size={15} /> : <Square size={15} />}
          全选
        </button>
        <span>
          {searched ? `${items.length} 条结果${truncated ? ' · 已截断，请缩小关键词' : ''}` : '等待搜索'}
        </span>
        <button
          className="btn compact"
          onClick={() => requestTransfer(selectedItems, 'batch')}
          disabled={!selectedItems.length || transferring}
        >
          <DownloadCloud size={15} />
          转存选中
        </button>
      </div>

      <div className="catalogTableWrap">
        <table className="dataTable catalogTable">
          <thead>
            <tr>
              <th>选择</th>
              <th>资源</th>
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
                <td colSpan={6} className="emptyCell">
                  {searched ? '没有搜到资源，换几个关键词试试' : '输入关键词后搜索资源目录'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {transferring && progressText && <div className="notice catalogProgress">{progressText}</div>}

      {pending && (
        <Modal title={pending.mode === 'batch' ? '批量转存确认' : `${actionLabel(pending.items[0])}确认`} onClose={() => setPending(null)}>
          <div className="modalBody catalogConfirm">
            <p>
              确认后会创建一个可取消任务，交给任务中心处理 <strong>{pending.items.length}</strong> 条到 <strong>{pending.target.label}</strong>
            </p>
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
