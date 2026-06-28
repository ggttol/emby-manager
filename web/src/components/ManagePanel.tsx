import { ArrowRight, CheckCircle2, FileWarning, FolderInput, RefreshCw, RotateCcw, Trash2 } from 'lucide-react';
import { FormEvent, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { ConfirmDanger } from './Modal';
import { useToast } from './Toast';

type EmbyLibrary = components['schemas']['EmbyLibrary'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];
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

export function ManagePanel() {
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [undoItems, setUndoItems] = useState<UndoEntry[]>([]);
  const [undoTotal, setUndoTotal] = useState(0);
  const [deleteLib, setDeleteLib] = useState('');
  const [deleteFolder, setDeleteFolder] = useState('');
  const [deleteItemId, setDeleteItemId] = useState('');
  const [deleteReason, setDeleteReason] = useState('');
  const [fromLib, setFromLib] = useState('');
  const [fromFolder, setFromFolder] = useState('');
  const [toLib, setToLib] = useState('');
  const [toFolder, setToFolder] = useState('');
  const [moveItemId, setMoveItemId] = useState('');
  const [moveReason, setMoveReason] = useState('');
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState<'delete' | 'move' | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState('');
  const [lastTask, setLastTask] = useState<TaskRun | null>(null);
  const toast = useToast();

  const sortedLibraries = useMemo(() => libraryOptions(libraries), [libraries]);

  const applyDefaultLibs = (next: EmbyLibrary[]) => {
    const first = libraryOptions(next)[0]?.name || '';
    setDeleteLib((value) => value || first);
    setFromLib((value) => value || first);
    setToLib((value) => value || first);
  };

  const load = async () => {
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
  };

  useEffect(() => {
    load();
  }, []);

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
      <div className="manageToolbar">
        <div>
          <strong>删除·移动</strong>
          <span>删除支持预览和真实执行；移动仍保持 dry-run 预览。</span>
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
          <strong>删除可执行</strong>
          <small>move remains preview</small>
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

        <form className="manageForm" onSubmit={submitMove}>
          <div className="manageFormHead">
            <ArrowRight size={18} />
            <strong>移动预览</strong>
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
          <button className="btn" disabled={submitting !== null}>
            {submitting === 'move' ? '创建中' : '生成移动预览任务'}
          </button>
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
