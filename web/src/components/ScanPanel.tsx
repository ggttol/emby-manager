import { Database, FileSearch, FolderSync, RefreshCw, ScanLine } from 'lucide-react';
import { FormEvent, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useToast } from './Toast';

type EmbyLibrary = components['schemas']['EmbyLibrary'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];
type ScanLibraryRequest = components['schemas']['ScanLibraryRequest'];
type StrmListResponse = components['schemas']['StrmListResponse'];
type StrmOverview = components['schemas']['StrmOverview'];
type TaskRun = components['schemas']['TaskRun'];

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

export function ScanPanel() {
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [selectedLib, setSelectedLib] = useState('');
  const [itemId, setItemId] = useState('');
  const [recursive, setRecursive] = useState(true);
  const [full, setFull] = useState(false);
  const [strm, setStrm] = useState<StrmListResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [overviewLoading, setOverviewLoading] = useState(false);
  const [starting, setStarting] = useState<'lib' | 'all' | 'item' | null>(null);
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

  const loadLibraries = async () => {
    setLoading(true);
    setError('');
    try {
      const data = await api<LibrariesResponse>('/api/v2/libraries');
      setLibraries(data.libraries);
      const first = [...data.libraries].sort((a, b) => a.name.localeCompare(b.name, 'zh-CN'))[0];
      const nextLib = selectedLib && data.libraries.some((library) => library.name === selectedLib)
        ? selectedLib
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

  useEffect(() => {
    loadLibraries();
  }, []);

  const submitOverview = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    loadStrm(selectedLib);
  };

  const startScan = async (mode: 'lib' | 'all' | 'item') => {
    setStarting(mode);
    setError('');
    try {
      let payload: ScanLibraryRequest;
      if (mode === 'item') {
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
        payload = { recursive, full };
      }
      const task = await api<TaskRun>('/api/v2/libraries/scan', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      toast.push(`已创建扫描任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`扫描任务创建失败：${message}`, 'error');
    } finally {
      setStarting(null);
    }
  };

  return (
    <section className="scanPanel">
      <div className="scanToolbar">
        <div>
          <strong>扫描工作台</strong>
          <span>当前 Rust 版触发 Emby Refresh；生成 strm 与清孤儿仍在迁移中。</span>
        </div>
        <button className="btn ghost" onClick={loadLibraries} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新库'}
        </button>
      </div>

      <div className="notice warn scanNotice">
        旧版扫描的 115 文件遍历、strm 写入、权限修复和孤儿清理尚未启用；这里不会修改媒体文件或删除 strm。
      </div>

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
        <label className="switchRow scanSwitch">
          <input type="checkbox" checked={recursive} onChange={(event) => setRecursive(event.target.checked)} />
          <span>递归刷新</span>
        </label>
        <label className="switchRow scanSwitch">
          <input type="checkbox" checked={full} onChange={(event) => setFull(event.target.checked)} />
          <span>Full Refresh</span>
        </label>
        <button className="btn ghost" disabled={overviewLoading}>
          <FileSearch size={16} />
          {overviewLoading ? '读取中' : '刷新 strm 概览'}
        </button>
      </form>

      {error && <div className="notice warn whitespaceNotice">{error}</div>}

      <div className="scanActions">
        <button className="btn" onClick={() => startScan('lib')} disabled={starting !== null || !selectedLib}>
          <ScanLine size={16} />
          {starting === 'lib' ? '创建中' : '刷新选中库'}
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
        <article className={`statCard ${overview?.subtitle_files ? 'ok' : 'neutral'}`}>
          <div><Database /></div>
          <span>字幕</span>
          <strong>{count(overview?.subtitle_files)}</strong>
          <small>{bytes(overview?.subtitle_bytes)}</small>
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
