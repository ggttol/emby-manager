import {
  CheckSquare,
  DownloadCloud,
  Eye,
  RefreshCw,
  RadioTower,
  RotateCw,
  SearchCheck,
  Square
} from 'lucide-react';
import { FormEvent, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useTaskCompletion } from '../hooks/useTaskCompletion';
import { ConfirmDanger } from './Modal';
import { useToast } from './Toast';

type C115SnapFile = components['schemas']['C115SnapFile'];
type C115SnapResponse = components['schemas']['C115SnapResponse'];
type C115TestResponse = components['schemas']['C115TestResponse'];
type C115AutoCidResponse = components['schemas']['C115AutoCidResponse'];
type ConfigResponse = components['schemas']['ConfigResponse'];
type AddNewReport = components['schemas']['AddNewReport'];
type AddNewItem = components['schemas']['AddNewItem'];
type AddNewRequest = components['schemas']['AddNewRequest'];
type AddNewAutoResolveItemReport = components['schemas']['AddNewAutoResolveItemReport'];
type DedupGroup = components['schemas']['DedupGroup'];
type DedupReviewGroup = components['schemas']['DedupReviewGroup'];
type DedupRow = components['schemas']['DedupRow'];
type ReplaceBatchRequest = components['schemas']['ReplaceBatchRequest'];
type TaskRun = components['schemas']['TaskRun'];

type InputLine = {
  url: string;
  pwd?: string;
};

type TransferTarget = {
  lib?: string;
  cid?: string;
  label: string;
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function parseJsonObject(value: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    return {};
  }
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

function parseLines(value: string): InputLine[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const [url, pwd] = line.split(/[\s,，\t]+/).filter(Boolean);
      return { url, pwd };
    })
    .filter((item) => item.url);
}

function isOfflineUrl(url: string) {
  const normalized = url.trim().toLowerCase();
  return normalized.startsWith('magnet:') || normalized.startsWith('ed2k://');
}

function inferAddNewKind(url: string): NonNullable<AddNewItem['kind']> {
  return isOfflineUrl(url) ? 'offline_download' : 'share115';
}

