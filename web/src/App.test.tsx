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

const systemSummary = {
  ok: false,
  version: '0.1.0',
  rust_version: '0.1.0',
  cd_root: '/volume1/docker/clouddrive2/CloudNAS/CloudDrive',
  strm_root: '/volume1/strm',
  docker_bin: '/usr/local/bin/docker',
  cd_root_exists: true,
  strm_root_exists: true,
  docker_bin_exists: true,
  database: {
    configured: true,
    url: 'postgres://***@postgres/emby_manager',
    status: 'ok',
    current_database: 'emby_manager',
    server_version: 'PostgreSQL 16',
    pool_size: 5,
    idle_connections: 2,
    warning: null
  },
  host: {
    os: 'linux',
    arch: 'x86_64',
    process_id: 42,
    memory: { total_bytes: 8_000_000_000, available_bytes: 2_000_000_000, used_percent: 75 },
    load_average: { one: 0.42, five: 0.36, fifteen: 0.31 }
  },
  configured_roots: [
    {
      key: 'strm_root',
      label: 'strm 根目录',
      path: '/volume1/strm',
      expected_kind: 'directory',
      exists: true,
      is_dir: true,
      is_file: false,
      readable: true,
      writable_hint: true,
      disk: {
        filesystem: '/dev/md0',
        mount_point: '/volume1',
        total_bytes: 10_000_000_000,
        used_bytes: 7_500_000_000,
        available_bytes: 2_500_000_000,
        used_percent: 75
      },
      warnings: []
    },
    {
      key: 'legacy_dir',
      label: '旧版数据目录',
      path: '/legacy',
      expected_kind: 'directory',
      exists: false,
      is_dir: false,
      is_file: false,
      readable: null,
      writable_hint: null,
      disk: null,
      warnings: ['旧版数据目录不存在']
    }
  ],
  warnings: ['旧版数据目录不存在']
};

const readonlyMeta = {
  generated_at: '2026-06-28T00:00:00Z',
  readonly: true,
  source: ['task_runs'],
  coverage: ['只读预检摘要'],
  limitations: ['不执行写操作']
};

const taskHistory = {
  total: 5,
  pending: 0,
  running: 1,
  stale_running: 0,
  done: 3,
  error: 1,
  cancelled: 0,
  interrupted: 0,
  last_updated_at: '2026-06-28T00:03:00Z',
  recent_issues: [
    {
      id: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',
      kind: 'scan',
      label: '失败扫库',
      status: 'error',
      message: 'Emby timeout',
      updated_at: '2026-06-28T00:02:00Z'
    }
  ]
};

const strmReadonly = {
  root: '/volume1/strm',
  exists: true,
  is_dir: true,
  max_depth: 8,
  entry_limit: 50000,
  directories: 8,
  top_level_dirs: 2,
  empty_directories: 1,
  files: 130,
  strm_files: 120,
  subtitle_files: 9,
  other_files: 1,
  extension_counts: [{ extension: 'srt', count: 7 }],
  samples: [{ kind: 'subtitle', rel_path: '电影/A.srt' }],
  truncated: false,
  warnings: []
};

const autostrmStatus = {
  ok: true,
  complete_business_port: false,
  meta: readonlyMeta,
  seen: { total: 20, libraries: 2, last_seen_at: '2026-06-28T00:01:00Z' },
  unmatched: {
    total: 3,
    without_emby_id: 2,
    libraries: 1,
    first_created_at: '2026-06-27T00:00:00Z',
    last_updated_at: '2026-06-28T00:02:00Z'
  },
  libraries: [{ lib: '电影', seen: 12, unmatched: 3 }],
  todos: [{ severity: 'high', area: 'autostrm', message: '需要处理 unmatched', count: 3, source: 'autostrm_unmatched' }],
  warnings: ['webhook worker 尚未接入']
};

const cleanupSummary = {
  ok: true,
  complete_business_port: false,
  meta: readonlyMeta,
  task_history: taskHistory,
  catalog: {
    available: true,
    total: 260000,
    packages: 1200,
    share115: 200000,
    magnet: 50000,
    ed2k: 10000,
    other: 0,
    duplicate_links: 10,
    duplicate_names: 8
  },
  strm: strmReadonly,
  autostrm: {
    seen: autostrmStatus.seen,
    unmatched: autostrmStatus.unmatched,
    libraries: autostrmStatus.libraries
  },
  schedules: { total: 2, enabled: 1, last_errors: 0 },
  logs: { errors_7d: 1, warnings_7d: 2, last_error_at: '2026-06-28T00:02:00Z' },
  todos: [{ severity: 'medium', area: 'tasks', message: '存在失败任务', count: 1, source: 'task_runs' }],
  warnings: []
};

