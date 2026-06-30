import { Database, FileSearch, FolderSync, Plus, RefreshCw, ScanLine } from 'lucide-react';
import { FormEvent, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useTaskCompletion } from '../hooks/useTaskCompletion';
import { ConfirmDanger } from './Modal';
import { TASK_COMPLETED_EVENT, type TaskCompleteDetail } from './TaskCenter';
import { useToast } from './Toast';

type EmbyLibrary = components['schemas']['EmbyLibrary'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];
type ScanLibraryRequest = components['schemas']['ScanLibraryRequest'];
type ScanLibraryResult = components['schemas']['ScanLibraryResult'];
type StrmGenerateResult = components['schemas']['StrmGenerateResult'];
type StrmListResponse = components['schemas']['StrmListResponse'];
type StrmOverview = components['schemas']['StrmOverview'];
type TaskRun = components['schemas']['TaskRun'];
type CreateLibraryRequest = components['schemas']['CreateLibraryRequest'];
type CreateLibraryResponse = components['schemas']['CreateLibraryResponse'];
type CreateLibraryType = 'movies' | 'tvshows';

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function count(value: number | null | undefined) {
  return Number(value || 0).toLocaleString('zh-CN');
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

function pathText(library?: EmbyLibrary) {
  if (!library?.paths?.length) return '未记录路径';
  return library.paths.join(' · ');
}

function overviewTone(overview: StrmOverview | null) {
  if (!overview) return 'neutral';
  if (overview.warnings.length || overview.truncated) return 'warn';
  if (overview.strm_files > 0) return 'ok';
  return 'warn';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function isStrmGenerateResult(value: unknown): value is StrmGenerateResult {
  return isRecord(value)
    && typeof value.new_count === 'number'
    && typeof value.orphans_cleaned === 'number'
    && typeof value.permissions_fixed === 'number'
    && Array.isArray(value.attention);
}

function isScanLibraryResult(value: unknown): value is ScanLibraryResult {
  return isRecord(value)
    && typeof value.triggered === 'number'
    && Array.isArray(value.items);
}

function scanResultFromTask(task: TaskRun): ScanLibraryResult | null {
  if (task.kind !== 'scan_library' || task.status !== 'done') return null;
  if (isScanLibraryResult(task.result)) return task.result;
  if (isStrmGenerateResult(task.result)) {
    return {
      ok: true,
      mode: 'strm',
      requested: task.label,
      global_refresh: false,
      triggered: 0,
      items: [],
      strm: task.result
    };
  }
  return null;
}

export function ScanPanel() {
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [selectedLib, setSelectedLib] = useState('');
  const [itemId, setItemId] = useState('');
  const [keyword, setKeyword] = useState('');
  const [newLibraryName, setNewLibraryName] = useState('');
  const [newLibraryType, setNewLibraryType] = useState<CreateLibraryType>('movies');
  const [recursive, setRecursive] = useState(true);
  const [full, setFull] = useState(false);
  const [fullauto, setFullauto] = useState(false);
  const [cleanupOrphans, setCleanupOrphans] = useState(false);
  const [strm, setStrm] = useState<StrmListResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [overviewLoading, setOverviewLoading] = useState(false);
  const [creatingLibrary, setCreatingLibrary] = useState(false);
  const [starting, setStarting] = useState<'lib' | 'all' | 'item' | 'strm' | null>(null);
  const [confirmCleanupScan, setConfirmCleanupScan] = useState(false);
  const [latestScan, setLatestScan] = useState<{ task: TaskRun; result: ScanLibraryResult } | null>(null);
  const [trackedTaskIds, setTrackedTaskIds] = useState<string[]>([]);
  const [libraryWarnings, setLibraryWarnings] = useState<string[]>([]);
  const [error, setError] = useState('');
  const toast = useToast();

  const selectedLibrary = useMemo(
    () => libraries.find((library) => library.name === selectedLib) || libraries[0],
    [libraries, selectedLib]
  );
  const overview = strm?.overview || null;
  const sortedLibraries = useMemo(
    () => [...libraries].sort((a, b) => a.name.localeCompare(b.name, 'zh-CN')),
    [libraries]
  );

  const loadLibraries = async (preferredLib?: string) => {
    setLoading(true);
    setError('');
    try {
      const data = await api<LibrariesResponse>('/api/v2/libraries');
      setLibraries(data.libraries);
      const first = [...data.libraries].sort((a, b) => a.name.localeCompare(b.name, 'zh-CN'))[0];
      const targetLib = (preferredLib ?? selectedLib).trim();
      const nextLib = targetLib && data.libraries.some((library) => library.name === targetLib)
        ? targetLib
        : first?.name || '';
      setSelectedLib(nextLib);
      await loadStrm(nextLib, false);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`库列表加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  const loadStrm = async (lib = selectedLib, showToast = true) => {
    const trimmed = lib.trim();
    setOverviewLoading(true);
    setError('');
    try {
      const params = new URLSearchParams({
        overview: 'true',
        overview_depth: '8',
        sample_limit: '30',
        limit: '80'
      });
      if (trimmed) params.set('lib', trimmed);
      const data = await api<StrmListResponse>(`/api/v2/libraries/strm?${params.toString()}`);
      setStrm(data);
      if (showToast) toast.push('strm 概览已刷新', 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`strm 概览加载失败：${message}`, 'error');
    } finally {
      setOverviewLoading(false);
    }
  };

  const trackTask = (task: TaskRun) => {
    setTrackedTaskIds((prev) => (prev.includes(task.id) ? prev : [task.id, ...prev].slice(0, 20)));
  };

  useEffect(() => {
    loadLibraries();
  }, []);

  useEffect(() => {
    const refreshStrmAfterTask = async (lib: string) => {
      const trimmed = lib.trim();
      if (!trimmed) return;
      setOverviewLoading(true);
      setError('');
      try {
        const params = new URLSearchParams({
          lib: trimmed,
          overview: 'true',
          overview_depth: '8',
          sample_limit: '30',
          limit: '80'
        });
        const data = await api<StrmListResponse>(`/api/v2/libraries/strm?${params.toString()}`);
        setStrm(data);
      } catch (e) {
        const message = errorMessage(e);
        setError(message);
        toast.push(`strm 概览加载失败：${message}`, 'error');
      } finally {
        setOverviewLoading(false);
      }
    };

    const onTaskCompleted = (event: Event) => {
      const detail = (event as CustomEvent<TaskCompleteDetail>).detail;
      if (!detail?.task) return;
      const result = scanResultFromTask(detail.task);
      if (!result) return;
      setLatestScan({ task: detail.task, result });
      if (result.strm?.lib) {
        void refreshStrmAfterTask(result.strm.lib);
      }
    };

    window.addEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
    return () => window.removeEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
  }, [toast]);

  useTaskCompletion(trackedTaskIds, (task) => {
    const result = scanResultFromTask(task);
    if (result) {
      setLatestScan({ task, result });
      if (result.strm?.lib) {
        void loadStrm(result.strm.lib, false);
      }
    }
    toast.push(
      task.status === 'done' ? `扫描任务完成：${task.label || task.kind}` : `扫描任务结束：${task.label || task.kind} · ${task.status}`,
      task.status === 'done' ? 'ok' : 'warn'
    );
  });

  const submitOverview = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    loadStrm(selectedLib);
  };

  const submitCreateLibrary = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = newLibraryName.trim();
    if (!name) {
      toast.push('先填写媒体库名称', 'warn');
      return;
    }
    setCreatingLibrary(true);
    setError('');
    setLibraryWarnings([]);
    try {
      const data = await api<CreateLibraryResponse>('/api/v2/libraries', {
        method: 'POST',
        body: JSON.stringify({ name, collection_type: newLibraryType } satisfies CreateLibraryRequest)
      });
      if (data.ok === false) {
        throw new Error('Emby 媒体库创建失败');
      }
      const createdName = data.library?.name || data.name || name;
      const warnings = Array.isArray(data.warnings) ? data.warnings : [];
      setLibraryWarnings(warnings);
      await loadLibraries(createdName);
      setNewLibraryName('');
      toast.push(
        warnings.length ? `已创建媒体库：${createdName}，有 ${warnings.length} 条警告` : `已创建媒体库：${createdName}`,
        warnings.length ? 'warn' : 'ok'
      );
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`媒体库创建失败：${message}`, 'error');
    } finally {
      setCreatingLibrary(false);
    }
  };

  const createScanTask = async (mode: 'lib' | 'all' | 'item' | 'strm') => {
    setStarting(mode);
    setError('');
    try {
      let payload: ScanLibraryRequest;
      if (mode === 'strm') {
        if (!selectedLib) {
          toast.push('先选择一个库', 'warn');
          return;
        }
        payload = {
          lib: selectedLib,
          recursive,
          full,
          generate_strm: true,
          force_refresh: true,
          keyword: keyword.trim() || null,
          fullauto,
          cleanup_orphans: cleanupOrphans
        };
      } else if (mode === 'item') {
        const id = itemId.trim();
        if (!id) {
          toast.push('先填写 Emby ItemId', 'warn');
          return;
        }
        payload = { item_id: id, lib: selectedLib || undefined, recursive, full };
      } else if (mode === 'lib') {
        if (!selectedLib) {
          toast.push('先选择一个库', 'warn');
          return;
        }
        payload = { lib: selectedLib, recursive, full };
      } else {
        payload = {
          recursive,
          full,
          generate_strm: true,
          fullauto,
          cleanup_orphans: cleanupOrphans
        };
      }
      const endpoint = mode === 'all' ? '/api/v2/libraries/scan-all' : '/api/v2/libraries/scan';
      const task = await api<TaskRun>(endpoint, {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      trackTask(task);
      toast.push(`已创建扫描任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`扫描任务创建失败：${message}`, 'error');
    } finally {
      setStarting(null);
    }
  };

  const startScan = (mode: 'lib' | 'all' | 'item' | 'strm') => {
    if (mode === 'strm' && cleanupOrphans) {
      if (!selectedLib) {
        toast.push('先选择一个库', 'warn');
        return;
      }
      setConfirmCleanupScan(true);
      return;
    }
    void createScanTask(mode);
  };

  const latestStrm = latestScan?.result.strm || null;
  const latestAttention = latestStrm?.attention || [];
  const latestItems = latestScan?.result.items || [];

  return (
    <section className="scanPanel">
      {confirmCleanupScan && (
        <ConfirmDanger
          title="确认清理孤儿 STRM"
          confirmText="确认生成并清理"
          onCancel={() => setConfirmCleanupScan(false)}
          onConfirm={() => {
            setConfirmCleanupScan(false);
            void createScanTask('strm');
          }}
          body={(
            <div className="dangerCopy">
              <p>将生成缺失 STRM，并真实删除当前扫描范围内识别为孤儿的 STRM 文件；请先核对目标库、关键词和 STRM 根目录。</p>
              <code>{selectedLib || '未选择库'}{keyword.trim() ? ` · ${keyword.trim()}` : ''}</code>
            </div>
          )}
        />
      )}
      <div className="scanToolbar">
        <div>
          <strong>扫描工作台</strong>
          <span>扫描入库会补缺失 STRM 并刷新 Emby；仅 Emby 刷新不会读取 115，也不会生成 STRM。</span>
        </div>
        <button className="btn ghost" onClick={() => loadLibraries()} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新库'}
        </button>
      </div>

      <div className="notice warn scanNotice">
        默认生成缺失 STRM 只新增 .strm、不覆盖已有文件；勾选清理孤儿后会真实删除已判定为孤儿的 STRM，提交前会再次确认。
      </div>

      <form className="scanGrid createLibraryForm" onSubmit={submitCreateLibrary}>
        <label>
          <span>新建库名</span>
          <input
            className="input"
            aria-label="新建媒体库名称"
            value={newLibraryName}
            onChange={(event) => setNewLibraryName(event.target.value)}
            placeholder="例如：电影 4K"
            disabled={creatingLibrary}
          />
        </label>
        <label>
          <span>库类型</span>
          <select
            className="input"
            aria-label="新建媒体库类型"
            value={newLibraryType}
            onChange={(event) => setNewLibraryType(event.target.value as CreateLibraryType)}
            disabled={creatingLibrary}
          >
            <option value="movies">电影</option>
            <option value="tvshows">剧集</option>
          </select>
        </label>
        <button className="btn" disabled={creatingLibrary || !newLibraryName.trim()}>
          <Plus size={16} />
          {creatingLibrary ? '创建中' : '创建媒体库'}
        </button>
      </form>

      {libraryWarnings.length > 0 && (
        <div className="notice warn whitespaceNotice">
          {libraryWarnings.map((warning) => <div key={warning}>{warning}</div>)}
        </div>
      )}

      <form className="scanGrid" onSubmit={submitOverview}>
        <label>
          <span>目标库</span>
          <select
            className="input"
            aria-label="扫描目标库"
            value={selectedLib}
            onChange={(event) => {
              setSelectedLib(event.target.value);
              loadStrm(event.target.value, false);
            }}
            disabled={!sortedLibraries.length}
          >
            {sortedLibraries.length === 0 && <option value="">未读取到库</option>}
            {sortedLibraries.map((library) => (
              <option key={library.id || library.name} value={library.name}>{library.name}</option>
            ))}
          </select>
        </label>
        <label>
          <span>Emby ItemId</span>
          <input
            className="input"
            aria-label="Emby ItemId"
            value={itemId}
            onChange={(event) => setItemId(event.target.value)}
            placeholder="可选，精确刷新单项"
          />
        </label>
        <label>
          <span>目录关键词</span>
          <input
            className="input"
            aria-label="扫描目录关键词"
            value={keyword}
            onChange={(event) => setKeyword(event.target.value)}
            placeholder="可选，只处理匹配的顶层目录"
          />
        </label>
        <label className="switchRow scanSwitch">
          <input type="checkbox" checked={recursive} onChange={(event) => setRecursive(event.target.checked)} />
          <span>递归刷新</span>
        </label>
        <label className="switchRow scanSwitch">
          <input type="checkbox" checked={full} onChange={(event) => setFull(event.target.checked)} />
          <span>Full Refresh</span>
        </label>
        <label className="switchRow scanSwitch">
          <input type="checkbox" checked={fullauto} onChange={(event) => setFullauto(event.target.checked)} />
          <span>首次无 tmdbid 也生成</span>
        </label>
        <label className="switchRow scanSwitch">
          <input type="checkbox" checked={cleanupOrphans} onChange={(event) => setCleanupOrphans(event.target.checked)} />
          <span>清理孤儿 STRM（危险）</span>
        </label>
        <button className="btn ghost" disabled={overviewLoading}>
          <FileSearch size={16} />
          {overviewLoading ? '读取中' : '刷新 strm 概览'}
        </button>
      </form>

      {error && <div className="notice warn whitespaceNotice">{error}</div>}

      <div className="scanActions">
        <button className="btn" onClick={() => startScan('strm')} disabled={starting !== null || !selectedLib}>
          <FolderSync size={16} />
          {starting === 'strm' ? '创建中' : '扫描入库'}
        </button>
        <button className="btn ghost" onClick={() => startScan('lib')} disabled={starting !== null || !selectedLib}>
          <ScanLine size={16} />
          {starting === 'lib' ? '创建中' : '仅 Emby 刷新'}
        </button>
        <button className="btn ghost" onClick={() => startScan('all')} disabled={starting !== null}>
          <FolderSync size={16} />
          {starting === 'all' ? '创建中' : '刷新全部库'}
        </button>
        <button className="btn ghost" onClick={() => startScan('item')} disabled={starting !== null || !itemId.trim()}>
          <Database size={16} />
          {starting === 'item' ? '创建中' : '刷新 ItemId'}
        </button>
      </div>

      <section className="scanLibraryCard">
        <div>
          <strong>{selectedLibrary?.name || selectedLib || '未选择库'}</strong>
          <span>{selectedLibrary?.type || 'unknown'} · {selectedLibrary?.id || '无 ItemId'}</span>
        </div>
        <code>{pathText(selectedLibrary)}</code>
      </section>

      {latestScan && (
        <>
          <section className="scanLibraryCard">
            <div>
              <strong>最近扫描结果</strong>
              <span>{latestScan.task.label || latestScan.task.kind} · {latestScan.result.mode}</span>
            </div>
            <code>{latestScan.result.requested || latestStrm?.lib || 'scan_library'}</code>
          </section>
          <div className="statGrid">
            <article className={`statCard ${latestScan.result.triggered ? 'ok' : 'neutral'}`}>
              <div><RefreshCw /></div>
              <span>Emby Refresh</span>
              <strong>{count(latestScan.result.triggered)}</strong>
              <small>{latestScan.result.global_refresh ? '全局刷新' : '按项刷新'}</small>
            </article>
            <article className={`statCard ${latestItems.length ? 'ok' : 'neutral'}`}>
              <div><Database /></div>
              <span>Items</span>
              <strong>{count(latestItems.length)}</strong>
              <small>返回条目</small>
            </article>
            <article className={`statCard ${latestStrm?.new_count ? 'ok' : 'neutral'}`}>
              <div><FolderSync /></div>
              <span>新增 STRM</span>
              <strong>{count(latestStrm?.new_count)}</strong>
              <small>{latestStrm ? `匹配 ${count(latestStrm.matched)}` : '未生成'}</small>
            </article>
            <article className={`statCard ${(latestStrm?.orphans_cleaned || latestStrm?.permissions_fixed) ? 'warn' : 'ok'}`}>
              <div><FileSearch /></div>
              <span>清孤儿 / 权限</span>
              <strong>{count(latestStrm?.orphans_cleaned)} / {count(latestStrm?.permissions_fixed)}</strong>
              <small>{latestStrm?.orphan_cleanup_skipped ? '清孤儿跳过' : '已执行结果'}</small>
            </article>
          </div>
          <div className="scanSplit">
            <section className="readonlyBlock">
              <h2>Items</h2>
              <div className="scanList">
                {latestItems.slice(0, 30).map((item) => (
                  <article key={`${item.id || item.name}-${item.code}`}>
                    <span className={`badge ${item.code >= 400 ? 'error' : 'done'}`}>{item.code}</span>
                    <strong>{item.name || item.id || '未命名条目'}</strong>
                    <small>{item.id || '无 ItemId'}</small>
                  </article>
                ))}
                {latestItems.length > 30 && <div className="empty inlineEmpty">仅显示前 30 条，共 {count(latestItems.length)} 条</div>}
                {latestItems.length === 0 && <div className="empty inlineEmpty">这次扫描没有返回 Item 条目</div>}
              </div>
            </section>
            <section className="readonlyBlock">
              <h2>Attention</h2>
              <div className="scanList">
                {latestAttention.map((message) => (
                  <article key={message}>
                    <span className="badge warn">注意</span>
                    <strong>{message}</strong>
                    <small>{latestStrm?.lib || selectedLib || 'scan_library'}</small>
                  </article>
                ))}
                {latestAttention.length === 0 && <div className="empty inlineEmpty">没有注意项</div>}
              </div>
            </section>
          </div>
        </>
      )}

      <div className="statGrid">
        <article className={`statCard ${overviewTone(overview)}`}>
          <div><FileSearch /></div>
          <span>.strm</span>
          <strong>{count(overview?.strm_files)}</strong>
          <small>{bytes(overview?.strm_bytes)}</small>
        </article>
        <article className="statCard neutral">
          <div><FolderSync /></div>
          <span>目录 / 文件</span>
          <strong>{count(overview?.directories)} / {count(overview?.files)}</strong>
          <small>深度 {count(overview?.max_depth)}</small>
        </article>
        <article className={`statCard ${overview?.warnings.length || overview?.truncated ? 'warn' : 'ok'}`}>
          <div><RefreshCw /></div>
          <span>概览状态</span>
          <strong>{overview?.truncated ? '截断' : '正常'}</strong>
          <small>上限 {count(overview?.entry_limit)}</small>
        </article>
      </div>

      {(overview?.warnings || []).length > 0 && (
        <div className="notice warn whitespaceNotice">
          {overview?.warnings.map((warning) => <div key={warning}>{warning}</div>)}
        </div>
      )}

      <div className="scanSplit">
        <section className="readonlyBlock">
          <h2>strm 条目</h2>
          <div className="scanList">
            {(strm?.items || []).map((item) => (
              <article key={`${item.rel_path}-${item.is_dir}`}>
                <span className={`badge ${item.is_dir ? 'pending' : 'done'}`}>{item.is_dir ? '目录' : 'strm'}</span>
                <strong>{item.rel_path}</strong>
                <small>{item.name} · {item.is_dir ? 'folder' : bytes(item.size)}</small>
              </article>
            ))}
            {strm && strm.items.length === 0 && <div className="empty inlineEmpty">没有找到 strm 条目</div>}
            {!strm && <div className="empty inlineEmpty">等待 strm 概览</div>}
          </div>
        </section>
        <section className="readonlyBlock">
          <h2>样例</h2>
          <div className="scanList">
            {(overview?.samples || []).map((sample) => (
              <article key={`${sample.kind}-${sample.rel_path}`}>
                <span className="badge">{sample.kind}</span>
                <strong>{sample.rel_path}</strong>
                <small>.{sample.extension || 'unknown'} · {bytes(sample.size)}</small>
              </article>
            ))}
            {overview && overview.samples.length === 0 && <div className="empty inlineEmpty">没有样例</div>}
            {!overview && <div className="empty inlineEmpty">等待样例</div>}
          </div>
        </section>
      </div>
    </section>
  );
}
