import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { api, clearAuthSession, getAuthSession, setAuthSession } from './api/client';

const tabLabels = [
  '仪表盘',
  '扫描',
  '115 转存',
  '找资源',
  '追更检查',
  '缺集检查',
  '海报修复',
  '字幕',
  '去重',
  '删除·移动',
  '智能清理',
  '系统',
  '定时',
  '日志',
  '用户',
  '设置'
];

function jsonResponse(body: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' }
    })
  );
}

function installLocalStorage() {
  if (window.localStorage) return;
  const store = new Map<string, string>();
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => store.set(key, value),
      removeItem: (key: string) => store.delete(key),
      clear: () => store.clear()
    }
  });
}

type ApiHandler = (url: URL, init?: RequestInit) => Promise<Response> | Response | undefined;

function mockApi(handler?: ApiHandler) {
  return vi.spyOn(globalThis, 'fetch').mockImplementation((input, init) => {
    const raw = typeof input === 'string' ? input : input instanceof Request ? input.url : String(input);
    const url = new URL(raw, window.location.origin);
    if (!url.pathname.startsWith('/api/v2')) {
      throw new Error(`unexpected API path: ${url.pathname}`);
    }
    const custom = handler?.(url, init);
    if (custom) return Promise.resolve(custom);
    if (url.pathname === '/api/v2/auth/me') {
      return jsonResponse({ authenticated: true, username: 'admin', csrf: 'csrf-me' });
    }
    if (url.pathname === '/api/v2/tasks') {
      return jsonResponse({ tasks: [], active_count: 0 });
    }
    return jsonResponse({ ok: true });
  });
}

describe('App shell', () => {
  beforeEach(() => {
    installLocalStorage();
    window.localStorage.clear();
    clearAuthSession();
  });

  afterEach(() => {
    cleanup();
    clearAuthSession();
    vi.restoreAllMocks();
  });

  it('renders all 16 tabs after auth check', async () => {
    mockApi();
    render(<App />);

    await screen.findByRole('button', { name: '仪表盘' });
    for (const label of tabLabels) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument();
    }
  });

  it('renders the task center button when authenticated', async () => {
    mockApi();
    render(<App />);

    expect(await screen.findByRole('button', { name: '任务中心' })).toBeInTheDocument();
  });

  it('opens task center, filters tasks, and requests cancellation', async () => {
    let cancelCalls = 0;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/tasks') {
        return jsonResponse({
          active_count: 1,
          tasks: [
            {
              id: '11111111-1111-4111-8111-111111111111',
              kind: 'scan',
              label: '扫描电影库',
              status: 'running',
              progress: 2,
              total: 4,
              status_text: '处理中',
              cancel_requested: false,
              queued_at: '2026-06-28T00:00:00Z',
              started_at: '2026-06-28T00:00:01Z',
              ended_at: null,
              updated_at: '2026-06-28T00:00:02Z'
            },
            {
              id: '22222222-2222-4222-8222-222222222222',
              kind: 'catalog',
              label: 'Catalog 导入',
              status: 'done',
              progress: 1,
              total: 1,
              status_text: '完成',
              result: { imported: 3 },
              cancel_requested: false,
              queued_at: '2026-06-28T00:01:00Z',
              started_at: '2026-06-28T00:01:01Z',
              ended_at: '2026-06-28T00:01:02Z',
              updated_at: '2026-06-28T00:01:02Z'
            },
            {
              id: '33333333-3333-4333-8333-333333333333',
              kind: 'cleanup',
              label: '清理预检',
              status: 'error',
              progress: 0,
              total: 1,
              status_text: '失败',
              error: '路径未配置',
              cancel_requested: false,
              queued_at: '2026-06-28T00:02:00Z',
              started_at: '2026-06-28T00:02:01Z',
              ended_at: '2026-06-28T00:02:02Z',
              updated_at: '2026-06-28T00:02:02Z'
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/tasks/11111111-1111-4111-8111-111111111111/cancel') {
        cancelCalls += 1;
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        return jsonResponse({ ok: true });
      }
      return undefined;
    });
    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '任务中心' }));

    expect(await screen.findByText('扫描电影库')).toBeInTheDocument();
    expect(screen.getAllByText('运行中').length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    await waitFor(() => expect(cancelCalls).toBe(1));

    fireEvent.click(screen.getByRole('button', { name: /异常/ }));
    expect(screen.getByText('清理预检')).toBeInTheDocument();
    expect(screen.getByText('路径未配置')).toBeInTheDocument();
  });

  it('shows login panel and enters the shell after login', async () => {
    mockApi((url) => {
      if (url.pathname === '/api/v2/auth/me') {
        return jsonResponse({ authenticated: false, username: null, csrf: null });
      }
      if (url.pathname === '/api/v2/auth/login') {
        return jsonResponse({ ok: true, token: 'token-login', csrf: 'csrf-login', username: 'admin' });
      }
      return undefined;
    });

    render(<App />);

    expect(await screen.findByRole('button', { name: '登录' })).toBeDisabled();
    fireEvent.change(screen.getByLabelText('密码'), { target: { value: 'secret' } });
    fireEvent.click(screen.getByRole('button', { name: '登录' }));

    expect(await screen.findByRole('button', { name: '仪表盘' })).toBeInTheDocument();
    expect(getAuthSession()).toMatchObject({ csrf: 'csrf-login', username: 'admin' });
    expect(getAuthSession()).not.toHaveProperty('token');
  });
});

describe('API client auth handling', () => {
  beforeEach(() => {
    installLocalStorage();
    window.localStorage.clear();
    clearAuthSession();
  });

  afterEach(() => {
    clearAuthSession();
    vi.restoreAllMocks();
  });

  it('adds csrf but not bearer headers to browser mutating requests', async () => {
    setAuthSession({ csrf: 'csrf-1', username: 'admin' });
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' }
      })
    );

    await api('/api/v2/tasks/demo', { method: 'POST', body: JSON.stringify({ seconds: 1 }) });

    const init = fetchSpy.mock.calls[0]?.[1];
    const headers = init?.headers as Headers;
    expect(headers.has('Authorization')).toBe(false);
    expect(headers.get('X-CSRF-Token')).toBe('csrf-1');
    expect(headers.get('Content-Type')).toBe('application/json');
  });

  it('clears the stored session on 401', async () => {
    setAuthSession({ csrf: 'csrf-expired', username: 'admin' });
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(JSON.stringify({ err: 'session expired' }), {
        status: 401,
        headers: { 'Content-Type': 'application/json' }
      })
    );

    await expect(api('/api/v2/tasks')).rejects.toThrow('session expired');
    expect(getAuthSession()).toEqual({});
  });
});
