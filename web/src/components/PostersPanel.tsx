import { AlertTriangle, CheckCircle2, ImageOff, RefreshCw, SearchCheck } from 'lucide-react';
import { FormEvent, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useToast } from './Toast';

type EmbyLibrary = components['schemas']['EmbyLibrary'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];
type PosterDetectRequest = components['schemas']['PosterDetectRequest'];
type PosterDetectResponse = components['schemas']['PosterDetectResponse'];
type PosterSignalItem = components['schemas']['PosterSignalItem'];

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
  const [error, setError] = useState('');
  const toast = useToast();

  const sortedLibraries = useMemo(
    () => [...libraries].sort((a, b) => a.name.localeCompare(b.name, 'zh-CN')),
    [libraries]
  );
  const topItems = useMemo(() => (result?.items || []).slice(0, 200), [result]);

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
      toast.push(`海报检测完成：${count(data.total)} 条信号`, data.total ? 'warn' : 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`海报检测失败：${message}`, 'error');
    } finally {
      setScanning(false);
    }
  };

  return (
    <section className="postersPanel">
      <div className="postersToolbar">
        <div>
          <strong>海报检测工作台</strong>
          <span>当前 Rust 版只检测缺 Primary poster 和 folder tmdbid 绑定异常；搜索与批量修复尚未接入。</span>
        </div>
        <button className="btn ghost" onClick={loadLibraries} disabled={loadingLibraries}>
          <RefreshCw size={16} />
          {loadingLibraries ? '加载中' : '刷新库'}
        </button>
      </div>

      <div className="notice warn scanNotice">
        修复动作未接入 Rust：这里不会改 Emby 元数据、不会下载 poster，也不会批量 Apply。确认信号后仍需回旧版或手工处理。
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
