import { FileText, RefreshCw, RotateCcw } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { Modal } from './Modal';
import { useToast } from './Toast';

type AppLogEntry = components['schemas']['AppLogEntry'];
type LogListResponse = components['schemas']['LogListResponse'];
type UndoEntry = components['schemas']['UndoEntry'];
type UndoExecuteResponse = components['schemas']['UndoExecuteResponse'];
type UndoListResponse = components['schemas']['UndoListResponse'];

type View = 'logs' | 'undo';
type LogLevel = 'all' | 'info' | 'warn' | 'error';

const levelOptions: Array<{ value: LogLevel; label: string }> = [
  { value: 'all', label: '全部级别' },
  { value: 'info', label: 'Info' },
  { value: 'warn', label: 'Warn' },
  { value: 'error', label: 'Error' }
];

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function formatDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
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

function objectValue(value: unknown, keys: string[]) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return '';
  const raw = value as Record<string, unknown>;
  for (const key of keys) {
    const item = raw[key];
    if (typeof item === 'string' && item.trim()) return item.trim();
  }
  return '';
}

function opLabel(op: string) {
  if (op === 'delete') return '删除';
  if (op === 'move') return '移动';
  if (op === 'replace') return '替换';
  if (op === 'smart_archive') return '智能归档';
  if (op === 'rebind') return '海报重绑';
  return op || '未知';
}

function actionLabel(action: string) {
  if (action === 'manual_restore') return '人工恢复';
  if (action === 'pending_port') return '待移植';
  if (action === 'already_undone') return '已处理';
  if (action === 'unsupported') return '不支持';
  return action;
}

function logMatches(log: AppLogEntry, text: string) {
  if (!text) return true;
  const haystack = `${log.level}\n${log.message}\n${stringify(log.detail)}\n${log.created_at}`.toLowerCase();
  return haystack.includes(text);
}

function undoMatches(item: UndoEntry, text: string) {
  if (!text) return true;
  const haystack = `${item.op}\n${item.legacy_id || ''}\n${item.created_at}\n${stringify(item.payload)}`.toLowerCase();
  return haystack.includes(text);
}

export function LogsPanel() {
  const [view, setView] = useState<View>('logs');
  const [level, setLevel] = useState<LogLevel>('all');
  const [filter, setFilter] = useState('');
  const [logs, setLogs] = useState<AppLogEntry[]>([]);
  const [undoItems, setUndoItems] = useState<UndoEntry[]>([]);
  const [logTotal, setLogTotal] = useState(0);
  const [undoTotal, setUndoTotal] = useState(0);
  const [loadingLogs, setLoadingLogs] = useState(true);
  const [loadingUndo, setLoadingUndo] = useState(true);
  const [error, setError] = useState('');
  const [executingId, setExecutingId] = useState<string | null>(null);
  const [undoResult, setUndoResult] = useState<UndoExecuteResponse | null>(null);
  const toast = useToast();

  const normalizedFilter = filter.trim().toLowerCase();
  const visibleLogs = useMemo(
    () => logs.filter((log) => logMatches(log, normalizedFilter)),
    [logs, normalizedFilter]
  );
  const visibleUndo = useMemo(
    () => undoItems.filter((item) => undoMatches(item, normalizedFilter)),
    [undoItems, normalizedFilter]
  );

  const loadLogs = async (nextLevel = level) => {
    setLoadingLogs(true);
    setError('');
    try {
      const params = new URLSearchParams({ limit: '200' });
      if (nextLevel !== 'all') params.set('level', nextLevel);
      const data = await api<LogListResponse>(`/api/v2/logs?${params.toString()}`);
      setLogs(data.logs);
      setLogTotal(data.total);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`日志加载失败：${message}`, 'error');
    } finally {
      setLoadingLogs(false);
    }
  };

  const loadUndo = async () => {
    setLoadingUndo(true);
    setError('');
    try {
      const data = await api<UndoListResponse>('/api/v2/manage/undo?limit=80');
      setUndoItems(data.items);
      setUndoTotal(data.total);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`Undo 记录加载失败：${message}`, 'error');
    } finally {
      setLoadingUndo(false);
    }
  };

  useEffect(() => {
    loadLogs(level);
  }, [level]);

  useEffect(() => {
    loadUndo();
  }, []);

  const refresh = () => {
    if (view === 'logs') loadLogs(level);
    else loadUndo();
  };

  const executeUndo = async (item: UndoEntry) => {
    setExecutingId(item.id);
    setError('');
    try {
      const data = await api<UndoExecuteResponse>('/api/v2/manage/undo/execute', {
        method: 'POST',
        body: JSON.stringify({ id: item.id })
      });
      setUndoResult(data);
      toast.push(data.ok ? 'Undo 操作已返回成功' : '已生成恢复指引', data.ok ? 'ok' : 'warn');
      if (data.ok) {
        setUndoItems((prev) => prev.map((entry) => (entry.id === item.id ? { ...entry, undone: true } : entry)));
      }
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`Undo 请求失败：${message}`, 'error');
    } finally {
      setExecutingId(null);
    }
  };

  return (
    <section className="logsPanel">
      <div className="logsToolbar">
        <div className="logsTabs" role="tablist" aria-label="日志视图">
          <button className={view === 'logs' ? 'active' : ''} onClick={() => setView('logs')}>
            <FileText size={15} />
            操作日志
          </button>
          <button className={view === 'undo' ? 'active' : ''} onClick={() => setView('undo')}>
            <RotateCcw size={15} />
            Undo 记录
          </button>
        </div>
        <button className="btn ghost" onClick={refresh} disabled={view === 'logs' ? loadingLogs : loadingUndo}>
          <RefreshCw size={16} />
          {view === 'logs' ? (loadingLogs ? '加载中' : '刷新日志') : (loadingUndo ? '加载中' : '刷新 Undo')}
        </button>
      </div>

      <div className="logsFilters">
        <label>
          <span>页面过滤</span>
          <input
            className="input"
            aria-label="日志过滤"
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="消息、路径、库名或操作类型"
          />
        </label>
        <label>
          <span>日志级别</span>
          <select
            className="input"
            aria-label="日志级别"
            value={level}
            onChange={(event) => setLevel(event.target.value as LogLevel)}
            disabled={view !== 'logs'}
          >
            {levelOptions.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
        </label>
        <p>
          {view === 'logs'
            ? `${visibleLogs.length}/${logTotal} 条日志`
            : `${visibleUndo.length}/${undoTotal} 条 Undo`}
        </p>
      </div>

      {error && <div className="notice warn whitespaceNotice">{error}</div>}

      {view === 'logs' ? (
        <LogTable logs={visibleLogs} loading={loadingLogs} />
      ) : (
        <UndoTable items={visibleUndo} loading={loadingUndo} executingId={executingId} onExecute={executeUndo} />
      )}

      {undoResult && (
        <Modal title="Undo 恢复指引" onClose={() => setUndoResult(null)}>
          <div className="modalBody undoResult">
            <span className={`badge ${undoResult.ok ? 'done' : 'warn'}`}>{actionLabel(undoResult.action)}</span>
            <p>{undoResult.msg}</p>
            {(undoResult.lib || undoResult.folder) && (
              <dl>
                {undoResult.lib && (
                  <div>
                    <dt>库</dt>
                    <dd>{undoResult.lib}</dd>
                  </div>
                )}
                {undoResult.folder && (
                  <div>
                    <dt>文件夹</dt>
                    <dd>{undoResult.folder}</dd>
                  </div>
                )}
              </dl>
            )}
            {undoResult.hint && <pre className="undoHint">{undoResult.hint}</pre>}
          </div>
          <footer className="modalActions">
            <button className="btn" onClick={() => setUndoResult(null)}>知道了</button>
          </footer>
        </Modal>
      )}
    </section>
  );
}

