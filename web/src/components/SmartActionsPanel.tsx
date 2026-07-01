import {
  AlertTriangle,
  CheckCircle2,
  ClipboardCheck,
  Clock3,
  ExternalLink,
  FileText,
  Gauge,
  Info,
  ListChecks,
  Play,
  RefreshCw,
  Search,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  SquareCheckBig,
  WandSparkles,
  Workflow
} from 'lucide-react';
import { ReactNode, useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { Drawer } from './Drawer';
import { ConfirmDanger } from './Modal';
import { useToast } from './Toast';

type SmartAction = components['schemas']['SmartAction'];
type SmartActionsListResponse = components['schemas']['SmartActionsListResponse'];
type SmartActionDetailResponse = components['schemas']['SmartActionDetailResponse'];
type SmartActionExecuteRequest = components['schemas']['SmartActionExecuteRequest'];
type SmartActionExecuteResponse = components['schemas']['SmartActionExecuteResponse'];
type SmartActionExecuteBatchResponse = components['schemas']['SmartActionExecuteBatchResponse'];
type SmartActionVerifyResponse = components['schemas']['SmartActionVerifyResponse'];
type SmartActionsSummary = components['schemas']['SmartActionsSummary'];
type SmartActionType = components['schemas']['SmartActionType'];
type SmartActionStatus = components['schemas']['SmartActionStatus'];
type SmartRiskLevel = components['schemas']['SmartRiskLevel'];
type SmartSubjectKind = components['schemas']['SmartSubjectKind'];
type SmartExecutionStep = components['schemas']['SmartExecutionStep'];
type TaskRun = components['schemas']['TaskRun'];

type BulkExecuteResult = {
  submitted: number;
  failed: Array<{ id: string; title: string; message: string }>;
};

type Filters = {
  q: string;
  actionType: 'all' | SmartActionType;
  status: 'all' | SmartActionStatus;
  risk: 'all' | SmartRiskLevel;
  subjectKind: 'all' | SmartSubjectKind;
  lib: string;
};

type JsonRecord = Record<string, unknown>;

type DangerousExecutionConfig = {
  mode: 'dedup' | 'archive' | 'transfer_add_new' | 'transfer_update_series' | 'unsupported';
  title: string;
  description: string;
  submitLabel: string;
  confirmText: string;
  payload: unknown | null;
  canSubmit: boolean;
  disabledReason: string;
  facts: Array<{ label: string; value: string }>;
};

type Tone = 'neutral' | 'ok' | 'warn' | 'error';

type CatalogTransferCandidate = {
  title: string;
  link: string;
  q: string;
  targetLib: string;
  targetCid: string;
  isPkg: boolean;
  linkType: string;
  share: string;
  rc: string;
};

type DangerousExecutionForm = {
  archiveTargetLib: string;
  transferTargetLib: string;
  transferTargetCid: string;
  transferPackageAck: string;
  updateCandidateName: string;
  updateCandidateLink: string;
  updateCandidateLinkType: string;
  updateCandidateRc: string;
  updateTargetLib: string;
  updateTargetCid: string;
};

const EMPTY_SUMMARY: SmartActionsSummary = {
  total: 0,
  suggested: 0,
  running: 0,
  failed: 0,
  auto_ready: 0,
  confirm_required: 0,
  low: 0,
  medium: 0,
  high: 0,
  critical: 0
};

const ACTION_TYPE_OPTIONS: Array<{ value: 'all' | SmartActionType; label: string }> = [
  { value: 'all', label: '全部类型' },
  { value: 'transfer_add_new', label: '新增转存' },
  { value: 'transfer_update_series', label: '追更更新' },
  { value: 'dedup_remove_old', label: '自动去重' },
  { value: 'dedup_review', label: '人工去重' },
  { value: 'poster_fix', label: '海报修复' },
  { value: 'metadata_refresh', label: '元数据刷新' },
  { value: 'library_scan', label: '媒体库扫描' },
  { value: 'archive_series', label: '完结归档' },
  { value: 'cleanup_empty_folder', label: '空目录清理' },
  { value: 'task_retry_or_diagnose', label: '任务诊断' }
];

const STATUS_OPTIONS: Array<{ value: 'all' | SmartActionStatus; label: string }> = [
  { value: 'all', label: '全部状态' },
  { value: 'suggested', label: '待处理' },
  { value: 'confirmed', label: '已确认' },
  { value: 'queued', label: '排队中' },
  { value: 'running', label: '执行中' },
  { value: 'verifying', label: '验证中' },
  { value: 'done', label: '已完成' },
  { value: 'partial', label: '部分完成' },
  { value: 'failed', label: '失败' },
  { value: 'cancelled', label: '已取消' },
  { value: 'dismissed', label: '已忽略' }
];

const RISK_OPTIONS: Array<{ value: 'all' | SmartRiskLevel; label: string }> = [
  { value: 'all', label: '全部风险' },
  { value: 'low', label: '低风险' },
  { value: 'medium', label: '中风险' },
  { value: 'high', label: '高风险' },
  { value: 'critical', label: '关键风险' }
];

const SUBJECT_KIND_OPTIONS: Array<{ value: 'all' | SmartSubjectKind; label: string }> = [
  { value: 'all', label: '全部对象' },
  { value: 'series', label: '电视剧' },
  { value: 'movie', label: '电影' },
  { value: 'season', label: '季' },
  { value: 'episode', label: '集' },
  { value: 'library', label: '媒体库' },
  { value: 'task', label: '任务' },
  { value: 'system', label: '系统' },
  { value: 'unknown', label: '未知对象' }
];

const ACTION_TYPE_LABEL: Record<SmartActionType, string> = Object.fromEntries(
  ACTION_TYPE_OPTIONS.filter((item) => item.value !== 'all').map((item) => [item.value, item.label])
) as Record<SmartActionType, string>;

const STATUS_LABEL: Record<SmartActionStatus, string> = Object.fromEntries(
  STATUS_OPTIONS.filter((item) => item.value !== 'all').map((item) => [item.value, item.label])
) as Record<SmartActionStatus, string>;

const RISK_LABEL: Record<SmartRiskLevel, string> = Object.fromEntries(
  RISK_OPTIONS.filter((item) => item.value !== 'all').map((item) => [item.value, item.label])
) as Record<SmartRiskLevel, string>;

const SUBJECT_KIND_LABEL: Record<string, string> = {
  movie: '电影',
  series: '电视剧',
  season: '季',
  episode: '集',
  library: '媒体库',
  task: '任务',
  system: '系统',
  unknown: '未知对象'
};

const SOURCE_LABEL: Record<string, string> = {
  emby_item: 'Emby 条目',
  emby_episodes: 'Emby 剧集',
  strm_scan: 'STRM 扫描',
  cloud_drive_path: 'CloudDrive 路径',
  c115_resource: '115 资源',
  catalog_candidate: '资源库候选',
  tmdb_metadata: 'TMDb 元数据',
  poster_detection: '海报检测',
  dedup_analysis: '去重分析',
  task_history: '任务历史',
  undo_log: 'Undo 日志',
  system_health: '系统健康',
  dashboard_todo: '仪表盘待办'
};

const EXECUTOR_LABEL: Record<string, string> = {
  open_tab: '打开功能页',
  existing_endpoint: '调用现有接口',
  task_pipeline: '任务流水线',
  manual_confirm: '人工确认'
};

const POLICY_MODE_LABEL: Record<string, string> = {
  auto: '可自动执行',
  confirm: '需要确认',
  disabled: '已禁用'
};

const TAB_LABEL: Record<string, string> = {
  dashboard: '仪表盘',
  smart_actions: '智能动作',
  'smart-actions': '智能动作',
  scan: '扫描',
  c115: '115 转存',
  catalog: '找资源',
  zhuigeng: '追更检查',
  gaps: '缺集检查',
  posters: '海报修复',
  dedup: '去重',
  manage: '删除移动',
  cleanup: '智能清理',
  system: '系统',
  schedules: '定时',
  logs: '日志',
  users: '用户',
  settings: '设置'
};

const TECH_KEY_LABEL: Record<string, string> = {
  current_max_episode: '当前最大集数',
  latest_episode: '资源最新集数',
  library: '媒体库',
  lib: '媒体库',
  title: '资源名',
  name: '名称',
  file_name: '文件名',
  link: '链接',
  url: '链接',
  share: '分享链接',
  share_url: '分享链接',
  share_link: '分享链接',
  cid: '115 cid',
  target_cid: '目标 cid',
  target_lib: '目标库',
  tmdb: 'TMDb',
  tmdb_id: 'TMDb',
  emby_id: 'Emby 条目',
  folder: '目录',
  path: '路径',
  strm_path: 'STRM',
  cd_path: 'CloudDrive 路径',
  is_pkg: '整包合集',
  rc: '提取码',
  pwd: '提取码',
  password: '提取码',
  receive_code: '提取码',
  status: '状态',
  message: '说明',
  error: '错误',
  err: '错误',
  ok: '结果'
};

const RISK_RANK: Record<SmartRiskLevel, number> = {
  low: 1,
  medium: 2,
  high: 3,
  critical: 4
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function userFacingError(error: unknown) {
  const raw = errorMessage(error).trim();
  if (!raw) return '操作失败，请稍后重试，或到任务中心/日志查看详细原因。';
  if (/^\s*[{[]/.test(raw) || /\b(stack|traceback|panic|panicked)\b/i.test(raw)) {
    return '接口返回了技术错误，先到任务中心或日志查看详情；当前动作没有提交成功。';
  }
  return raw
    .replaceAll('catalog_transfer_execute', '新增转存执行计划')
    .replaceAll('zhuigeng_update_execute', '追更更新执行计划')
    .replaceAll('dedup_execute_batch', '去重执行计划');
}

function count(value: number | null | undefined) {
  return Number(value || 0).toLocaleString('zh-CN');
}

function dateText(value?: string | null) {
  if (!value) return '未知';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('zh-CN', { hour12: false });
}

function riskTone(risk?: SmartRiskLevel | null): Tone {
  if (risk === 'critical' || risk === 'high') return 'error';
  if (risk === 'medium') return 'warn';
  if (risk === 'low') return 'ok';
  return 'neutral';
}

function policyTone(mode?: string | null): Tone {
  if (mode === 'auto') return 'ok';
  if (mode === 'confirm') return 'warn';
  if (mode === 'disabled') return 'error';
  return 'neutral';
}

function statusTone(status?: SmartActionStatus | string | null): Tone {
  if (status === 'done') return 'ok';
  if (status === 'failed' || status === 'cancelled') return 'error';
  if (status === 'partial' || status === 'running' || status === 'queued' || status === 'verifying') return 'warn';
  return 'neutral';
}

function readableJson(value: unknown) {
  if (value == null) return '';
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function objectValue(value: unknown): JsonRecord | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as JsonRecord : null;
}

function stringValue(value: unknown) {
  return typeof value === 'string' ? value.trim() : '';
}

function booleanValue(value: unknown) {
  return value === true || value === 1 || value === '1' || value === 'true';
}

function tabText(tab?: string | null) {
  if (!tab) return '';
  return TAB_LABEL[tab] || tab;
}

function actionSourceText(action: SmartAction) {
  const pieces = [
    action.tab ? `来自${tabText(action.tab)}` : '',
    action.action_label ? `建议：${action.action_label}` : ''
  ].filter(Boolean);
  if (pieces.length) return pieces.join(' · ');
  if (action.source.includes('.')) {
    const [prefix] = action.source.split('.');
    return `${tabText(prefix) || prefix} 信号`;
  }
  return label(SOURCE_LABEL, action.source);
}

function compactHumanText(value: unknown, max = 96) {
  let text = '';
  if (typeof value === 'boolean') text = value ? '是' : '否';
  else if (typeof value === 'number') text = Number.isFinite(value) ? value.toLocaleString('zh-CN') : String(value);
  else if (typeof value === 'string') text = value.trim();
  else if (Array.isArray(value)) text = `共 ${count(value.length)} 项`;
  else if (value && typeof value === 'object') text = `包含 ${count(Object.keys(value as JsonRecord).length)} 项信息`;
  else text = '无';
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

function friendlyKey(key: string) {
  return TECH_KEY_LABEL[key] || key.replaceAll('_', ' ');
}

function evidenceFacts(value: unknown) {
  const record = objectValue(value);
  if (!record) {
    if (Array.isArray(value)) return [{ label: '数量', value: `共 ${count(value.length)} 项` }];
    return [{ label: '内容', value: compactHumanText(value) }];
  }
  const keys = Object.keys(record);
  const preferred = [
    'title',
    'name',
    'file_name',
    'current_max_episode',
    'latest_episode',
    'library',
    'lib',
    'cid',
    'link',
    'share_url',
    'is_pkg',
    'tmdb',
    'tmdb_id',
    'emby_id',
    'folder',
    'path',
    'strm_path',
    'cd_path',
    'status',
    'message',
    'error',
    'err'
  ];
  const ordered = [
    ...preferred.filter((key) => Object.prototype.hasOwnProperty.call(record, key)),
    ...keys.filter((key) => !preferred.includes(key))
  ];
  const facts = ordered
    .filter((key) => record[key] !== null && record[key] !== undefined && record[key] !== '')
    .slice(0, 5)
    .map((key) => ({ label: friendlyKey(key), value: compactHumanText(record[key]) }));
  return facts.length ? facts : [{ label: '内容', value: `包含 ${count(keys.length)} 项信息` }];
}

function actionStateCopy(action: SmartAction, executionTask: TaskRun | null, verifyResult: SmartActionVerifyResponse | null) {
  if (verifyResult) {
    if (verifyResult.status === 'done') {
      return { tone: 'ok' as Tone, title: '验证通过', detail: '验收检查已经通过，可以把这个动作视为完成。' };
    }
    if (verifyResult.status === 'partial') {
      return { tone: 'warn' as Tone, title: '需要复查', detail: '部分检查还没完全通过，可能是 Emby 刷新延迟，也可能需要继续处理。' };
    }
    if (verifyResult.status === 'failed') {
      return { tone: 'error' as Tone, title: '验证失败', detail: '验收检查没有通过，建议查看任务中心的执行详情和日志。' };
    }
    return { tone: statusTone(verifyResult.status), title: label(STATUS_LABEL, verifyResult.status), detail: '验证请求已返回最新状态。' };
  }
  if (executionTask) {
    return {
      tone: statusTone(executionTask.status),
      title: '任务已提交',
      detail: `${executionTask.label || '智能动作任务'} 已进入任务中心，当前状态：${executionTask.status_text || executionTask.status}。`
    };
  }
  switch (action.status) {
    case 'queued':
      return { tone: 'warn' as Tone, title: '等待执行', detail: '动作已排队，稍后会由任务中心继续处理。' };
    case 'running':
      return { tone: 'warn' as Tone, title: '正在执行', detail: '后台任务正在运行，可以到任务中心看实时进度。' };
    case 'verifying':
      return { tone: 'warn' as Tone, title: '正在验证', detail: '系统正在确认执行结果，完成后会更新为通过、部分完成或失败。' };
    case 'done':
      return { tone: 'ok' as Tone, title: '已完成', detail: action.verification.success_message || '这个动作已经完成。' };
    case 'partial':
      return { tone: 'warn' as Tone, title: '部分完成', detail: action.verification.partial_message || '有些检查还需要复查。' };
    case 'failed':
      return { tone: 'error' as Tone, title: '上次失败', detail: '上次执行或验证失败，建议先查看任务中心详情，再决定是否重试。' };
    case 'cancelled':
      return { tone: 'error' as Tone, title: '已取消', detail: '这个动作已经取消，没有继续执行。' };
    case 'dismissed':
      return { tone: 'neutral' as Tone, title: '已忽略', detail: '这个建议已被忽略，后续刷新信号时可能重新出现。' };
    default:
      return { tone: 'neutral' as Tone, title: '待处理', detail: '建议先查看证据、风险和执行计划，再决定是否提交。' };
  }
}

function verifyResultLines(result: unknown) {
  const record = objectValue(result);
  if (!record) return [];
  const summaries = Array.isArray(record.check_summaries) ? record.check_summaries : [];
  const lines = summaries
    .map((item) => objectValue(item))
    .filter((item): item is JsonRecord => Boolean(item))
    .map((item) => stringValue(item.summary) || stringValue(item.title) || stringValue(item.warning) || stringValue(item.next_check))
    .filter(Boolean);
  if (lines.length) return lines.slice(0, 4);
  return [
    stringValue(record.summary),
    stringValue(record.message),
    stringValue(record.error),
    stringValue(record.err)
  ].filter(Boolean).slice(0, 3);
}

function findStep(action: SmartAction, key: string): SmartExecutionStep | null {
  return action.plan.steps.find((step) => step.key === key) || null;
}

function stepParams(step: SmartExecutionStep | null): JsonRecord {
  return objectValue(step?.params) || {};
}

function candidateTitleFromItem(item: JsonRecord | null) {
  return stringValue(item?.title) || stringValue(item?.name) || stringValue(item?.file_name) || stringValue(item?.n);
}

function candidateLinkFromItem(item: JsonRecord | null) {
  return stringValue(item?.link) || stringValue(item?.url) || stringValue(item?.share_url) || stringValue(item?.share_link);
}

function candidateShareFromItem(item: JsonRecord | null) {
  return stringValue(item?.share) || stringValue(item?.share_url) || stringValue(item?.share_link);
}

function candidateRcFromItem(item: JsonRecord | null) {
  return stringValue(item?.rc) || stringValue(item?.pwd) || stringValue(item?.password) || stringValue(item?.receive_code);
}

function catalogTransferCandidate(action: SmartAction): CatalogTransferCandidate | null {
  const params = stepParams(findStep(action, 'catalog_transfer_execute'));
  if (!Object.keys(params).length) return null;
  const item = objectValue(params.item);
  const target = objectValue(params.target);
  const hasCandidate = Boolean(
    (item && Object.keys(item).length) || stringValue(params.link) || stringValue(params.url) || stringValue(params.q)
  );
  if (!hasCandidate) return null;
  return {
    title: candidateTitleFromItem(item) || stringValue(params.q) || action.subject.name,
    link: stringValue(params.link) || candidateLinkFromItem(item) || stringValue(params.url),
    q: stringValue(params.q),
    targetLib: stringValue(target?.lib) || stringValue(params.lib) || stringValue(params.to_lib) || stringValue(params.target_lib),
    targetCid: stringValue(target?.cid) || stringValue(params.target_cid) || stringValue(params.to_cid),
    isPkg: booleanValue(item?.is_pkg) || booleanValue(params.is_pkg),
    linkType: stringValue(item?.link_type) || stringValue(params.link_type),
    share: candidateShareFromItem(item) || stringValue(params.share),
    rc: candidateRcFromItem(item) || candidateRcFromItem(params)
  };
}

function firstCandidateEvidence(action: SmartAction) {
  const evidence = action.evidence.find((item) => item.source === 'catalog_candidate' || item.source === 'c115_resource');
  return objectValue(evidence?.value);
}

function zhuigengUpdateCandidate(action: SmartAction): CatalogTransferCandidate {
  const params = stepParams(findStep(action, 'zhuigeng_update_execute'));
  const candidate = objectValue(params.candidate);
  const target = objectValue(params.target);
  const evidence = firstCandidateEvidence(action);
  const link = stringValue(params.link) || candidateLinkFromItem(candidate) || candidateLinkFromItem(evidence);
  return {
    title: candidateTitleFromItem(candidate) || candidateTitleFromItem(evidence) || action.subject.name,
    link,
    q: stringValue(params.q) || action.subject.name,
    targetLib: stringValue(target?.lib) || stringValue(params.lib) || stringValue(params.target_lib) || action.subject.lib || '',
    targetCid: stringValue(target?.cid) || stringValue(params.cid) || stringValue(params.target_cid),
    isPkg: booleanValue(candidate?.is_pkg) || booleanValue(evidence?.is_pkg),
    linkType: stringValue(candidate?.link_type) || stringValue(evidence?.link_type) || stringValue(params.link_type) || 'share115',
    share: candidateShareFromItem(candidate) || candidateShareFromItem(evidence) || link,
    rc: candidateRcFromItem(candidate) || candidateRcFromItem(evidence) || candidateRcFromItem(params)
  };
}

function compactText(value: unknown) {
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (value == null) return '无';
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function countDedupRemovals(request: unknown) {
  const groups = objectValue(request)?.groups;
  if (!Array.isArray(groups)) return { groups: 0, removals: 0 };
  const removals = groups.reduce((sum, group) => {
    const remove = objectValue(group)?.remove;
    return sum + (Array.isArray(remove) ? remove.length : 0);
  }, 0);
  return { groups: groups.length, removals };
}

function requiredConfirmText(action: SmartAction, fallback: string) {
  return stringValue(action.risk.requires_confirm_text) || fallback;
}

function transferTargetPayload(targetLibValue: string, targetCidValue: string) {
  const lib = targetLibValue.trim();
  const cid = targetCidValue.trim();
  if (cid && !/^[1-9]\d*$/.test(cid)) {
    return {
      target: null,
      label: cid,
      disabledReason: '自定义 cid 必须是正整数，0 根目录不允许。'
    };
  }
  if (cid) {
    return {
      target: lib ? { cid, lib } : { cid },
      label: lib ? `库「${lib}」/ cid ${cid}` : `cid ${cid}`,
      disabledReason: ''
    };
  }
  if (lib) {
    return {
      target: { lib },
      label: `库「${lib}」`,
      disabledReason: ''
    };
  }
  return {
    target: null,
    label: '未填写',
    disabledReason: '请填写目标库，或填写自定义 cid。'
  };
}

function buildDangerousExecutionConfig(action: SmartAction, form: DangerousExecutionForm): DangerousExecutionConfig | null {
  if (action.action_type === 'dedup_remove_old') {
    const step = findStep(action, 'dedup_execute_batch');
    const request = stepParams(step).request ?? null;
    const counts = countDedupRemovals(request);
    const hasRequest = Boolean(request);
    return {
      mode: 'dedup',
      title: '确认删除重复旧资源',
      description: '将按去重分析里的保留/删除清单提交批量去重任务。此动作会先走 Emby 删除，再处理磁盘资源，并写入 undo 记录。',
      submitLabel: '确认删除旧资源',
      confirmText: requiredConfirmText(action, '删除'),
      payload: hasRequest ? { request } : null,
      canSubmit: hasRequest,
      disabledReason: hasRequest ? '' : '执行计划缺少 dedup_execute_batch.request，不能提交删除。',
      facts: [
        { label: '重复组', value: count(counts.groups) },
        { label: '待删条目', value: count(counts.removals) },
        { label: '并发键', value: action.plan.concurrency_key || '无' }
      ]
    };
  }

  if (action.action_type === 'archive_series') {
    const step = findStep(action, 'zhuigeng_archive_execute');
    const params = stepParams(step);
    const target = form.archiveTargetLib.trim();
    return {
      mode: 'archive',
      title: '确认归档完结剧',
      description: '将把追更库里的完结条目移动到目标库，并触发后续媒体库刷新。目标库必须由你明确填写。',
      submitLabel: '确认归档',
      confirmText: requiredConfirmText(action, '归档'),
      payload: target ? { to_lib: target } : null,
      canSubmit: Boolean(step && target),
      disabledReason: !step ? '执行计划缺少 zhuigeng_archive_execute，不能提交归档。' : '请先填写目标库。',
      facts: [
        { label: '当前库', value: action.subject.lib || '未知' },
        { label: '目标库', value: target || '未填写' },
        { label: '冲突处理', value: compactText(params.on_conflict || 'smart') }
      ]
    };
  }

  if (action.action_type === 'transfer_add_new') {
    const candidate = catalogTransferCandidate(action);
    const target = transferTargetPayload(form.transferTargetLib, form.transferTargetCid);
    const packageAckRequired = Boolean(candidate?.isPkg);
    const packageAckOk = !packageAckRequired || form.transferPackageAck.trim() === '整包';
    const request = target.target && packageAckOk
      ? {
          target: target.target,
          ...(packageAckRequired ? { package_ack: '整包' } : {})
        }
      : null;
    const disabledReason = !candidate
      ? '执行计划缺少 catalog_transfer_execute 的 item/link/q，不能提交新增转存。'
      : target.disabledReason || (packageAckOk ? '' : '候选是整包合集，请输入「整包」确认。');
    return {
      mode: 'transfer_add_new',
      title: '确认新增转存',
      description: '将把当前候选资源交给智能动作执行新增转存。提交前必须明确目标库或自定义 115 cid，整包合集还要额外确认。',
      submitLabel: '确认新增转存',
      confirmText: '执行',
      payload: candidate && request ? { request } : null,
      canSubmit: Boolean(candidate && request),
      disabledReason,
      facts: [
        { label: '资源名', value: candidate?.title || action.subject.name },
        { label: '目标', value: target.label },
        { label: '整包', value: packageAckRequired ? '是' : '否' }
      ]
    };
  }

  if (action.action_type === 'transfer_update_series') {
    const target = transferTargetPayload(form.updateTargetLib, form.updateTargetCid);
    const name = form.updateCandidateName.trim() || action.subject.name;
    const link = form.updateCandidateLink.trim();
    const linkType = form.updateCandidateLinkType.trim() || 'share115';
    const rc = form.updateCandidateRc.trim();
    const candidate = link
      ? {
          name,
          link,
          link_type: linkType,
          share: link,
          ...(rc ? { rc } : {})
        }
      : null;
    return {
      mode: 'transfer_update_series',
      title: '确认追更更新',
      description: '将把你填写的候选资源交给追更一条龙更新。提交前必须填写候选链接，并明确目标库或自定义 115 cid。',
      submitLabel: '确认追更更新',
      confirmText: '执行',
      payload: candidate && target.target ? { candidate, target: target.target } : null,
      canSubmit: Boolean(candidate && target.target),
      disabledReason: !link ? '候选链接不能为空。' : target.disabledReason,
      facts: [
        { label: '剧名', value: action.subject.name },
        { label: '候选资源', value: name || '未填写' },
        { label: '目标', value: target.label },
        { label: 'link_type', value: linkType },
        { label: 'Emby ID', value: action.subject.emby_id || '未知' }
      ]
    };
  }

  if (action.risk.level === 'high' || action.risk.level === 'critical' || action.risk.requires_confirm_text) {
    return {
      mode: 'unsupported',
      title: '暂未接入安全执行表单',
      description: '这个动作需要人工确认，但当前版本还没有对应的参数表单。为避免误操作，先只展示证据和执行计划。',
      submitLabel: '暂不可提交',
      confirmText: requiredConfirmText(action, '确认'),
      payload: null,
      canSubmit: false,
      disabledReason: '缺少该动作类型的安全参数表单。',
      facts: [
        { label: '动作类型', value: label(ACTION_TYPE_LABEL, action.action_type) },
        { label: '风险', value: label(RISK_LABEL, action.risk.level) },
        { label: '确认词', value: requiredConfirmText(action, '确认') }
      ]
    };
  }

  return null;
}

function isRiskAllowed(risk: SmartRiskLevel, maxRisk: SmartRiskLevel) {
  return RISK_RANK[risk] <= RISK_RANK[maxRisk];
}

function executionBlocker(action: SmartAction) {
  if (action.status !== 'suggested' && action.status !== 'failed') return `当前状态为「${label(STATUS_LABEL, action.status)}」，不能直接执行`;
  if (!action.policy.enabled || action.policy.mode === 'disabled') return '策略已禁用，不能直接执行';
  if (action.risk.level === 'high' || action.risk.level === 'critical') return '高风险动作必须人工确认，不能直接自动执行';
  if (action.policy.mode === 'confirm' || action.risk.requires_confirm_text) return '需要人工确认，不能直接自动执行';
  if (action.policy.mode !== 'auto') return '当前策略不允许自动执行';
  if (!isRiskAllowed(action.risk.level, action.policy.max_risk)) return '风险超过策略上限，不能直接执行';
  return '';
}

function confirmationExecutionBlocker(action: SmartAction) {
  if (action.status !== 'suggested' && action.status !== 'failed') return `当前状态为「${label(STATUS_LABEL, action.status)}」，不能执行`;
  if (!action.policy.enabled || action.policy.mode === 'disabled') return '策略已禁用，不能执行';
  if (!isRiskAllowed(action.risk.level, action.policy.max_risk)) return `风险超过策略上限：${label(RISK_LABEL, action.policy.max_risk)}`;
  if (action.policy.mode !== 'confirm') return '当前策略不是人工确认模式';
  if (!action.risk.requires_confirm_text) return '缺少危险确认文本';
  return '';
}

function batchBlocker(action: SmartAction) {
  const directBlocker = executionBlocker(action);
  if (directBlocker) return directBlocker;
  if (action.risk.level !== 'low') return '批量执行只开放低风险 auto_ready 动作，请打开详情单独审阅';
  return '';
}

function isBatchExecutable(action: SmartAction) {
  return !batchBlocker(action);
}

function buildSmartActionsPath(filters: Filters) {
  const params = new URLSearchParams();
  const q = filters.q.trim();
  if (q) params.set('q', q);
  if (filters.actionType !== 'all') params.set('action_type', filters.actionType);
  if (filters.status !== 'all') params.set('status', filters.status);
  if (filters.risk !== 'all') params.set('risk', filters.risk);
  if (filters.subjectKind !== 'all') params.set('subject_kind', filters.subjectKind);
  const lib = filters.lib.trim();
  if (lib) params.set('lib', lib);
  params.set('limit', '80');
  const query = params.toString();
  return `/api/v2/smart-actions${query ? `?${query}` : ''}`;
}

function label<T extends string>(map: Record<string, string>, value: T | null | undefined) {
  if (!value) return '未知';
  return map[value] || value;
}

function StatCard({
  icon,
  label,
  value,
  hint,
  tone = 'neutral'
}: {
  icon: ReactNode;
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  tone?: 'neutral' | 'ok' | 'warn' | 'error';
}) {
  return (
    <article className={`statCard ${tone}`}>
      <div>{icon}</div>
      <span>{label}</span>
      <strong>{value}</strong>
      {hint && <small>{hint}</small>}
    </article>
  );
}

function FilterSelect<T extends string>({
  label,
  value,
  options,
  onChange
}: {
  label: string;
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
}) {
  return (
    <label>
      <span>{label}</span>
      <select className="input" value={value} onChange={(event) => onChange(event.target.value as T)}>
        {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
      </select>
    </label>
  );
}

function RiskFlags({ action }: { action: SmartAction }) {
  const flags = [
    action.risk.touches_emby ? 'Emby' : '',
    action.risk.touches_disk ? '磁盘' : '',
    action.risk.touches_c115 ? '115' : '',
    action.risk.destructive ? '破坏性' : ''
  ].filter(Boolean);
  if (!flags.length) return <span>只读或低影响</span>;
  return <span>{flags.join(' / ')}</span>;
}

function SmartActionCard({
  action,
  selected,
  batchBlockedReason,
  onDetail,
  onToggleSelect
}: {
  action: SmartAction;
  selected: boolean;
  batchBlockedReason: string;
  onDetail: (action: SmartAction) => void;
  onToggleSelect: (action: SmartAction, checked: boolean) => void;
}) {
  const tone = riskTone(action.risk.level);
  const directBlockedReason = executionBlocker(action);
  const safeText = directBlockedReason || (batchBlockedReason || '低风险 auto_ready，可加入批量执行队列');
  return (
    <article className={`smartActionItem ${tone}${selected ? ' selected' : ''}`}>
      <label className={`smartActionSelect ${batchBlockedReason ? 'disabled' : ''}`}>
        <input
          type="checkbox"
          checked={selected}
          disabled={Boolean(batchBlockedReason)}
          onChange={(event) => onToggleSelect(action, event.target.checked)}
          aria-label={`选择批量执行：${action.title}`}
        />
        <span>{batchBlockedReason ? '不可批量' : '可批量'}</span>
      </label>
      <div className="smartActionMain">
        <div className="smartActionTitleLine">
          <span className={`badge ${tone}`}>{label(RISK_LABEL, action.risk.level)}</span>
          <strong>{action.title}</strong>
        </div>
        <small>{label(ACTION_TYPE_LABEL, action.action_type)} · {label(STATUS_LABEL, action.status)} · {actionSourceText(action)}</small>
        <div className="smartActionPolicyStrip" aria-label={`策略：${action.title}`}>
          <span className={`badge ${policyTone(action.policy.mode)}`}>{label(POLICY_MODE_LABEL, action.policy.mode)}</span>
          <span>上限 {label(RISK_LABEL, action.policy.max_risk)}</span>
          <span>{action.policy.enabled ? '策略启用' : '策略关闭'}</span>
        </div>
      </div>
      <div className="smartActionQuickFacts">
        <span>推荐 <strong>{action.recommendation.primary_action}</strong></span>
        <span>分数 <strong>{action.recommendation.score}</strong></span>
        <span>策略 <strong>{label(POLICY_MODE_LABEL, action.policy.mode)}</strong></span>
      </div>
      <p>{action.summary}</p>
      <div className={`smartActionSafety ${directBlockedReason ? 'warn' : 'ok'}`}>
        {directBlockedReason ? <ShieldAlert size={15} /> : <ShieldCheck size={15} />}
        <span>{safeText}</span>
      </div>
      <button className="btn ghost compact" onClick={() => onDetail(action)} aria-label={`查看详情：${action.title}`}>
        <FileText size={14} />
        查看详情
      </button>
    </article>
  );
}

function SmartActionDetail({
  action,
  loading,
  error,
  onNavigate
}: {
  action: SmartAction;
  loading: boolean;
  error: string;
  onNavigate?: (tabId: string) => void;
}) {
  const tone = riskTone(action.risk.level);
  const [executing, setExecuting] = useState(false);
  const [executeError, setExecuteError] = useState('');
  const [executionTask, setExecutionTask] = useState<TaskRun | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [verifyError, setVerifyError] = useState('');
  const [verifyResult, setVerifyResult] = useState<SmartActionVerifyResponse | null>(null);
  const [archiveTargetLib, setArchiveTargetLib] = useState('');
  const [transferTargetLib, setTransferTargetLib] = useState('');
  const [transferTargetCid, setTransferTargetCid] = useState('');
  const [transferPackageAck, setTransferPackageAck] = useState('');
  const [updateCandidateName, setUpdateCandidateName] = useState('');
  const [updateCandidateLink, setUpdateCandidateLink] = useState('');
  const [updateCandidateLinkType, setUpdateCandidateLinkType] = useState('share115');
  const [updateCandidateRc, setUpdateCandidateRc] = useState('');
  const [updateTargetLib, setUpdateTargetLib] = useState('');
  const [updateTargetCid, setUpdateTargetCid] = useState('');
  const [pendingDanger, setPendingDanger] = useState<DangerousExecutionConfig | null>(null);
  const [dangerConfirmInput, setDangerConfirmInput] = useState('');
  const toast = useToast();
  const blockedReason = executionBlocker(action);
  const canExecute = !blockedReason;
  const transferCandidate = useMemo(() => (
    action.action_type === 'transfer_add_new' ? catalogTransferCandidate(action) : null
  ), [action]);
  const updateCandidate = useMemo(() => (
    action.action_type === 'transfer_update_series' ? zhuigengUpdateCandidate(action) : null
  ), [action]);
  const dangerousConfig = buildDangerousExecutionConfig(action, {
    archiveTargetLib,
    transferTargetLib,
    transferTargetCid,
    transferPackageAck,
    updateCandidateName,
    updateCandidateLink,
    updateCandidateLinkType,
    updateCandidateRc,
    updateTargetLib,
    updateTargetCid
  });
  const dangerPolicyBlocker = dangerousConfig ? confirmationExecutionBlocker(action) : '';
  const canSubmitDanger = Boolean(dangerousConfig?.canSubmit && !dangerPolicyBlocker && !executing);
  const safetyTone = canExecute || canSubmitDanger ? 'ok' : 'warn';
  const dangerFormMessage = dangerousConfig ? (dangerPolicyBlocker || dangerousConfig.disabledReason) : '';
  const stateCopy = actionStateCopy(action, executionTask, verifyResult);
  const relatedTabLabel = tabText(action.tab);
  const showNextSteps = Boolean(executionTask || verifyResult || ['queued', 'running', 'verifying', 'partial', 'failed', 'done'].includes(action.status));

  useEffect(() => {
    setArchiveTargetLib('');
    setTransferTargetLib(transferCandidate?.targetLib || '');
    setTransferTargetCid(transferCandidate?.targetCid || '');
    setTransferPackageAck('');
    setUpdateCandidateName(updateCandidate?.title || action.subject.name || '');
    setUpdateCandidateLink(updateCandidate?.share || updateCandidate?.link || '');
    setUpdateCandidateLinkType(updateCandidate?.linkType || 'share115');
    setUpdateCandidateRc(updateCandidate?.rc || '');
    setUpdateTargetLib(updateCandidate?.targetLib || action.subject.lib || '');
    setUpdateTargetCid(updateCandidate?.targetCid || '');
    setPendingDanger(null);
    setDangerConfirmInput('');
    setExecuteError('');
    setExecutionTask(null);
    setVerifyError('');
    setVerifyResult(null);
  }, [
    action.id,
    action.subject.name,
    action.subject.lib,
    transferCandidate?.targetLib,
    transferCandidate?.targetCid,
    updateCandidate?.title,
    updateCandidate?.share,
    updateCandidate?.link,
    updateCandidate?.linkType,
    updateCandidate?.rc,
    updateCandidate?.targetLib,
    updateCandidate?.targetCid
  ]);

  const executeAction = async (request?: SmartActionExecuteRequest) => {
    setExecuting(true);
    setExecuteError('');
    setVerifyError('');
    setVerifyResult(null);
    try {
      const result = await api<SmartActionExecuteResponse>(
        `/api/v2/smart-actions/${action.id}/execute`,
        {
          method: 'POST',
          body: request ? JSON.stringify(request) : undefined
        }
      );
      setExecutionTask(result.task || null);
      toast.push(result.task ? `智能动作已提交：${result.task.label}` : '智能动作已提交', 'ok');
    } catch (e) {
      const message = userFacingError(e);
      setExecuteError(message);
      toast.push(`智能动作执行失败：${message}`, 'error');
    } finally {
      setExecuting(false);
    }
  };

  const openTaskCenter = () => {
    const button = document.querySelector<HTMLButtonElement>('button[aria-label="任务中心"]');
    if (button) {
      button.click();
    } else {
      toast.push('请从右上角打开任务中心查看进度', 'warn');
    }
  };

  const verifyAction = async () => {
    setVerifying(true);
    setVerifyError('');
    try {
      const result = await api<SmartActionVerifyResponse>(`/api/v2/smart-actions/${action.id}/verify`, { method: 'POST' });
      setVerifyResult(result);
      if (result.status === 'done') toast.push('智能动作验证通过', 'ok');
      else if (result.status === 'partial') toast.push('智能动作需要复查', 'warn');
      else if (result.status === 'failed') toast.push('智能动作验证失败', 'error');
      else toast.push(`智能动作验证状态：${label(STATUS_LABEL, result.status)}`, 'warn');
    } catch (e) {
      const message = userFacingError(e);
      setVerifyError(message);
      toast.push(`智能动作验证失败：${message}`, 'error');
    } finally {
      setVerifying(false);
    }
  };

  const openDangerConfirm = () => {
    if (!dangerousConfig || !canSubmitDanger) return;
    setDangerConfirmInput('');
    setPendingDanger(dangerousConfig);
  };

  const submitDangerousAction = async () => {
    if (!pendingDanger || dangerConfirmInput.trim() !== pendingDanger.confirmText || !pendingDanger.payload) return;
    const request: SmartActionExecuteRequest = {
      confirm_text: pendingDanger.confirmText,
      payload: pendingDanger.payload
    };
    setPendingDanger(null);
    setDangerConfirmInput('');
    await executeAction(request);
  };

  return (
    <div className="smartActionDrawerBody">
      {loading && <div className="notice">正在读取最新详情...</div>}
      {error && <div className="notice warn">{error}</div>}
      <section className="readonlyBlock">
        <div className="smartActionDetailHead">
          <span className={`badge ${tone}`}>{label(RISK_LABEL, action.risk.level)}</span>
          <h3>{action.title}</h3>
          <p>{action.summary}</p>
        </div>
        <div className="systemKeyValueGrid">
          <span><strong>动作类型</strong>{label(ACTION_TYPE_LABEL, action.action_type)}</span>
          <span><strong>状态</strong>{label(STATUS_LABEL, action.status)}</span>
          <span><strong>对象</strong>{label(SUBJECT_KIND_LABEL, action.subject.kind)} · {action.subject.name}</span>
          <span><strong>策略</strong>{label(POLICY_MODE_LABEL, action.policy.mode)}</span>
          <span><strong>来源</strong>{actionSourceText(action)}</span>
          <span><strong>更新时间</strong>{dateText(action.updated_at)}</span>
        </div>
        <div className={`smartActionStateBanner ${stateCopy.tone}`} aria-label="智能动作当前状态">
          {stateCopy.tone === 'ok' ? <CheckCircle2 size={17} /> : stateCopy.tone === 'error' ? <ShieldAlert size={17} /> : <Clock3 size={17} />}
          <div>
            <strong>{stateCopy.title}</strong>
            <span>{stateCopy.detail}</span>
          </div>
        </div>
      </section>

      <section className="readonlyBlock">
        <h2>推荐理由</h2>
        <div className="miniStats smartActionMiniStats">
          <span>评分 <strong>{action.recommendation.score}</strong></span>
          <span>置信度 <strong>{action.recommendation.confidence}</strong></span>
          <span>风险 <strong>{label(RISK_LABEL, action.risk.level)}</strong></span>
          <span>影响 <strong><RiskFlags action={action} /></strong></span>
        </div>
        <ul className="smartActionTextList">
          {action.recommendation.reasons.map((reason) => <li key={reason}>{reason}</li>)}
        </ul>
        {action.recommendation.alternatives.length > 0 && (
          <div className="smartActionSubList">
            {action.recommendation.alternatives.map((item) => (
              <article key={`${item.action}-${item.reason}`}>
                <strong>{item.action}</strong>
                <p>{item.reason}</p>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="readonlyBlock">
        <h2>证据</h2>
        <div className="smartActionEvidenceList">
          {action.evidence.map((item, index) => (
            <article key={`${item.source}-${item.label}-${index}`}>
              <div>
                <strong>{item.label}</strong>
                <span className="badge">{label(SOURCE_LABEL, item.source)} · 权重 {item.weight}</span>
              </div>
              <div className="smartActionEvidenceFacts">
                {evidenceFacts(item.value).map((fact) => (
                  <span key={`${item.source}-${item.label}-${fact.label}`}>
                    <strong>{fact.label}</strong>
                    {fact.value}
                  </span>
                ))}
              </div>
              <details className="smartActionTechDetails">
                <summary>查看技术数据</summary>
                <pre>{readableJson(item.value)}</pre>
              </details>
              <small>{dateText(item.collected_at)}</small>
            </article>
          ))}
        </div>
      </section>

      <section className="readonlyBlock">
        <h2>风险与策略</h2>
        <div className="systemKeyValueGrid">
          <span><strong>策略模式</strong>{label(POLICY_MODE_LABEL, action.policy.mode)}</span>
          <span><strong>最高允许风险</strong>{label(RISK_LABEL, action.policy.max_risk)}</span>
          <span><strong>可取消</strong>{action.plan.can_cancel ? '是' : '否'}</span>
          <span><strong>并发键</strong>{action.plan.concurrency_key || '无'}</span>
        </div>
        <p className={`smartActionPolicy ${policyTone(action.policy.mode)}`}>{action.policy.reason}</p>
        {action.risk.requires_confirm_text && (
          <div className="notice warn">危险确认文本：{action.risk.requires_confirm_text}</div>
        )}
        {action.risk.warnings.length > 0 && (
          <ul className="smartActionTextList warn">
            {action.risk.warnings.map((warning) => <li key={warning}>{warning}</li>)}
          </ul>
        )}
      </section>

      <section className="readonlyBlock smartActionExecuteBlock">
        <div>
          <h2>执行动作</h2>
          <p>{dangerousConfig ? dangerousConfig.description : (blockedReason || '该动作符合自动策略和风险上限，可提交为单个后台任务。')}</p>
        </div>
        {!dangerousConfig && (
          <button className="btn primary" onClick={() => executeAction()} disabled={!canExecute || executing}>
            <Play size={16} />
            {executing ? '提交中' : '执行动作'}
          </button>
        )}
        {dangerousConfig && (
          <button className="btn danger" onClick={openDangerConfirm} disabled={!canSubmitDanger} title={canSubmitDanger ? undefined : dangerFormMessage}>
            <ShieldAlert size={16} />
            {executing ? '提交中' : dangerousConfig.submitLabel}
          </button>
        )}
        <div className={`smartActionPreflight ${safetyTone}`} aria-label="执行前安全提示">
          {canExecute || canSubmitDanger ? <ShieldCheck size={17} /> : <ShieldAlert size={17} />}
          <div>
            <strong>{canExecute ? '可直接提交' : canSubmitDanger ? '可人工确认提交' : '不可直接提交'}</strong>
            <span>{canExecute ? '系统只会提交后台任务，任务中心会继续展示进度和结果。' : dangerousConfig ? (dangerFormMessage || '需要完成危险确认后提交。') : blockedReason}</span>
          </div>
        </div>
        {dangerousConfig && (
          <div className="smartActionDangerPanel" aria-label="高风险执行确认">
            <div>
              <strong>{dangerousConfig.title}</strong>
              <span>{dangerFormMessage || `点击后需要输入「${dangerousConfig.confirmText}」二次确认。`}</span>
            </div>
            {dangerousConfig.mode === 'archive' && (
              <label className="smartActionDangerInput">
                <span>归档目标库</span>
                <input
                  className="input"
                  value={archiveTargetLib}
                  placeholder="例如 电视剧归档"
                  disabled={executing}
                  onChange={(event) => setArchiveTargetLib(event.target.value)}
                  aria-label="归档目标库"
                />
              </label>
            )}
            {dangerousConfig.mode === 'transfer_add_new' && (
              <div className="smartActionTransferForm" aria-label="新增转存执行参数">
                <label className="smartActionDangerInput">
                  <span>目标库</span>
                  <input
                    className="input"
                  value={transferTargetLib}
                  placeholder="例如 电影"
                  disabled={executing}
                  onChange={(event) => setTransferTargetLib(event.target.value)}
                  aria-label="新增转存目标库"
                />
                </label>
                <label className="smartActionDangerInput">
                  <span>自定义 cid</span>
                  <input
                    className="input"
                    value={transferTargetCid}
                  placeholder="可选，优先使用"
                  inputMode="numeric"
                  pattern="[1-9][0-9]*"
                  disabled={executing}
                  aria-invalid={Boolean(transferTargetCid.trim() && !/^[1-9]\d*$/.test(transferTargetCid.trim()))}
                  onChange={(event) => setTransferTargetCid(event.target.value)}
                  aria-label="新增转存自定义 cid"
                />
                </label>
                {transferCandidate?.isPkg && (
                  <label className="smartActionDangerInput smartActionPkgAck">
                    <span>整包确认</span>
                    <input
                      className="input"
                      value={transferPackageAck}
                      disabled={executing}
                      onChange={(event) => setTransferPackageAck(event.target.value)}
                      aria-label="含整包合集，输入“整包”确认"
                    />
                  </label>
                )}
              </div>
            )}
            {dangerousConfig.mode === 'transfer_update_series' && (
              <div className="smartActionTransferForm smartActionUpdateForm" aria-label="追更更新执行参数">
                <label className="smartActionDangerInput">
                  <span>资源名</span>
                  <input
                    className="input"
                    value={updateCandidateName}
                    placeholder={action.subject.name}
                    disabled={executing}
                    onChange={(event) => setUpdateCandidateName(event.target.value)}
                    aria-label="追更更新资源名"
                  />
                </label>
                <label className="smartActionDangerInput">
                  <span>link_type</span>
                  <input
                    className="input"
                    value={updateCandidateLinkType}
                    placeholder="share115"
                    disabled={executing}
                    onChange={(event) => setUpdateCandidateLinkType(event.target.value)}
                    aria-label="追更更新 link_type"
                  />
                </label>
                <label className="smartActionDangerInput smartActionWideInput">
                  <span>链接/分享链接</span>
                  <input
                    className="input"
                    value={updateCandidateLink}
                    placeholder="https://115.com/s/..."
                    disabled={executing}
                    aria-invalid={!updateCandidateLink.trim()}
                    onChange={(event) => setUpdateCandidateLink(event.target.value)}
                    aria-label="追更更新链接/分享链接"
                  />
                </label>
                <label className="smartActionDangerInput">
                  <span>提取码 rc</span>
                  <input
                    className="input"
                    value={updateCandidateRc}
                    disabled={executing}
                    onChange={(event) => setUpdateCandidateRc(event.target.value)}
                    aria-label="追更更新提取码 rc"
                  />
                </label>
                <label className="smartActionDangerInput">
                  <span>目标库</span>
                  <input
                    className="input"
                    value={updateTargetLib}
                    placeholder={action.subject.lib || '例如 电视剧'}
                    disabled={executing}
                    onChange={(event) => setUpdateTargetLib(event.target.value)}
                    aria-label="追更更新目标库"
                  />
                </label>
                <label className="smartActionDangerInput">
                  <span>自定义 cid</span>
                  <input
                    className="input"
                    value={updateTargetCid}
                    placeholder="可选，优先使用"
                    inputMode="numeric"
                    pattern="[1-9][0-9]*"
                    disabled={executing}
                    aria-invalid={Boolean(updateTargetCid.trim() && !/^[1-9]\d*$/.test(updateTargetCid.trim()))}
                    onChange={(event) => setUpdateTargetCid(event.target.value)}
                    aria-label="追更更新自定义 cid"
                  />
                </label>
              </div>
            )}
            <div className={`smartActionFormState ${canSubmitDanger ? 'ok' : 'warn'}`} role="status" aria-label="执行参数检查">
              {canSubmitDanger ? <ShieldCheck size={16} /> : <ShieldAlert size={16} />}
              <div>
                <strong>{canSubmitDanger ? '参数已就绪' : '还不能执行'}</strong>
                <span>{canSubmitDanger ? '下一步会弹出二次确认，提交后到任务中心看进度。' : (dangerFormMessage || '请先补全必填参数。')}</span>
              </div>
            </div>
            <div className="smartActionDangerFacts">
              {dangerousConfig.facts.map((fact) => (
                <span key={fact.label}><strong>{fact.label}</strong>{fact.value}</span>
              ))}
            </div>
            {dangerousConfig.mode === 'transfer_add_new' && (
              <div className="smartActionCandidateTransfer" aria-label="新增转存候选资源">
                <strong>候选转存线索</strong>
                {transferCandidate ? (
                  <>
                    <span><b>资源名</b>{transferCandidate.title || '未提供'}</span>
                    <span><b>链接</b>{transferCandidate.link || '未提供，请在找资源页重新选择'}</span>
                    <span><b>搜索词</b>{transferCandidate.q || action.subject.name}</span>
                    <span><b>类型</b>{transferCandidate.isPkg ? '整包合集' : (transferCandidate.linkType || '普通资源')}</span>
                  </>
                ) : (
                  <span>执行计划缺少 catalog_transfer_execute 的 item/link/q，不能提交新增转存。</span>
                )}
              </div>
            )}
          </div>
        )}
        <div className="smartActionSafetyChecklist">
          <span><strong>风险</strong>{label(RISK_LABEL, action.risk.level)}</span>
          <span><strong>影响</strong><RiskFlags action={action} /></span>
          <span><strong>策略</strong>{label(POLICY_MODE_LABEL, action.policy.mode)}</span>
          <span><strong>取消</strong>{action.plan.can_cancel ? '支持' : '不支持'}</span>
        </div>
        {executeError && (
          <div className="smartActionFailureNotice" role="alert">
            <ShieldAlert size={16} />
            <div>
              <strong>提交失败</strong>
              <span>{executeError}</span>
            </div>
          </div>
        )}
        {executionTask && (
          <div className="smartActionTaskInfo" aria-label="智能动作任务信息">
            <span><strong>任务</strong>{executionTask.label}</span>
            <span><strong>状态</strong>{executionTask.status_text || executionTask.status}</span>
            <span><strong>任务编号</strong>{executionTask.id.slice(0, 8)}</span>
          </div>
        )}
        {showNextSteps && (
          <div className="smartActionNextSteps" aria-label="智能动作下一步">
            <div>
              <strong>下一步</strong>
              <span>{executionTask ? '任务已提交，先看任务中心进度；任务完成后再验证结果更准确。' : '可以先查看任务中心，再按需重新验证这条动作。'}</span>
            </div>
            <div>
              <button className="btn ghost compact" onClick={openTaskCenter}>
                <ExternalLink size={14} />
                查看任务中心
              </button>
              <button className="btn ghost compact" onClick={verifyAction} disabled={verifying}>
                <ClipboardCheck size={14} />
                {verifying ? '验证中' : '验证结果'}
              </button>
              {onNavigate && action.tab && (
                <button className="btn ghost compact" onClick={() => onNavigate(action.tab || 'smart-actions')}>
                  <Workflow size={14} />
                  打开{relatedTabLabel || '对应功能'}
                </button>
              )}
            </div>
          </div>
        )}
        {verifyError && (
          <div className="smartActionFailureNotice" role="alert">
            <ShieldAlert size={16} />
            <div>
              <strong>验证失败</strong>
              <span>{verifyError}</span>
            </div>
          </div>
        )}
        {verifyResult && (
          <div className={`smartActionVerifyResult ${statusTone(verifyResult.status)}`} aria-label="智能动作验证结果">
            <div>
              <strong>{label(STATUS_LABEL, verifyResult.status)}</strong>
              <span>{verifyResult.ok ? '接口已完成验证请求' : '接口返回未通过，需要复查'}</span>
            </div>
            {verifyResultLines(verifyResult.result).map((line) => <p key={line}>{line}</p>)}
            {verifyResult.warnings.map((warning) => <p key={warning}>{warning}</p>)}
          </div>
        )}
      </section>

      {pendingDanger && (
        <ConfirmDanger
          title={pendingDanger.title}
          confirmText={pendingDanger.submitLabel}
          confirmDisabled={dangerConfirmInput.trim() !== pendingDanger.confirmText || executing}
          onCancel={() => setPendingDanger(null)}
          onConfirm={submitDangerousAction}
          body={(
            <div className="dangerCopy smartActionDangerModal">
              <p>{pendingDanger.description}</p>
              <p>请输入 <code>{pendingDanger.confirmText}</code> 确认提交。任务提交后仍会在任务中心展示执行进度和验收结果。</p>
              <label>
                <span>确认文本</span>
                <input
                  className="input"
                  value={dangerConfirmInput}
                  onChange={(event) => setDangerConfirmInput(event.target.value)}
                  aria-label={`输入确认文本：${pendingDanger.confirmText}`}
                  autoFocus
                />
              </label>
            </div>
          )}
        />
      )}

      <section className="readonlyBlock">
        <h2>执行计划</h2>
        <div className="smartActionStepList">
          {action.plan.steps.map((step, index) => (
            <article key={step.key}>
              <span>{index + 1}</span>
              <div>
                <strong>{step.title}</strong>
                <small>{label(EXECUTOR_LABEL, step.executor)}</small>
                {step.rollback && <p>回滚：{step.rollback.title}</p>}
              </div>
            </article>
          ))}
        </div>
      </section>

      <section className="readonlyBlock">
        <h2>验收条件</h2>
        <div className="smartActionStepList">
          {action.verification.checks.map((check) => (
            <article key={check.key}>
              <CheckCircle2 size={16} />
              <div>
                <strong>{check.title}</strong>
                <small>{label(SOURCE_LABEL, check.source)} · {check.expected}</small>
              </div>
            </article>
          ))}
        </div>
        <p className="smartActionResultText">{action.verification.success_message}</p>
        <p className="smartActionResultText muted">{action.verification.partial_message}</p>
      </section>
    </div>
  );
}

type SmartActionsPanelProps = {
  onNavigate?: (tabId: string) => void;
  focusAction?: SmartAction | null;
  onFocusActionConsumed?: () => void;
};

export function SmartActionsPanel({
  onNavigate,
  focusAction,
  onFocusActionConsumed
}: SmartActionsPanelProps = {}) {
  const [filters, setFilters] = useState<Filters>({
    q: '',
    actionType: 'all',
    status: 'all',
    risk: 'all',
    subjectKind: 'all',
    lib: ''
  });
  const [data, setData] = useState<SmartActionsListResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [selected, setSelected] = useState<SmartAction | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState('');
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [bulkExecuting, setBulkExecuting] = useState(false);
  const [bulkResult, setBulkResult] = useState<BulkExecuteResult | null>(null);
  const [refreshingSignals, setRefreshingSignals] = useState(false);
  const [refreshTask, setRefreshTask] = useState<TaskRun | null>(null);
  const toast = useToast();

  const path = useMemo(() => buildSmartActionsPath(filters), [filters]);
  const summary = data?.summary || EMPTY_SUMMARY;
  const actions = data?.actions || [];
  const batchableActions = actions.filter(isBatchExecutable);
  const selectedActions = actions.filter((action) => selectedIds.has(action.id));
  const selectedBatchableActions = selectedActions.filter(isBatchExecutable);
  const blockedVisibleCount = Math.max(0, actions.length - batchableActions.length);

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      setData(await api<SmartActionsListResponse>(path));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`智能动作加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, [path]);

  useEffect(() => {
    setSelectedIds((prev) => {
      if (!prev.size) return prev;
      const visibleIds = new Set(actions.map((action) => action.id));
      const next = new Set(Array.from(prev).filter((id) => visibleIds.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [actions]);

  const openDetail = async (action: SmartAction) => {
    setSelected(action);
    setDetailLoading(true);
    setDetailError('');
    try {
      const detail = await api<SmartActionDetailResponse>(`/api/v2/smart-actions/${action.id}`);
      setSelected(detail.action);
    } catch (e) {
      const message = errorMessage(e);
      setDetailError(message);
      toast.push(`动作详情加载失败：${message}`, 'error');
    } finally {
      setDetailLoading(false);
    }
  };

  useEffect(() => {
    if (!focusAction) return;
    openDetail(focusAction);
    onFocusActionConsumed?.();
  }, [focusAction?.id]);

  const clearFilters = () => {
    setFilters({ q: '', actionType: 'all', status: 'all', risk: 'all', subjectKind: 'all', lib: '' });
  };

  const refreshSignals = async () => {
    setRefreshingSignals(true);
    try {
      const task = await api<TaskRun>('/api/v2/smart-actions/refresh', { method: 'POST' });
      setRefreshTask(task);
      toast.push(`智能动作刷新任务已提交：${task.label}`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      toast.push(`智能动作刷新失败：${message}`, 'error');
    } finally {
      setRefreshingSignals(false);
    }
  };

  const toggleActionSelection = (action: SmartAction, checked: boolean) => {
    if (checked && !isBatchExecutable(action)) return;
    setBulkResult(null);
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (checked) next.add(action.id);
      else next.delete(action.id);
      return next;
    });
  };

  const selectAllBatchable = () => {
    setBulkResult(null);
    setSelectedIds(new Set(batchableActions.map((action) => action.id)));
  };

  const clearSelection = () => {
    setBulkResult(null);
    setSelectedIds(new Set());
  };

  const executeSelected = async () => {
    if (!selectedBatchableActions.length) return;
    setBulkExecuting(true);
    setBulkResult(null);
    try {
      const result = await api<SmartActionExecuteBatchResponse>('/api/v2/smart-actions/execute-batch', {
        method: 'POST',
        body: JSON.stringify({ ids: selectedBatchableActions.map((action) => action.id) })
      });
      const titleById = new Map(selectedBatchableActions.map((action) => [action.id, action.title]));
      const failed = result.results
        .filter((item) => !item.ok)
        .map((item) => ({
          id: item.id,
          title: titleById.get(item.id) || item.id,
          message: item.err || '未知错误'
        }));
      setBulkResult({ submitted: result.submitted, failed });
      if (failed.length) {
        toast.push(`批量执行完成：${result.submitted} 个已提交，${failed.length} 个失败`, 'warn');
      } else {
        toast.push(`已提交 ${result.submitted} 个低风险智能动作`, 'ok');
        setSelectedIds(new Set());
      }
    } catch (e) {
      const message = errorMessage(e);
      setBulkResult({
        submitted: 0,
        failed: selectedBatchableActions.map((action) => ({ id: action.id, title: action.title, message }))
      });
      toast.push(`批量执行失败：${message}`, 'error');
    } finally {
      setBulkExecuting(false);
    }
  };

  return (
    <section className="readonlyPanel smartActionsPanel">
      <div className="readonlyToolbar">
        <div>
          <strong>智能动作工作台</strong>
          <span>把仪表盘、追更、海报、去重和任务异常聚合成可审阅的下一步。</span>
        </div>
        <div className="readonlyToolbarActions">
          <button className="btn ghost" onClick={clearFilters} disabled={loading}>
            <ListChecks size={16} />
            清空筛选
          </button>
          <button className="btn ghost" onClick={refreshSignals} disabled={refreshingSignals}>
            <WandSparkles size={16} />
            {refreshingSignals ? '提交中' : '刷新建议'}
          </button>
          <button className="btn ghost" onClick={load} disabled={loading}>
            <RefreshCw size={16} />
            {loading ? '加载中' : '刷新'}
          </button>
        </div>
      </div>

      {error && <div className="notice warn">{error}</div>}
      {refreshTask && (
        <div className="notice" aria-label="智能动作刷新任务">
          已提交后台刷新：{refreshTask.label} · {refreshTask.status_text || refreshTask.status}
        </div>
      )}

      <div className="statGrid">
        <StatCard icon={<WandSparkles />} label="建议总数" value={count(summary.total)} tone={summary.total ? 'warn' : 'ok'} hint={`${count(summary.suggested)} 待处理`} />
        <StatCard icon={<Sparkles />} label="可自动" value={count(summary.auto_ready)} tone={summary.auto_ready ? 'ok' : 'neutral'} hint="低风险可提交任务" />
        <StatCard icon={<ShieldAlert />} label="需确认" value={count(summary.confirm_required)} tone={summary.confirm_required ? 'warn' : 'ok'} hint="高风险先审阅" />
        <StatCard icon={<Clock3 />} label="运行/失败" value={`${count(summary.running)} / ${count(summary.failed)}`} tone={summary.failed ? 'error' : summary.running ? 'warn' : 'ok'} hint="来自任务状态" />
        <StatCard icon={<Gauge />} label="低/中风险" value={`${count(summary.low)} / ${count(summary.medium)}`} tone={summary.medium ? 'warn' : 'ok'} />
        <StatCard icon={<AlertTriangle />} label="高/关键风险" value={`${count(summary.high)} / ${count(summary.critical)}`} tone={summary.high || summary.critical ? 'error' : 'ok'} />
      </div>

      <div className="readonlyFilter smartActionsFilters">
        <label className="smartActionsSearch">
          <span>搜索</span>
          <div className="inputWithIcon">
            <Search size={15} />
            <input
              className="input"
              value={filters.q}
              placeholder="剧名、来源、动作或证据"
              onChange={(event) => setFilters((prev) => ({ ...prev, q: event.target.value }))}
            />
          </div>
        </label>
        <FilterSelect
          label="动作类型"
          value={filters.actionType}
          options={ACTION_TYPE_OPTIONS}
          onChange={(actionType) => setFilters((prev) => ({ ...prev, actionType }))}
        />
        <FilterSelect
          label="状态"
          value={filters.status}
          options={STATUS_OPTIONS}
          onChange={(status) => setFilters((prev) => ({ ...prev, status }))}
        />
        <FilterSelect
          label="风险"
          value={filters.risk}
          options={RISK_OPTIONS}
          onChange={(risk) => setFilters((prev) => ({ ...prev, risk }))}
        />
        <FilterSelect
          label="对象"
          value={filters.subjectKind}
          options={SUBJECT_KIND_OPTIONS}
          onChange={(subjectKind) => setFilters((prev) => ({ ...prev, subjectKind }))}
        />
        <label>
          <span>库名</span>
          <input
            className="input"
            value={filters.lib}
            placeholder="例如 电视剧"
            onChange={(event) => setFilters((prev) => ({ ...prev, lib: event.target.value }))}
          />
        </label>
      </div>

      {data?.warnings && data.warnings.length > 0 && (
        <div className="notice warn whitespaceNotice">
          {data.warnings.map((warning) => <div key={warning}>{warning}</div>)}
        </div>
      )}

      <section className="smartActionsPolicyOverview" aria-label="智能动作策略执行视图">
        <article>
          <ShieldCheck size={18} />
          <div>
            <strong>{count(summary.auto_ready)} 个 auto_ready</strong>
            <span>当前可批量选择 {count(batchableActions.length)} 个低风险动作</span>
          </div>
        </article>
        <article>
          <ShieldAlert size={18} />
          <div>
            <strong>{count(summary.confirm_required)} 个需确认</strong>
            <span>confirm、高风险和破坏性动作只展示原因，不直接执行</span>
          </div>
        </article>
        <article>
          <Info size={18} />
          <div>
            <strong>{count(blockedVisibleCount)} 个不可批量</strong>
            <span>打开详情可查看证据、风险、计划和验收条件</span>
          </div>
        </article>
      </section>

      <section className="readonlyBlock">
        <div className="smartActionsListHead">
          <div>
            <h2>建议清单</h2>
            <span>{loading ? '正在刷新...' : `显示 ${count(data?.actions.length)} / ${count(data?.total)} 项`}</span>
          </div>
          <span className="badge neutral"><Workflow size={13} /> 对象级动作</span>
        </div>
        <div className="smartActionsBulkBar" aria-label="智能动作批量选择">
          <div>
            <strong>批量选择雏形</strong>
            <span>
              已选 {count(selectedBatchableActions.length)} 项 · 仅低风险 auto_ready 可批量执行
            </span>
          </div>
          <div className="smartActionsBulkButtons">
            <button className="btn ghost compact" onClick={selectAllBatchable} disabled={loading || !batchableActions.length || bulkExecuting}>
              <SquareCheckBig size={15} />
              选择全部可批量
            </button>
            <button className="btn ghost compact" onClick={clearSelection} disabled={!selectedIds.size || bulkExecuting}>
              清空选择
            </button>
            <button className="btn primary compact" onClick={executeSelected} disabled={!selectedBatchableActions.length || bulkExecuting}>
              <Play size={15} />
              {bulkExecuting ? '提交中' : `批量执行 ${count(selectedBatchableActions.length)} 项`}
            </button>
          </div>
          {bulkResult && (
            <div className={`smartActionsBulkResult ${bulkResult.failed.length ? 'warn' : 'ok'}`} aria-label="批量执行结果">
              <strong>已提交 {count(bulkResult.submitted)} 项</strong>
              {bulkResult.failed.length > 0 && <span>失败 {count(bulkResult.failed.length)} 项：{bulkResult.failed.map((item) => `${item.title}(${item.message})`).join('；')}</span>}
              {!bulkResult.failed.length && <span>低风险动作已进入任务中心，后续进度在那里查看。</span>}
            </div>
          )}
        </div>
        <div className="smartActionList">
          {actions.map((action) => (
            <SmartActionCard
              key={action.id}
              action={action}
              selected={selectedIds.has(action.id)}
              batchBlockedReason={batchBlocker(action)}
              onDetail={openDetail}
              onToggleSelect={toggleActionSelection}
            />
          ))}
          {!loading && data && data.actions.length === 0 && (
            <div className="empty inlineEmpty">没有匹配的智能动作</div>
          )}
          {loading && !data && <div className="empty inlineEmpty">正在加载智能动作...</div>}
        </div>
      </section>

      {selected && (
        <Drawer title="智能动作详情" onClose={() => setSelected(null)}>
          <SmartActionDetail action={selected} loading={detailLoading} error={detailError} onNavigate={onNavigate} />
        </Drawer>
      )}
    </section>
  );
}
