import { AlertTriangle, CheckCircle2, ImageOff, RefreshCw, SearchCheck, Wand2 } from 'lucide-react';
import { FormEvent, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useTaskCompletion } from '../hooks/useTaskCompletion';
import { useToast } from './Toast';

type EmbyLibrary = components['schemas']['EmbyLibrary'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];
type PosterDetectRequest = components['schemas']['PosterDetectRequest'];
type PosterDetectResponse = components['schemas']['PosterDetectResponse'];
type PosterApplyResponse = components['schemas']['PosterApplyResponse'];
type PosterSearchCandidate = components['schemas']['PosterSearchCandidate'];
type PosterSearchResponse = components['schemas']['PosterSearchResponse'];
type PosterSignalItem = components['schemas']['PosterSignalItem'];
type TaskRun = components['schemas']['TaskRun'];

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function count(value: number | null | undefined) {
  return Number(value || 0).toLocaleString('zh-CN');
}

function signalTone(item: PosterSignalItem) {
  if (item.signals.some((signal) => signal.severity === 'danger')) return 'error';
  if (item.signals.length || !item.has_poster) return 'warn';
  return 'ok';
}

function posterState(item: PosterSignalItem) {
  return item.has_poster ? '已有 poster' : '缺 Primary';
}

function tmdbText(value?: string | null) {
  return value && value.trim() ? value : '未绑定';
}

