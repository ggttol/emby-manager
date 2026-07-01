import { Bell, ChevronDown, ChevronUp, Copy, Lightbulb, RefreshCw, Search, XCircle } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { Drawer } from './Drawer';
import { useToast } from './Toast';

type TaskRun = components['schemas']['TaskRun'];
type TaskListResponse = components['schemas']['TaskListResponse'];
type SmartAction = components['schemas']['SmartAction'];
type SmartActionFromTaskResponse = components['schemas']['SmartActionFromTaskResponse'];
type SmartActionFromNextActionResponse = components['schemas']['SmartActionFromNextActionResponse'];
type SmartNextAction = components['schemas']['SmartNextAction'];

const active = new Set(['pending', 'running']);
const terminal = new Set(['done', 'error', 'cancelled', 'interrupted']);
type TaskFilter = 'all' | 'active' | 'done' | 'issue';

export const TASK_COMPLETED_EVENT = 'emby-manager:task-completed';

export type TaskCompleteDetail = {
  task: TaskRun;
  previousTask: TaskRun;
  previousStatus: string;
};

type TaskCenterProps = {
  onTaskComplete?: (detail: TaskCompleteDetail) => void;
  onOpenSmartAction?: (action: SmartAction) => void;
};

const statusLabels: Record<string, string> = {
  pending: '排队',
  running: '运行中',
  done: '完成',
  error: '失败',
  cancelled: '已取消',
  interrupted: '已中断'
};

const smartActionTypeLabels: Record<string, string> = {
  transfer_add_new: '一条龙新增转存',
  transfer_update_series: '追更更新转存',
  dedup_remove_old: '去重删除旧资源',
  dedup_review: '去重复核',
  undo_review: 'Undo 复查',
  poster_fix: '海报修复',
  metadata_refresh: '元数据刷新',
  library_scan: '媒体库刷新',
  archive_series: '完结剧归档',
  archive_review: '归档复查',
  config_tmdb: 'TMDb 配置',
  config_fix: '配置检查',
  cleanup_empty_folder: '清理空目录',
  task_retry_or_diagnose: '任务诊断/重试'
};

const smartNextTabLabels: Record<string, string> = {
  smart_actions: '智能动作',
  'smart-actions': '智能动作',
  dashboard: '仪表盘',
  scan: '扫描',
  c115: '115 转存',
  catalog: '找资源',
  zhuigeng: '追更检查',
  gaps: '缺集检查',
  posters: '海报修复',
  dedup: '去重',
  manage: '删除·移动',
  cleanup: '智能清理',
  system: '系统',
  schedules: '定时',
  logs: '日志',
  users: '用户',
  settings: '设置'
};

const smartSubjectKindLabels: Record<string, string> = {
  movie: '电影',
  series: '电视剧',
  season: '季',
  episode: '剧集',
  library: '媒体库',
  task: '任务',
  system: '系统',
  unknown: '对象'
};

const smartStepStatusLabels: Record<string, string> = {
  pending: '排队',
  running: '运行中',
  done: '完成',
  error: '失败',
  skipped: '跳过',
  cancelled: '已取消',
  interrupted: '已中断'
};

const smartVerificationStatusLabels: Record<string, string> = {
  done: '已通过',
  partial: '需复查',
  failed: '失败',
  cancelled: '已取消',
  running: '验收中',
  queued: '等待验收',
  suggested: '待执行'
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function statusLabel(status: string) {
  return statusLabels[status] || status;
}

function plainObject(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function numberLike(value: unknown) {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
}

function falseLike(value: unknown) {
  return value === false || value === 'false';
}

function nestedNumber(parent: Record<string, unknown> | null, key: string) {
  return numberLike(parent?.[key]) ?? 0;
}

function hasAddNewBlockingFailure(result: Record<string, unknown>) {
  const check = plainObject(result.check);
  if (!check) return null;

  const itemErrors = nestedNumber(check, 'item_error_count');
  const stageErrors = nestedNumber(check, 'stage_error_count');
  if (String(check.status || '') === 'errors' || itemErrors > 0 || stageErrors > 0) return true;

  const transfer = plainObject(result.transfer);
  if (falseLike(transfer?.ok) || nestedNumber(transfer, 'failed') > 0) return true;

  for (const key of ['strm', 'scan', 'poster']) {
    const section = plainObject(result[key]);
    if (falseLike(section?.ok)) return true;
  }

  for (const key of ['auto_resolve', 'poster_auto_fix']) {
    const section = plainObject(result[key]);
    if (falseLike(section?.ok) || nestedNumber(section, 'error_count') > 0) return true;
  }

  return false;
}

function hasFailedResult(task: TaskRun) {
  if (task.status !== 'done') return false;
  const result = plainObject(task.result);
  if (!result) return false;
  const addNewFailure = hasAddNewBlockingFailure(result);
  if (addNewFailure !== null) return addNewFailure;
  const errorCount = numberLike(result.error_count);
  if (errorCount !== null && errorCount > 0) return true;
  if (falseLike(result.ok)) return true;
  return false;
}

function effectiveStatus(task: TaskRun) {
  return hasFailedResult(task) ? 'error' : task.status;
}

function failedResultSummary(task: TaskRun) {
  if (!hasFailedResult(task)) return '';
  const result = plainObject(task.result);
  const errorCount = numberLike(result?.error_count);
  const total = numberLike(result?.total);
  if (errorCount !== null && total !== null) return `结果失败：${errorCount}/${total} 项失败`;
  if (errorCount !== null) return `结果失败：${errorCount} 项失败`;
  return '结果失败：任务返回 ok:false';
}

function hasPartialResult(task: TaskRun) {
  const result = plainObject(task.result);
  if (!result) return false;
  const verification = plainObject(result.verification);
  const values = [
    stringField(result.status),
    stringField(result.overall_status),
    stringField(result.outcome),
    stringField(verification?.status)
  ].map((value) => value.toLowerCase());
  return values.some((value) => ['partial', 'issue', 'issues'].includes(value));
}

function canGenerateDiagnosticAction(task: TaskRun) {
  if (active.has(task.status)) return false;
  const status = effectiveStatus(task).toLowerCase();
  return ['error', 'cancelled', 'interrupted', 'partial'].includes(status)
    || Boolean(task.error)
    || hasPartialResult(task);
}

function formatTime(value?: string | null) {
  if (!value) return '未记录';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  });
}