function pwdFromUrl(url: string) {
  try {
    const parsed = new URL(url, window.location.origin);
    return parsed.searchParams.get('password') || parsed.searchParams.get('pwd') || undefined;
  } catch {
    const match = url.match(/[?&](?:password|pwd)=([^&#]+)/i);
    return match ? decodeURIComponent(match[1]) : undefined;
  }
}

function resolvePwd(line: InputLine, fallback: string) {
  return line.pwd || pwdFromUrl(line.url) || fallback.trim() || undefined;
}

function humanSize(size: number) {
  if (!Number.isFinite(size) || size <= 0) return '';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = size;
  let idx = 0;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  return `${value >= 10 || idx === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[idx]}`;
}

function taskSummary(task: TaskRun) {
  return task.label || task.kind || task.id;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function isAutoCidResponse(value: unknown): value is C115AutoCidResponse {
  return isRecord(value) && value.ok === true && isRecord(value.matches) && typeof value.scanned === 'number';
}

function isAddNewReport(value: unknown): value is AddNewReport {
  return isRecord(value) && isRecord(value.transfer) && isRecord(value.dedup) && isRecord(value.strm);
}

function dedupRows(group: DedupGroup | DedupReviewGroup): DedupRow[] {
  if ('rows' in group) return group.rows;
  return [group.keep, ...group.remove];
}

function smartReplaceCandidate(group: DedupGroup | DedupReviewGroup) {
  const rows = dedupRows(group);
  const withSuffix = rows.find((row) => /\(\d+\)$/.test(row.folder));
  const withoutSuffix = rows.find((row) => row !== withSuffix && !/\(\d+\)$/.test(row.folder));
  if (!withSuffix || !withoutSuffix || withSuffix.lib !== withoutSuffix.lib) return null;
  if ((withSuffix.n || 0) < (withoutSuffix.n || 0)) return null;
  return {
    tmdb: group.tmdb,
    lib: withSuffix.lib,
    win_folder: withSuffix.folder,
    lose_folder: withoutSuffix.folder,
    win_n: withSuffix.n,
    lose_n: withoutSuffix.n
  };
}

function AddNewReportCard({
  task,
  report,
  replacing,
  onSmartReplace
}: {
  task: TaskRun;
  report: AddNewReport;
  replacing: boolean;
  onSmartReplace: (items: ReplaceBatchRequest['items']) => void;
}) {
  const groups = [...report.dedup.dups, ...report.dedup.review];
  const autoResolve = report.auto_resolve;
  const autoItems = autoResolve?.items || [];
  const candidates = groups.flatMap((group) => {
    const candidate = smartReplaceCandidate(group);
    return candidate
      ? [{
          lib: candidate.lib,
          win_folder: candidate.win_folder,
          lose_folder: candidate.lose_folder,
          reason: `一条龙智能替换 tmdb ${candidate.tmdb}`
        }]
      : [];
  });

  return (
    <div className="c115AutoCid c115WizardReport">
      <strong>一条龙报告 · {task.label}</strong>
      <div>
        <span className={report.transfer.failed ? 'badge warn' : 'badge done'}>
          转存 {report.transfer.succeeded}/{report.transfer.total}
        </span>
        <span className={report.strm.new_count ? 'badge done' : 'badge warn'}>
          STRM 新增 {report.strm.new_count}
        </span>
        <span className={groups.length ? 'badge warn' : 'badge done'}>
          重复 {groups.length}
        </span>
        <span className={autoResolve?.error_count ? 'badge warn' : autoResolve?.resolved_count ? 'badge done' : 'badge'}>
          自动处理 {autoResolve?.resolved_count || 0}
          {autoResolve?.skipped_count ? ` / 跳过 ${autoResolve.skipped_count}` : ''}
          {autoResolve?.error_count ? ` / 失败 ${autoResolve.error_count}` : ''}
        </span>
        <span className={report.poster.issue_count ? 'badge warn' : 'badge done'}>
          海报问题 {report.poster.issue_count}
        </span>
      </div>
      {autoItems.length > 0 && (
        <div className="c115WizardDupList">
          {autoItems.slice(0, 6).map((item: AddNewAutoResolveItemReport) => (
            <div className="c115WizardDup" key={`${item.tmdb}-${item.status}-${item.removed_lib || ''}-${item.removed_folder || ''}`}>
              <div>
                <span className={item.status === 'resolved' ? 'badge done' : 'badge warn'}>
                  {item.status === 'resolved' ? '已自动清理' : item.status === 'skipped' ? '已跳过' : '处理失败'}
                </span>
                <span>tmdb {item.tmdb}</span>
              </div>
              <p>
                保留 {item.kept_folder}
                <span> [{item.kept_lib}]</span>
              </p>
              {item.removed_folder && (
                <p>
                  清理 {item.removed_folder}
                  <span> [{item.removed_lib || '旧库'}]</span>
                </p>
              )}
              <p>{item.error || item.reason}</p>
            </div>
          ))}
          {autoItems.length > 6 && <p className="settingsHint">还有 {autoItems.length - 6} 条自动处理记录未展开。</p>}
        </div>
      )}
      {groups.length > 0 && (
        <div className="c115WizardDupList">
          {groups.slice(0, 8).map((group) => {
            const candidate = smartReplaceCandidate(group);
            return (
              <div className="c115WizardDup" key={`${group.tmdb}-${dedupRows(group).map((row) => row.folder).join('|')}`}>
                <div>
                  <span className="badge warn">tmdb {group.tmdb}</span>
                  {'why' in group && <span>{group.why}</span>}
                </div>
                {dedupRows(group).map((row) => (
                  <p key={`${row.lib}-${row.folder}`}>
                    {/\(\d+\)$/.test(row.folder) ? '新' : '旧'} {row.folder}
                    <span> [{row.lib} · {row.n} 文件 · score {row.score}]</span>
                  </p>
                ))}
                {candidate ? (
                  <button
                    className="btn danger compact"
                    disabled={replacing}
                    onClick={() => onSmartReplace([{
                      lib: candidate.lib,
                      win_folder: candidate.win_folder,
                      lose_folder: candidate.lose_folder,
                      reason: `一条龙智能替换 tmdb ${candidate.tmdb}`
                    }])}
                  >
                    用新替旧
                  </button>
                ) : (
                  <span className="badge">需手动去重</span>
                )}
              </div>
            );
          })}
          {groups.length > 8 && <p className="settingsHint">还有 {groups.length - 8} 组未展开，可去智能清理/去重页处理。</p>}
        </div>
      )}
      {candidates.length > 1 && (
        <button className="btn danger compact" disabled={replacing} onClick={() => onSmartReplace(candidates)}>
          一键智能替换 {candidates.length} 组
        </button>
      )}
    </div>
  );
}

export function C115Panel() {
  const [status, setStatus] = useState<C115TestResponse | null>(null);
  const [statusError, setStatusError] = useState('');
  const [cidMap, setCidMap] = useState<Record<string, string>>({});
  const [targetLib, setTargetLib] = useState('');
  const [customCid, setCustomCid] = useState('');
  const [shareText, setShareText] = useState('');
  const [defaultPwd, setDefaultPwd] = useState('');
  const [offlineText, setOfflineText] = useState('');
  const [snap, setSnap] = useState<C115SnapResponse | null>(null);
  const [snapSource, setSnapSource] = useState<InputLine | null>(null);
  const [selectedFileIds, setSelectedFileIds] = useState<Set<string>>(() => new Set());
  const [autoCid, setAutoCid] = useState<C115AutoCidResponse | null>(null);
  const [trackedTaskIds, setTrackedTaskIds] = useState<string[]>([]);
  const [completedTasks, setCompletedTasks] = useState<TaskRun[]>([]);
  const [replacing, setReplacing] = useState(false);
  const [pendingReplace, setPendingReplace] = useState<ReplaceBatchRequest['items'] | null>(null);
  const [error, setError] = useState('');
  const [loadingMeta, setLoadingMeta] = useState(true);
  const [previewing, setPreviewing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [offlining, setOfflining] = useState(false);
  const [wizarding, setWizarding] = useState(false);
  const [wizardDelayMs, setWizardDelayMs] = useState('500');
  const [scanning, setScanning] = useState(false);
  const [autoDetecting, setAutoDetecting] = useState(false);
  const [progressText, setProgressText] = useState('');
  const toast = useToast();

  const cidEntries = useMemo(() => Object.entries(cidMap).sort(([a], [b]) => a.localeCompare(b, 'zh-CN')), [cidMap]);
  const selectableFiles = useMemo(() => snap?.files.filter((file) => file.id) || [], [snap]);
  const allFilesSelected = selectableFiles.length > 0 && selectedFileIds.size === selectableFiles.length;
  const wizardStats = useMemo(() => {
    const lines = [...parseLines(shareText), ...parseLines(offlineText)];
    const offline = lines.filter((line) => isOfflineUrl(line.url)).length;
    return { total: lines.length, share: lines.length - offline, offline };
  }, [shareText, offlineText]);
  const currentTargetLabel = customCid.trim() ? `cid ${customCid.trim()}` : targetLib ? `库「${targetLib}」` : '未选目标';
  const wizardSummary = wizardStats.total
    ? `${wizardStats.total} 项 · 分享 ${wizardStats.share} · 离线 ${wizardStats.offline} · ${currentTargetLabel}`
    : `等待链接 · ${currentTargetLabel}`;
  const busy = previewing || saving || offlining || wizarding || scanning || autoDetecting || replacing;
  const addNewReports = completedTasks.filter((task) => task.status === 'done' && isAddNewReport(task.result));

  const trackTask = (task: TaskRun) => {
    setTrackedTaskIds((prev) => (prev.includes(task.id) ? prev : [task.id, ...prev].slice(0, 20)));
  };

  useTaskCompletion(trackedTaskIds, (task) => {
    setCompletedTasks((prev) => [task, ...prev.filter((item) => item.id !== task.id)].slice(0, 8));
    if (task.kind === 'c115_auto_cid' && task.status === 'done' && isAutoCidResponse(task.result)) {
      setAutoCid(task.result);
    }
    toast.push(
      task.status === 'done' ? `任务完成：${taskSummary(task)}` : `任务结束：${taskSummary(task)} · ${task.status}`,
      task.status === 'done' ? 'ok' : 'warn'
    );
  });

  const loadMeta = async () => {
    setLoadingMeta(true);
    setError('');
    const [statusResult, configResult] = await Promise.allSettled([
      api<C115TestResponse>('/api/v2/c115/test'),
      api<ConfigResponse>('/api/v2/config')
    ]);

    if (statusResult.status === 'fulfilled') {
      setStatus(statusResult.value);
      setStatusError('');
    } else {
      setStatus(null);
      setStatusError(errorMessage(statusResult.reason));
    }

    if (configResult.status === 'fulfilled') {
      const nextCidMap = parseCidMapValue(configResult.value.settings.c115_cid_map);
      const keys = Object.keys(nextCidMap).sort((a, b) => a.localeCompare(b, 'zh-CN'));
      setCidMap(nextCidMap);
      setTargetLib((prev) => (prev && nextCidMap[prev] ? prev : keys[0] || ''));
    } else {
      const message = errorMessage(configResult.reason);
      setError(message);
      toast.push(`115 目标库加载失败：${message}`, 'error');
    }
    setLoadingMeta(false);
  };

  useEffect(() => {
    loadMeta();
  }, []);

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

  const firstShareLine = () => parseLines(shareText).find((line) => !isOfflineUrl(line.url));

  const previewShare = async (event?: FormEvent) => {
    event?.preventDefault();
    const line = firstShareLine();
    if (!line) {
      toast.push('先贴 115 分享链接', 'warn');
      return;
    }
    setPreviewing(true);
    setError('');
    setSnap(null);
    setSnapSource(null);
    try {
      const data = await api<C115SnapResponse>('/api/v2/c115/snap', {
        method: 'POST',
        body: JSON.stringify({ url: line.url, pwd: resolvePwd(line, defaultPwd) })
      });
      setSnap(data);
      setSnapSource(line);
      setSelectedFileIds(new Set(data.files.map((file) => file.id).filter((id): id is string => Boolean(id))));
      toast.push(`已读取 ${data.files.length} 项`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`预览失败：${message}`, 'error');
    } finally {
      setPreviewing(false);
    }
  };

  const toggleFile = (file: C115SnapFile) => {
    if (!file.id) return;
    setSelectedFileIds((prev) => {
      const next = new Set(prev);
      if (next.has(file.id!)) next.delete(file.id!);
      else next.add(file.id!);
      return next;
    });
  };

  const setAllFiles = (mode: 'all' | 'none' | 'invert') => {
    if (mode === 'all') {
      setSelectedFileIds(new Set(selectableFiles.map((file) => file.id!).filter(Boolean)));
      return;
    }
    if (mode === 'none') {
      setSelectedFileIds(new Set());
      return;
    }
    setSelectedFileIds((prev) => {
      const next = new Set<string>();
      for (const file of selectableFiles) {
        if (file.id && !prev.has(file.id)) next.add(file.id);
      }
      return next;
    });
  };

  const buildShareItems = (shareLines: InputLine[]): AddNewItem[] => {
    const allSelected = selectableFiles.length > 0 && selectedFileIds.size === selectableFiles.length;
    return shareLines.map((line) => {
      const canUseFileSubset =
        shareLines.length === 1 && snapSource?.url === line.url && selectableFiles.length > 0 && !allSelected;
      return {
        url: line.url,
        kind: 'share115',
        pwd: resolvePwd(line, defaultPwd),
        file_ids: canUseFileSubset ? Array.from(selectedFileIds) : undefined,
        label: snapSource?.url === line.url && snap?.share_title ? snap.share_title : line.url
      };
    });
  };

  const saveShares = async () => {
    const target = parseTarget();
    if (!target) return;
    const shareLines = parseLines(shareText).filter((line) => !isOfflineUrl(line.url));
    if (!shareLines.length) {
      toast.push('没有可转存的 115 分享链接', 'warn');
      return;
    }
    if (shareLines.length === 1 && selectableFiles.length > 0 && selectedFileIds.size === 0) {
      toast.push('至少勾选一个文件', 'warn');
      return;
    }

    setSaving(true);
    setError('');
    try {
      const delayMs = parseWizardDelay();
      if (delayMs == null) return;
      setProgressText(`创建一条龙转存任务：${shareLines.length} 项`);
      const task = await api<TaskRun>('/api/v2/wizard/add-new', {
        method: 'POST',
        body: JSON.stringify({
          items: buildShareItems(shareLines),
          delay_ms: delayMs,
          ...(target.cid ? { cid: target.cid } : { lib: target.lib }),
        })
      });
      trackTask(task);
      toast.push(`一条龙转存任务已创建：${taskSummary(task)}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`一条龙转存任务创建失败：${message}`, 'error');
    } finally {
      setSaving(false);
      setProgressText('');
    }
  };

  const createOfflineTasks = async () => {
    const target = parseTarget();
    if (!target) return;
    const lines = parseLines(offlineText).filter((line) => isOfflineUrl(line.url));
    if (!lines.length) {
      toast.push('没有可离线下载的 magnet/ed2k 链接', 'warn');
      return;
    }

    setOfflining(true);
    setError('');
    try {
      setProgressText(`创建批量离线任务：${lines.length} 项`);
      const task = await api<TaskRun>('/api/v2/c115/offline/batch', {
        method: 'POST',
        body: JSON.stringify({
          items: lines.map((line) => ({ url: line.url, label: line.url })),
          ...(target.cid ? { cid: target.cid } : { lib: target.lib }),
          label: lines.length === 1 ? lines[0]?.url : `115 批量离线 ${lines.length} 项`
        })
      });
      trackTask(task);
      toast.push(`批量离线任务已创建：${taskSummary(task)}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`批量离线任务创建失败：${message}`, 'error');
    } finally {
      setOfflining(false);
      setProgressText('');
    }
  };

  const parseWizardDelay = () => {
    const raw = wizardDelayMs.trim();
    if (!raw) return 0;
    const value = Number(raw);
    if (!Number.isFinite(value) || value < 0 || !Number.isInteger(value)) {
      toast.push('delay_ms 必须是非负整数', 'warn');
      return null;
    }
    return value;
  };

  const createAddNewWizardTask = async () => {
    const target = parseTarget();
    if (!target) return;
    const delayMs = parseWizardDelay();
    if (delayMs == null) return;

    const lines = [...parseLines(shareText), ...parseLines(offlineText)];
    if (!lines.length) {
      toast.push('先贴分享链接或离线链接', 'warn');
      return;
    }

    const shareLines = lines.filter((line) => !isOfflineUrl(line.url));
    if (
      shareLines.length === 1 &&
      snapSource?.url === shareLines[0].url &&
      selectableFiles.length > 0 &&
      selectedFileIds.size === 0
    ) {
      toast.push('预览分享至少勾选一个文件', 'warn');
      return;
    }

    const items = lines.map((line): AddNewItem => {
      const kind = inferAddNewKind(line.url);
      const isShare = kind === 'share115';
      const canUseFileSubset =
        isShare &&
        shareLines.length === 1 &&
        snapSource?.url === line.url &&
        selectableFiles.length > 0 &&
        selectedFileIds.size !== selectableFiles.length;
      const pwd = isShare ? resolvePwd(line, defaultPwd) : undefined;
      return {
        url: line.url,
        kind,
        label: isShare && snapSource?.url === line.url && snap?.share_title ? snap.share_title : line.url,
        pwd,
        file_ids: canUseFileSubset ? Array.from(selectedFileIds) : undefined
      };
    });

    const request: AddNewRequest = {
      items,
      delay_ms: delayMs,
      ...(target.cid ? { cid: target.cid } : { lib: target.lib })
    };

    setWizarding(true);
    setError('');
    try {
      const task = await api<TaskRun>('/api/v2/wizard/add-new', {
        method: 'POST',
        body: JSON.stringify(request)
      });
      trackTask(task);
      toast.push(`一条龙任务已创建：${taskSummary(task)}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`创建一条龙任务失败：${message}`, 'error');
    } finally {
      setWizarding(false);
    }
  };

  const scanTarget = async () => {
    if (!targetLib || customCid.trim()) {
      toast.push('扫库需要选择已配置的目标库，不能只填 cid', 'warn');
      return;
    }
    setScanning(true);
    setError('');
    try {
      const task = await api<TaskRun>('/api/v2/libraries/scan', {
        method: 'POST',
        body: JSON.stringify({ lib: targetLib })
      });
      trackTask(task);
      toast.push(`扫库任务已创建：${taskSummary(task)}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`创建扫库任务失败：${message}`, 'error');
    } finally {
      setScanning(false);
    }
  };

  const detectCid = async () => {
    setAutoDetecting(true);
    setAutoCid(null);
    setError('');
    try {
      const data = await api<C115AutoCidResponse>('/api/v2/c115/auto-cid', {
        method: 'POST',
        body: JSON.stringify({ max_depth: 2 })
      });
      setAutoCid(data);
      toast.push(`已扫描 ${data.scanned} 个目录`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`检测 cid 失败：${message}`, 'error');
    } finally {
      setAutoDetecting(false);
    }
  };

  const detectCidTask = async () => {
    setAutoDetecting(true);
    setError('');
    try {
      const task = await api<TaskRun>('/api/v2/c115/auto-cid/task', {
        method: 'POST',
        body: JSON.stringify({ max_depth: 2 })
      });
      trackTask(task);
      toast.push(`cid 检测任务已创建：${taskSummary(task)}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`创建 cid 检测任务失败：${message}`, 'error');
    } finally {
      setAutoDetecting(false);
    }
  };

  const requestSmartReplace = (items: ReplaceBatchRequest['items']) => {
    if (!items.length) {
      toast.push('没有可智能替换的重复项', 'warn');
      return;
    }
    setPendingReplace(items);
  };

  const runSmartReplace = async () => {
    const items = pendingReplace || [];
    if (!items.length) return;
    setReplacing(true);
    setError('');
    try {
      const task = await api<TaskRun>('/api/v2/dedup/replace-batch', {
        method: 'POST',
        body: JSON.stringify({ items })
      });
      trackTask(task);
      toast.push(`智能替换任务已创建：${taskSummary(task)}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`创建智能替换任务失败：${message}`, 'error');
    } finally {
      setReplacing(false);
      setPendingReplace(null);
    }
  };

  return (
    <section className="c115Panel">
      {pendingReplace && (
        <ConfirmDanger
          title={`确认智能替换 ${pendingReplace.length} 组`}
          confirmText="确认替换"
          onCancel={() => setPendingReplace(null)}
          onConfirm={runSmartReplace}
          body={(
            <div className="dangerCopy">
              <p>将删除旧目录到 115 回收站，并把新目录改名为旧目录名。</p>
              <code>{pendingReplace.slice(0, 3).map((item) => `${item.lib}: ${item.win_folder} -> ${item.lose_folder}`).join('\n')}</code>
            </div>
          )}
        />
      )}
      <div className="c115Meta">
        <div>
          <strong>115 状态</strong>
          <span>{status ? `UID ${status.uid} · ${status.used || '空间未知'}` : statusError || '等待检测'}</span>
        </div>
        <div className="c115MetaActions">
          <button className="btn ghost" onClick={loadMeta} disabled={loadingMeta}>
            <RefreshCw size={16} />
            {loadingMeta ? '检测中' : '刷新'}
          </button>
          <button className="btn ghost" onClick={detectCid} disabled={busy}>
            <SearchCheck size={16} />
            {autoDetecting ? '检测中' : '检测 cid'}
          </button>
          <button className="btn ghost" onClick={detectCidTask} disabled={busy}>
            <SearchCheck size={16} />
            任务检测
          </button>
        </div>
      </div>

      <div className="c115TargetBar">
        <label>
          <span>目标库</span>
          <select
            className="input"
            aria-label="115 目标库"
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
            aria-label="115 自定义 cid"
            inputMode="numeric"
            value={customCid}
            onChange={(event) => setCustomCid(event.target.value)}
            placeholder="填了就优先使用"
          />
        </label>
        <button className="btn ghost" onClick={scanTarget} disabled={busy || !targetLib || Boolean(customCid.trim())}>
          <RotateCw size={16} />
          {scanning ? '创建中' : '扫目标库'}
        </button>
      </div>

      <form className="c115ShareGrid" onSubmit={previewShare}>
        <label className="c115WideField">
          <span>分享链接</span>
          <textarea
            className="input c115Textarea"
            aria-label="115 分享链接"
            value={shareText}
            onChange={(event) => setShareText(event.target.value)}
            placeholder="一行一个 115 分享链接；行尾可跟提取码"
          />
        </label>
        <label>
          <span>默认提取码</span>
          <input
            className="input"
            aria-label="默认提取码"
            value={defaultPwd}
            onChange={(event) => setDefaultPwd(event.target.value)}
          />
        </label>
        <div className="c115ActionStack">
          <button className="btn ghost" disabled={previewing}>
            <Eye size={16} />
            {previewing ? '预览中' : '先看文件'}
          </button>
          <button type="button" className="btn" onClick={saveShares} disabled={busy}>
            <DownloadCloud size={16} />
            {saving ? '提交中' : '一条龙转存'}
          </button>
        </div>
      </form>

      {snap && (
        <div className="c115SnapBox">
          <div className="c115SnapHead">
            <strong>{snap.share_title || snap.share}</strong>
            <span>{snap.files.length} 项 · 已选 {selectedFileIds.size}/{selectableFiles.length}</span>
          </div>
          <div className="c115FileTools">
            <button className="btn ghost compact" onClick={() => setAllFiles(allFilesSelected ? 'none' : 'all')}>
              {allFilesSelected ? <CheckSquare size={15} /> : <Square size={15} />}
              {allFilesSelected ? '全不选' : '全选'}
            </button>
            <button className="btn ghost compact" onClick={() => setAllFiles('invert')}>反选</button>
          </div>
          <div className="c115FileList">
            {snap.files.map((file, index) => (
              <label key={`${file.id || file.name}-${index}`} className="c115FileRow">
                <input
                  type="checkbox"
                  checked={Boolean(file.id && selectedFileIds.has(file.id))}
                  disabled={!file.id}
                  onChange={() => toggleFile(file)}
                />
                <span>{file.is_dir ? '目录' : '文件'}</span>
                <strong>{file.name}</strong>
                <em>{humanSize(file.size)}</em>
              </label>
            ))}
          </div>
        </div>
      )}

      <div className="c115OfflineBox">
        <label>
          <span>离线链接</span>
          <textarea
            className="input c115Textarea small"
            aria-label="115 离线链接"
            value={offlineText}
            onChange={(event) => setOfflineText(event.target.value)}
            placeholder="一行一个 magnet 或 ed2k"
          />
        </label>
        <button className="btn ghost" onClick={createOfflineTasks} disabled={busy}>
          <RadioTower size={16} />
          {offlining ? '提交中' : '创建离线任务'}
        </button>
      </div>

      <div className="c115TargetBar">
        <label>
          <span>一条龙加新资源</span>
          <input className="input" aria-label="一条龙加新资源条目" readOnly value={wizardSummary} />
        </label>
        <label>
          <span>delay_ms</span>
          <input
            className="input"
            aria-label="一条龙 delay_ms"
            inputMode="numeric"
            value={wizardDelayMs}
            onChange={(event) => setWizardDelayMs(event.target.value)}
          />
        </label>
        <button className="btn" onClick={createAddNewWizardTask} disabled={busy}>
          <DownloadCloud size={16} />
          {wizarding ? '创建中' : '创建一条龙任务'}
        </button>
      </div>

      {autoCid && (
        <div className="c115AutoCid">
          <strong>扫描 {autoCid.scanned} 个目录</strong>
          <div>
            {Object.entries(autoCid.matches).map(([lib, hits]) => (
              <span key={lib} className={hits.length === 1 ? 'badge done' : 'badge warn'}>
                {lib}: {hits.length ? hits.map((hit) => hit.cid).join(' / ') : '未找到'}
              </span>
            ))}
          </div>
        </div>
      )}

      {completedTasks.length > 0 && (
        <div className="c115AutoCid">
          <strong>最近任务结果</strong>
          <div>
            {completedTasks.map((task) => (
              <span key={task.id} className={task.status === 'done' ? 'badge done' : 'badge warn'}>
                {taskSummary(task)}: {task.status}
              </span>
            ))}
          </div>
        </div>
      )}

      {addNewReports.map((task) => (
        <AddNewReportCard
          key={task.id}
          task={task}
          report={task.result as AddNewReport}
          replacing={replacing}
          onSmartReplace={requestSmartReplace}
        />
      ))}

      {progressText && <div className="notice c115Progress">{progressText}</div>}
      {error && <div className="notice warn whitespaceNotice">{error}</div>}
    </section>
  );
}