function LogTable({ logs, loading }: { logs: AppLogEntry[]; loading: boolean }) {
  return (
    <div className="logsTableWrap">
      <table className="dataTable logsTable">
        <thead>
          <tr>
            <th>时间</th>
            <th>级别</th>
            <th>消息</th>
            <th>详情</th>
          </tr>
        </thead>
        <tbody>
          {logs.map((log) => (
            <tr key={log.id}>
              <td>{formatDate(log.created_at)}</td>
              <td><span className={`badge ${log.level.toLowerCase()}`}>{log.level}</span></td>
              <td><strong className="logMessage">{log.message}</strong></td>
              <td>
                {log.detail == null ? (
                  <span className="mutedText">无详情</span>
                ) : (
                  <details className="logDetail">
                    <summary>查看</summary>
                    <pre>{stringify(log.detail)}</pre>
                  </details>
                )}
              </td>
            </tr>
          ))}
          {!loading && logs.length === 0 && (
            <tr>
              <td colSpan={4} className="emptyCell">没有匹配的日志</td>
            </tr>
          )}
          {loading && logs.length === 0 && (
            <tr>
              <td colSpan={4} className="emptyCell">正在加载日志...</td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

function UndoTable({
  items,
  loading,
  executingId,
  onExecute
}: {
  items: UndoEntry[];
  loading: boolean;
  executingId: string | null;
  onExecute: (item: UndoEntry) => void;
}) {
  return (
    <div className="logsTableWrap">
      <table className="dataTable undoTable">
        <thead>
          <tr>
            <th>时间</th>
            <th>操作</th>
            <th>目标</th>
            <th>状态</th>
            <th>数据</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          {items.map((item) => {
            const lib = objectValue(item.payload, ['lib', 'from', 'to']);
            const folder = objectValue(item.payload, ['folder', 'lose_was', 'lose_folder']);
            return (
              <tr key={item.id}>
                <td>{formatDate(item.created_at)}</td>
                <td>
                  <strong>{opLabel(item.op)}</strong>
                  {item.legacy_id && <small>{item.legacy_id}</small>}
                </td>
                <td>
                  <strong className="undoTarget">{folder || '未记录文件夹'}</strong>
                  <small>{lib || '未记录库'}</small>
                </td>
                <td>
                  <span className={`badge ${item.undone ? 'done' : 'pending'}`}>{item.undone ? '已处理' : '待检查'}</span>
                </td>
                <td>
                  <details className="logDetail">
                    <summary>Payload</summary>
                    <pre>{stringify(item.payload)}</pre>
                  </details>
                </td>
                <td>
                  <button className="btn ghost compact" onClick={() => onExecute(item)} disabled={executingId === item.id}>
                    {executingId === item.id ? '请求中' : '查看恢复指引'}
                  </button>
                </td>
              </tr>
            );
          })}
          {!loading && items.length === 0 && (
            <tr>
              <td colSpan={6} className="emptyCell">没有匹配的 Undo 记录</td>
            </tr>
          )}
          {loading && items.length === 0 && (
            <tr>
              <td colSpan={6} className="emptyCell">正在加载 Undo 记录...</td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