function formatDuration(start?: string | null, end?: string | null) {
  if (!start || !end) return '未记录';
  const started = new Date(start).getTime();
  const ended = new Date(end).getTime();
  if (Number.isNaN(started) || Number.isNaN(ended) || ended < started) return '未记录';
  const seconds = Math.max(0, Math.round((ended - started) / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes < 60) return rest ? `${minutes} 分 ${rest} 秒` : `${minutes} 分`;
  const hours = Math.floor(minutes / 60);
  const minuteRest = minutes % 60;
  return minuteRest ? `${hours} 小时 ${minuteRest} 分` : `${hours} 小时`;
}

function compactJson(value: unknown) {
  try {
    return JSON.stringify(value) ?? '';
  } catch {
    return String(value);
  }
}

function resultPreview(task: TaskRun) {
  const result = plainObject(task.result);
  if (!result) {
    if (task.result === null || task.result === undefined) return '';
    return typeof task.result === 'string' ? task.result : String(task.result);
  }

  if (isSmartActionResultTask(task)) {
    return smartActionResultPreview(task, result);
  }

  if (task.kind === 'add_new' || task.kind === 'zhuigeng_update') {
    const transfer = plainObject(result.transfer);
    const strm = plainObject(result.strm);
    const autoResolve = plainObject(result.auto_resolve);
    const posterFix = plainObject(result.poster_auto_fix);
    const check = plainObject(result.check);
    const parts = [
      `转存 ${nestedNumber(transfer, 'succeeded')}/${nestedNumber(transfer, 'total')}`,
      `新增 STRM ${nestedNumber(strm, 'new_count')}`,
      nestedNumber(autoResolve, 'resolved_count') > 0 ? `清理旧版本 ${nestedNumber(autoResolve, 'resolved_count')}` : '',
      nestedNumber(posterFix, 'fixed_count') > 0 ? `修海报 ${nestedNumber(posterFix, 'fixed_count')}` : '',
      nestedNumber(check, 'suspicious_count') > 0 ? `可疑项 ${nestedNumber(check, 'suspicious_count')}` : ''
    ].filter(Boolean);
    return parts.join(' · ');
  }

  if (task.kind === 'zhuigeng_archive' || task.kind.includes('batch')) {
    const total = numberLike(result.total);
    const okCount = numberLike(result.ok_count);
    const errorCount = numberLike(result.error_count);
    const fromLib = typeof result.from_lib === 'string' ? result.from_lib : '';
    const toLib = typeof result.to_lib === 'string' ? result.to_lib : '';
    const route = fromLib && toLib ? `${fromLib} -> ${toLib}` : '';
    const parts = [
      okCount !== null && total !== null ? `成功 ${okCount}/${total}` : '',
      errorCount !== null && errorCount > 0 ? `失败 ${errorCount}` : '',
      route
    ].filter(Boolean);
    if (parts.length > 0) return parts.join(' · ');
  }

  const total = numberLike(result.total);
  const okCount = numberLike(result.ok_count);
  const errorCount = numberLike(result.error_count);
  if (okCount !== null && total !== null) {
    return [`成功 ${okCount}/${total}`, errorCount !== null && errorCount > 0 ? `失败 ${errorCount}` : ''].filter(Boolean).join(' · ');
  }
  const imported = numberLike(result.imported);
  if (imported !== null) return `导入 ${imported} 项`;
  return '';
}

type ResultFactTone = 'neutral' | 'ok' | 'warn' | 'error';

type ResultFact = {
  label: string;
  value: string;
  tone?: ResultFactTone;
};

type ResultStage = {
  title: string;
  tone: ResultFactTone;
  facts: ResultFact[];
  notes: string[];
};

function stringField(value: unknown) {
  return typeof value === 'string' && value.trim() ? value.trim() : '';
}

function booleanField(value: unknown) {
  return typeof value === 'boolean' ? value : null;
}

function fact(label: string, value: unknown, tone: ResultFactTone = 'neutral'): ResultFact | null {
  if (value === null || value === undefined || value === '') return null;
  return { label, value: String(value), tone };
}

function compactCount(value: unknown) {
  const number = numberLike(value);
  return number === null ? '0' : number.toLocaleString('zh-CN');
}

function collectionCount(value: unknown) {
  const number = numberLike(value);
  if (number !== null) return number;
  if (Array.isArray(value)) return value.length;
  const object = plainObject(value);
  return object ? Object.keys(object).length : 0;
}

function okTone(value: unknown, fallback: ResultFactTone = 'neutral'): ResultFactTone {
  const bool = booleanField(value);
  if (bool === true) return 'ok';
  if (bool === false) return 'error';
  return fallback;
}

function stage(title: string, tone: ResultFactTone, facts: Array<ResultFact | null>, notes: string[] = []): ResultStage {
  return {
    title,
    tone,
    facts: facts.filter(Boolean) as ResultFact[],
    notes: notes.filter(Boolean)
  };
}

function mappedLabel(labels: Record<string, string>, value: unknown, fallback = '未记录') {
  const raw = stringField(value);
  if (!raw) return fallback;
  return labels[raw] || raw;
}

function isSmartNextAction(value: unknown): value is SmartNextAction {
  const action = plainObject(value);
  return Boolean(action
    && stringField(action.action_type)
    && stringField(action.label)
    && stringField(action.tab)
    && stringField(action.reason));
}

function smartNextActions(value: unknown): Array<string | SmartNextAction> {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string | SmartNextAction => (
    typeof item === 'string' || isSmartNextAction(item)
  ));
}

