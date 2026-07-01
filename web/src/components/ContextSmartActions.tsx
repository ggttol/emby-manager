import { AlertTriangle, ArrowRight, RefreshCw, WandSparkles } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';

type SmartAction = components['schemas']['SmartAction'];
type SmartActionInspectRequest = components['schemas']['SmartActionInspectRequest'];
type SmartActionInspectResponse = components['schemas']['SmartActionInspectResponse'];
type SmartSubject = components['schemas']['SmartSubject'];
type CatalogItem = components['schemas']['CatalogItem'];
type CatalogLibraryContextResponse = components['schemas']['CatalogLibraryContextResponse'];

type SmartActionInspectPayload = Partial<SmartActionInspectRequest> & {
  catalog_items?: CatalogItem[];
  catalog_context?: CatalogLibraryContextResponse | null;
};

const ACTION_TYPE_LABEL: Record<string, string> = {
  transfer_add_new: '新增转存',
  transfer_update_series: '追更更新',
  dedup_remove_old: '自动去重',
  dedup_review: '人工去重',
  poster_fix: '海报修复',
  metadata_refresh: '元数据刷新',
  library_scan: '媒体库扫描',
  archive_series: '完结归档',
  cleanup_empty_folder: '空目录清理',
  task_retry_or_diagnose: '任务诊断'
};

const RISK_LABEL: Record<string, string> = {
  low: '低风险',
  medium: '中风险',
  high: '高风险',
  critical: '关键风险'
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function riskTone(risk?: string | null): 'ok' | 'warn' | 'error' | 'neutral' {
  if (risk === 'critical' || risk === 'high') return 'error';
  if (risk === 'medium') return 'warn';
  if (risk === 'low') return 'ok';
  return 'neutral';
}

function label(map: Record<string, string>, value: string | null | undefined) {
  if (!value) return '未知';
  return map[value] || value;
}

function actionSummary(actions: SmartAction[]) {
  const confirm = actions.filter((action) => action.policy.mode === 'confirm' || action.risk.requires_confirm_text).length;
  const auto = actions.filter((action) => action.policy.mode === 'auto' && action.risk.level === 'low').length;
  return { confirm, auto };
}

export function ContextSmartActions({
  title,
  q,
  subject,
  emptyText = '当前上下文没有新的智能动作。',
  limit = 4,
  inspectPayload,
  onNavigate
}: {
  title: string;
  q?: string | null;
  subject?: SmartSubject | null;
  emptyText?: string;
  limit?: number;
  inspectPayload?: SmartActionInspectPayload;
  onNavigate?: (tabId: string) => void;
}) {
  const query = (q || '').trim();
  const subjectKey = useMemo(() => (subject ? JSON.stringify(subject) : ''), [subject]);
  const inspectPayloadKey = useMemo(() => (inspectPayload ? JSON.stringify(inspectPayload) : ''), [inspectPayload]);
  const enabled = Boolean(query || subjectKey || inspectPayloadKey);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [actions, setActions] = useState<SmartAction[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const summary = actionSummary(actions);

  const load = async () => {
    if (!enabled) {
      setActions([]);
      setWarnings([]);
      setError('');
      return;
    }
    setLoading(true);
    setError('');
    try {
      const parsedSubject = subjectKey ? JSON.parse(subjectKey) as SmartSubject : undefined;
      const extraPayload = inspectPayloadKey ? JSON.parse(inspectPayloadKey) as SmartActionInspectPayload : undefined;
      const data = await api<SmartActionInspectResponse>('/api/v2/smart-actions/inspect', {
        method: 'POST',
        body: JSON.stringify({
          q: query || undefined,
          subject: parsedSubject,
          limit,
          ...extraPayload
        })
      });
      setActions(Array.isArray(data.actions) ? data.actions : []);
      setWarnings(Array.isArray(data.warnings) ? data.warnings : []);
    } catch (e) {
      setActions([]);
      setWarnings([]);
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, [query, subjectKey, limit, inspectPayloadKey]);

  if (!enabled) return null;

  return (
    <section className="contextSmartActions" aria-label={title}>
      <div className="contextSmartActionsHead">
        <div>
          <strong><WandSparkles size={16} /> {title}</strong>
          <span>
            {loading
              ? '正在匹配当前上下文...'
              : actions.length
                ? `${actions.length.toLocaleString('zh-CN')} 条建议 · ${summary.confirm.toLocaleString('zh-CN')} 条需确认`
                : emptyText}
          </span>
        </div>
        <div className="inlineActions compactActions">
          <button className="btn ghost compact" onClick={load} disabled={loading}>
            <RefreshCw size={14} />
            刷新
          </button>
          <button className="btn ghost compact" onClick={() => onNavigate?.('smart-actions')} disabled={!onNavigate}>
            <ArrowRight size={14} />
            工作台
          </button>
        </div>
      </div>

      {error && <div className="notice warn">智能动作匹配失败：{error}</div>}
      {warnings.length > 0 && (
        <div className="contextSmartWarnings">
          {warnings.map((warning) => <span key={warning}><AlertTriangle size={13} /> {warning}</span>)}
        </div>
      )}
      {actions.length > 0 && (
        <div className="contextSmartActionList">
          {actions.map((action) => (
            <article key={action.id} className={riskTone(action.risk.level)}>
              <div>
                <span className={`badge ${riskTone(action.risk.level)}`}>{label(RISK_LABEL, action.risk.level)}</span>
                <strong>{action.title}</strong>
              </div>
              <p>{action.summary}</p>
              <small>{label(ACTION_TYPE_LABEL, action.action_type)} · {action.recommendation.primary_action} · {action.policy.mode === 'auto' ? '可自动' : '需确认'}</small>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}