const gapsSummary = {
  ok: true,
  complete_business_port: false,
  meta: readonlyMeta,
  task_history: taskHistory,
  catalog: cleanupSummary.catalog,
  strm: strmReadonly,
  autostrm: cleanupSummary.autostrm,
  todos: [{ severity: 'low', area: 'catalog', message: '缺集只读预检', count: 1, source: 'catalog_items' }],
  warnings: []
};

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
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText }
    });
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
              source: 'manual',
              params: { library: 'Movies' },
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
              source: 'migration',
              params: {},
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
              source: 'schedule',
              params: { reason: 'missing_path' },
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
    expect(screen.getByText('manual')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '展开任务详情：扫描电影库' }));
    expect(screen.getByText('11111111-1111-4111-8111-111111111111')).toBeInTheDocument();
    expect(screen.getAllByText((_, element) => element?.textContent?.includes('"library": "Movies"') ?? false).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole('button', { name: '复制任务 ID：扫描电影库' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('11111111-1111-4111-8111-111111111111'));
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    await waitFor(() => expect(cancelCalls).toBe(1));

    fireEvent.click(screen.getByRole('button', { name: /异常/ }));
    expect(screen.getByText('清理预检')).toBeInTheDocument();
    expect(screen.getByText('路径未配置')).toBeInTheDocument();
  });

  it('renders the dashboard read-only overview with csrf protected insight calls', async () => {
    let cleanupCalled = false;
    let gapsCalled = false;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/system/summary') {
        return jsonResponse(systemSummary);
      }
      if (url.pathname === '/api/v2/cleanup/suggest') {
        cleanupCalled = true;
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        return jsonResponse(cleanupSummary);
      }
      if (url.pathname === '/api/v2/gaps/scan') {
        gapsCalled = true;
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        return jsonResponse(gapsSummary);
      }
      if (url.pathname === '/api/v2/autostrm/status') {
        return jsonResponse(autostrmStatus);
      }
      return undefined;
    });

    render(<App />);

    expect(await screen.findByText('Rust Preview 总览')).toBeInTheDocument();
    expect(await screen.findByText('需要处理 unmatched')).toBeInTheDocument();
    expect(await screen.findByText('失败扫库')).toBeInTheDocument();
    expect(await screen.findByText('120 / 9')).toBeInTheDocument();
    await waitFor(() => {
      expect(cleanupCalled).toBe(true);
      expect(gapsCalled).toBe(true);
    });
  });

  it('renders system health details from the system tab', async () => {
    mockApi((url) => {
      if (url.pathname === '/api/v2/system/summary') {
        return jsonResponse(systemSummary);
      }
      if (url.pathname === '/api/v2/cleanup/suggest') {
        return jsonResponse(cleanupSummary);
      }
      if (url.pathname === '/api/v2/gaps/scan') {
        return jsonResponse(gapsSummary);
      }
      if (url.pathname === '/api/v2/autostrm/status') {
        return jsonResponse(autostrmStatus);
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '系统' }));
    expect(await screen.findByText('系统健康')).toBeInTheDocument();
    expect(screen.getByText('strm 根目录')).toBeInTheDocument();
    expect(screen.getByText('/volume1/strm')).toBeInTheDocument();
    expect(screen.getAllByText('旧版数据目录不存在').length).toBeGreaterThan(0);
  });

  it('renders subtitle overview and reloads by library', async () => {
    const requestedLibs: Array<string | null> = [];
    mockApi((url) => {
      if (url.pathname === '/api/v2/system/summary') {
        return jsonResponse(systemSummary);
      }
      if (url.pathname === '/api/v2/cleanup/suggest') {
        return jsonResponse(cleanupSummary);
      }
      if (url.pathname === '/api/v2/gaps/scan') {
        return jsonResponse(gapsSummary);
      }
      if (url.pathname === '/api/v2/autostrm/status') {
        return jsonResponse(autostrmStatus);
      }
      if (url.pathname === '/api/v2/libraries/strm') {
        requestedLibs.push(url.searchParams.get('lib'));
        expect(url.searchParams.get('overview')).toBe('true');
        return jsonResponse({
          base: '/volume1/strm',
          items: [],
          truncated: false,
          overview: {
            base: '/volume1/strm',
            max_depth: 8,
            entry_limit: 100000,
            directories: 12,
            files: 144,
            strm_files: 120,
            subtitle_files: 9,
            other_files: 15,
            strm_bytes: 2048,
            subtitle_bytes: 1024,
            subtitle_extensions: [{ extension: 'srt', count: 7 }, { extension: 'ass', count: 2 }],
            samples: [{ rel_path: '电影/A.srt', kind: 'subtitle', extension: 'srt', size: 512 }],
            truncated: false,
            warnings: []
          }
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '字幕' }));
    expect(await screen.findByText('外挂字幕概览')).toBeInTheDocument();
    expect(screen.getByText('.srt')).toBeInTheDocument();
    expect(screen.getByText('电影/A.srt')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('字幕库名'), { target: { value: '电影' } });
    fireEvent.click(screen.getByRole('button', { name: '查看概览' }));
    await waitFor(() => expect(requestedLibs).toContain('电影'));
  });

  it('loads scan workspace and creates library/item refresh tasks with csrf', async () => {
    const strmLibs: Array<string | null> = [];
    const scanPayloads: unknown[] = [];
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/system/summary') {
        return jsonResponse(systemSummary);
      }
      if (url.pathname === '/api/v2/cleanup/suggest') {
        return jsonResponse(cleanupSummary);
      }
      if (url.pathname === '/api/v2/gaps/scan') {
        return jsonResponse(gapsSummary);
      }
      if (url.pathname === '/api/v2/autostrm/status') {
        return jsonResponse(autostrmStatus);
      }
      if (url.pathname === '/api/v2/libraries') {
        return jsonResponse({
          libraries: [
            { id: 'movie-lib', name: '电影', type: 'movies', paths: ['/strm/电影'] },
            { id: 'show-lib', name: '电视剧', type: 'tvshows', paths: ['/strm/电视剧'] }
          ]
        });
      }
      if (url.pathname === '/api/v2/libraries/strm') {
        strmLibs.push(url.searchParams.get('lib'));
        expect(url.searchParams.get('overview')).toBe('true');
        return jsonResponse({
          base: '/volume1/strm/电影',
          items: [
            { name: 'Movie.strm', rel_path: 'Movie/Movie.strm', is_dir: false, size: 128 },
            { name: 'Season 1', rel_path: 'Show/Season 1', is_dir: true, size: 0 }
          ],
          truncated: false,
          overview: {
            base: '/volume1/strm/电影',
            max_depth: 8,
            entry_limit: 100000,
            directories: 3,
            files: 4,
            strm_files: 2,
            subtitle_files: 1,
            other_files: 1,
            strm_bytes: 256,
            subtitle_bytes: 32,
            subtitle_extensions: [{ extension: 'srt', count: 1 }],
            samples: [{ rel_path: 'Movie/Movie.strm', kind: 'strm', extension: 'strm', size: 128 }],
            truncated: false,
            warnings: []
          }
        });
      }
      if (url.pathname === '/api/v2/libraries/scan') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        scanPayloads.push(JSON.parse(String(init?.body)));
        return jsonResponse({
          id: 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee',
          kind: 'scan_library',
          label: '扫描库: 电影',
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
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '扫描' }));
    expect(await screen.findByText('扫描工作台')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('扫描目标库'), { target: { value: '电影' } });
    expect((await screen.findAllByText('Movie/Movie.strm')).length).toBeGreaterThan(0);
    await waitFor(() => expect(strmLibs).toContain('电影'));

    fireEvent.click(screen.getByRole('button', { name: '刷新选中库' }));
    await waitFor(() => expect(scanPayloads[0]).toEqual({ lib: '电影', recursive: true, full: false }));

    fireEvent.change(screen.getByLabelText('扫描目录关键词'), { target: { value: 'Movie' } });
    fireEvent.click(screen.getByLabelText('首次无 tmdbid 也生成'));
    fireEvent.click(screen.getByRole('button', { name: '生成缺失 STRM' }));
    await waitFor(() => expect(scanPayloads[1]).toEqual({
      lib: '电影',
      recursive: true,
      full: false,
      generate_strm: true,
      keyword: 'Movie',
      fullauto: true
    }));

    fireEvent.change(screen.getByLabelText('Emby ItemId'), { target: { value: 'item-1' } });
    fireEvent.click(screen.getByRole('button', { name: '刷新 ItemId' }));
    await waitFor(() => expect(scanPayloads[2]).toEqual({
      item_id: 'item-1',
      lib: '电影',
      recursive: true,
      full: false
    }));
  });

  it('runs poster detection, search, apply, and batch tasks with csrf', async () => {
    let detectPayload: unknown = null;
    let searchPayload: unknown = null;
    let applyPayload: unknown = null;
    const batchPayloads: unknown[] = [];
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/libraries') {
        return jsonResponse({
          libraries: [
            { id: 'movie-lib', name: '电影', type: 'movies', paths: ['/strm/电影'] },
            { id: 'show-lib', name: '电视剧', type: 'tvshows', paths: ['/strm/电视剧'] }
          ]
        });
      }
      if (url.pathname === '/api/v2/posters/detect-mismatch') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        detectPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          scanned_libraries: 1,
          scanned_items: 80,
          total: 2,
          missing_primary_total: 1,
          mismatch_total: 1,
          truncated: false,
          warnings: ['library 电影 was truncated at 80 of unknown items'],
          items: [
            {
              id: 'item-1',
              emby_name: '错绑电影',
              name: '错绑电影',
              lib: '电影',
              type: 'Movie',
              path: '/strm/电影/错绑电影 [tmdbid-123]/movie.strm',
              folder: '错绑电影 [tmdbid-123]',
              folder_clean: '错绑电影',
              tmdb: '456',
              declared_tmdb: '123',
              has_poster: true,
              score: 100,
              reasons: ['folder 声明 tmdbid-123 但 Emby 绑了 456(确定绑错)'],
              signals: [
                {
                  kind: 'declared_tmdb_mismatch',
                  severity: 'danger',
                  message: 'folder 声明 tmdbid-123 与 ProviderIds.Tmdb=456 不一致'
                }
              ]
            },
            {
              id: 'item-2',
              emby_name: '无海报剧',
              name: '无海报剧',
              lib: '电影',
              type: 'Series',
              path: null,
              folder: '无海报剧',
              folder_clean: '无海报剧',
              tmdb: '',
              declared_tmdb: null,
              has_poster: false,
              score: 40,
              reasons: ['没有 Primary poster'],
              signals: [
                {
                  kind: 'missing_primary',
                  severity: 'warn',
                  message: '条目没有 Primary poster'
                }
              ]
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/posters/search') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        searchPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          candidates: [
            { name: '正确电影', year: 2024, tmdb: '123', img: 'https://img.example/poster.jpg', overview: '候选简介' },
            { name: '无图电影', year: 2023, tmdb: '124', img: '', overview: '' }
          ]
        });
      }
      if (url.pathname === '/api/v2/posters/apply') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        applyPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          name: '错绑电影',
          poster: true,
          tmdb: '123',
          apply_status: 204,
          refresh_status: 204,
          image_download_status: null
        });
      }
      if (url.pathname === '/api/v2/posters/fix-batch') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        batchPayloads.push(JSON.parse(String(init?.body)));
        return jsonResponse({
          id: '12121212-1212-4212-8212-121212121212',
          kind: 'poster_fix_batch',
          label: '批量海报修复: Series x 1',
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
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '海报修复' }));
    expect(await screen.findByText('海报检测工作台')).toBeInTheDocument();
    expect(screen.getByText(/Apply 会改 Emby ProviderIds.Tmdb/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('海报目标库'), { target: { value: '电影' } });
    fireEvent.change(screen.getByLabelText('海报扫描上限'), { target: { value: '80' } });
    fireEvent.click(screen.getByRole('button', { name: '开始检测' }));

    await waitFor(() => expect(detectPayload).toEqual({
      lib: '电影',
      limit: 80,
      include_missing_primary: true
    }));
    expect((await screen.findAllByText('错绑电影')).length).toBeGreaterThan(0);
    expect(screen.getAllByText('无海报剧').length).toBeGreaterThan(0);
    expect(screen.getByText('Emby: 456')).toBeInTheDocument();
    expect(screen.getByText('folder: 123')).toBeInTheDocument();
    expect(screen.getByText('library 电影 was truncated at 80 of unknown items')).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole('button', { name: '重搜候选' })[0]);
    await waitFor(() => expect(searchPayload).toEqual({
      id: 'item-1',
      name: '错绑电影',
      type: 'Movie',
      limit: 8
    }));
    expect(await screen.findByText('正确电影 2024')).toBeInTheDocument();

    fireEvent.click(screen.getByText('正确电影 2024'));
    await waitFor(() => expect(applyPayload).toEqual({
      id: 'item-1',
      tmdb: '123',
      type: 'Movie',
      name: '错绑电影'
    }));
    expect(await screen.findByText('已修复「错绑电影」海报')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '批量自动修复' }));
    await waitFor(() => expect(batchPayloads).toEqual([{ ids: ['item-2'], type: 'Series' }]));
  });

  it('renders zhuigeng readonly and starts real gaps library scans', async () => {
    const calls: string[] = [];
    let scanPayload: unknown = null;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/gaps/scan') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        calls.push(url.pathname);
        return jsonResponse(gapsSummary);
      }
      if (url.pathname === '/api/v2/libraries') {
        return jsonResponse({
          libraries: [
            { id: 'lib-shows', name: '剧集', type: 'tvshows', paths: ['/strm/剧集'] },
            { id: 'lib-movies', name: '电影', type: 'movies', paths: ['/strm/电影'] }
          ]
        });
      }
      if (url.pathname === '/api/v2/gaps/scan-lib') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        scanPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '44444444-4444-4444-8444-444444444444',
          kind: 'gaps_scan_lib',
          label: '全库缺集扫描 剧集',
          source: 'manual',
          params: { lib: '剧集' },
          status: 'running',
          progress: 0,
          total: 2,
          status_text: '扫 剧集',
          result: null,
          error: null,
          cancel_requested: false,
          queued_at: '2026-06-28T00:03:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:03:00Z'
        });
      }
      if (url.pathname === '/api/v2/tasks/44444444-4444-4444-8444-444444444444') {
        return jsonResponse({
          id: '44444444-4444-4444-8444-444444444444',
          kind: 'gaps_scan_lib',
          label: '全库缺集扫描 剧集',
          source: 'manual',
          params: { lib: '剧集' },
          status: 'done',
          progress: 2,
          total: 2,
          status_text: '全库缺集扫描完成',
          result: {
            ok: true,
            lib: '剧集',
            analyzed: 2,
            total: 1,
            copy_text: '求 剧 A [tmdb:123] — S01 E2',
            items: [{ name: '剧 A', id: 'series-a', tmdb: '123', fmt: 'S01 E2', gap_count: 1, behind: 0, score: 1, err: null }]
          },
          error: null,
          cancel_requested: false,
          queued_at: '2026-06-28T00:03:00Z',
          started_at: '2026-06-28T00:03:01Z',
          ended_at: '2026-06-28T00:03:02Z',
          updated_at: '2026-06-28T00:03:02Z'
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '追更检查' }));
    expect(await screen.findByText('追更只读预检')).toBeInTheDocument();
    expect(screen.getByText(/当前 Rust 版没有独立追更扫描接口/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '缺集检查' }));
    expect(await screen.findByText('缺集扫描')).toBeInTheDocument();
    expect(await screen.findByText('全库扫描只读 Emby 元数据，不修改媒体文件、不写 STRM、不调用 115。')).toBeInTheDocument();
    expect(await screen.findByRole('combobox', { name: '选择剧集库' })).toHaveValue('剧集');
    fireEvent.click(screen.getByRole('button', { name: '全库扫描' }));
    await waitFor(() => expect(scanPayload).toEqual({ lib: '剧集' }));
    expect(await screen.findByText('全库缺集扫描 剧集')).toBeInTheDocument();
    expect(await screen.findByText('全库缺集报告', undefined, { timeout: 2500 })).toBeInTheDocument();
    expect(screen.getByText('剧 A')).toBeInTheDocument();
    expect(screen.getByText('S01 E2')).toBeInTheDocument();
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
  });

  it('renders cleanup and dedup readonly summaries through cleanup suggest', async () => {
    const calls: string[] = [];
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/cleanup/suggest') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        calls.push(url.pathname);
        return jsonResponse(cleanupSummary);
      }
      if (url.pathname === '/api/v2/catalog/duplicates') {
        calls.push(url.pathname);
        return jsonResponse({
          ok: true,
          readonly: true,
          limit: 10,
          duplicate_link_groups: 1,
          duplicate_name_groups: 1,
          link_type_distribution: [{ link_type: 'share115', count: 2 }],
          link_groups: [{
            key: 'https://115.com/s/shared',
            count: 2,
            link_types: ['share115'],
            sample_names: ['重复资源 A', '重复资源 B'],
            sample_sheets: ['电影'],
            sample_links: ['https://115.com/s/shared']
          }],
          name_groups: [{
            key: '同名资源',
            count: 2,
            link_types: ['share115', 'magnet'],
            sample_names: ['同名资源'],
            sample_sheets: ['电影'],
            sample_links: ['https://115.com/s/one', 'magnet:?xt=urn:btih:two']
          }]
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能清理' }));
    expect(await screen.findByText('智能清理预检')).toBeInTheDocument();
    expect(screen.getByText('存在失败任务')).toBeInTheDocument();
    expect(screen.getByText(/当前 Rust 版智能清理只读预检/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '去重' }));
    expect(await screen.findByText('去重预检')).toBeInTheDocument();
    expect(screen.getByText('资源目录重复信号')).toBeInTheDocument();
    expect(screen.getByText('https://115.com/s/shared')).toBeInTheDocument();
    expect(screen.getByText('同名资源')).toBeInTheDocument();
    await waitFor(() => expect(calls).toEqual(expect.arrayContaining([
      '/api/v2/cleanup/suggest',
      '/api/v2/catalog/duplicates'
    ])));
  });

  it('creates delete preview tasks from the manage panel with csrf', async () => {
    let previewPayload: unknown = null;
    let executePayload: unknown = null;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/libraries') {
        return jsonResponse({
          libraries: [
            { id: 'movie-lib', name: '电影', type: 'movies', paths: ['/strm/电影'] },
            { id: 'show-lib', name: '电视剧', type: 'tvshows', paths: ['/strm/电视剧'] }
          ]
        });
      }
      if (url.pathname === '/api/v2/manage/undo') {
        return jsonResponse({
          total: 1,
          items: [
            {
              id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
              legacy_id: 'legacy-1',
              op: 'delete',
              payload: { lib: '电影', folder: '旧电影' },
              undone: false,
              created_at: '2026-06-28T00:00:00Z'
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/manage/delete') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        previewPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',
          kind: 'manage_delete_preview',
          label: '删除预览: 电影/旧电影',
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
      if (url.pathname === '/api/v2/manage/delete/execute') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        executePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee',
          kind: 'manage_delete_execute',
          label: '删除: 电影/旧电影',
          status: 'pending',
          progress: 0,
          total: 4,
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
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '删除·移动' }));
    expect(await screen.findByText(/先 Emby DELETE，再动磁盘/)).toBeInTheDocument();
    expect(screen.getByText('legacy-1')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('删除库名'), { target: { value: '电影' } });
    fireEvent.change(screen.getByLabelText('删除 folder'), { target: { value: '旧电影' } });
    fireEvent.change(screen.getByLabelText('删除 ItemId'), { target: { value: 'item-1' } });
    fireEvent.change(screen.getByLabelText('删除原因'), { target: { value: '重复资源' } });
    fireEvent.click(screen.getByRole('button', { name: '生成删除预览任务' }));

    await waitFor(() => expect(previewPayload).toEqual({
      lib: '电影',
      folder: '旧电影',
      item_id: 'item-1',
      reason: '重复资源'
    }));
    expect(await screen.findByText('删除预览: 电影/旧电影')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '真实删除' }));
    expect(await screen.findByText('确认真实删除')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认删除' }));

    await waitFor(() => expect(executePayload).toEqual({
      lib: '电影',
      folder: '旧电影',
      item_id: 'item-1',
      reason: '重复资源'
    }));
    expect(await screen.findByText('删除: 电影/旧电影')).toBeInTheDocument();
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

  it('loads settings, fills cid matches, and saves config with csrf', async () => {
    let savedPayload: unknown = null;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/config' && (!init?.method || init.method === 'GET')) {
        return jsonResponse({
          settings: {
            emby_url: 'http://emby.local:8096/emby',
            api_key: '***',
            c115_cookie: '***',
            c115_cid_map: { 电影: '12345' },
            trusted_proxies: ['192.168.2.1'],
            auto_strm_enabled: false,
            auto_strm_fullauto: false,
            cd2_mount_prefix: '/CloudNAS/CloudDrive',
            auto_strm_debounce_sec: 8,
            custom_flag: true
          }
        });
      }
      if (url.pathname === '/api/v2/libraries') {
        return jsonResponse({
          libraries: [
            { id: 'movie-lib', name: '电影', type: 'movies', paths: ['/strm/电影'] },
            { id: 'tv-lib', name: '电视剧', type: 'tvshows', paths: ['/strm/电视剧'] }
          ]
        });
      }
      if (url.pathname === '/api/v2/c115/auto-cid') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        return jsonResponse({
          ok: true,
          current: { 电影: '12345' },
          scanned: 6,
          matches: {
            电影: [{ cid: '12345', path: '/电影' }],
            电视剧: [{ cid: '67890', path: '/电视剧' }]
          }
        });
      }
      if (url.pathname === '/api/v2/config' && init?.method === 'PUT') {
        const headers = init.headers as Headers;
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        savedPayload = JSON.parse(String(init.body));
        return jsonResponse({
          settings: {
            emby_url: 'http://emby.new:8096/emby',
            api_key: '***',
            c115_cookie: '***',
            c115_cid_map: { 电影: '12345', 电视剧: '67890' },
            trusted_proxies: ['192.168.2.1', '10.0.0.1'],
            auto_strm_enabled: true,
            auto_strm_fullauto: false,
            cd2_mount_prefix: '/CloudNAS/CloudDrive',
            auto_strm_debounce_sec: 12,
            custom_flag: true
          }
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '设置' }));
    expect(await screen.findByDisplayValue('http://emby.local:8096/emby')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Emby 地址'), { target: { value: 'http://emby.new:8096/emby' } });
    fireEvent.change(screen.getByLabelText('115 Cookie'), { target: { value: 'UID=1; CID=2; SEID=3' } });
    fireEvent.change(screen.getByLabelText('反代信任 IP'), { target: { value: '192.168.2.1, 10.0.0.1' } });
    fireEvent.click(screen.getByLabelText('启用自动 strm'));
    fireEvent.change(screen.getByLabelText('自动 strm 防抖秒数'), { target: { value: '12' } });
    fireEvent.click(screen.getByRole('button', { name: /自动检测/ }));

    await screen.findByText('自动检测扫描 6 个目录，单匹配且空 cid 的行已填入。');
    expect(screen.getByLabelText('电视剧 cid')).toHaveValue('67890');

    fireEvent.click(screen.getByRole('button', { name: '保存全部' }));

    await waitFor(() => expect(savedPayload).toEqual({
      settings: {
        custom_flag: true,
        emby_url: 'http://emby.new:8096/emby',
        api_key: '***',
        c115_cookie: 'UID=1; CID=2; SEID=3',
        c115_cid_map: { 电影: '12345', 电视剧: '67890' },
        trusted_proxies: ['192.168.2.1', '10.0.0.1'],
        auto_strm_enabled: true,
        auto_strm_fullauto: false,
        cd2_mount_prefix: '/CloudNAS/CloudDrive',
        auto_strm_debounce_sec: 12
      }
    }));
    expect(await screen.findByText('配置已保存')).toBeInTheDocument();
  });

  it('manages schedules through the v2 scheduler api', async () => {
    const existing = {
      id: '99999999-9999-4999-8999-999999999999',
      name: '夜间扫库',
      kind: 'scan_all',
      params: {},
      schedule: { mode: 'daily', hour: 3, minute: 0, weekday: 0, day: 1 },
      enabled: true,
      last_run_at: null,
      last_ended_at: null,
      last_status: null,
      last_task_id: null,
      last_error: null,
      created_at: '2026-06-28T00:00:00Z',
      updated_at: '2026-06-28T00:00:00Z'
    };
    const created = {
      ...existing,
      id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
      name: '每周追更',
      kind: 'zhuigeng_scan_airing',
      schedule: { mode: 'weekly', hour: 4, minute: 30, weekday: 2, day: 1 }
    };
    let schedules = [existing];
    let createPayload: unknown = null;
    let togglePayload: unknown = null;
    let runCalls = 0;
    let deleteCalls = 0;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/schedules' && (!init?.method || init.method === 'GET')) {
        return jsonResponse(schedules);
      }
      if (url.pathname === '/api/v2/schedules' && init?.method === 'POST') {
        const headers = init.headers as Headers;
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        createPayload = JSON.parse(String(init.body));
        schedules = [created, ...schedules];
        return jsonResponse(created);
      }
      if (/^\/api\/v2\/schedules\/[^/]+$/.test(url.pathname) && init?.method === 'PUT') {
        const headers = init.headers as Headers;
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        togglePayload = JSON.parse(String(init.body));
        const id = url.pathname.split('/').at(-1);
        schedules = schedules.map((job) => job.id === id ? { ...job, enabled: false } : job);
        return jsonResponse({ ...(schedules.find((job) => job.id === id) || existing), enabled: false });
      }
      if (/^\/api\/v2\/schedules\/[^/]+\/run$/.test(url.pathname)) {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        runCalls += 1;
        return jsonResponse({
          tid: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
          task: {
            id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
            kind: 'scan_all',
            label: '定时任务（scheduler preview dry run）',
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
            source: 'manual'
          }
        });
      }
      if (/^\/api\/v2\/schedules\/[^/]+$/.test(url.pathname) && init?.method === 'DELETE') {
        const headers = init.headers as Headers;
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        deleteCalls += 1;
        const id = url.pathname.split('/').at(-1);
        schedules = schedules.filter((job) => job.id !== id);
        return jsonResponse({ ok: true });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '定时' }));
    expect(await screen.findByText('夜间扫库')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '新建定时' }));
    fireEvent.change(screen.getByLabelText('定时任务类型'), { target: { value: 'zhuigeng_scan_airing' } });
    fireEvent.change(screen.getByLabelText('定时任务名称'), { target: { value: '每周追更' } });
    fireEvent.change(screen.getByLabelText('触发模式'), { target: { value: 'weekly' } });
    fireEvent.change(await screen.findByLabelText('星期'), { target: { value: '2' } });
    fireEvent.change(screen.getByLabelText('小时'), { target: { value: '4' } });
    fireEvent.change(screen.getByLabelText('分钟'), { target: { value: '30' } });
    fireEvent.click(screen.getByRole('button', { name: '保存定时' }));

    await waitFor(() => expect(createPayload).toEqual({
      name: '每周追更',
      kind: 'zhuigeng_scan_airing',
      enabled: true,
      params: {},
      schedule: { mode: 'weekly', hour: 4, minute: 30, weekday: 2, day: 1 }
    }));
    expect(await screen.findByText('定时任务已创建')).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole('button', { name: '立即运行' })[0]);
    await waitFor(() => expect(runCalls).toBe(1));
    expect(await screen.findByText('已创建任务：定时任务（scheduler preview dry run）')).toBeInTheDocument();

    fireEvent.click(screen.getAllByRole('button', { name: '停用' })[0]);
    await waitFor(() => expect(togglePayload).toEqual(expect.objectContaining({ enabled: false })));

    fireEvent.click(screen.getAllByRole('button', { name: '删除' })[0]);
    fireEvent.click(screen.getAllByRole('button', { name: '删除' }).at(-1)!);
    await waitFor(() => expect(deleteCalls).toBe(1));
  });

  it('loads logs, filters levels, and shows undo recovery guidance', async () => {
    const logLevels: string[] = [];
    let undoPayload: unknown = null;
    const undoId = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';

    mockApi((url, init) => {
      if (url.pathname === '/api/v2/logs') {
        const level = url.searchParams.get('level') || 'all';
        logLevels.push(level);
        if (level === 'warn') {
          return jsonResponse({
            total: 1,
            logs: [
              {
                id: 3,
                level: 'warn',
                message: 'CloudDrive 读取过快',
                detail: { path: '/volume1/CloudNAS/CloudDrive/movie.mkv' },
                created_at: '2026-06-28T00:02:00Z'
              }
            ]
          });
        }
        return jsonResponse({
          total: 2,
          logs: [
            {
              id: 2,
              level: 'error',
              message: 'Emby 删除失败',
              detail: { path: '/strm/电影/deleted.mkv' },
              created_at: '2026-06-28T00:01:00Z'
            },
            {
              id: 1,
              level: 'info',
              message: '定时启动',
              detail: { kind: 'scan_all' },
              created_at: '2026-06-28T00:00:00Z'
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/manage/undo' && (!init?.method || init.method === 'GET')) {
        return jsonResponse({
          total: 1,
          items: [
            {
              id: undoId,
              legacy_id: 'u-1',
              op: 'delete',
              payload: { lib: '电影', folder: '片源 A' },
              undone: false,
              created_at: '2026-06-28T00:03:00Z'
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/manage/undo/execute') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        undoPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: false,
          id: undoId,
          op: 'delete',
          action: 'manual_restore',
          msg: '「删除」已把 115 文件夹送入回收站,请先去 115 web 还原它,再用扫描补 strm',
          lib: '电影',
          folder: '片源 A',
          hint: '115 web -> 回收站 -> 找「片源 A」-> 还原 -> 回到工具扫描对应库'
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '日志' }));
    expect(await screen.findByText('Emby 删除失败')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('日志过滤'), { target: { value: '删除' } });
    expect(screen.queryByText('定时启动')).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('日志过滤'), { target: { value: '' } });
    fireEvent.change(screen.getByLabelText('日志级别'), { target: { value: 'warn' } });
    await waitFor(() => expect(logLevels).toContain('warn'));
    expect(await screen.findByText('CloudDrive 读取过快')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Undo 记录' }));
    expect(await screen.findByText('片源 A')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '查看恢复指引' }));

    await waitFor(() => expect(undoPayload).toEqual({ id: undoId }));
    expect(await screen.findByText('人工恢复')).toBeInTheDocument();
    expect(screen.getByText(/请先去 115 web 还原它/)).toBeInTheDocument();
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