function smartNextActionKey(task: TaskRun, action: SmartNextAction, index: number) {
  return `${task.id}:${action.action_type}:${action.tab}:${action.label}:${index}`;
}

function smartNextActionText(action: SmartNextAction, index: number) {
  const label = action.label || `动作 ${index + 1}`;
  const tabLabel = action.tab ? mappedLabel(smartNextTabLabels, action.tab, action.tab) : '';
  return [label, tabLabel ? `入口：${tabLabel}` : '', action.reason].filter(Boolean).join(' · ');
}

function listMessages(value: unknown, max = 3) {
  if (!Array.isArray(value)) return [];
  return value
    .map((item) => {
      if (typeof item === 'string') return item;
      const row = plainObject(item);
      if (!row) return '';
      return [
        stringField(row.name) || stringField(row.label) || stringField(row.stage) || stringField(row.tmdb),
        stringField(row.status),
        stringField(row.reason) || stringField(row.message) || stringField(row.error) || stringField(row.err)
      ].filter(Boolean).join(' · ');
    })
    .filter(Boolean)
    .slice(0, max);
}

function isSmartActionResultTask(task: TaskRun) {
  const result = plainObject(task.result);
  return task.kind === 'smart_action_execute' || Boolean(result && (
    Object.prototype.hasOwnProperty.call(result, 'action_id')
    || Object.prototype.hasOwnProperty.call(result, 'action_type')
    || Object.prototype.hasOwnProperty.call(result, 'steps')
    || Object.prototype.hasOwnProperty.call(result, 'verification')
  ));
}

function smartSubjectTitle(subject: Record<string, unknown> | null) {
  if (!subject) return '未记录对象';
  const name = stringField(subject.name)
    || stringField(subject.folder)
    || stringField(subject.strm_path)
    || stringField(subject.cd_path)
    || stringField(subject.emby_id)
    || '未记录对象';
  const year = numberLike(subject.year);
  return year ? `${name} (${year})` : name;
}

function smartSubjectFacts(subject: Record<string, unknown> | null) {
  if (!subject) return [];
  return [
    fact('类型', mappedLabel(smartSubjectKindLabels, subject.kind, '对象')),
    fact('库', stringField(subject.lib)),
    fact('TMDb', stringField(subject.tmdb)),
    fact('Emby', stringField(subject.emby_id)),
    fact('路径', stringField(subject.folder) || stringField(subject.strm_path) || stringField(subject.cd_path))
  ].filter(Boolean) as ResultFact[];
}

function smartActionSteps(result: Record<string, unknown>) {
  const steps = Array.isArray(result.steps) ? result.steps : [];
  return steps.map((item, index) => {
    const row = plainObject(item);
    if (!row) {
      return {
        key: `step-${index + 1}`,
        title: String(item),
        executor: '',
        status: '',
        message: '',
        result: null as Record<string, unknown> | null
      };
    }
    const stepResult = plainObject(row.result);
    return {
      key: stringField(row.key) || `step-${index + 1}`,
      title: stringField(row.title) || stringField(row.key) || `步骤 ${index + 1}`,
      executor: stringField(row.executor),
      status: stringField(row.status),
      message: stringField(row.message)
        || stringField(stepResult?.reason)
        || stringField(stepResult?.message)
        || stringField(stepResult?.err)
        || stringField(stepResult?.error),
      result: stepResult
    };
  });
}

function smartStepTone(step: ReturnType<typeof smartActionSteps>[number]): ResultFactTone {
  const status = step.status.toLowerCase();
  const ok = booleanField(step.result?.ok);
  const skipped = booleanField(step.result?.skipped);
  if (['error', 'failed'].includes(status)) return 'error';
  if (['skipped', 'cancelled', 'interrupted'].includes(status) || skipped === true) return 'warn';
  if (ok === false) return 'warn';
  if (status === 'done') return 'ok';
  return 'neutral';
}

function smartVerificationTone(verification: Record<string, unknown> | null): ResultFactTone {
  const status = stringField(verification?.status).toLowerCase();
  if (status === 'done') return 'ok';
  if (status === 'partial') return 'warn';
  if (['failed', 'error', 'cancelled', 'interrupted'].includes(status)) return 'error';
  return 'neutral';
}

function smartActionResultPreview(task: TaskRun, result: Record<string, unknown>) {
  const subject = plainObject(result.subject);
  const steps = smartActionSteps(result);
  const finished = steps.filter((step) => ['done', 'skipped'].includes(step.status.toLowerCase())).length;
  const failed = steps.filter((step) => ['error', 'failed'].includes(step.status.toLowerCase())).length;
  const verification = plainObject(result.verification);
  const verificationStatus = stringField(verification?.status);
  const parts = [
    mappedLabel(smartActionTypeLabels, result.action_type, task.label || '智能动作'),
    smartSubjectTitle(subject),
    steps.length > 0 ? `步骤 ${finished}/${steps.length}${failed > 0 ? `，失败 ${failed}` : ''}` : '',
    verificationStatus ? `验收 ${mappedLabel(smartVerificationStatusLabels, verificationStatus)}` : ''
  ].filter(Boolean);
  return parts.join(' · ');
}

