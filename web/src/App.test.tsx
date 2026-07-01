import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { api, clearAuthSession, getAuthSession, setAuthSession } from './api/client';

const tabLabels = [
  '仪表盘',
  '智能动作',
  '扫描',
  '115 转存',
  '找资源',
  '追更检查',
  '缺集检查',
  '海报修复',
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

function getTaskCenterDrawer() {
  const drawer = screen.getByRole('heading', { name: '任务中心' }).closest('aside');
  expect(drawer).not.toBeNull();
  return drawer as HTMLElement;
}

function clickTaskCenterRefresh() {
  fireEvent.click(within(getTaskCenterDrawer()).getByRole('button', { name: '刷新' }));
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
  emby: {
    base_url: 'http://emby:8096/emby',
    configured: true,
    http_status: 200,
    online: true,
    operating_system: 'Linux',
    server_id: 'emby-server',
    server_name: 'NAS Emby',
    status: 'ok',
    version: '4.9.5',
    warning: null
  },
  docker: {
    available: true,
    configured: true,
    containers: [{
      id: 'container-1',
      image: 'emby-manager:latest',
      name: 'emby-manager',
      ports: '8097/tcp',
      state: 'running',
      status: 'Up'
    }],
    docker_bin: '/usr/local/bin/docker',
    running: 1,
    status: 'ok',
    total: 1,
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

const smartAction = {
  id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  action_type: 'transfer_update_series',
  status: 'suggested',
  subject: {
    kind: 'series',
    name: '莫离',
    year: 2026,
    tmdb: '12345',
    emby_id: 'emby-show-1',
    lib: '电视剧',
    folder: '/volume1/strm/电视剧/莫离',
    strm_path: '/volume1/strm/电视剧/莫离/S01E26.strm',
    cd_path: '/volume1/docker/clouddrive2/CloudNAS/CloudDrive/115/莫离'
  },
  title: '莫离有新集可更新',
  summary: 'Emby 当前 25 集，资源侧发现 26 集，建议一条龙转存、生成 STRM 并刷新媒体库。',
  recommendation: {
    score: 91,
    confidence: 'high',
    primary_action: '更新转存并处理旧目录',
    reasons: ['追更库集数落后', '资源标题和年份匹配', '旧目录需要在更新后清理'],
    alternatives: [{ action: '仅打开追更检查', reason: '人工确认候选资源后再处理' }]
  },
  evidence: [
    {
      source: 'emby_episodes',
      label: 'Emby 当前剧集',
      value: { current_max_episode: 25, library: '电视剧' },
      weight: 88,
      collected_at: '2026-06-29T00:00:00Z'
    },
    {
      source: 'catalog_candidate',
      label: '115 候选资源',
      value: { title: 'The.First.Jasmine.2026.S01E26', cid: 'cid-26' },
      weight: 91,
      collected_at: '2026-06-29T00:00:01Z'
    }
  ],
  plan: {
    steps: [
      {
        key: 'open-zhuigeng',
        title: '打开追更检查并锁定莫离',
        executor: 'open_tab',
        params: { tab: 'zhuigeng' },
        rollback: null
      },
      {
        key: 'transfer',
        title: '转存候选资源并生成 STRM',
        executor: 'task_pipeline',
        params: { cid: 'cid-26' },
        rollback: { title: '用 undo 记录回滚 STRM 和移动操作', params: {} }
      },
      {
        key: 'refresh',
        title: '刷新 Emby 媒体库并修复海报',
        executor: 'existing_endpoint',
        params: { endpoint: '/api/v2/posters/fix-batch' },
        rollback: null
      }
    ],
    estimated_seconds: 180,
    concurrency_key: 'clouddrive',
    can_cancel: true
  },
  risk: {
    level: 'high',
    destructive: true,
    touches_emby: true,
    touches_disk: true,
    touches_c115: true,
    requires_confirm_text: '执行',
    warnings: ['会删除同剧旧目录，必须保留 undo 记录']
  },
  policy: {
    enabled: true,
    mode: 'confirm',
    max_risk: 'high',
    reason: '涉及 115、磁盘和 Emby 写操作，需要用户确认'
  },
  verification: {
    checks: [
      {
        key: 'episode-visible',
        title: '新集可见',
        source: 'emby_episodes',
        expected: 'Emby 显示第 26 集'
      },
      {
        key: 'poster-ok',
        title: '海报可见',
        source: 'poster_detection',
        expected: '条目存在 Primary 图片'
      }
    ],
    success_message: '新集入库、旧目录已清理、海报可见',
    partial_message: '如果 Emby 刷新延迟，任务中心会提示复查'
  },
  source: 'zhuigeng.update_needed',
  tab: 'zhuigeng',
  action_label: '更新转存',
  created_at: '2026-06-29T00:00:00Z',
  updated_at: '2026-06-29T00:00:02Z'
};

const lowRiskSmartAction = {
  ...smartAction,
  id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
  action_type: 'library_scan',
  title: '低风险媒体库扫描',
  summary: '只提交媒体库扫描任务，不删除文件、不调用 115。',
  recommendation: {
    ...smartAction.recommendation,
    primary_action: '刷新媒体库',
    reasons: ['新 STRM 已生成', '只需要通知 Emby 扫描']
  },
  plan: {
    ...smartAction.plan,
    steps: [{
      key: 'scan',
      title: '刷新 Emby 媒体库',
      executor: 'existing_endpoint',
      params: { endpoint: '/api/v2/scan' },
      rollback: null
    }],
    concurrency_key: 'emby'
  },
  risk: {
    level: 'low',
    destructive: false,
    touches_emby: true,
    touches_disk: false,
    touches_c115: false,
    requires_confirm_text: null,
    warnings: []
  },
  policy: {
    enabled: true,
    mode: 'auto',
    max_risk: 'medium',
    reason: '低风险 Emby 扫描允许自动提交'
  },
  action_label: '刷新媒体库'
};

const transferAddNewSmartAction = {
  ...smartAction,
  id: 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee',
  action_type: 'transfer_add_new',
  title: '新片可从找资源转存',
  summary: '资源库发现新片候选，需要确认目标库或 cid 后安全转存。',
  subject: {
    ...smartAction.subject,
    kind: 'movie',
    name: '星河样片',
    year: 2026,
    lib: null,
    emby_id: null
  },
  recommendation: {
    ...smartAction.recommendation,
    primary_action: '确认新增转存',
    reasons: ['资源库存在 115 分享候选', '新增入库必须确认目标库或 cid']
  },
  evidence: [
    {
      source: 'catalog_candidate',
      label: '找资源候选',
      value: { title: 'Galaxy.Sample.2026.2160p', link: 'https://115.com/s/galaxy?password=xy12', is_pkg: true },
      weight: 89,
      collected_at: '2026-06-29T00:00:01Z'
    }
  ],
  plan: {
    steps: [
      {
        key: 'open-catalog',
        title: '打开找资源并带入候选',
        executor: 'open_tab',
        params: { tab: 'catalog', q: '星河样片 2026' },
        rollback: null
      },
      {
        key: 'catalog_transfer_execute',
        title: '在找资源转存弹窗中确认目标库和 cid',
        executor: 'manual_confirm',
        params: {
          q: '星河样片 2026',
          link: 'https://115.com/s/galaxy?password=xy12',
          item: {
            title: 'Galaxy.Sample.2026.2160p',
            link: 'https://115.com/s/galaxy?password=xy12',
            link_type: 'share115',
            is_pkg: true
          }
        },
        rollback: null
      }
    ],
    estimated_seconds: 120,
    concurrency_key: 'clouddrive',
    can_cancel: true
  },
  risk: {
    level: 'high',
    destructive: false,
    touches_emby: true,
    touches_disk: true,
    touches_c115: true,
    requires_confirm_text: '执行',
    warnings: ['新增转存前必须确认目标库或 cid']
  },
  policy: {
    enabled: true,
    mode: 'confirm',
    max_risk: 'high',
    reason: '新增转存需要人工确认目标库'
  },
  source: 'catalog.add_new',
  tab: 'catalog',
  action_label: '确认新增转存'
};

const dedupSmartAction = {
  ...smartAction,
  id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
  action_type: 'dedup_remove_old',
  title: '莫离自动去重建议',
  summary: '识别到同剧旧目录和新目录并存，建议删除旧资源。',
  recommendation: {
    ...smartAction.recommendation,
    primary_action: '删除旧资源',
    reasons: ['同 TMDb 下存在旧目录', '新目录集数更完整', '旧目录已经不应保留']
  },
  plan: {
    steps: [
      {
        key: 'open_context',
        title: '打开去重上下文',
        executor: 'open_tab',
        params: { tab: 'dedup', tmdb: '12345' },
        rollback: null
      },
      {
        key: 'review_keep_remove',
        title: '复核保留项和删除项',
        executor: 'manual_confirm',
        params: { keep: [{ lib: '电视剧', folder: '莫离 新版', item_id: 'item-new' }], remove: [{ lib: '电视剧', folder: '莫离 旧版', item_id: 'item-old' }] },
        rollback: null
      },
      {
        key: 'dedup_execute_batch',
        title: '确认后提交批量去重',
        executor: 'task_pipeline',
        params: {
          endpoint: '/api/v2/dedup/execute-batch',
          request: {
            groups: [{
              tmdb: '12345',
              remove: [{ lib: '电视剧', folder: '莫离 旧版', item_id: 'item-old' }]
            }]
          }
        },
        rollback: { title: '按 undo_entries 恢复删除影响', params: { source: 'undo_entries' } }
      }
    ],
    estimated_seconds: 90,
    concurrency_key: 'clouddrive',
    can_cancel: true
  },
  risk: {
    level: 'critical',
    destructive: true,
    touches_emby: true,
    touches_disk: true,
    touches_c115: true,
    requires_confirm_text: '删除',
    warnings: ['会删除 Emby 条目和旧目录，必须确认保留项正确']
  },
  policy: {
    enabled: true,
    mode: 'confirm',
    max_risk: 'critical',
    reason: '破坏性去重必须人工确认'
  },
  source: 'dedup.auto_groups',
  tab: 'dedup',
  action_label: '删除旧资源'
};

const archiveSmartAction = {
  ...smartAction,
  id: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',
  action_type: 'archive_series',
  title: '莫离已完结可归档',
  summary: '追更库中莫离已经完结，建议移动到完结剧库。',
  recommendation: {
    ...smartAction.recommendation,
    primary_action: '归档到完结剧库',
    reasons: ['TMDb 状态为已完结', '当前仍在追更库']
  },
  plan: {
    steps: [
      {
        key: 'open-zhuigeng',
        title: '打开追更检查并锁定莫离',
        executor: 'open_tab',
        params: { tab: 'zhuigeng' },
        rollback: null
      },
      {
        key: 'zhuigeng_archive_execute',
        title: '确认目标库后归档',
        executor: 'task_pipeline',
        params: {
          endpoint: '/api/v2/zhuigeng/archive/execute',
          requires_policy_param: 'archive_to_lib',
          item: { lib: '追更', title: '莫离', emby_id: 'emby-show-1', folder: '/volume1/strm/追更/莫离' },
          on_conflict: 'smart'
        },
        rollback: { title: '按 move undo 反向移动回追更库', params: { source: 'undo_entries' } }
      }
    ],
    estimated_seconds: 120,
    concurrency_key: 'clouddrive',
    can_cancel: true
  },
  risk: {
    level: 'high',
    destructive: true,
    touches_emby: true,
    touches_disk: true,
    touches_c115: false,
    requires_confirm_text: '归档',
    warnings: ['会移动目录并刷新 Emby，执行前必须确认目标库']
  },
  policy: {
    enabled: true,
    mode: 'confirm',
    max_risk: 'high',
    reason: '归档移动必须人工确认'
  },
  source: 'zhuigeng.archive_ready',
  tab: 'zhuigeng',
  action_label: '归档'
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
  empty_directory_samples: ['电影/空目录'],
  other_file_samples: ['电影/poster.jpg'],
  truncated: false,
  warnings: []
};

const autostrmStatus = {
  ok: true,
  complete_business_port: true,
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
  warnings: []
};

const cleanupSummary = {
  ok: true,
  complete_business_port: true,
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
  cleanup_candidates: [
    {
      id: 'movie-old',
      item_id: 'movie-old',
      lib: '电影',
      name: '旧电影',
      folder: '旧电影 [tmdb-100]',
      path: '/strm/电影/旧电影 [tmdb-100]',
      tmdb: '100',
      rating: 4.2,
      year: 1995,
      size_gb: null,
      score: 82.5,
      total_score: 82.5,
      scores: { rating: 35, idle: 30, size: 17.5 },
      reasons: ['低评分', '长期未播放'],
      dimensions: {
        rating: { score: 35, value: '4.2', reason: '低于阈值' },
        idle: { score: 30, value: '420 天', reason: '长时间未播放' },
        size: { score: 17.5, value: '48 GB', reason: '占用较大', warning: '挂载统计可能滞后' }
      }
    },
    {
      id: 'show-stale',
      item_id: 'show-stale',
      lib: '剧集',
      name: '冷门剧',
      folder: '冷门剧',
      path: '/strm/剧集/冷门剧',
      tmdb: null,
      rating: null,
      year: 2020,
      size_gb: null,
      score: 66,
      total_score: 66,
      scores: { meta: 40, idle: 26 },
      reasons: ['缺少元数据'],
      dimensions: {
        meta: { score: 40, value: 'no tmdb', reason: '缺少 tmdbid', warning: '元数据待刷新' },
        idle: { score: 26, value: '300 天', reason: '长时间未播放' }
      }
    }
  ],
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

const zhuigengStatus = {
  ok: true,
  continuing: 1,
  ended: 1,
  total: 2,
  copy_text: '求 示例剧 [tmdb:100] — S01 E3',
  items: [
    {
      lib: '剧集',
      name: '示例剧',
      id: 'series-100',
      folder: '示例剧 [tmdb-100]',
      tmdb: '100',
      tmdb_status: 'Returning Series',
      state: 'continuing',
      continuing: true,
      ended: false,
      local_count: 2,
      local_latest: '2026-06-21',
      local_latest_episode: 'S01E02',
      last_episode_to_air: { season_number: 1, episode_number: 3, air_date: '2026-06-28', name: '第三集' },
      next_episode_to_air: { season_number: 1, episode_number: 4, air_date: '2026-07-05', name: '第四集' },
      behind: 1,
      behind_hint: '落后 1 集',
      resource_hint: 'S01 E3',
      err: null
    },
    {
      lib: '剧集',
      name: '完结剧',
      id: 'series-200',
      folder: '完结剧 [tmdb-200]',
      tmdb: '200',
      tmdb_status: 'Ended',
      state: 'ended',
      continuing: false,
      ended: true,
      local_count: 10,
      local_latest: '2026-01-01',
      local_latest_episode: 'S01E10',
      last_episode_to_air: null,
      next_episode_to_air: null,
      behind: 0,
      behind_hint: null,
      resource_hint: null,
      err: null
    }
  ]
};

const zhuigengCandidate = {
  name: '示例剧 S01E03',
  sheet: 'TG Resource API',
  link: 'https://115.com/s/swabc',
  is_pkg: false,
  link_type: 'share115',
  transfer: true,
  share: 'https://115.com/s/swabc',
  rc: 'abcd',
  recommendation: {
    score: 230,
    level: 'best',
    action: '推荐转存',
    reasons: ['115 可直接转存', '资源到 E3，适合补缺'],
    episode_ranges: ['S01E03'],
    covers_missing: true,
    duplicate_risk: false,
    already_have: false
  }
};

const zhuigengWorkbench = {
  ok: true,
  status: zhuigengStatus,
  copy_text: zhuigengStatus.copy_text,
  note: '已按追更运营流分组: 需更新 1，补齐后归档 0，可归档 1，异常 0',
  counts: {
    total: 2,
    healthy_airing: 0,
    update_needed: 1,
    archive_ready: 1,
    complete_after_update: 0,
    metadata_error: 0,
    target_error: 0,
    unknown: 0,
    behind_total: 1
  },
  rows: [
    {
      item: zhuigengStatus.items[0],
      lane: 'update_needed',
      priority: 721,
      action: '找资源并一条龙更新',
      resource_query: '示例剧 S01 E3',
      archiveable: false,
      updateable: true,
      blockers: []
    },
    {
      item: zhuigengStatus.items[1],
      lane: 'archive_ready',
      priority: 640,
      action: '一键归档到完结库',
      resource_query: null,
      archiveable: true,
      updateable: false,
      blockers: []
    }
  ]
};

const zhuigengScanAiring = {
  ok: true,
  total: 1,
  note: '最小 TMDb 语义版',
  copy_text: '求 示例剧 [tmdb:100] — S01 E3',
  results: [
    {
      lib: '剧集',
      name: '示例剧',
      id: 'series-100',
      tmdb: '100',
      status: 'Returning Series',
      behind: 1,
      hint: 'S01 E3',
      ok: true,
      err: null
    }
  ]
};

const zhuigengGaps = {
  ok: true,
  total: 1,
  copy_text: '求 示例剧 [tmdb:100] — S01 E3',
  items: [
    { lib: '剧集', name: '示例剧', id: 'series-100', tmdb: '100', fmt: 'S01 E3', behind: 1 }
  ]
};

function taskRun(id: string, kind: string, label: string, status = 'running', result: unknown = null) {
  return {
    id,
    kind,
    label,
    source: 'api',
    params: {},
    status,
    progress: status === 'done' ? 1 : 0,
    total: 1,
    status_text: status === 'done' ? '完成' : '运行中',
    result,
    error: null,
    cancel_requested: false,
    queued_at: '2026-06-29T00:00:00Z',
    started_at: '2026-06-29T00:00:01Z',
    ended_at: status === 'done' ? '2026-06-29T00:00:02Z' : null,
    updated_at: '2026-06-29T00:00:02Z'
  };
}

function dispatchTaskDone(previousTask: ReturnType<typeof taskRun>, result: unknown) {
  const task = { ...previousTask, status: 'done', progress: previousTask.total, status_text: '完成', result };
  window.dispatchEvent(new CustomEvent('emby-manager:task-completed', {
    detail: {
      task,
      previousTask,
      previousStatus: previousTask.status
    }
  }));
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
              id: '55555555-5555-4555-8555-555555555555',
              kind: 'zhuigeng_update',
              label: '追更一条龙更新',
              status: 'done',
              progress: 1,
              total: 1,
              source: 'zhuigeng',
              params: {},
              status_text: '完成，发现 2 个可疑项',
              result: {
                ok: false,
                transfer: { ok: true, total: 1, succeeded: 1, failed: 0 },
                strm: { ok: true, new_count: 12 },
                scan: { ok: true },
                poster: { ok: true },
                auto_resolve: { ok: true, error_count: 0 },
                poster_auto_fix: { ok: true, error_count: 0 },
                check: {
                  ok: false,
                  status: 'suspicious',
                  item_error_count: 0,
                  stage_error_count: 0,
                  suspicious_count: 2
                }
              },
              error: null,
              cancel_requested: false,
              queued_at: '2026-06-28T00:01:30Z',
              started_at: '2026-06-28T00:01:31Z',
              ended_at: '2026-06-28T00:01:32Z',
              updated_at: '2026-06-28T00:01:32Z'
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
            },
            {
              id: '44444444-4444-4444-8444-444444444444',
              kind: 'zhuigeng_archive',
              label: '追更完结归档',
              status: 'done',
              progress: 2,
              total: 2,
              source: 'zhuigeng',
              params: {},
              status_text: '完成',
              result: { ok: false, total: 2, error_count: 2, results: [{ err: 'Permission denied' }] },
              error: null,
              cancel_requested: false,
              queued_at: '2026-06-28T00:03:00Z',
              started_at: '2026-06-28T00:03:01Z',
              ended_at: '2026-06-28T00:03:02Z',
              updated_at: '2026-06-28T00:03:02Z'
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
    const drawer = getTaskCenterDrawer();
    expect(within(drawer).getByRole('button', { name: /完成\s*2/ })).toBeInTheDocument();
    expect(within(drawer).getByRole('button', { name: /异常\s*2/ })).toBeInTheDocument();
    fireEvent.change(within(drawer).getByLabelText('任务搜索'), { target: { value: 'Catalog' } });
    expect(within(drawer).getByText('Catalog 导入')).toBeInTheDocument();
    expect(within(drawer).getByText('导入 3 项')).toBeInTheDocument();
    expect(within(drawer).queryByText('{"imported":3}')).not.toBeInTheDocument();
    expect(within(drawer).queryByText('扫描电影库')).not.toBeInTheDocument();
    fireEvent.click(within(drawer).getByRole('button', { name: '清空任务搜索' }));
    expect(await within(drawer).findByText('扫描电影库')).toBeInTheDocument();
    expect(within(drawer).getByText('转存 1/1 · 新增 STRM 12 · 可疑项 2')).toBeInTheDocument();
    expect(within(drawer).queryByText(/"transfer":/)).not.toBeInTheDocument();

    fireEvent.click(within(drawer).getByRole('button', { name: '展开可见' }));
    expect(screen.getByText('22222222-2222-4222-8222-222222222222')).toBeInTheDocument();
    fireEvent.click(within(drawer).getByRole('button', { name: '收起可见' }));
    expect(screen.queryByText('22222222-2222-4222-8222-222222222222')).not.toBeInTheDocument();

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
    expect(screen.getByText('追更完结归档')).toBeInTheDocument();
    expect(screen.getAllByText('结果失败：2/2 项失败').length).toBeGreaterThan(0);
    expect(within(drawer).queryByText('追更一条龙更新')).not.toBeInTheDocument();
  });

  it('renders smart action task results as structured summaries in task center', async () => {
    const smartActionTask = taskRun(
      '88888888-8888-4888-8888-888888888888',
      'smart_action_execute',
      '智能动作执行：莫离',
      'done',
      {
        action_id: smartAction.id,
        action_type: 'transfer_update_series',
        subject: smartAction.subject,
        dry_run: false,
        steps: [
          {
            key: 'open_context',
            title: '打开追更检查并锁定莫离',
            executor: 'open_tab',
            status: 'done',
            message: '已记录建议对应的功能入口。',
            result: { ok: true }
          },
          {
            key: 'zhuigeng_update_execute',
            title: '转存候选资源并生成 STRM',
            executor: 'task_pipeline',
            status: 'done',
            message: '步骤完成。',
            result: {
              ok: true,
              task: taskRun(
                '99999999-9999-4999-8999-999999999999',
                'zhuigeng_update',
                '追更一条龙更新: 莫离'
              )
            }
          },
          {
            key: 'poster_fix',
            title: '刷新 Emby 媒体库并修复海报',
            executor: 'existing_endpoint',
            status: 'error',
            message: 'TMDb ID 缺失，等待人工补齐',
            result: { ok: false, err: 'tmdb missing' }
          }
        ],
        verification: {
          status: 'partial',
          message: '如果 Emby 刷新延迟，任务中心会提示复查',
          checks: [{
            key: 'poster-ok',
            title: '海报可见',
            source: 'poster_detection',
            expected: '条目存在 Primary 图片'
          }]
        },
        next_actions: [{
          action_type: 'transfer_update_series',
          label: '更新转存',
          tab: 'zhuigeng',
          reason: '该建议仍需在具体功能页完成对象级处理。',
          subject: smartAction.subject
        }]
      }
    );
    let nextActionRequest: Record<string, unknown> = {};
    let nextActionMethod = '';
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/tasks') {
        return jsonResponse({ active_count: 0, tasks: [smartActionTask] });
      }
      if (url.pathname === '/api/v2/smart-actions/from-next-action') {
        nextActionMethod = init?.method || '';
        nextActionRequest = JSON.parse(String(init?.body || '{}'));
        return jsonResponse({
          ok: true,
          persisted: true,
          warnings: [],
          action: {
            ...smartAction,
            id: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',
            title: '下一步：更新转存',
            summary: '该建议仍需在具体功能页完成对象级处理。',
            source: 'task_next_actions',
            tab: 'zhuigeng',
            action_label: '更新转存'
          }
        });
      }
      if (url.pathname === '/api/v2/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 1,
          limit: 80,
          offset: 0,
          actions: [smartAction],
          warnings: [],
          summary: {
            total: 1,
            suggested: 1,
            auto_ready: 0,
            confirm_required: 1,
            running: 0,
            failed: 0,
            high: 1,
            medium: 0,
            low: 0,
            critical: 0
          }
        });
      }
      if (url.pathname === '/api/v2/smart-actions/dddddddd-dddd-4ddd-8ddd-dddddddddddd') {
        return jsonResponse({
          ok: true,
          action: {
            ...smartAction,
            id: 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',
            title: '下一步：更新转存',
            summary: '该建议仍需在具体功能页完成对象级处理。',
            source: 'task_next_actions',
            tab: 'zhuigeng',
            action_label: '更新转存'
          }
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '任务中心' }));

    const drawer = getTaskCenterDrawer();
    expect(await within(drawer).findByText('智能动作执行：莫离')).toBeInTheDocument();
    expect(within(drawer).getByText('追更更新转存 · 莫离 (2026) · 步骤 2/3，失败 1 · 验收 需复查')).toBeInTheDocument();
    expect(within(drawer).queryByText((_, element) => element?.textContent?.includes('"action_id"') ?? false)).not.toBeInTheDocument();

    fireEvent.click(within(drawer).getByRole('button', { name: '展开任务详情：智能动作执行：莫离' }));

    expect(within(drawer).getByRole('heading', { name: '智能动作结果' })).toBeInTheDocument();
    expect(within(drawer).getByText('动作类型')).toBeInTheDocument();
    expect(within(drawer).getByText('追更更新转存')).toBeInTheDocument();
    expect(within(drawer).getByText('对象')).toBeInTheDocument();
    expect(within(drawer).getByText('莫离 (2026)')).toBeInTheDocument();
    expect(within(drawer).getByText('步骤列表')).toBeInTheDocument();
    expect(within(drawer).getByText('转存候选资源并生成 STRM')).toBeInTheDocument();
    expect(within(drawer).getAllByText((_, element) => (
      element?.textContent?.includes('下游任务 追更一条龙更新: 莫离') ?? false
    )).length).toBeGreaterThan(0);
    expect(within(drawer).getByText('刷新 Emby 媒体库并修复海报')).toBeInTheDocument();
    expect(within(drawer).getByText('TMDb ID 缺失，等待人工补齐')).toBeInTheDocument();
    expect(within(drawer).getByText('验收状态')).toBeInTheDocument();
    expect(within(drawer).getByText('需复查')).toBeInTheDocument();
    expect(within(drawer).getByText('海报可见：条目存在 Primary 图片')).toBeInTheDocument();
    expect(within(drawer).getByText('后续动作')).toBeInTheDocument();
    expect(within(drawer).getByText('更新转存 · 入口：追更检查 · 该建议仍需在具体功能页完成对象级处理。')).toBeInTheDocument();
    fireEvent.click(within(drawer).getByRole('button', { name: '生成智能动作' }));
    expect(await within(drawer).findByText('下一步：更新转存')).toBeInTheDocument();
    const nextActionBody = nextActionRequest;
    const nextActionPayload = nextActionBody.next_action as Record<string, unknown>;
    expect(nextActionMethod).toBe('POST');
    expect(nextActionBody.task_id).toBe(smartActionTask.id);
    expect(nextActionBody.source_action_id).toBe(smartAction.id);
    expect(nextActionPayload.action_type).toBe('transfer_update_series');
    expect((nextActionPayload.subject as Record<string, unknown>)?.name).toBe('莫离');
    expect(within(drawer).getByText('建议入口：zhuigeng · 更新转存')).toBeInTheDocument();
    fireEvent.click(within(drawer).getByRole('button', { name: '打开详情' }));
    expect(await screen.findByRole('heading', { name: '智能动作详情' })).toBeInTheDocument();
    expect((await screen.findAllByText('下一步：更新转存')).length).toBeGreaterThan(0);
    expect(within(drawer).getByText('技术详情 JSON')).toBeInTheDocument();
  });

  it('generates a diagnostic smart action from a failed task detail', async () => {
    const normalTask = taskRun(
      'aaaaaaaa-1111-4111-8111-aaaaaaaa1111',
      'scan',
      '扫描电影库',
      'done',
      { ok: true, total: 1, ok_count: 1 }
    );
    const failedTask = {
      ...taskRun('bbbbbbbb-2222-4222-8222-bbbbbbbb2222', 'scan', '扫描电视剧库', 'error'),
      status_text: '失败',
      error: 'Emby timeout',
      ended_at: '2026-06-29T00:00:04Z'
    };
    const diagnosticAction = {
      ...smartAction,
      id: 'cccccccc-3333-4333-8333-cccccccc3333',
      action_type: 'task_retry_or_diagnose',
      title: '扫描电视剧库失败诊断',
      summary: '检测到 Emby timeout，建议先查看系统日志并重试扫描。',
      tab: 'system',
      action_label: '查看日志'
    };
    let generateCalls = 0;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/tasks') {
        return jsonResponse({ active_count: 0, tasks: [normalTask, failedTask] });
      }
      if (url.pathname === `/api/v2/smart-actions/from-task/${failedTask.id}`) {
        generateCalls += 1;
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        return jsonResponse({ ok: true, action: diagnosticAction });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '任务中心' }));
    const drawer = getTaskCenterDrawer();
    expect(await within(drawer).findByText('扫描电影库')).toBeInTheDocument();

    fireEvent.click(within(drawer).getByRole('button', { name: '展开任务详情：扫描电影库' }));
    expect(within(drawer).queryByRole('button', { name: '生成诊断智能动作' })).not.toBeInTheDocument();

    fireEvent.click(within(drawer).getByRole('button', { name: '展开任务详情：扫描电视剧库' }));
    fireEvent.click(within(drawer).getByRole('button', { name: '生成诊断智能动作' }));

    await waitFor(() => expect(generateCalls).toBe(1));
    expect(await screen.findByText('已生成诊断智能动作：扫描电视剧库失败诊断')).toBeInTheDocument();
    const generated = within(drawer).getByLabelText('已生成诊断动作');
    expect(within(generated).getByText('扫描电视剧库失败诊断')).toBeInTheDocument();
    expect(within(generated).getByText('检测到 Emby timeout，建议先查看系统日志并重试扫描。')).toBeInTheDocument();
    expect(within(generated).getByText('建议入口：system · 查看日志')).toBeInTheDocument();
  });

  it('renders the dashboard read-only overview with csrf protected insight calls', async () => {
    let cleanupCalled = false;
    let gapsCalled = false;
    let dashboardSmartCalled = false;
    let smartSummaryCalled = false;
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
      if (url.pathname === '/api/v2/dashboard/smart-actions') {
        dashboardSmartCalled = true;
        expect(init?.method || 'GET').toBe('GET');
        return jsonResponse({
          ok: true,
          total: 2,
          warnings: [],
          todo: {
            noposter: 2,
            no_rating: 3,
            dups_auto: 1,
            dups_review: 1,
            airing_count: 1,
            airing_low_count: 1,
            noposter_by_lib: { '剧集': 1, '电影': 1 },
            no_rating_by_lib: { '剧集': 2, '电影': 1 },
            noposter_err: null,
            no_rating_err: null,
            dups_err: null,
            airing_err: null
          },
          actions: [
            {
              severity: 'high',
              area: 'zhuigeng',
              title: '追更剧有新集可更新',
              message: '1 部在更剧落后，可智能找资源并一条龙更新。',
              count: 1,
              tab: 'zhuigeng',
              action: '智能找资源',
              source: 'zhuigeng.update_needed'
            },
            {
              severity: 'medium',
              area: 'dedup',
              title: '复核人工去重组',
              message: '1 个重复组需要人工确认。',
              count: 1,
              tab: 'dedup',
              action: '复核重复资源',
              source: 'dedup.review_groups'
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/smart-actions/summary') {
        smartSummaryCalled = true;
        expect(init?.method || 'GET').toBe('GET');
        return jsonResponse({
          ok: true,
          warnings: [],
          summary: {
            total: 4,
            suggested: 2,
            running: 1,
            failed: 1,
            auto_ready: 1,
            confirm_required: 2,
            low: 1,
            medium: 1,
            high: 1,
            critical: 1
          }
        });
      }
      return undefined;
    });

    render(<App />);

    expect(await screen.findByText('Rust Preview 总览')).toBeInTheDocument();
    expect(await screen.findByText('需要处理 unmatched')).toBeInTheDocument();
    expect(await screen.findByText('失败扫库')).toBeInTheDocument();
    expect(await screen.findByText('strm')).toBeInTheDocument();
    expect(screen.getAllByText('120').length).toBeGreaterThan(0);
    expect(screen.queryByText('字幕')).not.toBeInTheDocument();
    expect(screen.getByText('无海报')).toBeInTheDocument();
    expect(screen.getByText('无评分')).toBeInTheDocument();
    expect(screen.getByText('智能下一步')).toBeInTheDocument();
    expect(screen.getByText('可自动执行')).toBeInTheDocument();
    expect(screen.getByText('需要确认')).toBeInTheDocument();
    expect(screen.getByText('执行中/失败')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '打开工作台' })).toBeInTheDocument();
    expect(screen.getByText('追更剧有新集可更新')).toBeInTheDocument();
    expect(screen.queryByText('旧版待办计数')).not.toBeInTheDocument();
    expect(screen.queryByText('无海报 1')).not.toBeInTheDocument();
    expect(screen.queryByText('无评分 2')).not.toBeInTheDocument();
    await waitFor(() => {
      expect(cleanupCalled).toBe(true);
      expect(gapsCalled).toBe(true);
      expect(dashboardSmartCalled).toBe(true);
      expect(smartSummaryCalled).toBe(true);
    });
  });

  it('loads the smart actions workbench and shows read-only detail evidence', async () => {
    const requested: string[] = [];
    mockApi((url, init) => {
      requested.push(`${init?.method || 'GET'} ${url.pathname}${url.search}`);
      if (url.pathname.includes('/execute')) {
        throw new Error(`smart action execute endpoint should not be called: ${url.pathname}`);
      }
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
      if (url.pathname === '/api/v2/dashboard/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 0,
          warnings: [],
          todo: {
            noposter: 0,
            no_rating: 0,
            dups_auto: 0,
            dups_review: 0,
            airing_count: 0,
            airing_low_count: 0,
            noposter_by_lib: {},
            no_rating_by_lib: {},
            noposter_err: null,
            no_rating_err: null,
            dups_err: null,
            airing_err: null
          },
          actions: []
        });
      }
      if (url.pathname === '/api/v2/smart-actions') {
        expect(init?.method || 'GET').toBe('GET');
        return jsonResponse({
          ok: true,
          total: 1,
          limit: Number(url.searchParams.get('limit') || 80),
          offset: 0,
          warnings: ['TMDb key 未配置时仅展示可判断的动作'],
          summary: {
            total: 1,
            suggested: 1,
            running: 0,
            failed: 0,
            auto_ready: 0,
            confirm_required: 1,
            low: 0,
            medium: 0,
            high: 1,
            critical: 0
          },
          actions: [smartAction]
        });
      }
      if (url.pathname === '/api/v2/smart-actions/refresh') {
        expect(init?.method).toBe('POST');
        return jsonResponse(taskRun('99999999-9999-4999-8999-999999999999', 'smart_actions_refresh', '刷新智能动作建议'));
      }
      if (url.pathname === `/api/v2/smart-actions/${smartAction.id}`) {
        return jsonResponse({ ok: true, action: smartAction });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能动作' }));
    expect(await screen.findByText('智能动作工作台')).toBeInTheDocument();
    expect(screen.getByText('建议清单')).toBeInTheDocument();
    expect(screen.getByText('莫离有新集可更新')).toBeInTheDocument();
    expect(screen.getByText('Emby 当前 25 集，资源侧发现 26 集，建议一条龙转存、生成 STRM 并刷新媒体库。')).toBeInTheDocument();
    expect(screen.getByText('TMDb key 未配置时仅展示可判断的动作')).toBeInTheDocument();
    expect(screen.getByText('对象级动作')).toBeInTheDocument();
    expect(screen.getByLabelText('智能动作策略执行视图')).toBeInTheDocument();
    expect(screen.getByText('0 个 auto_ready')).toBeInTheDocument();
    expect(screen.getByText('1 个需确认')).toBeInTheDocument();
    expect(screen.getByLabelText('智能动作批量选择')).toBeInTheDocument();
    expect(screen.getByLabelText('选择批量执行：莫离有新集可更新')).toBeDisabled();
    expect(screen.getByText('高风险动作必须人工确认，不能直接自动执行')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '刷新建议' }));
    expect(await screen.findByText('智能动作刷新任务已提交：刷新智能动作建议')).toBeInTheDocument();
    expect(screen.getByLabelText('智能动作刷新任务')).toHaveTextContent('刷新智能动作建议');

    fireEvent.change(screen.getByLabelText('搜索'), { target: { value: '莫离' } });
    await waitFor(() => {
      expect(requested.some((item) => item.includes('/api/v2/smart-actions?q=%E8%8E%AB%E7%A6%BB'))).toBe(true);
    });
    fireEvent.change(screen.getByLabelText('对象'), { target: { value: 'series' } });
    fireEvent.change(screen.getByLabelText('库名'), { target: { value: '电视剧' } });
    await waitFor(() => {
      expect(requested.some((item) => item.includes('subject_kind=series') && item.includes('lib=%E7%94%B5%E8%A7%86%E5%89%A7'))).toBe(true);
    });

    fireEvent.click(screen.getByRole('button', { name: '查看详情：莫离有新集可更新' }));
    expect(await screen.findByRole('heading', { name: '智能动作详情' })).toBeInTheDocument();
    expect(screen.getByText('推荐理由')).toBeInTheDocument();
    expect(screen.getByText('更新转存并处理旧目录')).toBeInTheDocument();
    expect(screen.getByText('追更库集数落后')).toBeInTheDocument();
    expect(screen.getByText('Emby 当前剧集')).toBeInTheDocument();
    expect(screen.getByText(/current_max_episode/)).toBeInTheDocument();
    expect(screen.getByText('会删除同剧旧目录，必须保留 undo 记录')).toBeInTheDocument();
    expect(screen.getByText('打开追更检查并锁定莫离')).toBeInTheDocument();
    expect(screen.getByText('转存候选资源并生成 STRM')).toBeInTheDocument();
    expect(screen.getByText('新集可见')).toBeInTheDocument();
    expect(screen.getByText('新集入库、旧目录已清理、海报可见')).toBeInTheDocument();
    expect(screen.getByLabelText('执行前安全提示')).toBeInTheDocument();
    expect(screen.getByText('将把你填写的候选资源交给追更一条龙更新。提交前必须填写候选链接，并明确目标库或自定义 115 cid。')).toBeInTheDocument();
    expect(screen.getByLabelText('追更更新执行参数')).toBeInTheDocument();
    expect(screen.getByLabelText('追更更新资源名')).toHaveValue('The.First.Jasmine.2026.S01E26');
    expect(screen.getByLabelText('追更更新 link_type')).toHaveValue('share115');
    expect(screen.getByLabelText('追更更新目标库')).toHaveValue('电视剧');
    expect(screen.getByLabelText('高风险执行确认')).toHaveTextContent('候选链接不能为空');
    expect(screen.getByLabelText('执行参数检查')).toHaveTextContent('还不能执行');
    expect(screen.getByLabelText('执行参数检查')).toHaveTextContent('候选链接不能为空');
    expect(screen.getByRole('button', { name: '确认追更更新' })).toBeDisabled();
    expect(requested.some((item) => item.includes('/api/v2/smart-actions/execute'))).toBe(false);
  });

  it('executes transfer update smart actions with candidate and target payload', async () => {
    const requested: string[] = [];
    let executeBody: unknown = null;
    let verifyCalls = 0;
    mockApi((url, init) => {
      requested.push(`${init?.method || 'GET'} ${url.pathname}${url.search}`);
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
      if (url.pathname === '/api/v2/dashboard/smart-actions') {
        return jsonResponse({ ok: true, total: 0, warnings: [], todo: {}, actions: [] });
      }
      if (url.pathname === '/api/v2/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 1,
          limit: 80,
          offset: 0,
          warnings: [],
          summary: {
            total: 1,
            suggested: 1,
            running: 0,
            failed: 0,
            auto_ready: 0,
            confirm_required: 1,
            low: 0,
            medium: 0,
            high: 1,
            critical: 0
          },
          actions: [smartAction]
        });
      }
      if (url.pathname === `/api/v2/smart-actions/${smartAction.id}`) {
        return jsonResponse({ ok: true, action: smartAction });
      }
      if (url.pathname === `/api/v2/smart-actions/${smartAction.id}/execute`) {
        executeBody = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          task: taskRun('aaaaaaaa-eeee-4aaa-8aaa-aaaaaaaaaaa1', 'smart_action_execute', '智能动作追更更新')
        });
      }
      if (url.pathname === `/api/v2/smart-actions/${smartAction.id}/verify`) {
        expect(init?.method).toBe('POST');
        verifyCalls += 1;
        return jsonResponse({
          id: smartAction.id,
          ok: true,
          status: 'partial',
          result: {
            check_summaries: [
              { summary: '新集还未在 Emby 可见，等待媒体库刷新后再复查。' }
            ]
          },
          warnings: ['Emby 刷新可能有延迟']
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能动作' }));
    expect(await screen.findByText('莫离有新集可更新')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '查看详情：莫离有新集可更新' }));
    expect(await screen.findByRole('heading', { name: '智能动作详情' })).toBeInTheDocument();

    const submitButton = screen.getByRole('button', { name: '确认追更更新' });
    expect(submitButton).toBeDisabled();
    fireEvent.change(screen.getByLabelText('追更更新链接/分享链接'), { target: { value: 'https://115.com/s/jasmine' } });
    expect(submitButton).not.toBeDisabled();

    fireEvent.change(screen.getByLabelText('追更更新目标库'), { target: { value: '' } });
    expect(submitButton).toBeDisabled();
    expect(screen.getByLabelText('高风险执行确认')).toHaveTextContent('请填写目标库');

    fireEvent.change(screen.getByLabelText('追更更新自定义 cid'), { target: { value: '0' } });
    expect(submitButton).toBeDisabled();
    expect(screen.getByLabelText('高风险执行确认')).toHaveTextContent('正整数');

    fireEvent.change(screen.getByLabelText('追更更新自定义 cid'), { target: { value: '67890' } });
    fireEvent.change(screen.getByLabelText('追更更新提取码 rc'), { target: { value: 'rc' } });
    expect(submitButton).not.toBeDisabled();

    fireEvent.click(submitButton);
    const modal = screen.getByRole('heading', { name: '确认追更更新' }).closest('.modal') as HTMLElement;
    const confirmButton = within(modal).getByRole('button', { name: '确认追更更新' });
    expect(confirmButton).toBeDisabled();
    fireEvent.change(within(modal).getByLabelText('输入确认文本：执行'), { target: { value: '执行' } });
    fireEvent.click(confirmButton);

    await waitFor(() => {
      expect(executeBody).toEqual({
        confirm_text: '执行',
        payload: {
          candidate: {
            name: 'The.First.Jasmine.2026.S01E26',
            link: 'https://115.com/s/jasmine',
            link_type: 'share115',
            share: 'https://115.com/s/jasmine',
            rc: 'rc'
          },
          target: { cid: '67890' }
        }
      });
    });
    expect(await screen.findByText('智能动作已提交：智能动作追更更新')).toBeInTheDocument();
    const nextSteps = await screen.findByLabelText('智能动作下一步');
    expect(within(nextSteps).getByRole('button', { name: '查看任务中心' })).toBeInTheDocument();
    expect(within(nextSteps).getByRole('button', { name: '验证结果' })).toBeInTheDocument();
    fireEvent.click(within(nextSteps).getByRole('button', { name: '验证结果' }));
    expect(await screen.findByLabelText('智能动作验证结果')).toHaveTextContent('新集还未在 Emby 可见');
    expect(verifyCalls).toBe(1);
    expect(requested.some((item) => item === `POST /api/v2/smart-actions/${smartAction.id}/execute`)).toBe(true);
  });

  it('executes transfer add-new smart actions with target and package acknowledgement', async () => {
    const requested: string[] = [];
    let executeBody: unknown = null;
    mockApi((url, init) => {
      requested.push(`${init?.method || 'GET'} ${url.pathname}${url.search}`);
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
      if (url.pathname === '/api/v2/dashboard/smart-actions') {
        return jsonResponse({ ok: true, total: 0, warnings: [], todo: {}, actions: [] });
      }
      if (url.pathname === '/api/v2/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 1,
          limit: 80,
          offset: 0,
          warnings: [],
          summary: {
            total: 1,
            suggested: 1,
            running: 0,
            failed: 0,
            auto_ready: 0,
            confirm_required: 1,
            low: 0,
            medium: 0,
            high: 1,
            critical: 0
          },
          actions: [transferAddNewSmartAction]
        });
      }
      if (url.pathname === `/api/v2/smart-actions/${transferAddNewSmartAction.id}`) {
        return jsonResponse({ ok: true, action: transferAddNewSmartAction });
      }
      if (url.pathname === `/api/v2/smart-actions/${transferAddNewSmartAction.id}/execute`) {
        executeBody = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          task: taskRun('eeeeeeee-eeee-4eee-8eee-eeeeeeeeeee1', 'smart_action_execute', '智能动作新增转存')
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能动作' }));
    expect(await screen.findByText('新片可从找资源转存')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '查看详情：新片可从找资源转存' }));

    expect(await screen.findByRole('heading', { name: '智能动作详情' })).toBeInTheDocument();
    expect(screen.getByText('将把当前候选资源交给智能动作执行新增转存。提交前必须明确目标库或自定义 115 cid，整包合集还要额外确认。')).toBeInTheDocument();
    expect(screen.getByLabelText('新增转存执行参数')).toBeInTheDocument();
    const candidate = screen.getByLabelText('新增转存候选资源');
    expect(candidate).toHaveTextContent('Galaxy.Sample.2026.2160p');
    expect(candidate).toHaveTextContent('https://115.com/s/galaxy?password=xy12');
    expect(candidate).toHaveTextContent('整包合集');

    const submitButton = screen.getByRole('button', { name: '确认新增转存' });
    expect(submitButton).toBeDisabled();
    fireEvent.change(screen.getByLabelText('新增转存目标库'), { target: { value: '电影' } });
    fireEvent.change(screen.getByLabelText('新增转存自定义 cid'), { target: { value: '12345' } });
    expect(submitButton).toBeDisabled();
    fireEvent.change(screen.getByLabelText('含整包合集，输入“整包”确认'), { target: { value: '整包' } });
    expect(submitButton).not.toBeDisabled();

    fireEvent.click(submitButton);
    const modal = screen.getByRole('heading', { name: '确认新增转存' }).closest('.modal') as HTMLElement;
    const confirmButton = within(modal).getByRole('button', { name: '确认新增转存' });
    expect(confirmButton).toBeDisabled();
    fireEvent.change(within(modal).getByLabelText('输入确认文本：执行'), { target: { value: '执行' } });
    fireEvent.click(confirmButton);

    await waitFor(() => {
      expect(executeBody).toEqual({
        confirm_text: '执行',
        payload: {
          request: {
            target: { cid: '12345', lib: '电影' },
            package_ack: '整包'
          }
        }
      });
    });
    expect(await screen.findByText('智能动作已提交：智能动作新增转存')).toBeInTheDocument();
    expect(await screen.findByLabelText('智能动作下一步')).toHaveTextContent('查看任务中心');
    expect(screen.getByLabelText('智能动作下一步')).toHaveTextContent('验证结果');
    expect(requested.some((item) => item === `POST /api/v2/smart-actions/${transferAddNewSmartAction.id}/execute`)).toBe(true);
  });

  it('executes a low-risk auto smart action from the detail drawer', async () => {
    let executeCalls = 0;
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
      if (url.pathname === '/api/v2/dashboard/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 0,
          warnings: [],
          todo: {
            noposter: 0,
            no_rating: 0,
            dups_auto: 0,
            dups_review: 0,
            airing_count: 0,
            airing_low_count: 0,
            noposter_by_lib: {},
            no_rating_by_lib: {},
            noposter_err: null,
            no_rating_err: null,
            dups_err: null,
            airing_err: null
          },
          actions: []
        });
      }
      if (url.pathname === '/api/v2/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 1,
          limit: 80,
          offset: 0,
          warnings: [],
          summary: {
            total: 1,
            suggested: 1,
            running: 0,
            failed: 0,
            auto_ready: 1,
            confirm_required: 0,
            low: 1,
            medium: 0,
            high: 0,
            critical: 0
          },
          actions: [lowRiskSmartAction]
        });
      }
      if (url.pathname === `/api/v2/smart-actions/${lowRiskSmartAction.id}`) {
        return jsonResponse({ ok: true, action: lowRiskSmartAction });
      }
      if (url.pathname === `/api/v2/smart-actions/${lowRiskSmartAction.id}/execute`) {
        executeCalls += 1;
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        return jsonResponse({
          ok: true,
          task: taskRun('66666666-6666-4666-8666-666666666666', 'smart_action_execute', '低风险媒体库扫描任务')
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能动作' }));
    expect(await screen.findByText('低风险媒体库扫描')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '查看详情：低风险媒体库扫描' }));
    expect(await screen.findByRole('heading', { name: '智能动作详情' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '执行动作' }));

    await waitFor(() => expect(executeCalls).toBe(1));
    expect(await screen.findByText('智能动作已提交：低风险媒体库扫描任务')).toBeInTheDocument();
    const taskInfo = screen.getByLabelText('智能动作任务信息');
    expect(within(taskInfo).getByText('低风险媒体库扫描任务')).toBeInTheDocument();
    expect(within(taskInfo).getByText('运行中')).toBeInTheDocument();
  });

  it('requires typed confirmation before executing a destructive dedup smart action', async () => {
    let executeBody: unknown = null;
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
      if (url.pathname === '/api/v2/dashboard/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 0,
          warnings: [],
          todo: {
            noposter: 0,
            no_rating: 0,
            dups_auto: 0,
            dups_review: 0,
            airing_count: 0,
            airing_low_count: 0,
            noposter_by_lib: {},
            no_rating_by_lib: {},
            noposter_err: null,
            no_rating_err: null,
            dups_err: null,
            airing_err: null
          },
          actions: []
        });
      }
      if (url.pathname === '/api/v2/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 1,
          limit: 80,
          offset: 0,
          warnings: [],
          summary: {
            total: 1,
            suggested: 1,
            running: 0,
            failed: 0,
            auto_ready: 0,
            confirm_required: 1,
            low: 0,
            medium: 0,
            high: 0,
            critical: 1
          },
          actions: [dedupSmartAction]
        });
      }
      if (url.pathname === `/api/v2/smart-actions/${dedupSmartAction.id}`) {
        return jsonResponse({ ok: true, action: dedupSmartAction });
      }
      if (url.pathname === `/api/v2/smart-actions/${dedupSmartAction.id}/execute`) {
        executeBody = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          task: taskRun('88888888-8888-4888-8888-888888888888', 'smart_action_execute', '智能动作批量去重')
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能动作' }));
    expect(await screen.findByText('莫离自动去重建议')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '查看详情：莫离自动去重建议' }));
    expect(await screen.findByRole('heading', { name: '智能动作详情' })).toBeInTheDocument();
    expect(screen.getByLabelText('高风险执行确认')).toHaveTextContent('待删条目1');

    fireEvent.click(screen.getByRole('button', { name: '确认删除旧资源' }));
    const modal = screen.getByRole('heading', { name: '确认删除重复旧资源' }).closest('.modal') as HTMLElement;
    const confirmButton = within(modal).getByRole('button', { name: '确认删除旧资源' });
    expect(confirmButton).toBeDisabled();

    fireEvent.change(within(modal).getByLabelText('输入确认文本：删除'), { target: { value: '删除' } });
    expect(confirmButton).not.toBeDisabled();
    fireEvent.click(confirmButton);

    await waitFor(() => {
      expect(executeBody).toEqual({
        confirm_text: '删除',
        payload: {
          request: {
            groups: [{
              tmdb: '12345',
              remove: [{ lib: '电视剧', folder: '莫离 旧版', item_id: 'item-old' }]
            }]
          }
        }
      });
    });
    expect(await screen.findByText('智能动作已提交：智能动作批量去重')).toBeInTheDocument();
  });

  it('requires an archive target library before confirming an archive smart action', async () => {
    let executeBody: unknown = null;
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
      if (url.pathname === '/api/v2/dashboard/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 0,
          warnings: [],
          todo: {
            noposter: 0,
            no_rating: 0,
            dups_auto: 0,
            dups_review: 0,
            airing_count: 0,
            airing_low_count: 0,
            noposter_by_lib: {},
            no_rating_by_lib: {},
            noposter_err: null,
            no_rating_err: null,
            dups_err: null,
            airing_err: null
          },
          actions: []
        });
      }
      if (url.pathname === '/api/v2/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 1,
          limit: 80,
          offset: 0,
          warnings: [],
          summary: {
            total: 1,
            suggested: 1,
            running: 0,
            failed: 0,
            auto_ready: 0,
            confirm_required: 1,
            low: 0,
            medium: 0,
            high: 1,
            critical: 0
          },
          actions: [archiveSmartAction]
        });
      }
      if (url.pathname === `/api/v2/smart-actions/${archiveSmartAction.id}`) {
        return jsonResponse({ ok: true, action: archiveSmartAction });
      }
      if (url.pathname === `/api/v2/smart-actions/${archiveSmartAction.id}/execute`) {
        executeBody = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          task: taskRun('99999999-9999-4999-8999-999999999998', 'smart_action_execute', '智能动作完结归档')
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能动作' }));
    expect(await screen.findByText('莫离已完结可归档')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '查看详情：莫离已完结可归档' }));
    expect(await screen.findByRole('heading', { name: '智能动作详情' })).toBeInTheDocument();

    const submitButton = screen.getByRole('button', { name: '确认归档' });
    expect(submitButton).toBeDisabled();
    fireEvent.change(screen.getByLabelText('归档目标库'), { target: { value: '完结剧' } });
    expect(submitButton).not.toBeDisabled();

    fireEvent.click(submitButton);
    const modal = screen.getByRole('heading', { name: '确认归档完结剧' }).closest('.modal') as HTMLElement;
    const confirmButton = within(modal).getByRole('button', { name: '确认归档' });
    expect(confirmButton).toBeDisabled();
    fireEvent.change(within(modal).getByLabelText('输入确认文本：归档'), { target: { value: '归档' } });
    fireEvent.click(confirmButton);

    await waitFor(() => {
      expect(executeBody).toEqual({
        confirm_text: '归档',
        payload: { to_lib: '完结剧' }
      });
    });
    expect(await screen.findByText('智能动作已提交：智能动作完结归档')).toBeInTheDocument();
  });

  it('only batch-executes low-risk auto-ready smart actions', async () => {
    const executed: string[] = [];
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
      if (url.pathname === '/api/v2/dashboard/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 0,
          warnings: [],
          todo: {
            noposter: 0,
            no_rating: 0,
            dups_auto: 0,
            dups_review: 0,
            airing_count: 0,
            airing_low_count: 0,
            noposter_by_lib: {},
            no_rating_by_lib: {},
            noposter_err: null,
            no_rating_err: null,
            dups_err: null,
            airing_err: null
          },
          actions: []
        });
      }
      if (url.pathname === '/api/v2/smart-actions') {
        return jsonResponse({
          ok: true,
          total: 2,
          limit: 80,
          offset: 0,
          warnings: [],
          summary: {
            total: 2,
            suggested: 2,
            running: 0,
            failed: 0,
            auto_ready: 1,
            confirm_required: 1,
            low: 1,
            medium: 0,
            high: 1,
            critical: 0
          },
          actions: [lowRiskSmartAction, smartAction]
        });
      }
      if (url.pathname === '/api/v2/smart-actions/execute-batch') {
        executed.push(url.pathname);
        expect(init?.method).toBe('POST');
        expect(JSON.parse(String(init?.body))).toEqual({ ids: [lowRiskSmartAction.id] });
        return jsonResponse({
          ok: true,
          total: 1,
          submitted: 1,
          failed: 0,
          results: [{
            id: lowRiskSmartAction.id,
            ok: true,
            status: 'queued',
            task: taskRun('77777777-7777-4777-8777-777777777777', 'smart_action_execute', '批量低风险媒体库扫描'),
            err: null
          }]
        });
      }
      if (url.pathname === `/api/v2/smart-actions/${smartAction.id}/execute`) {
        throw new Error('high risk action must not be batch executed');
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能动作' }));
    expect(await screen.findByText('低风险媒体库扫描')).toBeInTheDocument();
    expect(screen.getByText('1 个 auto_ready')).toBeInTheDocument();
    expect(screen.getByText('1 个需确认')).toBeInTheDocument();

    const lowRiskCheckbox = screen.getByLabelText('选择批量执行：低风险媒体库扫描');
    const highRiskCheckbox = screen.getByLabelText('选择批量执行：莫离有新集可更新');
    expect(lowRiskCheckbox).not.toBeDisabled();
    expect(highRiskCheckbox).toBeDisabled();

    fireEvent.click(screen.getByRole('button', { name: '选择全部可批量' }));
    expect(lowRiskCheckbox).toBeChecked();
    expect(highRiskCheckbox).not.toBeChecked();

    fireEvent.click(screen.getByRole('button', { name: '批量执行 1 项' }));

    await waitFor(() => {
      expect(executed).toEqual(['/api/v2/smart-actions/execute-batch']);
    });
    expect(await screen.findByText('已提交 1 个低风险智能动作')).toBeInTheDocument();
    const bulkResult = screen.getByLabelText('批量执行结果');
    expect(within(bulkResult).getByText('已提交 1 项')).toBeInTheDocument();
    expect(within(bulkResult).getByText('低风险动作已进入任务中心，后续进度在那里查看。')).toBeInTheDocument();
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

  it('loads scan workspace and creates library/item refresh tasks with csrf', async () => {
    const strmLibs: Array<string | null> = [];
    const libraryPayloads: unknown[] = [];
    const scanPayloads: unknown[] = [];
    let libraries = [
      { id: 'movie-lib', name: '电影', type: 'movies', paths: ['/strm/电影'] },
      { id: 'show-lib', name: '电视剧', type: 'tvshows', paths: ['/strm/电视剧'] }
    ];
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
        if (init?.method === 'POST') {
          const headers = init?.headers as Headers;
          expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
          const payload = JSON.parse(String(init?.body));
          libraryPayloads.push(payload);
          const created = {
            id: 'anime-lib',
            name: String(payload.name),
            type: String(payload.collection_type),
            paths: ['/strm/动画']
          };
          libraries = [...libraries, created];
          return jsonResponse({
            ok: true,
            name: created.name,
            id: created.id,
            library: created,
            created_dirs: ['/volume1/strm/动画'],
            emby_status: { status: 'created' },
            warnings: ['目录需要稍后确认']
          });
        }
        return jsonResponse({ libraries });
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
    fireEvent.change(screen.getByLabelText('新建媒体库名称'), { target: { value: '动画' } });
    fireEvent.change(screen.getByLabelText('新建媒体库类型'), { target: { value: 'tvshows' } });
    fireEvent.click(screen.getByRole('button', { name: '创建媒体库' }));
    await waitFor(() => expect(libraryPayloads[0]).toEqual({ name: '动画', collection_type: 'tvshows' }));
    expect(await screen.findByText('目录需要稍后确认')).toBeInTheDocument();
    await waitFor(() => expect((screen.getByLabelText('扫描目标库') as HTMLSelectElement).value).toBe('动画'));
    await waitFor(() => expect(strmLibs).toContain('动画'));

    fireEvent.change(screen.getByLabelText('扫描目标库'), { target: { value: '电影' } });
    expect((await screen.findAllByText('Movie/Movie.strm')).length).toBeGreaterThan(0);
    await waitFor(() => expect(strmLibs).toContain('电影'));

    fireEvent.click(screen.getByRole('button', { name: '仅 Emby 刷新' }));
    await waitFor(() => expect(scanPayloads[0]).toEqual({ lib: '电影', recursive: true, full: false }));

    fireEvent.change(screen.getByLabelText('扫描目录关键词'), { target: { value: 'Movie' } });
    fireEvent.click(screen.getByLabelText('首次无 tmdbid 也生成'));
    fireEvent.click(screen.getByLabelText('清理孤儿 STRM（危险）'));
    fireEvent.click(screen.getByRole('button', { name: '扫描入库' }));
    expect(await screen.findByText('确认清理孤儿 STRM')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认生成并清理' }));
    await waitFor(() => expect(scanPayloads[1]).toEqual({
      lib: '电影',
      recursive: true,
      full: false,
      generate_strm: true,
      force_refresh: true,
      keyword: 'Movie',
      fullauto: true,
      cleanup_orphans: true
    }));
    expect(screen.queryByText(/迁移中/)).not.toBeInTheDocument();

    const previousScanTask = {
      id: 'scan-done',
      kind: 'scan_library',
      label: '扫描库: 电影',
      status: 'running',
      progress: 1,
      total: 2,
      status_text: '扫描中',
      cancel_requested: false,
      queued_at: '2026-06-28T00:00:00Z',
      started_at: '2026-06-28T00:00:01Z',
      ended_at: null,
      updated_at: '2026-06-28T00:00:02Z',
      params: {},
      result: null,
      source: 'api'
    };
    const completedScanTask = {
      ...previousScanTask,
      status: 'done',
      progress: 2,
      ended_at: '2026-06-28T00:00:03Z',
      updated_at: '2026-06-28T00:00:03Z',
      status_text: '完成',
      result: {
        ok: true,
        mode: 'library',
        requested: '电影',
        global_refresh: false,
        triggered: 7,
        items: [
          { code: 204, id: 'item-a', name: 'Movie A' },
          { code: 204, id: 'item-b', name: 'Movie B' }
        ],
        strm: {
          lib: '电影',
          keyword: 'Movie',
          matched: 5,
          new_count: 3,
          new_folders: { Movie: 3 },
          orphan_cleanup_skipped: false,
          orphans_cleaned: 2,
          permissions_fixed: 1,
          refreshed: true,
          refresh_code: 204,
          attention: ['需要人工确认']
        }
      },
      source: 'api'
    };
    window.dispatchEvent(new CustomEvent('emby-manager:task-completed', {
      detail: {
        task: completedScanTask,
        previousTask: previousScanTask,
        previousStatus: 'running'
      }
    }));
    expect(await screen.findByText('最近扫描结果')).toBeInTheDocument();
    expect(screen.getByText('新增 STRM')).toBeInTheDocument();
    expect(screen.getByText('清孤儿 / 权限')).toBeInTheDocument();
    expect(screen.getByText('2 / 1')).toBeInTheDocument();
    expect(screen.getByText('需要人工确认')).toBeInTheDocument();

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
    expect(screen.getByRole('img', { name: '正确电影 海报' })).toHaveAttribute(
      'src',
      '/api/v2/posters/image-proxy?url=https%3A%2F%2Fimg.example%2Fposter.jpg'
    );

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

  it('renders zhuigeng live list and starts real gaps library scans', async () => {
    const calls: string[] = [];
    let scanAiringCalled = false;
    let gapsSummaryCalled = false;
    let scanPayload: unknown = null;
    let seriesDetailQuery = '';
    let archivePayload: unknown = null;
    let resourcePlanPayload: unknown = null;
    let updatePayload: unknown = null;
    const scanAiringTask = taskRun('task-zhuigeng-airing', 'zhuigeng_scan_airing', '追更扫描在更剧');
    const gapsSummaryTask = taskRun('task-zhuigeng-gaps', 'zhuigeng_gaps_summary', '追更缺集汇总');
    const updateTask = taskRun('task-zhuigeng-update', 'zhuigeng_update', '追更一条龙更新: 示例剧');
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/zhuigeng/workbench') {
        expect(init?.method || 'GET').toBe('GET');
        calls.push(url.pathname);
        return jsonResponse(zhuigengWorkbench);
      }
      if (url.pathname === '/api/v2/zhuigeng/resource-plan') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        resourcePlanPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          item: zhuigengWorkbench.rows[0].item,
          query: '示例剧 S01 E3',
          missing_hint: 'S01 E3',
          fallback_queries: [],
          recommended: zhuigengCandidate,
          search: {
            items: [zhuigengCandidate],
            total: 1,
            limit: 16,
            offset: 0,
            has_more: false,
            query: '示例剧 S01 E3',
            exact: false,
            sort: 'resource',
            truncated: false,
            disk_types: [{ disk_type: '115', count: 1 }],
            context: {
              ok: true,
              query: '示例剧',
              total_matches: 1,
              truncated: false,
              warnings: [],
              summary: {
                matched: true,
                duplicate: false,
                duplicate_groups: 0,
                libraries: ['剧集'],
                tmdb_ids: ['100'],
                years: [],
                episode_ranges: ['S01E01-2'],
                missing_ranges: ['S01E3'],
                max_episode: 2,
                total_episodes: 2,
                note: '库内已有条目但存在缺集，可优先选补缺或全集资源'
              },
              items: []
            }
          }
        });
      }
      if (url.pathname === '/api/v2/zhuigeng/update/execute') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        updatePayload = JSON.parse(String(init?.body));
        return jsonResponse(updateTask);
      }
      if (url.pathname === '/api/v2/zhuigeng/scan-airing') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        scanAiringCalled = true;
        calls.push(url.pathname);
        return jsonResponse(scanAiringTask);
      }
      if (url.pathname === '/api/v2/zhuigeng/gaps-summary') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        gapsSummaryCalled = true;
        calls.push(url.pathname);
        return jsonResponse(gapsSummaryTask);
      }
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
            { id: 'lib-ended', name: '完结剧库', type: 'tvshows', paths: ['/strm/完结剧库'] },
            { id: 'lib-movies', name: '电影', type: 'movies', paths: ['/strm/电影'] }
          ]
        });
      }
      if (url.pathname === '/api/v2/zhuigeng/archive/execute') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        archivePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          ok: true,
          total: 1,
          tasks: [{
            id: '77777777-7777-4777-8777-777777777777',
            kind: 'zhuigeng_archive',
            label: '追更完结归档: 批量移动: 剧集 -> 完结剧库 (1 项)',
            source: 'zhuigeng',
            params: {},
            status: 'pending',
            progress: 0,
            total: 1,
            status_text: '排队中',
            result: null,
            error: null,
            cancel_requested: false,
            queued_at: '2026-06-28T00:03:00Z',
            started_at: null,
            ended_at: null,
            updated_at: '2026-06-28T00:03:00Z'
          }]
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
      if (url.pathname === '/api/v2/gaps/series') {
        expect(init?.method || 'GET').toBe('GET');
        seriesDetailQuery = url.searchParams.get('id') || '';
        return jsonResponse({
          ok: true,
          id: seriesDetailQuery,
          mode: 'season',
          have: 2,
          gaps: 1,
          max_ep: 3,
          tmdb_max: 3,
          noidx: 0,
          gap_list: [],
          seasons: [
            { season: 1, count: 2, lo: 1, hi: 3, gaps: ['2'], gapcount: 1 }
          ]
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
    expect(await screen.findByText('追更工作台')).toBeInTheDocument();
    expect(await screen.findByText('求资源文本')).toBeInTheDocument();
    expect(screen.queryByText(/当前 Rust 版没有独立追更扫描接口/)).not.toBeInTheDocument();
    expect(screen.getByText('示例剧')).toBeInTheDocument();
    expect(screen.getByText('完结剧')).toBeInTheDocument();
    expect(screen.getByText('求 示例剧 [tmdb:100] — S01 E3')).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole('button', { name: '找资源' }).at(-1)!);
    await waitFor(() => expect(resourcePlanPayload).toMatchObject({
      item: { name: '示例剧', lib: '剧集', resource_hint: 'S01 E3' },
      limit: 16
    }));
    expect(await screen.findByText('示例剧 S01E03')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '更新推荐 1' })).toBeEnabled();
    fireEvent.click(screen.getByRole('button', { name: '更新推荐 1' }));
    expect(await screen.findByText('确认批量一条龙更新')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认更新' }));
    await waitFor(() => expect(updatePayload).toMatchObject({
      item: { name: '示例剧', lib: '剧集' },
      candidate: { name: '示例剧 S01E03', link_type: 'share115', share: 'https://115.com/s/swabc' },
      target: { lib: '剧集' },
      delay_ms: 500
    }));
    expect(await screen.findByRole('combobox', { name: '归档目标库' })).toHaveValue('完结剧库');
    fireEvent.click(screen.getByLabelText('选择归档：完结剧'));
    fireEvent.click(screen.getByRole('button', { name: /归档 1/ }));
    expect(await screen.findByText('确认智能归档完结剧')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认归档' }));
    await waitFor(() => expect(archivePayload).toEqual({
      to_lib: '完结剧库',
      items: [{ lib: '剧集', name: '完结剧', id: 'series-200', folder: '完结剧 [tmdb-200]', tmdb: '200', behind: 0, resource_hint: null }],
      on_conflict: 'smart'
    }));
    expect(await screen.findByText(/归档任务：追更完结归档: 批量移动: 剧集 -> 完结剧库/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'scan-airing' }));
    await waitFor(() => expect(scanAiringCalled).toBe(true));
    dispatchTaskDone(scanAiringTask, zhuigengScanAiring);
    expect(await screen.findByText('在更扫描结果')).toBeInTheDocument();
    expect(screen.getByText('最小 TMDb 语义版')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'gaps-summary' }));
    await waitFor(() => expect(gapsSummaryCalled).toBe(true));
    dispatchTaskDone(gapsSummaryTask, zhuigengGaps);
    expect(await screen.findByText('追更缺集汇总')).toBeInTheDocument();

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
    fireEvent.change(screen.getByLabelText('Emby Series Id'), { target: { value: 'series-a' } });
    fireEvent.click(screen.getByRole('button', { name: '查询缺集' }));
    await waitFor(() => expect(seriesDetailQuery).toBe('series-a'));
    expect(await screen.findByText('series-a')).toBeInTheDocument();
    expect(screen.getByText('S01')).toBeInTheDocument();
    expect(screen.getByText('E2')).toBeInTheDocument();
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
  });

  it('renders cleanup and dedup duplicates with minimal execute flow', async () => {
    const calls: string[] = [];
    const emptyPayloads: unknown[] = [];
    const cleanupPayloads: unknown[] = [];
    let cleanupBatchPayload: unknown = null;
    let emptyFolderPayload: unknown = null;
    let emptyFolderDeletePayload: unknown = null;
    let refreshNoRatingPayload: unknown = null;
    let dedupExecuteBatchPayload: unknown = null;
    let replacePayload: unknown = null;
    let autoAllCalled = false;
    let cleanupSuggestCalls = 0;
    let cleanupTaskStarted = false;
    let cleanupTaskPolls = 0;
    let emptyFolderTaskStarted = false;
    let emptyFolderTaskPolls = 0;
    let dedupBatchTaskStarted = false;
    let dedupBatchTaskPolls = 0;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/libraries') {
        return jsonResponse({
          libraries: [
            { id: 'movie-lib', name: '电影', type: 'movies', paths: ['/strm/电影'] },
            { id: 'show-lib', name: '剧集', type: 'tvshows', paths: ['/strm/剧集'] }
          ]
        });
      }
      if (url.pathname === '/api/v2/cleanup/suggest') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        cleanupPayloads.push(JSON.parse(String(init?.body)));
        cleanupSuggestCalls += 1;
        calls.push(url.pathname);
        return jsonResponse(cleanupSummary);
      }
      if (url.pathname === '/api/v2/cleanup/refresh-no-rating') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        refreshNoRatingPayload = JSON.parse(String(init?.body));
        calls.push(url.pathname);
        return jsonResponse({
          id: 'cdcdcdcd-cdcd-4cdc-8cdc-cdcdcdcdcdcd',
          kind: 'cleanup_refresh_no_rating',
          label: '刷新无评分',
          status: 'pending',
          progress: 0,
          total: 1,
          source: 'api',
          params: {},
          status_text: '排队中',
          result: null,
          error: null,
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:00Z'
        });
      }
      if (url.pathname === '/api/v2/cleanup/empty-dirs') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        const payload = JSON.parse(String(init?.body));
        emptyPayloads.push(payload);
        calls.push(url.pathname);
        if (payload.execute) {
          cleanupTaskStarted = true;
          return jsonResponse({
            ok: true,
            dry_run: false,
            execute: true,
            root: '/volume1/strm',
            candidate_count: 1,
            samples: ['电影/空目录'],
            truncated: false,
            warnings: [],
            task: {
              id: 'abababab-abab-4aba-8aba-abababababab',
              kind: 'cleanup_empty_strm_dirs',
              label: '清理空 STRM 目录',
              status: 'pending',
              progress: 0,
              total: 1,
              source: 'manual',
              params: { execute: true },
              status_text: '排队中',
              result: null,
              error: null,
              cancel_requested: false,
              queued_at: '2026-06-28T00:00:00Z',
              started_at: null,
              ended_at: null,
              updated_at: '2026-06-28T00:00:00Z'
            }
          });
        }
        return jsonResponse({
          ok: true,
          dry_run: true,
          execute: false,
          root: '/volume1/strm',
          candidate_count: 1,
          samples: ['电影/空目录'],
          truncated: false,
          warnings: [],
          task: null
        });
      }
      if (url.pathname === '/api/v2/cleanup/empty-folders') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        emptyFolderPayload = JSON.parse(String(init?.body));
        emptyFolderTaskStarted = true;
        calls.push(url.pathname);
        return jsonResponse({
          id: 'efefefef-efef-4efe-8efe-efefefefefef',
          kind: 'cleanup_empty_folders',
          label: '扫描空 115 folder: 电影',
          status: 'pending',
          progress: 0,
          total: 1,
          source: 'manual',
          params: { lib: '电影' },
          status_text: '排队中',
          result: null,
          error: null,
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:00Z'
        });
      }
      if (url.pathname === '/api/v2/tasks') {
        if (!cleanupTaskStarted && !emptyFolderTaskStarted && !dedupBatchTaskStarted) {
          return jsonResponse({ active_count: 0, tasks: [] });
        }
        const tasks = [];
        if (cleanupTaskStarted) {
          cleanupTaskPolls += 1;
          const done = cleanupTaskPolls >= 2;
          tasks.push({
            id: 'abababab-abab-4aba-8aba-abababababab',
            kind: 'cleanup_empty_strm_dirs',
            label: '清理空 STRM 目录',
            status: done ? 'done' : 'running',
            progress: done ? 1 : 0,
            total: 1,
            source: 'manual',
            params: { execute: true },
            status_text: done ? '完成' : '清理中',
            result: done ? { ok: true, deleted: 1 } : null,
            error: null,
            cancel_requested: false,
            queued_at: '2026-06-28T00:00:00Z',
            started_at: '2026-06-28T00:00:01Z',
            ended_at: done ? '2026-06-28T00:00:02Z' : null,
            updated_at: done ? '2026-06-28T00:00:02Z' : '2026-06-28T00:00:01Z'
          });
        }
        if (emptyFolderTaskStarted) {
          emptyFolderTaskPolls += 1;
          const done = emptyFolderTaskPolls >= 2;
          tasks.push({
            id: 'efefefef-efef-4efe-8efe-efefefefefef',
            kind: 'cleanup_empty_folders',
            label: '扫描空 115 folder: 电影',
            status: done ? 'done' : 'running',
            progress: done ? 3 : 1,
            total: 3,
            source: 'manual',
            params: { lib: '电影' },
            status_text: done ? '115 空 folder 扫描完成' : '扫描 115 空 folder',
            result: done ? {
              ok: true,
              lib: '电影',
              folder: '电影',
              root: '/volume1/docker/clouddrive2/CloudNAS/CloudDrive/电影',
              items: [{ folder: '空壳电影 [tmdb-300]', other_files: 1, size_bytes: 1024, size_kb: 1 }],
              total_scanned: 3,
              total_size_kb: 1,
              truncated: false,
              warnings: []
            } : null,
            error: null,
            cancel_requested: false,
            queued_at: '2026-06-28T00:00:00Z',
            started_at: '2026-06-28T00:00:01Z',
            ended_at: done ? '2026-06-28T00:00:02Z' : null,
            updated_at: done ? '2026-06-28T00:00:02Z' : '2026-06-28T00:00:01Z'
          });
        }
        if (dedupBatchTaskStarted) {
          dedupBatchTaskPolls += 1;
          const done = dedupBatchTaskPolls >= 2;
          tasks.push({
            id: 'bcbcbcbc-bcbc-4bcb-8bcb-bcbcbcbcbcbc',
            kind: 'dedup_exec_batch',
            label: '批量去重: 1 组',
            status: done ? 'done' : 'running',
            progress: done ? 1 : 0,
            total: 1,
            source: 'api',
            params: dedupExecuteBatchPayload || {},
            status_text: done ? '批量去重完成: 1/1' : '去重 tmdb 200',
            result: done ? {
              results: [{ tmdb: '200', ok: true, removed: 1, err: null }],
              ok_count: 1,
              total: 1
            } : null,
            error: null,
            cancel_requested: false,
            queued_at: '2026-06-28T00:00:00Z',
            started_at: '2026-06-28T00:00:01Z',
            ended_at: done ? '2026-06-28T00:00:02Z' : null,
            updated_at: done ? '2026-06-28T00:00:02Z' : '2026-06-28T00:00:01Z'
          });
        }
        return jsonResponse({
          active_count: tasks.filter((task) => ['pending', 'running'].includes(task.status)).length,
          tasks
        });
      }
      if (url.pathname === '/api/v2/manage/delete/batch/execute') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        const payload = JSON.parse(String(init?.body));
        if (String(payload.reason || '').startsWith('115 empty-folders')) {
          emptyFolderDeletePayload = payload;
        } else {
          cleanupBatchPayload = payload;
        }
        calls.push(url.pathname);
        return jsonResponse({
          id: 'dededede-dede-4ded-8ded-dededededede',
          kind: 'manage_delete_batch_execute',
          label: '智能清理删除: 1 项',
          status: 'pending',
          progress: 0,
          total: 1,
          source: 'api',
          params: {},
          status_text: '排队中',
          result: null,
          error: null,
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:00Z'
        });
      }
      if (url.pathname === '/api/v2/dedup/duplicates') {
        calls.push(url.pathname);
        return jsonResponse({
          dups: [{
            tmdb: '100',
            keep: { lib: '剧集', folder: '示例剧 [tmdb-100]', score: 10, n: 12 },
            remove: [{ lib: '剧集', folder: '示例剧 重复 [tmdb-100]', score: 4, n: 12 }]
          }],
          review: [{
            tmdb: '200',
            why: '分数接近，需要人工确认',
            rows: [
              { lib: '剧集', folder: '复核剧 A [tmdb-200]', score: 8, n: 10 },
              { lib: '剧集', folder: '复核剧 B [tmdb-200]', score: 7, n: 10 }
            ]
          }]
        });
      }
      if (url.pathname === '/api/v2/dedup/auto-all') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        autoAllCalled = true;
        calls.push(url.pathname);
        return jsonResponse({
          async_requested: false,
          total: 1,
          ok_count: 1,
          review_count: 1,
          total_removed_folders: 1,
          results: [{ tmdb: '100', ok: true, kept: '示例剧 [tmdb-100]', removed: 1, err: null }]
        });
      }
      if (url.pathname === '/api/v2/dedup/execute-batch') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        dedupExecuteBatchPayload = JSON.parse(String(init?.body));
        dedupBatchTaskStarted = true;
        calls.push(url.pathname);
        return jsonResponse({
          id: 'bcbcbcbc-bcbc-4bcb-8bcb-bcbcbcbcbcbc',
          kind: 'dedup_exec_batch',
          label: '批量去重: 1 组',
          status: 'pending',
          progress: 0,
          total: 1,
          source: 'api',
          params: dedupExecuteBatchPayload,
          status_text: '排队中',
          result: null,
          error: null,
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:00Z'
        });
      }
      if (url.pathname === '/api/v2/dedup/replace') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        replacePayload = JSON.parse(String(init?.body));
        calls.push(url.pathname);
        return jsonResponse({
          ok: true,
          lib: '剧集',
          kept_as: '复核剧 A [tmdb-200]',
          dropped: '复核剧 B [tmdb-200]',
          renamed: true,
          deleted_from: ['/volume1/strm/剧集/复核剧 B [tmdb-200]'],
          emby_updates: [{ Path: '/strm/剧集/复核剧 A [tmdb-200]', UpdateType: 'Modified' }],
          notified: true,
          undo_id: '55555555-5555-4555-8555-555555555555',
          msg: '替换完成'
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '智能清理' }));
    expect(await screen.findByText('智能清理预检')).toBeInTheDocument();
    expect(screen.getByText('存在失败任务')).toBeInTheDocument();
    expect(screen.getByText(/size \/ idle 仍受挂载状态、播放记录和媒体元数据完整度影响/)).toBeInTheDocument();
    expect(await screen.findByText('旧电影')).toBeInTheDocument();
    expect(screen.getByText(/低评分；长期未播放/)).toBeInTheDocument();
    expect(screen.getByText(/size: 17.5 · 48 GB · 挂载统计可能滞后/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('智能清理媒体库'), { target: { value: '电影' } });
    fireEvent.change(screen.getByLabelText('智能清理 top'), { target: { value: '25' } });
    fireEvent.change(screen.getByLabelText('智能清理 min_score'), { target: { value: '70' } });
    fireEvent.click(screen.getByLabelText('元数据'));
    fireEvent.click(screen.getByRole('button', { name: '生成建议' }));
    await waitFor(() => expect(cleanupPayloads).toContainEqual({
      lib: '电影',
      top: 25,
      min_score: 70,
      dimensions: ['rating', 'idle', 'size']
    }));
    fireEvent.click(screen.getByLabelText('选择清理候选：旧电影'));
    fireEvent.click(screen.getByRole('button', { name: /删除选中 1/ }));
    expect(await screen.findByText('确认删除智能清理候选')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认删除选中' }));
    await waitFor(() => expect(cleanupBatchPayload).toEqual({
      items: [{
        lib: '电影',
        folder: '旧电影 [tmdb-100]',
        item_id: 'movie-old',
        reason: '智能清理 score 82.5'
      }],
      reason: '智能清理 min_score 70'
    }));
    expect(await screen.findByText(/已创建任务：智能清理删除: 1 项/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '刷新无评分' }));
    await waitFor(() => expect(refreshNoRatingPayload).toEqual({ lib: '电影' }));
    expect(await screen.findByText(/无评分刷新任务：刷新无评分 · pending/)).toBeInTheDocument();
    expect(await screen.findByText('可清理')).toBeInTheDocument();
    expect(screen.getByText('电影/空目录')).toBeInTheDocument();
    expect(screen.getByText('电影/poster.jpg')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('空目录清理 lib'), { target: { value: '电影' } });
    fireEvent.click(screen.getByRole('button', { name: '清理空 STRM 目录' }));
    expect(await screen.findByText('确认清理空 STRM 目录')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认清理' }));
    await waitFor(() => expect(emptyPayloads).toEqual([
      { execute: false, lib: null },
      { execute: false, lib: null },
      { execute: true, lib: '电影' }
    ]));
    expect(await screen.findByText(/已创建任务：清理空 STRM 目录/)).toBeInTheDocument();
    expect(screen.getByText('115 empty-folders')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('115 empty-folders lib'), { target: { value: '电影' } });
    fireEvent.click(screen.getByRole('button', { name: '扫描 115 空文件夹' }));
    await waitFor(() => expect(emptyFolderPayload).toEqual({ lib: '电影' }));
    expect((await screen.findAllByText(/扫描任务：扫描空 115 folder: 电影/)).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole('button', { name: '任务中心' }));
    await waitFor(() => expect(within(getTaskCenterDrawer()).getByText('清理空 STRM 目录')).toBeInTheDocument());
    expect(within(getTaskCenterDrawer()).getByText('扫描空 115 folder: 电影')).toBeInTheDocument();
    clickTaskCenterRefresh();
    await waitFor(() => expect(cleanupSuggestCalls).toBeGreaterThanOrEqual(2));
    await waitFor(() => expect(emptyPayloads).toContainEqual({ execute: false, lib: '电影' }));
    expect(await screen.findByText('空壳电影 [tmdb-300]')).toBeInTheDocument();
    fireEvent.click(within(getTaskCenterDrawer()).getByRole('button', { name: '关闭' }));
    const emptyFolderBlock = screen.getByRole('heading', { name: '115 empty-folders' }).closest('section');
    expect(emptyFolderBlock).not.toBeNull();
    fireEvent.click(within(emptyFolderBlock as HTMLElement).getByLabelText('选择 115 空文件夹：空壳电影 [tmdb-300]'));
    fireEvent.click(within(emptyFolderBlock as HTMLElement).getByRole('button', { name: /删除选中 1/ }));
    expect(await screen.findByText('确认删除 115 空文件夹候选')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认删除选中' }));
    await waitFor(() => expect(emptyFolderDeletePayload).toEqual({
      items: [{
        lib: '电影',
        folder: '空壳电影 [tmdb-300]',
        item_id: null,
        reason: '115 empty-folders 扫描候选'
      }],
      reason: '115 empty-folders 扫描 电影'
    }));

    fireEvent.click(screen.getByRole('button', { name: '去重' }));
    expect(await screen.findByText('去重闭环')).toBeInTheDocument();
    expect(screen.getAllByText(/示例剧 重复 \[tmdb-100\]/).length).toBeGreaterThan(0);
    expect(screen.getByText('分数接近，需要人工确认')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '智能选择' }));
    expect(screen.getByRole('button', { name: /删除 2/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '清空' }));
    fireEvent.click(screen.getByLabelText('选择去重删除：剧集/复核剧 B [tmdb-200]'));
    fireEvent.click(screen.getByRole('button', { name: /删除 1/ }));
    expect(await screen.findByText('确认人工去重删除')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认删除选中重复目录' }));
    await waitFor(() => expect(dedupExecuteBatchPayload).toEqual({
      groups: [{
        tmdb: '200',
        remove: [{ lib: '剧集', folder: '复核剧 B [tmdb-200]', item_id: null }]
      }]
    }));
    expect(await screen.findByText('批量去重任务')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '任务中心' }));
    await waitFor(() => expect(within(getTaskCenterDrawer()).getByText('批量去重: 1 组')).toBeInTheDocument());
    clickTaskCenterRefresh();
    expect((await screen.findAllByText('批量去重完成: 1/1')).length).toBeGreaterThan(0);
    expect(screen.getByText(/removed 1/)).toBeInTheDocument();
    fireEvent.click(within(getTaskCenterDrawer()).getByRole('button', { name: '关闭' }));
    fireEvent.click(screen.getByRole('button', { name: 'auto-all' }));
    expect(await screen.findByText('确认自动去重')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认执行 auto-all' }));
    await waitFor(() => expect(autoAllCalled).toBe(true));
    expect(await screen.findByText('Auto-all 结果')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('替换 lib'), { target: { value: '剧集' } });
    fireEvent.change(screen.getByLabelText('替换 win_folder'), { target: { value: '复核剧 A [tmdb-200]' } });
    fireEvent.change(screen.getByLabelText('替换 lose_folder'), { target: { value: '复核剧 B [tmdb-200]' } });
    fireEvent.change(screen.getByLabelText('替换原因'), { target: { value: '人工复核' } });
    fireEvent.click(screen.getByRole('button', { name: 'replace' }));
    expect(await screen.findByText('确认替换重复目录')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认 replace' }));
    await waitFor(() => expect(replacePayload).toEqual({
      lib: '剧集',
      win_folder: '复核剧 A [tmdb-200]',
      lose_folder: '复核剧 B [tmdb-200]',
      reason: '人工复核'
    }));
    expect(await screen.findByText('Replace 结果')).toBeInTheDocument();
    await waitFor(() => expect(calls).toEqual(expect.arrayContaining([
      '/api/v2/cleanup/suggest',
      '/api/v2/cleanup/empty-dirs',
      '/api/v2/cleanup/empty-folders',
      '/api/v2/cleanup/refresh-no-rating',
      '/api/v2/manage/delete/batch/execute',
      '/api/v2/dedup/duplicates',
      '/api/v2/dedup/execute-batch',
      '/api/v2/dedup/auto-all',
      '/api/v2/dedup/replace'
    ])));
  });

  it('smart-selects Emby ProviderIds review duplicate rows with item ids', async () => {
    let dedupExecuteBatchPayload: unknown = null;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/dedup/duplicates') {
        return jsonResponse({
          dups: [],
          review: [{
            tmdb: '661029',
            why: 'Emby ProviderIds.Tmdb 相同，媒体库内仍有重复 Item；可勾选旧目录/副本删除',
            rows: [
              { lib: '合集', folder: '精灵宝可梦（系列）', score: 0, n: 0, item_id: '53139' },
              { lib: '合集', folder: '精灵宝可梦：XY', score: 0, n: 0, item_id: '53148' }
            ]
          }]
        });
      }
      if (url.pathname === '/api/v2/dedup/execute-batch') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        dedupExecuteBatchPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: 'ded66102-6610-4661-8661-000000000029',
          kind: 'dedup_exec_batch',
          label: '批量去重: 1 组',
          status: 'pending',
          progress: 0,
          total: 1,
          source: 'api',
          params: dedupExecuteBatchPayload,
          status_text: '排队中',
          result: null,
          error: null,
          cancel_requested: false,
          queued_at: '2026-06-30T00:00:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-30T00:00:00Z'
        });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '去重' }));
    const group = (await screen.findByText(/媒体库内仍有重复 Item/)).closest('article');
    expect(group).not.toBeNull();
    expect(within(group as HTMLElement).getByText('精灵宝可梦：XY')).toBeInTheDocument();
    fireEvent.click(within(group as HTMLElement).getByRole('button', { name: /智能选本组/ }));
    expect(screen.getByRole('button', { name: /删除 1/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /删除 1/ }));
    expect(await screen.findByText('确认人工去重删除')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认删除选中重复目录' }));

    await waitFor(() => expect(dedupExecuteBatchPayload).toEqual({
      groups: [{
        tmdb: '661029',
        remove: [{ lib: '合集', folder: '精灵宝可梦：XY', item_id: '53148' }]
      }]
    }));
  });

  it('creates delete and move tasks from the manage panel with csrf', async () => {
    let previewPayload: unknown = null;
    let executePayload: unknown = null;
    let batchPayload: unknown = null;
    let movePreviewPayload: unknown = null;
    let moveExecutePayload: unknown = null;
    let batchMovePayload: unknown = null;
    let undoCalls = 0;
    let batchTaskStarted = false;
    let batchTaskPolls = 0;
    const batchMoveTask = taskRun(
      'a1a1a1a1-a1a1-4a1a-8a1a-a1a1a1a1a1a1',
      'manage_move_batch_execute',
      '批量移动: 电影 -> 电视剧 (2 项)',
      'pending'
    );
    batchMoveTask.total = 2;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/libraries') {
        return jsonResponse({
          libraries: [
            { id: 'movie-lib', name: '电影', type: 'movies', paths: ['/strm/电影'] },
            { id: 'show-lib', name: '电视剧', type: 'tvshows', paths: ['/strm/电视剧'] }
          ]
        });
      }
      if (url.pathname === '/api/v2/libraries/items') {
        const lib = url.searchParams.get('lib') || '电影';
        return jsonResponse({
          lib,
          item_types: 'Movie',
          total_record_count: 1,
          truncated: false,
          items: [{
            id: 'item-browser',
            name: '浏览电影',
            folder: '浏览电影 [tmdbid-777]',
            tmdb: '777',
            year: 2026,
            path: '/strm/电影/浏览电影 [tmdbid-777]'
          }]
        });
      }
      if (url.pathname === '/api/v2/manage/undo') {
        undoCalls += 1;
        return jsonResponse({
          total: undoCalls >= 2 ? 2 : 1,
          items: [
            ...(undoCalls >= 2 ? [{
              id: 'abababab-abab-4aba-8aba-abababababab',
              legacy_id: 'legacy-2',
              op: 'delete',
              payload: { lib: '电影', folder: '批量旧电影' },
              undone: false,
              created_at: '2026-06-28T00:02:00Z'
            }] : []),
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
      if (url.pathname === '/api/v2/tasks' && batchTaskStarted) {
        batchTaskPolls += 1;
        const done = batchTaskPolls >= 2;
        return jsonResponse({
          active_count: done ? 0 : 1,
          tasks: [{
            id: 'babababa-baba-4bab-8bab-babababababa',
            kind: 'manage_delete_batch_execute',
            label: '批量删除: 2 项',
            status: done ? 'done' : 'running',
            progress: done ? 2 : 1,
            total: 2,
            status_text: done ? '完成' : '删除中',
            cancel_requested: false,
            queued_at: '2026-06-28T00:02:00Z',
            started_at: '2026-06-28T00:02:01Z',
            ended_at: done ? '2026-06-28T00:02:03Z' : null,
            updated_at: done ? '2026-06-28T00:02:03Z' : '2026-06-28T00:02:02Z',
            params: { reason: '批量重复' },
            result: done ? { ok: true, total: 2, ok_count: 2, error_count: 0, results: [] } : null,
            source: 'api'
          }]
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
      if (url.pathname === '/api/v2/manage/delete/batch/execute') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        batchPayload = JSON.parse(String(init?.body));
        batchTaskStarted = true;
        return jsonResponse({
          id: 'babababa-baba-4bab-8bab-babababababa',
          kind: 'manage_delete_batch_execute',
          label: '批量删除: 2 项',
          status: 'pending',
          progress: 0,
          total: 2,
          status_text: '排队中',
          cancel_requested: false,
          queued_at: '2026-06-28T00:02:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:02:00Z',
          params: {},
          result: null,
          source: 'api'
        });
      }
      if (url.pathname === '/api/v2/manage/move') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        movePreviewPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: 'ffffffff-ffff-4fff-8fff-ffffffffffff',
          kind: 'manage_move_preview',
          label: '移动预览: 电影/旧电影 -> 电视剧',
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
      if (url.pathname === '/api/v2/manage/move/execute') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        moveExecutePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '99999999-9999-4999-8999-999999999999',
          kind: 'manage_move_execute',
          label: '移动: 电影/旧电影 -> 电视剧/归档电影',
          status: 'pending',
          progress: 0,
          total: 5,
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
      if (url.pathname === '/api/v2/manage/move/batch/execute') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        batchMovePayload = JSON.parse(String(init?.body));
        return jsonResponse(batchMoveTask);
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '删除·移动' }));
    expect(await screen.findByText(/先 Emby DELETE，再动磁盘/)).toBeInTheDocument();
    expect(screen.getByText('legacy-1')).toBeInTheDocument();
    expect(await screen.findByText(/浏览电影/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('项目关键词'), { target: { value: '777' } });
    fireEvent.change(screen.getByLabelText('库项目列表'), { target: { value: '0' } });
    const browserLibValue = (screen.getByLabelText('浏览库名') as HTMLSelectElement).value;
    expect(screen.getByLabelText('删除库名')).toHaveValue(browserLibValue);
    expect(screen.getByLabelText('删除 folder')).toHaveValue('浏览电影 [tmdbid-777]');
    expect(screen.getByLabelText('删除 ItemId')).toHaveValue('item-browser');
    fireEvent.click(screen.getByRole('button', { name: '加入批量删除文本' }));
    expect(screen.getByLabelText('批量删除内容')).toHaveValue(`${browserLibValue}/浏览电影 [tmdbid-777]/item-browser`);

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

    fireEvent.change(screen.getByLabelText('批量删除内容'), { target: { value: '电影/批量旧电影/item-batch\n电视剧/批量旧剧' } });
    fireEvent.change(screen.getByLabelText('批量删除原因'), { target: { value: '批量重复' } });
    fireEvent.click(screen.getByRole('button', { name: '检查并确认批量删除' }));
    expect(await screen.findByText('确认批量真实删除')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认批量删除' }));

    await waitFor(() => expect(batchPayload).toEqual({
      items: [
        { lib: '电影', folder: '批量旧电影', item_id: 'item-batch', reason: null },
        { lib: '电视剧', folder: '批量旧剧', item_id: null, reason: null }
      ],
      reason: '批量重复'
    }));
    expect(await screen.findByText('批量删除: 2 项')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '任务中心' }));
    await waitFor(() => expect(within(getTaskCenterDrawer()).getByText('批量删除: 2 项')).toBeInTheDocument());
    clickTaskCenterRefresh();
    await waitFor(() => expect(undoCalls).toBeGreaterThanOrEqual(2));
    expect(await screen.findByText('legacy-2')).toBeInTheDocument();
    fireEvent.click(within(getTaskCenterDrawer()).getByRole('button', { name: '关闭' }));

    fireEvent.change(screen.getByLabelText('来源库名'), { target: { value: '电影' } });
    fireEvent.change(screen.getByLabelText('来源 folder'), { target: { value: '旧电影' } });
    fireEvent.change(screen.getByLabelText('目标库名'), { target: { value: '电视剧' } });
    fireEvent.change(screen.getByLabelText('目标 folder'), { target: { value: '归档电影' } });
    fireEvent.change(screen.getByLabelText('移动 ItemId'), { target: { value: 'item-move' } });
    fireEvent.change(screen.getByLabelText('移动原因'), { target: { value: '归档' } });
    fireEvent.click(screen.getByRole('button', { name: '生成移动预览任务' }));

    await waitFor(() => expect(movePreviewPayload).toEqual({
      from_lib: '电影',
      from_folder: '旧电影',
      to_lib: '电视剧',
      to_folder: '归档电影',
      item_id: 'item-move',
      reason: '归档'
    }));
    expect(await screen.findByText('移动预览: 电影/旧电影 -> 电视剧')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '真实移动' }));
    expect(await screen.findByText('确认真实移动')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认移动' }));

    await waitFor(() => expect(moveExecutePayload).toEqual({
      from_lib: '电影',
      from_folder: '旧电影',
      to_lib: '电视剧',
      to_folder: '归档电影',
      item_id: 'item-move',
      reason: '归档'
    }));
    expect(await screen.findByText('移动: 电影/旧电影 -> 电视剧/归档电影')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('批量移动来源库'), { target: { value: '电影' } });
    fireEvent.change(screen.getByLabelText('批量移动目标库'), { target: { value: '电视剧' } });
    fireEvent.change(screen.getByLabelText('批量移动内容'), { target: { value: '批量旧电影 | item-batch-move | 批量归档电影\n批量旧剧' } });
    fireEvent.change(screen.getByLabelText('批量移动冲突处理'), { target: { value: 'smart' } });
    fireEvent.change(screen.getByLabelText('批量移动原因'), { target: { value: '批量归档' } });
    fireEvent.click(screen.getByRole('button', { name: '生成批量移动预览' }));
    expect(await screen.findByText('2 项 · 电影 → 电视剧')).toBeInTheDocument();
    expect(screen.getByText('批量旧电影 → 批量归档电影')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '真实执行批量移动' }));
    expect(await screen.findByText('确认批量真实移动')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认批量移动' }));

    await waitFor(() => expect(batchMovePayload).toEqual({
      from_lib: '电影',
      to_lib: '电视剧',
      items: [
        { folder: '批量旧电影', item_id: 'item-batch-move', to_folder: '批量归档电影' },
        { folder: '批量旧剧', item_id: null, to_folder: null }
      ],
      on_conflict: 'smart',
      reason: '批量归档'
    }));
    expect(await screen.findByText('批量移动: 电影 -> 电视剧 (2 项)')).toBeInTheDocument();
    dispatchTaskDone(batchMoveTask, {
      ok: false,
      from_lib: '电影',
      to_lib: '电视剧',
      total: 2,
      ok_count: 1,
      error_count: 1,
      smart_count: 1,
      results: []
    });
    expect(await screen.findByText('✓ 1 / 2 · 智能 1 · 失败 1')).toBeInTheDocument();
  });

  it('creates, saves, and deletes Emby users from the users tab with csrf', async () => {
    const rootUser = {
      id: 'root',
      name: 'Root',
      admin: true,
      disabled: false,
      last_activity_date: null,
      remote_bitrate_mbps: null,
      policy: {
        RemoteClientBitrateLimit: null,
        SimultaneousStreamLimit: null
      }
    };
    const aliceUser = {
      id: 'user/1',
      name: 'Alice',
      admin: false,
      disabled: false,
      last_activity_date: '2026-06-28T08:00:00Z',
      remote_bitrate_mbps: 25,
      policy: {
        RemoteClientBitrateLimit: 25_000_000,
        SimultaneousStreamLimit: 2
      }
    };
    const bobUser = {
      id: 'user/2',
      name: 'Bob',
      admin: false,
      disabled: false,
      last_activity_date: null,
      remote_bitrate_mbps: null,
      policy: {
        RemoteClientBitrateLimit: null,
        SimultaneousStreamLimit: null
      }
    };

    let users = [rootUser, aliceUser];
    let createdPayload: unknown = null;
    let savedPayload: unknown = null;
    let deletedPath = '';
    let createCsrf = '';
    let deleteCsrf = '';
    mockApi((url, init) => {
      const method = (init?.method || 'GET').toUpperCase();
      if (url.pathname === '/api/v2/users' && method === 'GET') {
        return jsonResponse({ users });
      }
      if (url.pathname === '/api/v2/users' && method === 'POST') {
        const headers = init?.headers as Headers;
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        createCsrf = headers.get('X-CSRF-Token') || '';
        createdPayload = JSON.parse(String(init?.body));
        users = [...users, bobUser];
        return jsonResponse({ ok: true, user: bobUser });
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
            admin: false,
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
      if (url.pathname === '/api/v2/users/user%2F2' && method === 'DELETE') {
        const headers = init?.headers as Headers;
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        deleteCsrf = headers.get('X-CSRF-Token') || '';
        deletedPath = url.pathname;
        users = users.filter((user) => user.id !== 'user/2');
        return jsonResponse({ ok: true, code: 204 });
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '用户' }));
    expect(await screen.findByText('Alice')).toBeInTheDocument();
    expect(screen.getByText('Root')).toBeInTheDocument();
    const rootRow = screen.getByText('Root').closest('tr');
    expect(rootRow).not.toBeNull();
    expect(within(rootRow as HTMLElement).queryByRole('button', { name: '删除 Root' })).toBeNull();

    fireEvent.change(screen.getByLabelText('新用户用户名'), { target: { value: 'Bob' } });
    fireEvent.click(screen.getByRole('button', { name: '新建用户' }));

    await waitFor(() => expect(createdPayload).toEqual({ name: 'Bob', password: null }));
    expect(createCsrf).toBe('csrf-me');
    expect(await screen.findByText('Bob')).toBeInTheDocument();
    expect(await screen.findByText('已创建 Bob，复制用户名给亲友即可')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Alice 远程限速 Mbps'), { target: { value: '12.5' } });
    fireEvent.change(screen.getByLabelText('Alice 同时播放数'), { target: { value: '3' } });
    const aliceRow = screen.getByText('Alice').closest('tr');
    expect(aliceRow).not.toBeNull();
    fireEvent.click(within(aliceRow as HTMLElement).getByRole('checkbox'));
    fireEvent.click(within(aliceRow as HTMLElement).getByRole('button', { name: '保存' }));

    await waitFor(() => expect(savedPayload).toEqual({
      remote_bitrate_mbps: 12.5,
      simultaneous_stream_limit: 3,
      disabled: true
    }));
    expect(await screen.findByText('已保存 Alice 的用户策略')).toBeInTheDocument();

    const bobRow = screen.getByText('Bob').closest('tr');
    expect(bobRow).not.toBeNull();
    fireEvent.click(within(bobRow as HTMLElement).getByRole('button', { name: '删除 Bob' }));
    expect(await screen.findByText('删除「Bob」后不可恢复。')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '确认删除 Bob' }));

    await waitFor(() => expect(deletedPath).toBe('/api/v2/users/user%2F2'));
    expect(deleteCsrf).toBe('csrf-me');
    await waitFor(() => expect(screen.queryByText('Bob')).not.toBeInTheDocument());
    expect(await screen.findByText('已删除 Bob')).toBeInTheDocument();
  });

  it('searches catalog and creates one-dragon add-new tasks with csrf', async () => {
    const planPayloads: unknown[] = [];
    const wizardPayloads: unknown[] = [];
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
        const item = payload.item;
        const isOffline = item.link_type === 'magnet' || item.link_type === 'ed2k';
        const action = isOffline ? 'offline_download' : item.link_type === 'share115' ? 'save_share' : 'unsupported';
        return jsonResponse({
          ok: action !== 'unsupported',
          transfer: action !== 'unsupported',
          action,
          link_type: item.link_type,
          is_pkg: Boolean(item.is_pkg),
          label: item.name,
          target: payload.cid ? { cid: payload.cid } : { lib: payload.lib },
          save: action === 'save_share'
            ? {
                endpoint: '/api/v2/c115/save',
                method: 'POST',
                share: item.share,
                receive_code: item.rc,
                payload: { url: item.link, pwd: item.rc, lib: payload.lib, cid: payload.cid, label: item.name }
              }
            : null,
          offline: action === 'offline_download'
            ? {
                endpoint: '/api/v2/c115/offline',
                method: 'POST',
                protocol: item.link_type,
                payload: { url: item.link, lib: payload.lib, cid: payload.cid, label: item.name }
              }
            : null,
          unsupported: action === 'unsupported' ? { link: item.link, reason: 'unsupported type' } : null
        });
      }
      if (url.pathname === '/api/v2/wizard/add-new') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        const payload = JSON.parse(String(init?.body));
        wizardPayloads.push(payload);
        return jsonResponse({
          id: wizardPayloads.length === 1
            ? '44444444-4444-4444-8444-444444444444'
            : '55555555-5555-4555-8555-555555555555',
          kind: 'add_new',
          label: wizardPayloads.length === 1 ? '一条龙加新资源: 1 项 -> 库「电影」' : '一条龙加新资源: 2 项 -> 库「电影」',
          status: 'pending',
          progress: 0,
          total: payload.items.length + 3,
          status_text: '排队中',
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:00Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:00Z',
          params: payload,
          result: null,
          source: 'api'
        });
      }
      if (['/api/v2/c115/save', '/api/v2/c115/offline'].includes(url.pathname)) {
        throw new Error(`legacy catalog transfer endpoint called: ${url.pathname}`);
      }
      return undefined;
    });

    render(<App />);

    fireEvent.click(await screen.findByRole('button', { name: '找资源' }));
    fireEvent.change(screen.getByLabelText('资源数据源'), { target: { value: 'local' } });
    expect(await screen.findByText('库内 260,000 条 · 整包 1,200')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('资源关键词'), { target: { value: 'movie' } });
    fireEvent.click(screen.getByRole('button', { name: '搜索' }));

    expect(await screen.findByText('The Movie')).toBeInTheDocument();
    expect(screen.getByText('The Magnet')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '转存' }));
    await waitFor(() => expect(planPayloads).toHaveLength(1));
    expect(planPayloads[0]).toEqual({
      item: {
        name: 'The Movie',
        sheet: '电影',
        link: 'https://115.com/s/abc?password=xy12',
        link_type: 'share115',
        is_pkg: false,
        share: 'abc',
        rc: 'xy12'
      },
      lib: '电影'
    });
    fireEvent.click(screen.getByRole('button', { name: '创建一条龙任务' }));

    await waitFor(() => expect(wizardPayloads).toHaveLength(1));
    expect(wizardPayloads[0]).toEqual({
      target: { lib: '电影' },
      delay_ms: 500,
      items: [{
        url: 'https://115.com/s/abc?password=xy12',
        kind: 'share115',
        pwd: 'xy12',
        label: 'The Movie'
      }]
    });
    expect(await screen.findByText('一条龙任务已交给任务中心，会继续生成 STRM、扫库、修海报和检查重复：一条龙加新资源: 1 项 -> 库「电影」')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '全选' }));
    fireEvent.click(screen.getByRole('button', { name: '转存选中' }));
    await waitFor(() => expect(planPayloads).toHaveLength(3));
    expect(planPayloads.slice(1)).toEqual([
      {
        item: {
          name: 'The Movie',
          sheet: '电影',
          link: 'https://115.com/s/abc?password=xy12',
          link_type: 'share115',
          is_pkg: false,
          share: 'abc',
          rc: 'xy12'
        },
        lib: '电影'
      },
      {
        item: {
          name: 'The Magnet',
          sheet: '电影',
          link: 'magnet:?xt=urn:btih:123',
          link_type: 'magnet',
          is_pkg: false,
          share: null,
          rc: null
        },
        lib: '电影'
      }
    ]);
    fireEvent.click(screen.getByRole('button', { name: '创建一条龙任务' }));

    await waitFor(() => expect(wizardPayloads).toHaveLength(2));
    expect(wizardPayloads[1]).toEqual({
      target: { lib: '电影' },
      delay_ms: 500,
      items: [
        {
          url: 'https://115.com/s/abc?password=xy12',
          kind: 'share115',
          pwd: 'xy12',
          label: 'The Movie'
        },
        {
          url: 'magnet:?xt=urn:btih:123',
          kind: 'offline_download',
          label: 'The Magnet'
        }
      ]
    });
    expect(await screen.findByText('一条龙任务已交给任务中心，会继续生成 STRM、扫库、修海报和检查重复：一条龙加新资源: 2 项 -> 库「电影」')).toBeInTheDocument();
  });

  it('shows remote catalog context and smart-selects recommended 115 resources', async () => {
    const planPayloads: unknown[] = [];
    const inspectPayloads: unknown[] = [];
    let wizardPayload: unknown = null;
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/catalog/stats') {
        return jsonResponse({ available: true, total: 260000, packages: 1200 });
      }
      if (url.pathname === '/api/v2/config') {
        return jsonResponse({ settings: { c115_cid_map: { 电视剧: '67890' } } });
      }
      if (url.pathname === '/api/v2/catalog/remote-search') {
        expect(url.searchParams.get('q')).toBe('莫离');
        expect(url.searchParams.get('limit')).toBe('80');
        return jsonResponse({
          total: 2,
          limit: 80,
          offset: 0,
          has_more: false,
          query: '莫离',
          exact: false,
          sort: 'resource',
          truncated: false,
          disk_types: [{ disk_type: '115', count: 2 }],
          context: {
            ok: true,
            query: '莫离',
            total_matches: 1,
            truncated: false,
            warnings: [],
            summary: {
              matched: true,
              duplicate: false,
              duplicate_groups: 0,
              libraries: ['电视剧'],
              tmdb_ids: ['123456'],
              years: [2026],
              episode_ranges: ['S01E01-E08'],
              missing_ranges: ['S01E09-E40'],
              max_episode: 8,
              total_episodes: 8,
              note: '库内已有条目但存在缺集，可优先选补缺或全集资源'
            },
            items: [{
              id: 'series-1',
              name: '莫离',
              item_type: 'Series',
              library: '电视剧',
              folder: '莫离',
              path: '/strm/电视剧/莫离',
              year: 2026,
              tmdb: '123456',
              has_primary_image: true,
              duplicate: false,
              episode_count: 8,
              episode_ranges: ['S01E01-E08'],
              missing_ranges: ['S01E09-E40'],
              max_episode: 8
            }]
          },
          items: [
            {
              name: '莫离 S01E01-E40 2160p',
              sheet: 'TG Resource API',
              link: 'https://115cdn.com/s/swfull?password=8888',
              is_pkg: true,
              link_type: 'share115',
              transfer: true,
              share: 'swfull',
              rc: '8888',
              recommendation: {
                score: 205,
                level: 'best',
                action: '推荐转存',
                reasons: ['115 可直接转存', '资源到 E40，本地到 E8，适合补缺'],
                episode_ranges: ['S01E01-E40'],
                covers_missing: true,
                duplicate_risk: false,
                already_have: false
              }
            },
            {
              name: '莫离 S01E05 2160p',
              sheet: 'TG Resource API',
              link: 'https://115cdn.com/s/swsingle?password=8888',
              is_pkg: false,
              link_type: 'share115',
              transfer: true,
              share: 'swsingle',
              rc: '8888',
              recommendation: {
                score: 0,
                level: 'skip',
                action: '可能已存在',
                reasons: ['疑似单集 E5，本地大概率已有'],
                episode_ranges: ['S01E05'],
                covers_missing: false,
                duplicate_risk: false,
                already_have: true
              }
            }
          ]
        });
      }
      if (url.pathname === '/api/v2/smart-actions/inspect') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        inspectPayloads.push(JSON.parse(String(init?.body)));
        return jsonResponse({
          ok: true,
          warnings: [],
          actions: [{
            ...smartAction,
            id: 'eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee',
            title: '莫离资源候选可一条龙更新',
            summary: '远端搜索结果已结合本库缺集，建议一条龙转存并刷新媒体库。'
          }]
        });
      }
      if (url.pathname === '/api/v2/catalog/transfer-plan') {
        const headers = init?.headers as Headers;
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        const payload = JSON.parse(String(init?.body));
        planPayloads.push(payload);
        return jsonResponse({
          ok: true,
          transfer: true,
          action: 'save_share',
          link_type: 'share115',
          is_pkg: Boolean(payload.item.is_pkg),
          label: payload.item.name,
          target: { lib: payload.lib },
          save: {
            endpoint: '/api/v2/c115/save',
            method: 'POST',
            share: payload.item.share,
            receive_code: payload.item.rc,
            payload: { url: payload.item.link, pwd: payload.item.rc, lib: payload.lib, label: payload.item.name }
          }
        });
      }
      if (url.pathname === '/api/v2/wizard/add-new') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        wizardPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
          kind: 'add_new',
          label: '一条龙加新资源: 1 项 -> 库「电视剧」',
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

    fireEvent.click(await screen.findByRole('button', { name: '找资源' }));
    fireEvent.change(screen.getByLabelText('资源关键词'), { target: { value: '莫离' } });
    fireEvent.click(screen.getByRole('button', { name: '搜索' }));

    expect(await screen.findByText('库内已有条目但存在缺集，可优先选补缺或全集资源')).toBeInTheDocument();
    expect(screen.getByText('莫离 S01E01-E40 2160p')).toBeInTheDocument();
    expect(screen.getByText('推荐转存')).toBeInTheDocument();
    expect(screen.getByText('可能已存在')).toBeInTheDocument();
    expect(await screen.findByText('莫离资源候选可一条龙更新')).toBeInTheDocument();
    await waitFor(() => expect(inspectPayloads).toHaveLength(1));
    expect(inspectPayloads[0]).toMatchObject({
      q: '莫离',
      limit: 4,
      catalog_context: {
        query: '莫离',
        summary: {
          missing_ranges: ['S01E09-E40']
        }
      }
    });
    expect((inspectPayloads[0] as { catalog_items?: Array<{ name: string }> }).catalog_items?.map((item) => item.name)).toEqual([
      '莫离 S01E01-E40 2160p',
      '莫离 S01E05 2160p'
    ]);

    fireEvent.click(screen.getByRole('button', { name: '智能选择 1' }));
    fireEvent.click(screen.getByRole('button', { name: '转存选中' }));

    await waitFor(() => expect(planPayloads).toHaveLength(1));
    expect(planPayloads[0]).toEqual({
      item: {
        name: '莫离 S01E01-E40 2160p',
        sheet: 'TG Resource API',
        link: 'https://115cdn.com/s/swfull?password=8888',
        link_type: 'share115',
        is_pkg: true,
        share: 'swfull',
        rc: '8888'
      },
      lib: '电视剧'
    });
    expect(await screen.findByText('本库情况：库内已有条目但存在缺集，可优先选补缺或全集资源')).toBeInTheDocument();
    expect(screen.getByText('将使用一条龙加新')).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('含整包合集，输入“整包”确认'), { target: { value: '整包' } });
    fireEvent.click(screen.getByRole('button', { name: '创建一条龙任务' }));

    await waitFor(() => expect(wizardPayload).toEqual({
      target: { lib: '电视剧' },
      delay_ms: 500,
      items: [{
        url: 'https://115cdn.com/s/swfull?password=8888',
        kind: 'share115',
        pwd: '8888',
        label: '莫离 S01E01-E40 2160p'
      }]
    }));

    fireEvent.change(screen.getByLabelText('资源数据源'), { target: { value: 'local' } });
    expect(screen.queryByText('莫离 S01E01-E40 2160p')).not.toBeInTheDocument();
    expect(screen.getByText('等待搜索')).toBeInTheDocument();
  });

  it('previews 115 share files and creates save/offline/scan tasks with csrf', async () => {
    let snapPayload: unknown = null;
    let wizardPayload: unknown = null;
    let offlinePayload: unknown = null;
    let scanPayload: unknown = null;
    let replacePayload: unknown = null;
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
      if (url.pathname === '/api/v2/wizard/add-new') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        wizardPayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '66666666-6666-4666-8666-666666666666',
          kind: 'add_new',
          label: '一条龙加新资源: 1 项 -> 库「电影」',
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
      if (url.pathname === '/api/v2/tasks/66666666-6666-4666-8666-666666666666') {
        return jsonResponse({
          id: '66666666-6666-4666-8666-666666666666',
          kind: 'add_new',
          label: '一条龙加新资源: 1 项 -> 库「电影」',
          status: 'done',
          progress: 4,
          total: 4,
          status_text: '完成，发现 1 个可疑项',
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:00Z',
          started_at: '2026-06-28T00:00:00Z',
          ended_at: '2026-06-28T00:00:03Z',
          updated_at: '2026-06-28T00:00:03Z',
          params: {},
          source: 'api',
          result: {
            ok: false,
            target: { cid: '12345', lib: '电影' },
            transfer: { ok: true, total: 1, succeeded: 1, failed: 0, items: [] },
            strm: { ok: true, triggered: true, lib: '电影', matched: 2, new_count: 1, new_folders: { '示例剧 (1)': 1 }, attention: [], retried: false, warnings: [] },
            scan: { ok: true, triggered: true, mode: 'library', lib: '电影', item_id: 'movie-lib', code: 204, delay_ms: 500, warning: null, error: null },
            poster: { ok: true, triggered: true, status: 'ok', scanned_libraries: 1, scanned_items: 2, issue_count: 0, missing_primary_count: 0, mismatch_count: 0, truncated: false, warnings: [], items: [] },
            dedup: {
              ok: true,
              triggered: true,
              lib: '电影',
              dups_count: 1,
              review_count: 0,
              warnings: [],
              error: null,
              dups: [{
                tmdb: '100',
                keep: { lib: '电影', folder: '示例剧 (1)', n: 2, score: 100 },
                remove: [{ lib: '电影', folder: '示例剧', n: 1, score: 80 }]
              }],
              review: []
            },
            check: { ok: false, status: 'suspicious', item_success_count: 1, item_error_count: 0, stage_error_count: 0, suspicious_count: 1, items: [], errors: [], suspicious: [], message: '检查完成' }
          }
        });
      }
      if (url.pathname === '/api/v2/dedup/replace-batch') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        replacePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '99999999-9999-4999-8999-999999999999',
          kind: 'replace_batch',
          label: '批量替换: 1 组',
          status: 'pending',
          progress: 0,
          total: 1,
          status_text: '排队中',
          cancel_requested: false,
          queued_at: '2026-06-28T00:00:04Z',
          started_at: null,
          ended_at: null,
          updated_at: '2026-06-28T00:00:04Z',
          params: {},
          result: null,
          source: 'api'
        });
      }
      if (url.pathname === '/api/v2/c115/offline/batch') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        offlinePayload = JSON.parse(String(init?.body));
        return jsonResponse({
          id: '77777777-7777-4777-8777-777777777777',
          kind: 'c115_offline_batch',
          label: 'magnet:?xt=urn:btih:abc',
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
    fireEvent.click(screen.getByRole('button', { name: '一条龙转存' }));

    await waitFor(() => expect(wizardPayload).toEqual({
      lib: '电影',
      delay_ms: 500,
      items: [{
        url: 'https://115.com/s/abc?password=urlpw',
        kind: 'share115',
        pwd: 'urlpw',
        file_ids: ['fid-1'],
        label: 'Share Title'
      }]
    }));
    expect(await screen.findByText('重复 1')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '用新替旧' }));
    fireEvent.click(screen.getByRole('button', { name: '确认替换' }));
    await waitFor(() => expect(replacePayload).toEqual({
      items: [{
        lib: '电影',
        win_folder: '示例剧 (1)',
        lose_folder: '示例剧',
        reason: '一条龙智能替换 tmdb 100'
      }]
    }));

    fireEvent.change(screen.getByLabelText('115 离线链接'), { target: { value: 'magnet:?xt=urn:btih:abc' } });
    fireEvent.click(screen.getByRole('button', { name: '创建离线任务' }));

    await waitFor(() => expect(offlinePayload).toEqual({
      lib: '电影',
      label: 'magnet:?xt=urn:btih:abc',
      items: [{
        url: 'magnet:?xt=urn:btih:abc',
        label: 'magnet:?xt=urn:btih:abc'
      }]
    }));

    fireEvent.click(screen.getByRole('button', { name: '扫目标库' }));
    await waitFor(() => expect(scanPayload).toEqual({ lib: '电影' }));
  });

  it('loads settings, fills cid matches, and saves config with csrf', async () => {
    let savedPayload: unknown = null;
    const importPayloads: unknown[] = [];
    let passwordPayload: unknown = null;
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText }
    });
    mockApi((url, init) => {
      if (url.pathname === '/api/v2/config' && (!init?.method || init.method === 'GET')) {
        return jsonResponse({
          settings: {
            emby_url: 'http://emby.local:8096/emby',
            api_key: '***',
            tmdb_base_url: 'https://api.themoviedb.org',
            tmdb_api_key: '***',
            tmdb_timeout_secs: 8,
            c115_cookie: '***',
            cd2_webhook_secret: '***',
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
      if (url.pathname === '/api/v2/autostrm/status') {
        return jsonResponse(autostrmStatus);
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
      if (url.pathname === '/api/v2/config/export') {
        return jsonResponse({
          settings: {
            emby_url: 'http://emby.exported:8096/emby',
            api_key: '***',
            c115_cid_map: { 电影: '12345', 电视剧: '67890' }
          }
        });
      }
      if (url.pathname === '/api/v2/config/import') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        const payload = JSON.parse(String(init?.body));
        importPayloads.push(payload);
        return jsonResponse({
          accepted: ['emby_url', 'c115_cid_map'],
          rejected: [],
          warnings: [],
          applied: payload.apply ? ['emby_url', 'c115_cid_map'] : [],
          dry_run: !payload.apply
        });
      }
      if (url.pathname === '/api/v2/auth/password') {
        const headers = init?.headers as Headers;
        expect(init?.method).toBe('POST');
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        passwordPayload = JSON.parse(String(init?.body));
        return jsonResponse({ ok: true, invalidated_sessions: 2 });
      }
      if (url.pathname === '/api/v2/config' && init?.method === 'PUT') {
        const headers = init.headers as Headers;
        expect(headers.get('X-CSRF-Token')).toBe('csrf-me');
        savedPayload = JSON.parse(String(init.body));
        return jsonResponse({
          settings: {
            emby_url: 'http://emby.new:8096/emby',
            api_key: '***',
            tmdb_base_url: 'https://api.themoviedb.org',
            tmdb_api_key: '***',
            tmdb_timeout_secs: 12,
            c115_cookie: '***',
            c115_cid_map: { 电影: '12345', 电视剧: '67890' },
            trusted_proxies: ['192.168.2.1', '10.0.0.1'],
            auto_strm_enabled: true,
            auto_strm_fullauto: false,
            cd2_mount_prefix: '/CloudNAS/CloudDrive',
            cd2_webhook_secret: '***',
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
    fireEvent.change(screen.getByLabelText('TMDb 超时秒数'), { target: { value: '12' } });
    fireEvent.change(screen.getByLabelText('115 Cookie'), { target: { value: 'UID=1; CID=2; SEID=3' } });
    fireEvent.change(screen.getByLabelText('反代信任 IP'), { target: { value: '192.168.2.1, 10.0.0.1' } });
    fireEvent.click(screen.getByLabelText('启用自动 strm'));
    fireEvent.change(screen.getByLabelText('自动 strm 防抖秒数'), { target: { value: '12' } });
    fireEvent.click(screen.getByRole('button', { name: /自动检测/ }));

    await screen.findByText('自动检测扫描 6 个目录，单匹配且空 cid 的行已填入。');
    expect(screen.getByLabelText('电视剧 cid')).toHaveValue('67890');

    fireEvent.change(screen.getByLabelText('CD2 webhook 密钥'), { target: { value: 'secret-123' } });
    fireEvent.click(screen.getByRole('button', { name: '复制 URL' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(`${window.location.origin}/api/v2/autostrm/webhook?key=secret-123`));

    fireEvent.click(screen.getByRole('button', { name: '刷新状态' }));
    expect(await screen.findByText((_, element) => element?.textContent === 'unmatched 3')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '保存全部' }));

    await waitFor(() => expect(savedPayload).toEqual({
      settings: {
        custom_flag: true,
        emby_url: 'http://emby.new:8096/emby',
        api_key: '***',
        tmdb_base_url: 'https://api.themoviedb.org',
        tmdb_api_key: '***',
        tmdb_timeout_secs: 12,
        tg_resource_api_base_url: 'http://gaotao.cc:8100',
        c115_cookie: 'UID=1; CID=2; SEID=3',
        c115_cid_map: { 电影: '12345', 电视剧: '67890' },
        trusted_proxies: ['192.168.2.1', '10.0.0.1'],
        auto_strm_enabled: true,
        auto_strm_fullauto: false,
        cd2_mount_prefix: '/CloudNAS/CloudDrive',
        cd2_webhook_secret: 'secret-123',
        auto_strm_debounce_sec: 12
      }
    }));
    expect(await screen.findByText('配置已保存')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '导出' }));
    expect(await screen.findByDisplayValue(/emby\.exported/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'dry-run 预检' }));
    await waitFor(() => expect(importPayloads[0]).toEqual({
      settings: {
        emby_url: 'http://emby.exported:8096/emby',
        api_key: '***',
        c115_cid_map: { 电影: '12345', 电视剧: '67890' }
      },
      mode: 'dry_run',
      dry_run: true,
      apply: false,
      confirm: false
    }));
    expect(await screen.findByText('dry-run 预检结果')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '确认导入' }));
    const importModal = (await screen.findByText(/将只应用 dry-run accepted/)).closest('section');
    expect(importModal).not.toBeNull();
    fireEvent.click(within(importModal as HTMLElement).getByRole('button', { name: '确认导入' }));
    await waitFor(() => expect(importPayloads[1]).toEqual({
      settings: {
        emby_url: 'http://emby.exported:8096/emby',
        api_key: '***',
        c115_cid_map: { 电影: '12345', 电视剧: '67890' }
      },
      mode: 'apply',
      dry_run: false,
      apply: true,
      confirm: true
    }));

    fireEvent.change(screen.getByLabelText('当前密码'), { target: { value: 'old-secret' } });
    fireEvent.change(screen.getByLabelText('新密码'), { target: { value: 'new-secret-123' } });
    fireEvent.change(screen.getByLabelText('确认新密码'), { target: { value: 'new-secret-123' } });
    fireEvent.click(screen.getByRole('button', { name: '更新密码' }));
    await waitFor(() => expect(passwordPayload).toEqual({
      current_password: 'old-secret',
      new_password: 'new-secret-123'
    }));
    expect(await screen.findByText('密码已更新，已退出其他 2 个会话')).toBeInTheDocument();
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
    fireEvent.click(screen.getByRole('button', { name: '执行/查看 Undo' }));
    const undoConfirm = (await screen.findByText(/部分类型会直接移动/)).closest('section');
    expect(undoConfirm).not.toBeNull();
    fireEvent.click(within(undoConfirm as HTMLElement).getByRole('button', { name: '执行 Undo' }));

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

    await api('/api/v2/auth/logout', { method: 'POST', body: JSON.stringify({}) });

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
