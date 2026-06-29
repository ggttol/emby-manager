import { useEffect, useMemo, useRef } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { TASK_COMPLETED_EVENT, type TaskCompleteDetail } from '../components/TaskCenter';

type TaskRun = components['schemas']['TaskRun'];

const terminal = new Set(['done', 'error', 'cancelled', 'interrupted']);

type UseTaskCompletionOptions = {
  intervalMs?: number;
};

export function isTerminalTask(task: TaskRun | null | undefined) {
  return Boolean(task && terminal.has(task.status));
}

export function useTaskCompletion(
  taskIds: string[],
  onComplete: (task: TaskRun) => void,
  options: UseTaskCompletionOptions = {}
) {
  const onCompleteRef = useRef(onComplete);
  const completedRef = useRef<Set<string>>(new Set());
  const idsKey = useMemo(() => [...new Set(taskIds.filter(Boolean))].sort().join('\n'), [taskIds]);
  const intervalMs = options.intervalMs ?? 1200;

  useEffect(() => {
    onCompleteRef.current = onComplete;
  }, [onComplete]);

  useEffect(() => {
    const ids = idsKey.split('\n').filter(Boolean);
    const idSet = new Set(ids);
    if (idSet.size === 0) return undefined;

    const markComplete = (task: TaskRun) => {
      if (!idSet.has(task.id) || !isTerminalTask(task) || completedRef.current.has(task.id)) return;
      completedRef.current.add(task.id);
      onCompleteRef.current(task);
    };

    const onTaskCompleted = (event: Event) => {
      const detail = (event as CustomEvent<TaskCompleteDetail>).detail;
      if (detail?.task) markComplete(detail.task);
    };

    let cancelled = false;
    let timer = 0;

    const poll = async () => {
      const pending = ids.filter((id) => !completedRef.current.has(id));
      if (pending.length === 0 || cancelled) return;
      await Promise.all(pending.map(async (id) => {
        try {
          const task = await api<TaskRun>(`/api/v2/tasks/${id}`);
          if (!cancelled) markComplete(task);
        } catch {
          // TaskCenter still reports load errors; this hook is a best-effort replay guard.
        }
      }));
      if (!cancelled && pending.some((id) => !completedRef.current.has(id))) {
        timer = window.setTimeout(poll, intervalMs);
      }
    };

    window.addEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
    void poll();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      window.removeEventListener(TASK_COMPLETED_EVENT, onTaskCompleted);
    };
  }, [idsKey, intervalMs]);
}
