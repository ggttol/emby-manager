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

  it('loads and saves Emby user policy from the users tab', async () => {
    let savedPayload: unknown = null;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/users') {
        return jsonResponse({
          users: [
            {
              id: 'user/1',
              name: 'Alice',
              disabled: false,
              last_activity_date: '2026-06-28T08:00:00Z',
              remote_bitrate_mbps: 25,
              policy: {
                RemoteClientBitrateLimit: 25_000_000,
                SimultaneousStreamLimit: 2
              }
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/users/user%2F1/policy') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('PUT');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        savedPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          user: {
            id: 'user/1',
            name: 'Alice',
            disabled: true,
            last_activity_date: '2026-06-28T08:00:00Z',
            remote_bitrate_mbps: 12.5,
            policy: {
              RemoteClientBitrateLimit: 12_500_000,
              SimultaneousStreamLimit: 3
            }
          }
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '用户' }));
    expect(await screen.findByText('Alice')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Alice 远程限速 Mbps'), { target: { value: '12.5' } });
    fireEvent.change(screen.getByLabelText('Alice 同时播放数'), { target: { value: '3' } });
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    await waitFor(() => expect(savedPayload).toEqual({
      remote_bitrate_mbps: 12.5,
      simultaneous_stream_limit: 3,
      disabled: true
    }));
    expect(await screen.findByText('已保存 Alice 的用户策略')).toBeInTheDocument();
  });

  it('searches catalog and creates 115 save/offline tasks with csrf', async () => {
    const planPayloads: unknown[] = [];
    let savePayload: unknown = null;
    let offlinePayload: unknown = null;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/catalog/stats') {
        return jsonResponse({ available: true, total: 260000, packages: 1200 });
      }
      if (url.pathname === '/api/v2/config') {
        return jsonResponse({ settings: { c115_cid_map: { 电影: '12345' } } });
      }
      if (url.pathname === '/api/v2/catalog/search') {
        expect(url.searchParams.get('q')).toBe('movie');
        expect(url.searchParams.get('limit')).toBe('80');
        return jsonResponse({
          total: 2,
          truncated: false,
          items: [
            {
              name: 'The Movie',
              sheet: '电影',
              link: 'https://115.com/s/abc?password=xy12',
              is_pkg: false,
              link_type: 'share115',
              transfer: true,
              share: 'abc',
              rc: 'xy12'
            },
            {
              name: 'The Magnet',
              sheet: '电影',
              link: 'magnet:?xt=urn:btih:123',
              is_pkg: false,
              link_type: 'magnet',
              transfer: false,
              share: null,
              rc: null
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/catalog/transfer-plan') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        const payload = JSON.parse(String(init?.body));
        planPayloads.push(payload);
        expect(payload.lib).toBe('电影');
        if (payload.item.name === 'The Movie') {
          return jsonResponse({
            ok: true,
            action: 'save_share',
            link_type: 'share115',
            transfer: true,
            is_pkg: false,
            label: 'The Movie',
            target: { lib: '电影', cid: null },
            save: {
              endpoint: '/api/v2/c115/save',
              method: 'POST',
              share: 'abc',
              receive_code: 'xy12',
              payload: {
                url: 'https://115.com/s/abc?password=xy12',
                pwd: 'xy12',
                lib: '电影',
                cid: null,
                label: 'The Movie'
              }
            },
            offline: null,
            unsupported: null
          });
        }
        return jsonResponse({
          ok: true,
          action: 'offline_download',
          link_type: 'magnet',
          transfer: false,
          is_pkg: false,
          label: 'The Magnet',
          target: { lib: '电影', cid: null },
          save: null,
          offline: {
            endpoint: '/api/v2/c115/offline',
            method: 'POST',
            protocol: 'magnet',
            payload: {
              url: 'magnet:?xt=urn:btih:123',
              lib: '电影',
              cid: null,
              label: 'The Magnet'
            }
          },
          unsupported: null
        });
      }
      if (url.pathname === '/api/v2/c115/save') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        savePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '44444444-4444-4444-8444-444444444444',
          kind: 'c115_save',
          label: 'The Movie',
          status: 'pending',
          progress: 0,
          total: 1,
          status_text: '排队中',
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:00Z',
          params: {},
          result: null,
          source: 'api'
        });
      }
      if (url.pathname === '/api/v2/c115/offline') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        offlinePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '55555555-5555-4555-8555-555555555555',
          kind: 'c115_offline',
          label: 'The Magnet',
          status: 'pending',
          progress: 0,
          total: 1,
          status_text: '排队中',
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:01Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:01Z',
          params: {},
          result: null,
          source: 'api'
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '找资源' }));
    expect(await screen.findByText('库内 260,000 条 · 整包 1,200')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('资源关键词'), { target: { value: 'movie' } });
    fireEvent.click(screen.getByRole('button', { name: '搜索' }));

    expect(await screen.findByText('The Movie')).toBeInTheDocument();
    expect(screen.getByText('The Magnet')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '转存' }));
    fireEvent.click(screen.getAllByRole('button', { name: '转存' }).at(-1)!);

    await waitFor(() => expect(savePayload).toEqual({
      url: 'https://115.com/s/abc?password=xy12',
      pwd: 'xy12',
      lib: '电影',
      cid: null,
      label: 'The Movie'
    }));
    expect(await screen.findByText('任务已创建：The Movie')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '离线' }));
    fireEvent.click(screen.getAllByRole('button', { name: '离线' }).at(-1)!);

    await waitFor(() => expect(offlinePayload).toEqual({
      url: 'magnet:?xt=urn:btih:123',
      lib: '电影',
      cid: null,
      label: 'The Magnet'
    }));
    expect(planPayloads).toHaveLength(2);
    expect(planPayloads).toEqual([
      expect.objectContaining({ label: 'The Movie', lib: '电影' }),
      expect.objectContaining({ label: 'The Magnet', lib: '电影' })
    ]);
  });

  it('previews 115 share files and creates save/offline/scan tasks with csrf', async () => {
    let snapPayload: unknown = null;
    let savePayload: unknown = null;
    let offlinePayload: unknown = null;
    let scanPayload: unknown = null;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/c115/test') {
        return jsonResponse({ ok: true, uid: '115-user', used: '8.2 TB' });
      }
      if (url.pathname === '/api/v2/config') {
        return jsonResponse({ settings: { c115_cid_map: { 电影: '12345' } } });
      }
      if (url.pathname === '/api/v2/c115/snap') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        snapPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          share: 'abc',
          rc: 'urlpw',
          share_title: 'Share Title',
          file_size: null,
          files: [
            { id: 'fid-1', name: 'Episode 1', size: 1024, is_dir: false },
            { id: 'fid-2', name: 'Episode 2', size: 2048, is_dir: false }
          ]
        });
      }
      if (url.pathname === '/api/v2/c115/save') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        savePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '66666666-6666-4666-8666-666666666666',
          kind: 'c115_save',
          label: '115 转存: Share Title',
          status: 'pending',
          progress: 0,
          total: 1,
          status_text: '排队中',
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:00Z',
          params: {},
          result: null,
          source: 'api'
        });
      }
      if (url.pathname === '/api/v2/c115/offline') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        offlinePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '77777777-7777-4777-8777-777777777777',
          kind: 'c115_offline',
          label: '115 离线: magnet',
          status: 'pending',
          progress: 0,
          total: 1,
          status_text: '排队中',
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:01Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:01Z',
          params: {},
          result: null,
          source: 'api'
        });
      }
      if (url.pathname === '/api/v2/libraries/scan') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        scanPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '88888888-8888-4888-8888-888888888888',
          kind: 'scan_library',
          label: '扫描电影',
          status: 'pending',
          progress: 0,
          total: 1,
          status_text: '排队中',
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:02Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:02Z',
          params: {},
          result: null,
          source: 'api'
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '115 转存' }));
    expect(await screen.findByText('UID 115-user · 8.2 TB')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('115 分享链接'), {
      target: { value: 'https://115.com/s/abc?password=urlpw' }
    });
    fireEvent.change(screen.getByLabelText('默认提取码'), { target: { value: 'fallback' } });
    fireEvent.click(screen.getByRole('button', { name: '先看文件' }));

    expect(await screen.findByText('Share Title')).toBeInTheDocument();
    expect(snapPayload).toEqual({ url: 'https://115.com/s/abc?password=urlpw', pwd: 'urlpw' });
    fireEvent.click(screen.getAllByRole('checkbox')[1]);
    fireEvent.click(screen.getByRole('button', { name: '创建转存任务' }));

    await waitFor(() => expect(savePayload).toEqual({
      url: 'https://115.com/s/abc?password=urlpw',
      pwd: 'urlpw',
      lib: '电影',
      file_ids: ['fid-1'],
      label: 'Share Title'
    }));

    fireEvent.change(screen.getByLabelText('115 离线链接'), { target: { value: 'magnet:?xt=urn:btih:abc' } });
    fireEvent.click(screen.getByRole('button', { name: '创建离线任务' }));

    await waitFor(() => expect(offlinePayload).toEqual({
      url: 'magnet:?xt=urn:btih:abc',
      lib: '电影',
      label: 'magnet:?xt=urn:btih:abc'
    }));

    fireEvent.click(screen.getByRole('button', { name: '扫目标库' }));
    await waitFor(() => expect(scanPayload).toEqual({ lib: '电影' }));
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
