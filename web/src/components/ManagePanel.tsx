import { ArrowRight, CheckCircle2, FileWarning, FolderInput, ListChecks, Plus, RefreshCw, RotateCcw, Search, Trash2 } from 'lucide-react';
import { FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { ConfirmDanger } from './Modal';
import { TASK_COMPLETED_EVENT, type TaskCompleteDetail } from './TaskCenter';
import { useToast } from './Toast';

type EmbyLibrary = components['schemas']['EmbyLibrary'];
type LibraryItemEntry = components['schemas']['LibraryItemEntry'];
type LibraryItemsResponse = components['schemas']['LibraryItemsResponse'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];
type ManageDeleteBatchRequest = components['schemas']['ManageDeleteBatchRequest'];
type ManageDeleteRequest = components['schemas']['ManageDeleteRequest'];
type ManageMoveRequest = components['schemas']['ManageMoveRequest'];
type TaskRun = components['schemas']['TaskRun'];
type UndoEntry = components['schemas']['UndoEntry'];
type UndoListResponse = components['schemas']['UndoListResponse'];

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function count(value: number | null | undefined) {
  return Number(value || 0).toLocaleString('zh-CN');
}

function formatDate(value: string) {
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

function stringify(value: unknown) {
  if (value == null) return '';
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function opLabel(op: string) {
  if (op === 'delete') return '删除';
  if (op === 'move') return '移动';
  if (op === 'replace') return '替换';
  if (op === 'smart_archive') return '智能归档';
  if (op === 'rebind') return '海报重绑';
  return op || '未知';
}

function libraryOptions(libraries: EmbyLibrary[]) {
  return [...libraries].sort((a, b) => a.name.localeCompare(b.name, 'zh-CN'));
}

const manageRefreshKinds = new Set([
  'manage_delete_execute',
  'manage_delete_batch_execute',
  'manage_move_execute'
]);

function shouldRefreshManage(task: TaskRun) {
  return task.status === 'done' && manageRefreshKinds.has(task.kind);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function optionalText(value: unknown) {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function parseBatchJsonItem(value: unknown, index: number): ManageDeleteRequest {
  if (!isRecord(value)) throw new Error(`第 ${index + 1} 项不是对象`);
  const lib = optionalText(value.lib);
  const folder = optionalText(value.folder);
  if (!lib || !folder) throw new Error(`第 ${index + 1} 项缺少 lib 或 folder`);
  return {
    lib,
    folder,
    item_id: optionalText(value.item_id),
    reason: optionalText(value.reason)
  };
}

function parseBatchLine(line: string, index: number): ManageDeleteRequest {
  const delimiter = ['\t', '|', ','].find((item) => line.includes(item)) || '/';
  const parts = line.split(delimiter).map((item) => item.trim()).filter(Boolean);
  if (parts.length < 2) throw new Error(`第 ${index + 1} 行至少需要 lib 和 folder`);
  const lib = parts[0];
  const itemId = parts.length >= 3 ? parts[parts.length - 1] : '';
  const folderParts = parts.length >= 3 ? parts.slice(1, -1) : parts.slice(1);
  const folder = folderParts.join(delimiter).trim();
  if (!lib || !folder) throw new Error(`第 ${index + 1} 行缺少 lib 或 folder`);
  return {
    lib,
    folder,
    item_id: itemId || null,
    reason: null
  };
}

function parseBatchDeleteInput(text: string, reason: string): ManageDeleteBatchRequest {
  const trimmed = text.trim();
  if (!trimmed) throw new Error('先填写批量删除内容');
  const requestReason = optionalText(reason);
  if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(trimmed);
    } catch (e) {
      throw new Error(`JSON 解析失败：${errorMessage(e)}`);
    }
    if (Array.isArray(parsed)) {
      return {
        items: parsed.map(parseBatchJsonItem),
        reason: requestReason
      };
    }
    if (isRecord(parsed) && Array.isArray(parsed.items)) {
      return {
        items: parsed.items.map(parseBatchJsonItem),
        reason: optionalText(parsed.reason) || requestReason
      };
    }
    throw new Error('JSON 需要是数组，或包含 items 数组的对象');
  }
  const items = trimmed.split(/\r?\n/).map((line) => line.trim()).filter(Boolean).map(parseBatchLine);
  return {
    items,
    reason: requestReason
  };
}

function itemBrowserLabel(item: LibraryItemEntry) {
  const year = item.year ? ` (${item.year})` : '';
  const tmdb = item.tmdb ? ` · tmdb ${item.tmdb}` : '';
  return `${item.name}${year} · ${item.folder}${tmdb}`;
}

function itemBrowserHaystack(item: LibraryItemEntry) {
  return [
    item.name,
    item.folder,
    item.id,
    item.tmdb,
    item.year,
    item.path
  ].filter((value) => value != null).join('\n').toLocaleLowerCase('zh-CN');
}

function batchDeleteItemFromBrowser(lib: string, item: LibraryItemEntry): ManageDeleteRequest {
  return {
    lib,
    folder: item.folder,
    item_id: item.id || null,
    reason: null
  };
}

function appendBatchDeleteItem(text: string, item: ManageDeleteRequest) {
  const trimmed = text.trim();
  if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
    try {
      const parsed = JSON.parse(trimmed) as unknown;
      if (Array.isArray(parsed)) {
        return JSON.stringify([...parsed, item], null, 2);
      }
      if (isRecord(parsed) && Array.isArray(parsed.items)) {
        return JSON.stringify({ ...parsed, items: [...parsed.items, item] }, null, 2);
      }
    } catch {
      // Fall through to line append when the textarea is not valid JSON yet.
    }
  }
  const line = [item.lib, item.folder, item.item_id].filter(Boolean).join('/');
  return text.trimEnd() ? `${text.trimEnd()}\n${line}` : line;
}

export function ManagePanel() {
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [undoItems, setUndoItems] = useState<UndoEntry[]>([]);
  const [undoTotal, setUndoTotal] = useState(0);
  const [browserLib, setBrowserLib] = useState('');
  const [browserItems, setBrowserItems] = useState<LibraryItemEntry[]>([]);
  const [browserFilter, setBrowserFilter] = useState('');
  const [browserSelectedIndex, setBrowserSelectedIndex] = useState<number | null>(null);
  const [browserLoading, setBrowserLoading] = useState(false);
  const [browserError, setBrowserError] = useState('');
  const [deleteLib, setDeleteLib] = useState('');
  const [deleteFolder, setDeleteFolder] = useState('');
  const [deleteItemId, setDeleteItemId] = useState('');
  const [deleteReason, setDeleteReason] = useState('');
  const [batchDeleteText, setBatchDeleteText] = useState('');
  const [batchDeleteReason, setBatchDeleteReason] = useState('');
  const [pendingBatchDelete, setPendingBatchDelete] = useState<ManageDeleteBatchRequest | null>(null);
  const [fromLib, setFromLib] = useState('');
  const [fromFolder, setFromFolder] = useState('');
  const [toLib, setToLib] = useState('');
  const [toFolder, setToFolder] = useState('');
  const [moveItemId, setMoveItemId] = useState('');
  const [moveReason, setMoveReason] = useState('');
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState<'delete' | 'move' | 'batch-delete' | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmBatchDelete, setConfirmBatchDelete] = useState(false);
  const [error, setError] = useState('');
  const [lastTask, setLastTask] = useState<TaskRun | null>(null);
  const [confirmMove, setConfirmMove] = useState(false);
  const toast = useToast();

  const sortedLibraries = useMemo(() => libraryOptions(libraries), [libraries]);

  const applyDefaultLibs = useCallback((next: EmbyLibrary[]) => {
    const first = libraryOptions(next)[0]?.name || '';
    setBrowserLib((value) => value || first);
    setDeleteLib((value) => value || first);
    setFromLib((value) => value || first);
    setToLib((value) => value || first);
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const [libraryData, undoData] = await Promise.all([
        api<LibrariesResponse>('/api/v2/libraries').catch(() => ({ libraries: [] })),
        api<UndoListResponse>('/api/v2/manage/undo?limit=20')
      ]);
      setLibraries(libraryData.libraries);
      applyDefaultLibs(libraryData.libraries);
      setUndoItems(undoData.items);
      setUndoTotal(undoData.total);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`删除移动工作台加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [applyDefaultLibs, toast]);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    let cancelled = false;
    const lib = browserLib.trim();
    setBrowserSelectedIndex(null);
    if (!lib) {
      setBrowserItems([]);
      setBrowserError('');
      setBrowserLoading(false);
      return () => {
        cancelled = true;
      };
    }
    setBrowserItems([]);
    setBrowserLoading(true);
    setBrowserError('');
    const params = new URLSearchParams({ lib, limit: '500' });
    api<LibraryItemsResponse>(`/api/v2/libraries/items?${params.toString()}`)
      .then((data) => {
        if (cancelled) return;
        setBrowserItems(Array.isArray(data.items) ? data.items : []);
      })
      .catch((e) => {
        if (cancelled) return;
        const message = errorMessage(e);
        setBrowserItems([]);
        setBrowserError(message);
        toast.push(`库项目加载失败：${message}`, 'error');
      })
      .finally(() => {
        if (!cancelled) setBrowserLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [browserLib, toast]);

  useEffect(() => {
    const onTaskCompleted = (event: Event) => {
      const detail = (event as CustomEvent<TaskCompleteDetail>).detail;
      if (detail?.task && shouldRefreshManage(detail.task)) {
        load();
      }
    };
    window.addEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
    return () => window.removeEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
  }, [load]);

  const submitDelete = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!deleteLib.trim() || !deleteFolder.trim()) {
      toast.push('先填写库名和 folder', 'warn');
      return;
    }
    setSubmitting('delete');
    setError('');
    try {
      const payload: ManageDeleteRequest = {
        lib: deleteLib.trim(),
        folder: deleteFolder.trim(),
        item_id: deleteItemId.trim() || null,
        reason: deleteReason.trim() || null
      };
      const task = await api<TaskRun>('/api/v2/manage/delete', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setLastTask(task);
      toast.push(`已创建删除预览任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`删除预览创建失败：${message}`, 'error');
    } finally {
      setSubmitting(null);
    }
  };

  const executeDelete = async () => {
    if (!deleteLib.trim() || !deleteFolder.trim()) {
      toast.push('先填写库名和 folder', 'warn');
      return;
    }
    setConfirmDelete(false);
    setSubmitting('delete');
    setError('');
    try {
      const payload: ManageDeleteRequest = {
        lib: deleteLib.trim(),
        folder: deleteFolder.trim(),
        item_id: deleteItemId.trim() || null,
        reason: deleteReason.trim() || null
      };
      const task = await api<TaskRun>('/api/v2/manage/delete/execute', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setLastTask(task);
      toast.push(`已创建真实删除任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`真实删除任务创建失败：${message}`, 'error');
    } finally {
      setSubmitting(null);
    }
  };

  const submitBatchDelete = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError('');
    try {
      const payload = parseBatchDeleteInput(batchDeleteText, batchDeleteReason);
      if (payload.items.length === 0) {
        toast.push('批量删除至少需要 1 项', 'warn');
        return;
      }
      setPendingBatchDelete(payload);
      setConfirmBatchDelete(true);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`批量删除内容无效：${message}`, 'error');
    }
  };

  const executeBatchDelete = async () => {
    if (!pendingBatchDelete) return;
    setConfirmBatchDelete(false);
    setSubmitting('batch-delete');
    setError('');
    try {
      const task = await api<TaskRun>('/api/v2/manage/delete/batch/execute', {
        method: 'POST',
        body: JSON.stringify(pendingBatchDelete)
      });
      setLastTask(task);
      toast.push(`已创建批量真实删除任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`批量真实删除任务创建失败：${message}`, 'error');
    } finally {
      setSubmitting(null);
    }
  };

  const browserRows = useMemo(() => {
    const tokens = browserFilter.trim().toLocaleLowerCase('zh-CN').split(/\s+/).filter(Boolean);
    return browserItems
      .map((item, index) => ({ item, index }))
      .filter(({ item }) => {
        if (tokens.length === 0) return true;
        const haystack = itemBrowserHaystack(item);
        return tokens.every((token) => haystack.includes(token));
      })
      .slice(0, 80);
  }, [browserFilter, browserItems]);

  const browserSelectedVisible = browserSelectedIndex != null && browserRows.some(({ index }) => index === browserSelectedIndex);
  const browserSelectedItem = browserSelectedVisible && browserSelectedIndex != null ? browserItems[browserSelectedIndex] || null : null;

  const selectBrowserItem = (index: number) => {
    const item = browserItems[index];
    if (!item) return;
    const lib = browserLib.trim();
    setBrowserSelectedIndex(index);
    setDeleteLib(lib);
    setDeleteFolder(item.folder);
    setDeleteItemId(item.id || '');
    toast.push(`已填入删除项：${item.name}`, 'ok');
  };

  const addBrowserItemToBatch = () => {
    if (!browserSelectedItem || !browserLib.trim()) {
      toast.push('先选择一个库项目', 'warn');
      return;
    }
    const item = batchDeleteItemFromBrowser(browserLib.trim(), browserSelectedItem);
    setBatchDeleteText((value) => appendBatchDeleteItem(value, item));
    toast.push(`已加入批量删除：${browserSelectedItem.name}`, 'ok');
  };

  const submitMove = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!fromLib.trim() || !fromFolder.trim() || !toLib.trim()) {
      toast.push('先填写来源库、来源 folder 和目标库', 'warn');
      return;
    }
    setSubmitting('move');
    setError('');
    try {
      const payload: ManageMoveRequest = {
        from_lib: fromLib.trim(),
        from_folder: fromFolder.trim(),
        to_lib: toLib.trim(),
        to_folder: toFolder.trim() || null,
        item_id: moveItemId.trim() || null,
        reason: moveReason.trim() || null
      };
      const task = await api<TaskRun>('/api/v2/manage/move', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setLastTask(task);
      toast.push(`已创建移动预览任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`移动预览创建失败：${message}`, 'error');
    } finally {
      setSubmitting(null);
    }
  };

  const executeMove = async () => {
    if (!fromLib.trim() || !fromFolder.trim() || !toLib.trim()) {
      toast.push('先填写来源库、来源 folder 和目标库', 'warn');
      return;
    }
    setConfirmMove(false);
    setSubmitting('move');
    setError('');
    try {
      const payload: ManageMoveRequest = {
        from_lib: fromLib.trim(),
        from_folder: fromFolder.trim(),
        to_lib: toLib.trim(),
        to_folder: toFolder.trim() || null,
        item_id: moveItemId.trim() || null,
        reason: moveReason.trim() || null
      };
      const task = await api<TaskRun>('/api/v2/manage/move/execute', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setLastTask(task);
      toast.push(`已创建真实移动任务：${task.label || task.kind}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`真实移动任务创建失败：${message}`, 'error');
    } finally {
      setSubmitting(null);
    }
  };

  const renderLibSelect = (label: string, value: string, onChange: (value: string) => void) => (
    <label>
      <span>{label}</span>
      <select className="input" aria-label={label} value={value} onChange={(event) => onChange(event.target.value)}>
        {sortedLibraries.length === 0 && <option value="">手动输入不可用</option>}
        {sortedLibraries.map((library) => <option key={library.id || library.name} value={library.name}>{library.name}</option>)}
      </select>
    </label>
  );

  return (
    <section className="managePanel">
      {confirmDelete && (
        <ConfirmDanger
          title="确认真实删除"
          confirmText="确认删除"
          onCancel={() => setConfirmDelete(false)}
          onConfirm={executeDelete}
          body={(
            <div className="dangerCopy">
              <p>将先删除 Emby Item，再删除媒体目录和 STRM 目录，并写入 undo 记录。</p>
              <code>{deleteLib.trim()}/{deleteFolder.trim()}</code>
            </div>
          )}
        />
      )}
      {confirmBatchDelete && pendingBatchDelete && (
        <ConfirmDanger
          title="确认批量真实删除"
          confirmText="确认批量删除"
          onCancel={() => setConfirmBatchDelete(false)}
          onConfirm={executeBatchDelete}
          body={(
            <div className="dangerCopy">
              <p>将逐项执行真实删除，并由任务写入 undo 记录。</p>
              <code>{pendingBatchDelete.items.length} 项 · {pendingBatchDelete.items[0]?.lib}/{pendingBatchDelete.items[0]?.folder}</code>
            </div>
          )}
        />
      )}
      {confirmMove && (
        <ConfirmDanger
          title="确认真实移动"
          confirmText="确认移动"
          onCancel={() => setConfirmMove(false)}
          onConfirm={executeMove}
          body={(
            <div className="dangerCopy">
              <p>将移动媒体目录，重建目标 STRM，删除旧 STRM，并刷新目标 Emby 库。</p>
              <code>{fromLib.trim()}/{fromFolder.trim()} → {toLib.trim()}/{toFolder.trim() || fromFolder.trim().split('/').filter(Boolean).at(-1) || fromFolder.trim()}</code>
            </div>
          )}
        />
      )}
      <div className="manageToolbar">
        <div>
          <strong>删除·移动</strong>
          <span>删除和移动都支持预览与真实执行。</span>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新'}
        </button>
      </div>

      <div className="notice warn scanNotice">
        真实删除会按“先 Emby DELETE，再动磁盘”的顺序执行；请先生成预览并核对路径。
      </div>
      {error && <div className="notice warn whitespaceNotice">{error}</div>}

      <div className="statGrid">
        <article className="statCard warn">
          <div><FileWarning /></div>
          <span>写操作</span>
          <strong>删除 / 移动</strong>
          <small>preview first, then execute</small>
        </article>
        <article className="statCard neutral">
          <div><RotateCcw /></div>
          <span>Undo 记录</span>
          <strong>{count(undoTotal)}</strong>
          <small>最近显示 {count(undoItems.length)}</small>
        </article>
        <article className="statCard neutral">
          <div><FolderInput /></div>
          <span>库列表</span>
          <strong>{count(libraries.length)}</strong>
          <small>来自 Emby libraries</small>
        </article>
        <article className={`statCard ${lastTask ? 'ok' : 'neutral'}`}>
          <div><CheckCircle2 /></div>
          <span>最近任务</span>
          <strong>{lastTask?.status || '无'}</strong>
          <small>{lastTask?.id || '尚未创建任务'}</small>
        </article>
      </div>

      <div className="manageForms">
        <form className="manageForm" onSubmit={submitDelete}>
          <div className="manageFormHead">
            <Trash2 size={18} />
            <strong>删除</strong>
          </div>
          {renderLibSelect('删除库名', deleteLib, setDeleteLib)}
          <label>
            <span>folder</span>
            <input className="input" aria-label="删除 folder" value={deleteFolder} onChange={(event) => setDeleteFolder(event.target.value)} placeholder="例如 电影名 [tmdbid-123]" />
          </label>
          <label>
            <span>Emby ItemId</span>
            <input className="input" aria-label="删除 ItemId" value={deleteItemId} onChange={(event) => setDeleteItemId(event.target.value)} placeholder="可选" />
          </label>
          <label>
            <span>原因</span>
            <input className="input" aria-label="删除原因" value={deleteReason} onChange={(event) => setDeleteReason(event.target.value)} placeholder="可选，写入预览参数" />
          </label>
          <div className="inlineActions">
            <button className="btn" disabled={submitting !== null}>
              {submitting === 'delete' ? '创建中' : '生成删除预览任务'}
            </button>
            <button type="button" className="btn danger" disabled={submitting !== null} onClick={() => setConfirmDelete(true)}>
              真实删除
            </button>
          </div>
        </form>

        <form className="manageForm" onSubmit={submitBatchDelete}>
          <div className="manageFormHead">
            <ListChecks size={18} />
            <strong>批量删除</strong>
          </div>
          <label>
            <span>批量项</span>
            <textarea
              className="input"
              aria-label="批量删除内容"
              value={batchDeleteText}
              onChange={(event) => setBatchDeleteText(event.target.value)}
              rows={7}
              placeholder={'电影/旧电影/item-1\n电视剧/旧剧\n或 JSON: {"items":[{"lib":"电影","folder":"旧电影","item_id":"item-1"}]}'}
            />
          </label>
          <label>
            <span>批量原因</span>
            <input className="input" aria-label="批量删除原因" value={batchDeleteReason} onChange={(event) => setBatchDeleteReason(event.target.value)} placeholder="可选，写入批量任务参数" />
          </label>
          <div className="inlineActions">
            <button className="btn danger" disabled={submitting !== null}>
              {submitting === 'batch-delete' ? '创建中' : '检查并确认批量删除'}
            </button>
          </div>
        </form>

        <div className="manageForm">
          <div className="manageFormHead">
            <Search size={18} />
            <strong>库项目选择器</strong>
          </div>
          {renderLibSelect('浏览库名', browserLib, setBrowserLib)}
          <label>
            <span>关键词</span>
            <input className="input" aria-label="项目关键词" value={browserFilter} onChange={(event) => setBrowserFilter(event.target.value)} placeholder="名称 / folder / tmdb / 年份" />
          </label>
          <label>
            <span>项目</span>
            <select
              className="input"
              aria-label="库项目列表"
              size={8}
              value={browserSelectedIndex == null ? '' : String(browserSelectedIndex)}
              onChange={(event) => selectBrowserItem(Number(event.target.value))}
              disabled={browserLoading || browserRows.length === 0}
            >
              {browserRows.map(({ item, index }) => (
                <option key={`${item.id || item.folder}-${index}`} value={index}>
                  {itemBrowserLabel(item)}
                </option>
              ))}
            </select>
          </label>
          {browserError && <div className="notice warn whitespaceNotice">{browserError}</div>}
          {!browserError && (
            <small className="mutedParagraph">
              {browserLoading ? '加载项目中' : `显示 ${count(browserRows.length)} / ${count(browserItems.length)} 项`}
            </small>
          )}
          {browserSelectedItem && (
            <div className="taskMeta manageTaskMeta">
              <div><dt>folder</dt><dd>{browserSelectedItem.folder}</dd></div>
              <div><dt>ItemId</dt><dd>{browserSelectedItem.id || '无'}</dd></div>
              <div><dt>路径</dt><dd>{browserSelectedItem.path || '无'}</dd></div>
            </div>
          )}
          <div className="inlineActions">
            <button type="button" className="btn ghost" onClick={addBrowserItemToBatch} disabled={!browserSelectedItem}>
              <Plus size={16} />
              加入批量删除文本
            </button>
          </div>
        </div>

        <form className="manageForm" onSubmit={submitMove}>
          <div className="manageFormHead">
            <ArrowRight size={18} />
            <strong>移动</strong>
          </div>
          {renderLibSelect('来源库名', fromLib, setFromLib)}
          <label>
            <span>来源 folder</span>
            <input className="input" aria-label="来源 folder" value={fromFolder} onChange={(event) => setFromFolder(event.target.value)} placeholder="原目录名" />
          </label>
          {renderLibSelect('目标库名', toLib, setToLib)}
          <label>
            <span>目标 folder</span>
            <input className="input" aria-label="目标 folder" value={toFolder} onChange={(event) => setToFolder(event.target.value)} placeholder="可选，留空沿用来源名" />
          </label>
          <label>
            <span>Emby ItemId</span>
            <input className="input" aria-label="移动 ItemId" value={moveItemId} onChange={(event) => setMoveItemId(event.target.value)} placeholder="可选" />
          </label>
          <label>
            <span>原因</span>
            <input className="input" aria-label="移动原因" value={moveReason} onChange={(event) => setMoveReason(event.target.value)} placeholder="可选" />
          </label>
          <div className="inlineActions">
            <button className="btn" disabled={submitting !== null}>
              {submitting === 'move' ? '创建中' : '生成移动预览任务'}
            </button>
            <button type="button" className="btn danger" disabled={submitting !== null} onClick={() => setConfirmMove(true)}>
              真实移动
            </button>
          </div>
        </form>
      </div>

      {lastTask && (
        <section className="readonlyBlock">
          <h2>最近任务</h2>
          <div className="taskMeta manageTaskMeta">
            <div><dt>任务</dt><dd>{lastTask.label || lastTask.kind}</dd></div>
            <div><dt>状态</dt><dd>{lastTask.status}</dd></div>
            <div><dt>ID</dt><dd>{lastTask.id}</dd></div>
            <div><dt>结果</dt><dd>{stringify(lastTask.result) || '等待任务中心更新'}</dd></div>
          </div>
        </section>
      )}

      <section className="readonlyBlock">
        <h2>最近 Undo 记录</h2>
        <div className="undoMiniList">
          {undoItems.map((item) => (
            <article key={item.id}>
              <span className={`badge ${item.undone ? 'done' : 'pending'}`}>{item.undone ? 'done' : opLabel(item.op)}</span>
              <strong>{item.legacy_id || item.id}</strong>
              <small>{formatDate(item.created_at)}</small>
              <code>{stringify(item.payload)}</code>
            </article>
          ))}
          {undoItems.length === 0 && <div className="empty inlineEmpty">暂无 undo 记录</div>}
        </div>
      </section>
    </section>
  );
}