function addNewStages(result: Record<string, unknown>): ResultStage[] {
  const transfer = plainObject(result.transfer);
  const strm = plainObject(result.strm);
  const scan = plainObject(result.scan);
  const poster = plainObject(result.poster);
  const posterFix = plainObject(result.poster_auto_fix);
  const autoResolve = plainObject(result.auto_resolve);
  const check = plainObject(result.check);
  const dedup = plainObject(result.dedup);

  return [
    stage('转存', okTone(transfer?.ok), [
      fact('成功', `${compactCount(transfer?.succeeded)} / ${compactCount(transfer?.total)}`, okTone(transfer?.ok)),
      nestedNumber(transfer, 'failed') > 0 ? fact('失败', compactCount(transfer?.failed), 'error') : null
    ], listMessages(transfer?.items)),
    stage('STRM', okTone(strm?.ok), [
      fact('新增', compactCount(strm?.new_count), nestedNumber(strm, 'new_count') > 0 ? 'ok' : 'neutral'),
      fact('匹配', compactCount(strm?.matched)),
      collectionCount(strm?.new_folders) > 0 ? fact('新目录', compactCount(collectionCount(strm?.new_folders))) : null
    ], [
      ...listMessages(strm?.attention),
      ...listMessages(strm?.warnings),
      stringField(strm?.error)
    ]),
    stage('Emby 刷新', okTone(scan?.ok), [
      fact('模式', stringField(scan?.mode)),
      fact('库', stringField(scan?.lib)),
      fact('HTTP', scan?.code)
    ], [stringField(scan?.warning), stringField(scan?.error)]),
    stage('海报', okTone(poster?.ok), [
      fact('问题', compactCount(poster?.issue_count), nestedNumber(poster, 'issue_count') > 0 ? 'warn' : 'ok'),
      fact('缺主图', compactCount(poster?.missing_primary_count), nestedNumber(poster, 'missing_primary_count') > 0 ? 'warn' : 'neutral'),
      fact('错绑', compactCount(poster?.mismatch_count), nestedNumber(poster, 'mismatch_count') > 0 ? 'warn' : 'neutral')
    ], listMessages(poster?.items)),
    stage('海报自动修复', okTone(posterFix?.ok, nestedNumber(posterFix, 'fixed_count') > 0 ? 'ok' : 'neutral'), [
      fact('修复', compactCount(posterFix?.fixed_count), nestedNumber(posterFix, 'fixed_count') > 0 ? 'ok' : 'neutral'),
      nestedNumber(posterFix, 'skipped_count') > 0 ? fact('跳过', compactCount(posterFix?.skipped_count), 'warn') : null,
      nestedNumber(posterFix, 'error_count') > 0 ? fact('失败', compactCount(posterFix?.error_count), 'error') : null
    ], listMessages(posterFix?.items)),
    stage('重复旧版本', okTone(autoResolve?.ok, nestedNumber(autoResolve, 'resolved_count') > 0 ? 'ok' : 'neutral'), [
      fact('清理', compactCount(autoResolve?.resolved_count), nestedNumber(autoResolve, 'resolved_count') > 0 ? 'ok' : 'neutral'),
      nestedNumber(autoResolve, 'skipped_count') > 0 ? fact('跳过', compactCount(autoResolve?.skipped_count), 'warn') : null,
      nestedNumber(autoResolve, 'error_count') > 0 ? fact('失败', compactCount(autoResolve?.error_count), 'error') : null,
      dedup ? fact('待复核', compactCount(dedup.review_count), nestedNumber(dedup, 'review_count') > 0 ? 'warn' : 'neutral') : null
    ], listMessages(autoResolve?.items)),
    stage('结果检查', okTone(check?.ok, nestedNumber(check, 'suspicious_count') > 0 ? 'warn' : 'ok'), [
      fact('状态', stringField(check?.status)),
      nestedNumber(check, 'suspicious_count') > 0 ? fact('可疑项', compactCount(check?.suspicious_count), 'warn') : null,
      nestedNumber(check, 'stage_error_count') > 0 ? fact('阶段错误', compactCount(check?.stage_error_count), 'error') : null
    ], [
      stringField(check?.message),
      ...listMessages(check?.errors),
      ...listMessages(check?.suspicious)
    ])
  ];
}

function genericResultStages(result: Record<string, unknown>): ResultStage[] {
  const stages: ResultStage[] = [];
  const total = numberLike(result.total);
  const okCount = numberLike(result.ok_count);
  const errorCount = numberLike(result.error_count);
  if (total !== null || okCount !== null || errorCount !== null) {
    stages.push(stage('执行结果', errorCount && errorCount > 0 ? 'error' : 'ok', [
      okCount !== null && total !== null ? fact('成功', `${compactCount(okCount)} / ${compactCount(total)}`, errorCount && errorCount > 0 ? 'warn' : 'ok') : null,
      errorCount && errorCount > 0 ? fact('失败', compactCount(errorCount), 'error') : null,
      fact('总数', total !== null ? compactCount(total) : null)
    ], listMessages(result.results || result.items)));
  }
  if (typeof result.ok === 'boolean') {
    stages.push(stage('状态', result.ok ? 'ok' : 'error', [
      fact('ok', result.ok ? 'true' : 'false', result.ok ? 'ok' : 'error'),
      fact('message', stringField(result.message) || stringField(result.msg))
    ], listMessages(result.warnings)));
  }
  return stages;
}

function dedupBatchStages(result: Record<string, unknown>): ResultStage[] {
  const rows = Array.isArray(result.results) ? result.results : [];
  const warnings = rows.flatMap((item) => {
    const row = plainObject(item);
    return row ? listMessages(row.warnings, 8) : [];
  });
  const errors = rows.flatMap((item) => {
    const row = plainObject(item);
    if (!row) return [];
    return [
      ...listMessages(row.errors, 8),
      stringField(row.err)
    ].filter(Boolean);
  });
  return [
    stage('批量去重', numberLike(result.error_count) && numberLike(result.error_count)! > 0 ? 'error' : warnings.length > 0 ? 'warn' : 'ok', [
      fact('成功', `${compactCount(result.ok_count)} / ${compactCount(result.total)}`, numberLike(result.error_count) ? 'warn' : 'ok'),
      numberLike(result.error_count) && numberLike(result.error_count)! > 0 ? fact('失败', compactCount(result.error_count), 'error') : null,
      warnings.length > 0 ? fact('警告', compactCount(warnings.length), 'warn') : null
    ], [
      ...errors,
      ...warnings
    ])
  ];
}