export function PostersPanel() {
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [selectedLib, setSelectedLib] = useState('');
  const [limit, setLimit] = useState(30000);
  const [includeMissing, setIncludeMissing] = useState(true);
  const [result, setResult] = useState<PosterDetectResponse | null>(null);
  const [loadingLibraries, setLoadingLibraries] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [rowLoading, setRowLoading] = useState<string | null>(null);
  const [batchStarting, setBatchStarting] = useState(false);
  const [candidates, setCandidates] = useState<Record<string, PosterSearchCandidate[]>>({});
  const [manualTmdb, setManualTmdb] = useState<Record<string, string>>({});
  const [trackedTaskIds, setTrackedTaskIds] = useState<string[]>([]);
  const [completedTasks, setCompletedTasks] = useState<TaskRun[]>([]);
  const [error, setError] = useState('');
  const toast = useToast();

  const sortedLibraries = useMemo(
    () => [...libraries].sort((a, b) => a.name.localeCompare(b.name, 'zh-CN')),
    [libraries]
  );
  const topItems = useMemo(() => (result?.items || []).slice(0, 200), [result]);

  const trackTask = (task: TaskRun) => {
    setTrackedTaskIds((prev) => (prev.includes(task.id) ? prev : [task.id, ...prev].slice(0, 20)));
  };

  const loadLibraries = async () => {
    setLoadingLibraries(true);
    setError('');
    try {
      const data = await api<LibrariesResponse>('/api/v2/libraries');
      setLibraries(data.libraries);
      if (selectedLib && !data.libraries.some((library) => library.name === selectedLib || library.id === selectedLib)) {
        setSelectedLib('');
      }
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`海报库列表加载失败：${message}`, 'error');
    } finally {
      setLoadingLibraries(false);
    }
  };

  useEffect(() => {
    loadLibraries();
  }, []);

  const runDetect = async (event?: FormEvent<HTMLFormElement>) => {
    event?.preventDefault();
    setScanning(true);
    setError('');
    try {
      const payload: PosterDetectRequest = {
        lib: selectedLib || null,
        limit,
        include_missing_primary: includeMissing
      };
      const data = await api<PosterDetectResponse>('/api/v2/posters/detect-mismatch', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setResult(data);
      setCandidates({});
      setManualTmdb({});
      toast.push(`海报检测完成：${count(data.total)} 条信号`, data.total ? 'warn' : 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`海报检测失败：${message}`, 'error');
    } finally {
      setScanning(false);
    }
  };

  useTaskCompletion(trackedTaskIds, (task) => {
    setCompletedTasks((prev) => [task, ...prev.filter((item) => item.id !== task.id)].slice(0, 6));
    if (task.kind === 'poster_fix_batch' && task.status === 'done') {
      void runDetect();
    }
    toast.push(
      task.status === 'done' ? `海报任务完成：${task.label || task.kind}` : `海报任务结束：${task.label || task.kind} · ${task.status}`,
      task.status === 'done' ? 'ok' : 'warn'
    );
  });

  const searchName = (item: PosterSignalItem) => item.folder_clean || item.folder || item.name || item.emby_name;

  const searchItem = async (item: PosterSignalItem) => {
    setRowLoading(item.id);
    setError('');
    try {
      const data = await api<PosterSearchResponse>('/api/v2/posters/search', {
        method: 'POST',
        body: JSON.stringify({
          id: item.id,
          name: searchName(item),
          type: item.type,
          limit: 8
        })
      });
      const sorted = [...data.candidates].sort((a, b) => Number(Boolean(b.img)) - Number(Boolean(a.img)));
      setCandidates((prev) => ({ ...prev, [item.id]: sorted }));
      toast.push(sorted.length ? `找到 ${sorted.length} 个候选` : '没有搜到候选', sorted.length ? 'ok' : 'warn');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`海报搜索失败：${message}`, 'error');
    } finally {
      setRowLoading(null);
    }
  };

  const removeResolvedItem = (item: PosterSignalItem) => {
    setResult((prev) => {
      if (!prev) return prev;
      const wasMismatch = item.signals.some((signal) => signal.kind !== 'missing_primary');
      return {
        ...prev,
        total: Math.max(0, prev.total - 1),
        missing_primary_total: item.has_poster ? prev.missing_primary_total : Math.max(0, prev.missing_primary_total - 1),
        mismatch_total: wasMismatch ? Math.max(0, prev.mismatch_total - 1) : prev.mismatch_total,
        items: prev.items.filter((row) => row.id !== item.id)
      };
    });
    setCandidates((prev) => {
      const next = { ...prev };
      delete next[item.id];
      return next;
    });
  };

  const applyItem = async (item: PosterSignalItem, tmdb: string) => {
    const value = tmdb.trim();
    if (!value) {
      toast.push('先选择或填写 tmdbid', 'warn');
      return;
    }
    setRowLoading(item.id);
    setError('');
    try {
      const data = await api<PosterApplyResponse>('/api/v2/posters/apply', {
        method: 'POST',
        body: JSON.stringify({
          id: item.id,
          tmdb: value,
          type: item.type,
          name: item.name || item.emby_name
        })
      });
      if (data.poster) {
        toast.push(`已修复「${data.name || item.name}」海报`, 'ok');
        removeResolvedItem(item);
      } else {
        toast.push(`已绑定 ${data.tmdb}，但 Primary poster 还没拉到`, 'warn');
      }
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`海报应用失败：${message}`, 'error');
    } finally {
      setRowLoading(null);
    }
  };

  const startBatch = async () => {
    const items = (result?.items || []).filter((item) => item.id);
    if (!items.length) {
      toast.push('当前没有可批量处理的条目', 'warn');
      return;
    }
    const groups = items.reduce<Record<string, string[]>>((acc, item) => {
      const kind = item.type === 'Series' ? 'Series' : 'Movie';
      acc[kind] = [...(acc[kind] || []), item.id];
      return acc;
    }, {});
    setBatchStarting(true);
    setError('');
    try {
      const tasks: TaskRun[] = [];
      for (const [kind, ids] of Object.entries(groups)) {
        const task = await api<TaskRun>('/api/v2/posters/fix-batch', {
          method: 'POST',
          body: JSON.stringify({ ids, type: kind })
        });
        trackTask(task);
        tasks.push(task);
      }
      toast.push(`已创建 ${tasks.length} 个海报批量任务，打开任务中心查看进度`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`批量海报任务创建失败：${message}`, 'error');
    } finally {
      setBatchStarting(false);
    }
  };

  return (
    <section className="postersPanel">
      <div className="postersToolbar">
        <div>
          <strong>海报检测工作台</strong>
          <span>检测缺 Primary poster 和 folder tmdbid 绑定异常，可重搜候选、手动绑定或启动批量自动修复。</span>
        </div>
        <div className="postersToolbarActions">
          <button className="btn ghost" onClick={loadLibraries} disabled={loadingLibraries}>
            <RefreshCw size={16} />
            {loadingLibraries ? '加载中' : '刷新库'}
          </button>
          <button className="btn ghost" onClick={startBatch} disabled={batchStarting || !(result?.items || []).length}>
            <Wand2 size={16} />
            {batchStarting ? '创建中' : '批量自动修复'}
          </button>
        </div>
      </div>

      <div className="notice warn scanNotice">
        Apply 会改 Emby ProviderIds.Tmdb 并触发 FullRefresh；Rust 会先写 rebind undo 记录。批量自动修复按旧版保守策略串行执行，只接受名字匹配且带图的候选。
      </div>

      <form className="postersFilterBar" onSubmit={runDetect}>
        <label>
          <span>目标库</span>
          <select
            className="input"
            aria-label="海报目标库"
            value={selectedLib}
            onChange={(event) => setSelectedLib(event.target.value)}
          >
            <option value="">全部库</option>
            {sortedLibraries.map((library) => (
              <option key={library.id || library.name} value={library.name}>
                {library.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>扫描上限</span>
          <input
            className="input"
            aria-label="海报扫描上限"
            type="number"
            min={1}
            max={100000}
            value={limit}
            onChange={(event) => setLimit(Math.max(1, Number(event.target.value) || 1))}
          />
        </label>
        <label className="switchRow postersSwitch">
          <input
            aria-label="包含缺 Primary poster"
            type="checkbox"
            checked={includeMissing}
            onChange={(event) => setIncludeMissing(event.target.checked)}
          />
          <span>包含缺 Primary poster</span>
        </label>
        <button className="btn" disabled={scanning}>
          <SearchCheck size={16} />
          {scanning ? '检测中' : '开始检测'}
        </button>
      </form>

      {error && <div className="notice warn whitespaceNotice">{error}</div>}

      {completedTasks.length > 0 && (
        <div className="notice scanNotice">
          {completedTasks.map((task) => (
            <div key={task.id}>{task.label || task.kind} · {task.status}</div>
          ))}
        </div>
      )}

      <div className="statGrid">
        <article className={`statCard ${result?.total ? 'warn' : 'ok'}`}>
          <div><AlertTriangle /></div>
          <span>信号总数</span>
          <strong>{count(result?.total)}</strong>
          <small>{result?.truncated ? '结果已截断' : '完整结果'}</small>
        </article>
        <article className={`statCard ${result?.mismatch_total ? 'error' : 'neutral'}`}>
          <div><SearchCheck /></div>
          <span>错绑 / 未绑定</span>
          <strong>{count(result?.mismatch_total)}</strong>
          <small>tmdbid 对照</small>
        </article>
        <article className={`statCard ${result?.missing_primary_total ? 'warn' : 'neutral'}`}>
          <div><ImageOff /></div>
          <span>缺 Primary</span>
          <strong>{count(result?.missing_primary_total)}</strong>
          <small>{includeMissing ? '已纳入结果' : '仅统计'}</small>
        </article>
        <article className="statCard neutral">
          <div><CheckCircle2 /></div>
          <span>已扫描</span>
          <strong>{count(result?.scanned_items)}</strong>
          <small>{count(result?.scanned_libraries)} 个库</small>
        </article>
      </div>

      {(result?.warnings || []).length > 0 && (
        <div className="notice warn whitespaceNotice">
          {result?.warnings.map((warning) => <div key={warning}>{warning}</div>)}
        </div>
      )}

      <section className="readonlyBlock">
        <div className="postersResultHead">
          <h2>检测结果</h2>
          <span>最多显示前 {count(topItems.length)} / {count(result?.items.length)} 条</span>
        </div>
        <div className="postersTableWrap">
          <table className="dataTable postersTable">
            <thead>
              <tr>
                <th>分数</th>
                <th>条目</th>
                <th>目录</th>
                <th>Tmdb</th>
                <th>信号</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              {topItems.map((item) => (
                <tr key={item.id || `${item.lib}-${item.folder}-${item.emby_name}`}>
                  <td>
                    <span className={`badge ${signalTone(item)}`}>{item.score}</span>
                  </td>
                  <td>
                    <strong>{item.emby_name || item.name}</strong>
                    <small>{item.lib} · {item.type} · {posterState(item)}</small>
                    {item.path && <code>{item.path}</code>}
                  </td>
                  <td>
                    <strong>{item.folder}</strong>
                    <small>{item.folder_clean || '未提取清洗名称'}</small>
                  </td>
                  <td>
                    <span>Emby: {tmdbText(item.tmdb)}</span>
                    <small>folder: {tmdbText(item.declared_tmdb)}</small>
                  </td>
                  <td>
                    <div className="signalList">
                      {item.signals.map((signal, index) => (
                        <span className={`badge ${signal.severity === 'danger' ? 'error' : 'pending'}`} key={`${signal.kind}-${index}`}>
                          {signal.message}
                        </span>
                      ))}
                      {item.reasons.map((reason) => <small key={reason}>{reason}</small>)}
                    </div>
                  </td>
                  <td>
                    <div className="posterActions">
                      <button className="btn compact ghost" onClick={() => searchItem(item)} disabled={rowLoading === item.id || !item.id}>
                        {rowLoading === item.id ? '处理中' : '重搜候选'}
                      </button>
                      <div className="posterManualApply">
                        <input
                          className="input"
                          aria-label={`${item.emby_name} 手动 tmdbid`}
                          value={manualTmdb[item.id] || ''}
                          onChange={(event) => setManualTmdb((prev) => ({ ...prev, [item.id]: event.target.value }))}
                          placeholder="tmdbid"
                        />
                        <button className="btn compact" onClick={() => applyItem(item, manualTmdb[item.id] || '')} disabled={rowLoading === item.id}>
                          绑定
                        </button>
                      </div>
                      {(candidates[item.id] || []).length > 0 && (
                        <div className="posterCandidates">
                          {(candidates[item.id] || []).map((candidate) => (
                            <button
                              className="posterCandidate"
                              key={`${item.id}-${candidate.tmdb}-${candidate.name}`}
                              onClick={() => applyItem(item, candidate.tmdb)}
                              disabled={rowLoading === item.id || !candidate.tmdb}
                            >
                              {candidate.img ? <img src={candidate.img} alt="" loading="lazy" /> : <span />}
                              <strong>{candidate.name || '未命名'} {candidate.year || ''}</strong>
                              <small>tmdb:{candidate.tmdb || '无'} · {candidate.img ? '有图' : '无图'}</small>
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {result && result.items.length === 0 && <div className="empty inlineEmpty">没有发现海报或 tmdb 绑定信号</div>}
          {!result && <div className="empty inlineEmpty">选择范围后开始检测</div>}
        </div>
      </section>
    </section>
  );
}