function resultStages(task: TaskRun): ResultStage[] {
  const result = plainObject(task.result);
  if (!result) return [];
  if (isSmartActionResultTask(task)) return [];
  if (task.kind === 'add_new' || task.kind === 'zhuigeng_update') {
    return addNewStages(result);
  }
  if (task.kind === 'dedup_exec_batch') {
    return dedupBatchStages(result);
  }
  return genericResultStages(result);
}

function SmartActionResultPanel({
  task,
  creatingNextActionKey,
  nextActionSmartActions,
  onCreateNextAction,
  onOpenSmartAction
}: {
  task: TaskRun;
  creatingNextActionKey: string | null;
  nextActionSmartActions: Record<string, SmartAction>;
  onCreateNextAction: (task: TaskRun, action: SmartNextAction, index: number) => void;
  onOpenSmartAction?: (action: SmartAction) => void;
}) {
  const result = plainObject(task.result);
  if (!result || !isSmartActionResultTask(task)) return null;

  const subject = plainObject(result.subject);
  const subjectFacts = smartSubjectFacts(subject);
  const steps = smartActionSteps(result);
  const verification = plainObject(result.verification);
  const verificationTone = smartVerificationTone(verification);
  const verificationChecks = Array.isArray(verification?.checks) ? verification.checks : [];
  const nextActions = smartNextActions(result.next_actions);

  return (
    <section className="taskSmartActionPanel">
      <h4>智能动作结果</h4>
      <div className="taskSmartActionOverview">
        <article>
          <span>动作类型</span>
          <strong>{mappedLabel(smartActionTypeLabels, result.action_type, task.label || '智能动作')}</strong>
        </article>
        <article>
          <span>对象</span>
          <strong>{smartSubjectTitle(subject)}</strong>
          {subjectFacts.length > 0 && (
            <dl>
              {subjectFacts.map((entry) => (
                <div key={entry.label}>
                  <dt>{entry.label}</dt>
                  <dd>{entry.value}</dd>
                </div>
              ))}
            </dl>
          )}
        </article>
      </div>

      {steps.length > 0 && (
        <div className="taskSmartActionBlock">
          <h5>步骤列表</h5>
          <ol className="taskSmartActionSteps">
            {steps.map((step) => {
              const tone = smartStepTone(step);
              const childTask = plainObject(step.result?.task);
              const childTaskLabel = stringField(childTask?.label) || stringField(childTask?.kind);
              const childTaskStatus = stringField(childTask?.status);
              return (
                <li className={tone} key={step.key}>
                  <span className={`badge ${tone}`}>
                    {mappedLabel(smartStepStatusLabels, step.status, step.status || '未记录')}
                  </span>
                  <div>
                    <strong>{step.title}</strong>
                    {step.message && <p>{step.message}</p>}
                    {(step.executor || childTaskLabel) && (
                      <small>
                        {[
                          step.executor ? `执行器 ${step.executor}` : '',
                          childTaskLabel ? `下游任务 ${childTaskLabel}${childTaskStatus ? `（${statusLabel(childTaskStatus)}）` : ''}` : ''
                        ].filter(Boolean).join(' · ')}
                      </small>
                    )}
                  </div>
                </li>
              );
            })}
          </ol>
        </div>
      )}

      {verification && (
        <div className={`taskSmartActionVerification ${verificationTone}`}>
          <h5>验收状态</h5>
          <strong>{mappedLabel(smartVerificationStatusLabels, verification.status, '未记录')}</strong>
          {stringField(verification.message) && <p>{stringField(verification.message)}</p>}
          {verificationChecks.length > 0 && (
            <ul>
              {verificationChecks.map((item, index) => {
                const check = plainObject(item);
                const title = stringField(check?.title) || stringField(check?.key) || `检查 ${index + 1}`;
                const expected = stringField(check?.expected);
                return <li key={`${title}-${index}`}>{expected ? `${title}：${expected}` : title}</li>;
              })}
            </ul>
          )}
        </div>
      )}

      {nextActions.length > 0 && (
        <div className="taskSmartActionBlock">
          <h5>后续动作</h5>
          <ul className="taskSmartActionNext">
            {nextActions.map((item, index) => {
              if (typeof item === 'string') return <li key={`${item}-${index}`}>{item}</li>;
              const key = smartNextActionKey(task, item, index);
              const createdAction = nextActionSmartActions[key];
              return (
                <li className="taskSmartActionNextItem" key={key}>
                  <div>
                    <span>{smartNextActionText(item, index)}</span>
                    {createdAction && (
                      <article className="taskDiagnosticAction" aria-label="已生成后续智能动作">
                        <span className="badge info">已生成</span>
                        <div>
                          <strong>{createdAction.title}</strong>
                          <p>{createdAction.summary}</p>
                          <small>
                            建议入口：{createdAction.tab || '智能动作'}
                            {createdAction.action_label ? ` · ${createdAction.action_label}` : ''}
                          </small>
                        </div>
                        {onOpenSmartAction && (
                          <button className="btn ghost compact" onClick={() => onOpenSmartAction(createdAction)}>
                            打开详情
                          </button>
                        )}
                      </article>
                    )}
                  </div>
                  <button
                    className="btn ghost compact"
                    onClick={() => onCreateNextAction(task, item, index)}
                    disabled={creatingNextActionKey === key}
                  >
                    <Lightbulb size={14} />
                    {creatingNextActionKey === key ? '生成中' : createdAction ? '重新生成' : '生成智能动作'}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </section>
  );
}

function TaskResultPanel({
  task,
  creatingNextActionKey,
  nextActionSmartActions,
  onCreateNextAction,
  onOpenSmartAction
}: {
  task: TaskRun;
  creatingNextActionKey: string | null;
  nextActionSmartActions: Record<string, SmartAction>;
  onCreateNextAction: (task: TaskRun, action: SmartNextAction, index: number) => void;
  onOpenSmartAction?: (action: SmartAction) => void;
}) {
  if (isSmartActionResultTask(task)) {
    return (
      <SmartActionResultPanel
        task={task}
        creatingNextActionKey={creatingNextActionKey}
        nextActionSmartActions={nextActionSmartActions}
        onCreateNextAction={onCreateNextAction}
        onOpenSmartAction={onOpenSmartAction}
      />
    );
  }
  const stages = resultStages(task).filter((item) => item.facts.length > 0 || item.notes.length > 0);
  if (!stages.length) return null;
  return (
    <section className="taskResultPanel">
      <h4>结果摘要</h4>
      <div className="taskResultStages">
        {stages.map((item) => (
          <article className={`taskResultStage ${item.tone}`} key={item.title}>
            <strong>{item.title}</strong>
            {item.facts.length > 0 && (
              <dl>
                {item.facts.map((entry) => (
                  <div key={`${item.title}-${entry.label}`}>
                    <dt>{entry.label}</dt>
                    <dd className={entry.tone || ''}>{entry.value}</dd>
                  </div>
                ))}
              </dl>
            )}
            {item.notes.map((note) => <p key={`${item.title}-${note}`}>{note}</p>)}
          </article>
        ))}
      </div>
    </section>
  );
}

function prettyJson(value: unknown) {
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function hasJsonPayload(value: unknown) {
  if (value === null || value === undefined) return false;
  if (typeof value === 'object' && !Array.isArray(value)) return Object.keys(value).length > 0;
  if (Array.isArray(value)) return value.length > 0;
  return true;
}

function searchText(value: unknown) {
  return compactJson(value).toLocaleLowerCase('zh-CN');
}

function taskMatchesQuery(task: TaskRun, tokens: string[]) {
  if (tokens.length === 0) return true;
  const haystack = [
    task.id,
    task.kind,
    task.label,
    task.status,
    effectiveStatus(task),
    task.status_text,
    task.source,
    task.error,
    searchText(task.params),
    searchText(task.result)
  ].filter(Boolean).join('\n').toLocaleLowerCase('zh-CN');
  return tokens.every((token) => haystack.includes(token));
}

function isCompletedTransition(previous?: TaskRun, next?: TaskRun) {
  if (!previous || !next) return false;
  return active.has(previous.status) && terminal.has(next.status);
}

function dateMs(value?: string | null) {
  if (!value) return 0;
  const ms = new Date(value).getTime();
  return Number.isNaN(ms) ? 0 : ms;
}

export function TaskCenter({ onTaskComplete, onOpenSmartAction }: TaskCenterProps = {}) {
  const [open, setOpen] = useState(false);
  const [tasks, setTasks] = useState<TaskRun[]>([]);
  const [activeCount, setActiveCount] = useState(0);
  const [loadError, setLoadError] = useState('');
  const [cancellingId, setCancellingId] = useState<string | null>(null);
  const [diagnosingId, setDiagnosingId] = useState<string | null>(null);
  const [diagnosticActions, setDiagnosticActions] = useState<Record<string, SmartAction>>({});
  const [creatingNextActionKey, setCreatingNextActionKey] = useState<string | null>(null);
  const [nextActionSmartActions, setNextActionSmartActions] = useState<Record<string, SmartAction>>({});
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => new Set());
  const [filter, setFilter] = useState<TaskFilter>('all');
  const [query, setQuery] = useState('');
  const knownTasksRef = useRef<Map<string, TaskRun>>(new Map());
  const emittedIdsRef = useRef<Set<string>>(new Set());
  const initializedRef = useRef(false);
  const lastSnapshotAtRef = useRef(0);
  const toast = useToast();

  const emitCompletedTasks = useCallback((nextTasks: TaskRun[]) => {
    const knownTasks = knownTasksRef.current;
    const wasInitialized = initializedRef.current;
    const lastSnapshotAt = lastSnapshotAtRef.current;
    const completed = nextTasks
      .map((task) => {
        const previousTask = knownTasks.get(task.id);
        const firstSeenTerminal = wasInitialized
          && !previousTask
          && terminal.has(task.status)
          && Math.max(dateMs(task.queued_at), dateMs(task.updated_at)) >= lastSnapshotAt;
        return { task, previousTask, firstSeenTerminal };
      })
      .filter(({ task, previousTask, firstSeenTerminal }) => (
        (isCompletedTransition(previousTask, task) || firstSeenTerminal)
        && !emittedIdsRef.current.has(task.id)
      ));

    knownTasksRef.current = new Map(nextTasks.map((task) => [task.id, task]));
    initializedRef.current = true;
    lastSnapshotAtRef.current = Date.now();

    for (const { task, previousTask } of completed) {
      emittedIdsRef.current.add(task.id);
      const detail: TaskCompleteDetail = {
        task,
        previousTask: previousTask || { ...task, status: 'running' },
        previousStatus: previousTask?.status || 'running'
      };
      onTaskComplete?.(detail);
      window.dispatchEvent(new CustomEvent<TaskCompleteDetail>(TASK_COMPLETED_EVENT, { detail }));
    }
  }, [onTaskComplete]);

  const load = useCallback(async (options: { silent?: boolean } = {}) => {
    try {
      const data = await api<TaskListResponse>('/api/v2/tasks?limit=50');
      emitCompletedTasks(data.tasks);
      setTasks(data.tasks);
      setActiveCount(data.active_count);
      setLoadError('');
    } catch (e) {
      const message = errorMessage(e);
      setLoadError(message);
      if (!options.silent) {
        toast.push(`任务中心加载失败：${message}`, 'error');
      }
    }
  }, [emitCompletedTasks, toast]);

  useEffect(() => {
    load({ silent: true });
    const timer = window.setInterval(() => {
      load({ silent: true });
    }, activeCount > 0 ? 900 : 5000);
    return () => window.clearInterval(timer);
  }, [activeCount, load]);

  const pct = useMemo(() => {
    const running = tasks.filter((task) => active.has(task.status));
    const total = running.reduce((sum, task) => sum + (task.total || 0), 0);
    const progress = running.reduce((sum, task) => sum + (task.progress || 0), 0);
    return total > 0 ? Math.min(100, Math.round((progress / total) * 100)) : activeCount ? 5 : 0;
  }, [activeCount, tasks]);

  const counts = useMemo(() => ({
    all: tasks.length,
    active: tasks.filter((task) => active.has(task.status)).length,
    done: tasks.filter((task) => effectiveStatus(task) === 'done').length,
    issue: tasks.filter((task) => ['error', 'cancelled', 'interrupted'].includes(effectiveStatus(task))).length
  }), [tasks]);

  const visibleTasks = useMemo(() => {
    const tokens = query.trim().toLocaleLowerCase('zh-CN').split(/\s+/).filter(Boolean);
    return tasks.filter((task) => {
      const status = effectiveStatus(task);
      if (filter === 'active' && !active.has(task.status)) return false;
      if (filter === 'done' && status !== 'done') return false;
      if (filter === 'issue' && !['error', 'cancelled', 'interrupted'].includes(status)) return false;
      return taskMatchesQuery(task, tokens);
    });
  }, [filter, query, tasks]);

  const cancel = async (task: TaskRun) => {
    setCancellingId(task.id);
    try {
      const res = await api<components['schemas']['TaskCancelResponse']>(`/api/v2/tasks/${task.id}/cancel`, { method: 'POST' });
      toast.push(res.ok ? `已请求取消：${task.label || task.kind}` : '任务已结束或不存在', res.ok ? 'warn' : 'info');
      await load({ silent: true });
    } catch (e) {
      toast.push(`取消任务失败：${errorMessage(e)}`, 'error');
    } finally {
      setCancellingId(null);
    }
  };

  const toggleExpanded = (taskId: string) => {
    setExpandedIds((current) => {
      const next = new Set(current);
      if (next.has(taskId)) next.delete(taskId);
      else next.add(taskId);
      return next;
    });
  };

  const copyTaskId = async (task: TaskRun) => {
    try {
      await navigator.clipboard.writeText(task.id);
      toast.push('任务 ID 已复制', 'ok');
    } catch (e) {
      toast.push(`复制失败：${errorMessage(e)}`, 'error');
    }
  };

  const generateDiagnosticAction = async (task: TaskRun) => {
    setDiagnosingId(task.id);
    try {
      const data = await api<SmartActionFromTaskResponse>(`/api/v2/smart-actions/from-task/${task.id}`, { method: 'POST' });
      setDiagnosticActions((current) => ({ ...current, [task.id]: data.action }));
      toast.push(`已生成诊断智能动作：${data.action.title}`, 'ok');
    } catch (e) {
      toast.push(`生成诊断智能动作失败：${errorMessage(e)}`, 'error');
    } finally {
      setDiagnosingId(null);
    }
  };

  const createNextSmartAction = async (task: TaskRun, nextAction: SmartNextAction, index: number) => {
    const key = smartNextActionKey(task, nextAction, index);
    const result = plainObject(task.result);
    const sourceActionId = stringField(result?.action_id);
    setCreatingNextActionKey(key);
    try {
      const data = await api<SmartActionFromNextActionResponse>('/api/v2/smart-actions/from-next-action', {
        method: 'POST',
        body: JSON.stringify({
          task_id: task.id,
          source_action_id: sourceActionId || undefined,
          next_action: nextAction,
          persist: true
        })
      });
      setNextActionSmartActions((current) => ({ ...current, [key]: data.action }));
      const warningText = data.warnings.length ? `，${data.warnings[0]}` : '';
      toast.push(`已生成后续智能动作：${data.action.title}${warningText}`, data.warnings.length ? 'warn' : 'ok');
    } catch (e) {
      toast.push(`生成后续智能动作失败：${errorMessage(e)}`, 'error');
    } finally {
      setCreatingNextActionKey(null);
    }
  };

  const expandVisible = () => {
    setExpandedIds((current) => {
      const next = new Set(current);
      visibleTasks.forEach((task) => next.add(task.id));
      return next;
    });
  };

  const collapseVisible = () => {
    setExpandedIds((current) => {
      const next = new Set(current);
      visibleTasks.forEach((task) => next.delete(task.id));
      return next;
    });
  };

  return (
    <>
      <button
        className="bell"
        onClick={() => {
          setOpen(true);
          load();
        }}
        aria-label="任务中心"
      >
        <Bell size={18} />
        {activeCount > 0 && <span>{activeCount}</span>}
      </button>
      {activeCount > 0 && <div className="globalProgress"><i style={{ width: `${pct}%` }} /></div>}
      {open && (
        <Drawer title="任务中心" onClose={() => setOpen(false)}>
          <div className="drawerToolbar">
            <span>{activeCount ? `${activeCount} 个进行中` : '无进行中任务'} · 共 {tasks.length} 条</span>
            <button className="iconBtn" onClick={() => load()} aria-label="刷新"><RefreshCw size={16} /></button>
          </div>
          <div className="taskFilters" role="group" aria-label="任务过滤">
            {([
              ['all', '全部', counts.all],
              ['active', '进行中', counts.active],
              ['done', '完成', counts.done],
              ['issue', '异常', counts.issue]
            ] as const).map(([key, label, count]) => (
              <button key={key} className={filter === key ? 'active' : ''} onClick={() => setFilter(key)}>
                {label}<span>{count}</span>
              </button>
            ))}
          </div>
          <div className="taskSearchBar">
            <Search size={15} />
            <input
              className="input"
              aria-label="任务搜索"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="搜索名称、类型、ID、参数或错误"
            />
            {query && (
              <button className="iconBtn" onClick={() => setQuery('')} aria-label="清空任务搜索">
                <XCircle size={15} />
              </button>
            )}
          </div>
          <div className="taskBulkActions">
            <span>显示 {visibleTasks.length} / {tasks.length}</span>
            <div>
              <button className="btn ghost compact" onClick={expandVisible} disabled={visibleTasks.length === 0}>
                <ChevronDown size={14} />
                展开可见
              </button>
              <button className="btn ghost compact" onClick={collapseVisible} disabled={visibleTasks.length === 0}>
                <ChevronUp size={14} />
                收起可见
              </button>
            </div>
          </div>
          {loadError && <div className="taskError">加载失败：{loadError}</div>}
          <div className="taskList">
            {visibleTasks.length === 0 && <p className="empty">{tasks.length === 0 ? '没有任务' : '当前过滤下没有任务'}</p>}
            {visibleTasks.map((task) => {
              const taskPct = task.total ? Math.min(100, Math.round((task.progress / task.total) * 100)) : 0;
              const canCancel = active.has(task.status) && !task.cancel_requested;
              const preview = resultPreview(task);
              const expanded = expandedIds.has(task.id);
              const taskName = task.label || task.kind;
              const duration = formatDuration(task.started_at || task.queued_at, task.ended_at || task.updated_at);
              const showParams = hasJsonPayload(task.params);
              const showResult = hasJsonPayload(task.result);
              const displayStatus = effectiveStatus(task);
              const resultFailure = failedResultSummary(task);
              const canDiagnose = canGenerateDiagnosticAction(task);
              const diagnosticAction = diagnosticActions[task.id];
              return (
                <article className="taskCard" key={task.id}>
                  <div>
                    <strong>{taskName}</strong>
                    <span className={`badge ${displayStatus}`}>{statusLabel(displayStatus)}</span>
                    <button
                      className="iconBtn taskDetailToggle"
                      onClick={() => toggleExpanded(task.id)}
                      title={expanded ? '收起详情' : '展开详情'}
                      aria-label={`${expanded ? '收起' : '展开'}任务详情：${taskName}`}
                    >
                      {expanded ? <ChevronUp size={15} /> : <ChevronDown size={15} />}
                    </button>
                  </div>
                  <dl className="taskMeta">
                    <div><dt>类型</dt><dd>{task.kind}</dd></div>
                    <div><dt>来源</dt><dd>{task.source || 'manual'}</dd></div>
                    <div><dt>排队</dt><dd>{formatTime(task.queued_at)}</dd></div>
                    <div><dt>耗时</dt><dd>{duration}</dd></div>
                    <div><dt>更新</dt><dd>{formatTime(task.updated_at)}</dd></div>
                    {task.started_at && <div><dt>开始</dt><dd>{formatTime(task.started_at)}</dd></div>}
                    {task.ended_at && <div><dt>结束</dt><dd>{formatTime(task.ended_at)}</dd></div>}
                  </dl>
                  <p>{resultFailure || task.status_text || task.kind}</p>
                  {active.has(task.status) && (
                    <>
                      <div className="miniProgress"><i style={{ width: `${task.total ? taskPct : 5}%` }} /></div>
                      <small>{task.progress}/{task.total || '?'} · {taskPct}%</small>
                      <button className="btn ghost compact" onClick={() => cancel(task)} disabled={!canCancel || cancellingId === task.id}>
                        <XCircle size={14} /> {task.cancel_requested || cancellingId === task.id ? '取消中' : '取消'}
                      </button>
                    </>
                  )}
                  {preview && <p className="resultText">{preview}</p>}
                  {(task.error || resultFailure) && <p className="errorText">{task.error || resultFailure}</p>}
                  {expanded && (
                    <div className="taskDetails">
                      <div className="taskIdLine">
                        <span>{task.id}</span>
                        <button
                          className="iconBtn"
                          onClick={() => copyTaskId(task)}
                          title="复制任务 ID"
                          aria-label={`复制任务 ID：${taskName}`}
                        >
                          <Copy size={14} />
                        </button>
                      </div>
                      <TaskResultPanel
                        task={task}
                        creatingNextActionKey={creatingNextActionKey}
                        nextActionSmartActions={nextActionSmartActions}
                        onCreateNextAction={createNextSmartAction}
                        onOpenSmartAction={onOpenSmartAction}
                      />
                      {canDiagnose && (
                        <section className="taskDiagnosticPanel" aria-label={`任务诊断智能动作：${taskName}`}>
                          <div>
                            <h4>诊断智能动作</h4>
                            <p>根据这次任务的错误、部分成功或异常结果，生成一条可复核的智能动作建议。</p>
                          </div>
                          <button
                            className="btn compact"
                            onClick={() => generateDiagnosticAction(task)}
                            disabled={diagnosingId === task.id}
                          >
                            <Lightbulb size={14} />
                            {diagnosingId === task.id ? '生成中' : '生成诊断智能动作'}
                          </button>
                          {diagnosticAction && (
                            <article className="taskDiagnosticAction" aria-label="已生成诊断动作">
                              <span className="badge info">已生成</span>
                              <div>
                                <strong>{diagnosticAction.title}</strong>
                                <p>{diagnosticAction.summary}</p>
                                <small>
                                  建议入口：{diagnosticAction.tab || '智能动作'}
                                  {diagnosticAction.action_label ? ` · ${diagnosticAction.action_label}` : ''}
                                </small>
                              </div>
                            </article>
                          )}
                        </section>
                      )}
                      {showParams && (
                        <section>
                          <h4>参数</h4>
                          <pre>{prettyJson(task.params)}</pre>
                        </section>
                      )}
                      {showResult && (
                        <details className="taskJsonDetails">
                          <summary>技术详情 JSON</summary>
                          <pre>{prettyJson(task.result)}</pre>
                        </details>
                      )}
                      {task.error && (
                        <section>
                          <h4>错误</h4>
                          <pre>{task.error}</pre>
                        </section>
                      )}
                    </div>
                  )}
                </article>
              );
            })}
          </div>
        </Drawer>
      )}
    </>
  );
}
