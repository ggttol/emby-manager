import {
  Activity,
  CalendarClock,
  Database,
  Eraser,
  FileSearch,
  FolderSearch,
  HardDrive,
  Image,
  LayoutDashboard,
  ListChecks,
  LogOut,
  MonitorCog,
  Search,
  Settings,
  Shield,
  Sparkles,
  Trash2,
  UserRound,
  UsersRound,
  type LucideIcon
} from 'lucide-react';
import { useEffect, useMemo, useState, type FormEvent } from 'react';
import {
  ApiError,
  api,
  login as apiLogin,
  logout as apiLogout,
  me as apiMe,
  subscribeAuthSession,
  type AuthSession
} from './api/client';
import { TaskCenter } from './components/TaskCenter';
import { ToastProvider, useToast } from './components/Toast';
import { C115Panel } from './components/C115Panel';
import { CatalogPanel } from './components/CatalogPanel';
import { CleanupPanel, DedupPanel, ZhuigengGapsPanel } from './components/InsightPanels';
import { LogsPanel } from './components/LogsPanel';
import { ManagePanel } from './components/ManagePanel';
import { PostersPanel } from './components/PostersPanel';
import { DashboardPanel, SystemPanel } from './components/ReadOnlyPanels';
import { ScanPanel } from './components/ScanPanel';
import { SchedulesPanel } from './components/SchedulesPanel';
import { SettingsPanel } from './components/SettingsPanel';
import { UsersPanel } from './components/UsersPanel';

type Tab = {
  id: string;
  label: string;
  endpoint: string;
  description: string;
  icon: LucideIcon;
  group: 'overview' | 'media' | 'resources' | 'repair' | 'ops';
};

const tabs: Tab[] = [
  { id: 'dashboard', label: '仪表盘', endpoint: '/api/v2/system/summary', description: '在线状态、库卡片和待办入口', icon: LayoutDashboard, group: 'overview' },
  { id: 'scan', label: '扫描', endpoint: '/api/v2/libraries', description: '单库 / 全库扫描与孤儿 strm 清理', icon: FolderSearch, group: 'media' },
  { id: 'c115', label: '115 转存', endpoint: '/api/v2/c115/test', description: '分享链接 snap、转存、离线下载', icon: HardDrive, group: 'resources' },
  { id: 'catalog', label: '找资源', endpoint: '/api/v2/catalog/stats', description: 'Postgres catalog 搜索和转存入口', icon: Search, group: 'resources' },
  { id: 'zhuigeng', label: '追更检查', endpoint: '/api/v2/gaps/scan', description: '在更剧扫描和缺集汇总', icon: Sparkles, group: 'repair' },
  { id: 'gaps', label: '缺集检查', endpoint: '/api/v2/gaps/scan', description: '本地剧集和 TMDb 季集表对照', icon: FileSearch, group: 'repair' },
  { id: 'posters', label: '海报修复', endpoint: '/api/v2/posters/detect-mismatch', description: '无海报与 tmdbid 错绑检测', icon: Image, group: 'repair' },
  { id: 'dedup', label: '去重', endpoint: '/api/v2/manage/undo', description: '重复资源分析、删除、替换', icon: Eraser, group: 'media' },
  { id: 'manage', label: '删除·移动', endpoint: '/api/v2/manage/undo', description: '危险操作、移动和 undo', icon: Trash2, group: 'media' },
  { id: 'cleanup', label: '智能清理', endpoint: '/api/v2/cleanup/suggest', description: '评分、无观看、空间和元数据维度', icon: ListChecks, group: 'media' },
  { id: 'system', label: '系统', endpoint: '/api/v2/system/summary', description: 'Docker、负载、磁盘、健康预警', icon: MonitorCog, group: 'ops' },
  { id: 'schedules', label: '定时', endpoint: '/api/v2/schedules', description: '每日 / 每周 / 每月任务编排', icon: CalendarClock, group: 'ops' },
  { id: 'logs', label: '日志', endpoint: '/api/v2/logs', description: '应用日志和审计记录', icon: Activity, group: 'ops' },
  { id: 'users', label: '用户', endpoint: '/api/v2/users', description: 'Emby 用户策略、限速和并发', icon: UsersRound, group: 'ops' },
  { id: 'settings', label: '设置', endpoint: '/api/v2/config', description: '路径、密钥、导入导出和迁移状态', icon: Settings, group: 'ops' }
];

const navGroups: Array<{ id: Tab['group']; label: string }> = [
  { id: 'overview', label: '总览' },
  { id: 'resources', label: '加资源' },
  { id: 'media', label: '媒体库' },
  { id: 'repair', label: '修复检查' },
  { id: 'ops', label: '系统' }
];

type AuthState =
  | { status: 'checking' }
  | { status: 'anonymous' }
  | { status: 'authenticated'; username: string };

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function Shell({ username, onLogout }: { username: string; onLogout: () => void }) {
  const [active, setActive] = useState(tabs[0].id);
  const tab = useMemo(() => tabs.find((item) => item.id === active) || tabs[0], [active]);
  const groupLabel = navGroups.find((group) => group.id === tab.group)?.label || '';

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <Shield size={24} />
          <div><strong>Emby Manager</strong><span>Rust Preview</span></div>
        </div>
        <nav>
          {navGroups.map((group) => (
            <div className="navSection" key={group.id}>
              <span className="navGroupLabel">{group.label}</span>
              {tabs.filter((item) => item.group === group.id).map((item) => {
                const Icon = item.icon;
                return (
                  <button
                    key={item.id}
                    className={`navButton ${item.id === active ? 'active' : ''}`}
                    onClick={() => setActive(item.id)}
                    title={item.description}
                  >
                    <Icon size={17} />
                    <span>{item.label}</span>
                  </button>
                );
              })}
            </div>
          ))}
        </nav>
      </aside>
      <main>
        <header className="topbar">
          <div className="topbarTitle">
            <span>{groupLabel}</span>
            <h1>{tab.label}</h1>
            <p>{tab.description}</p>
          </div>
          <div className="topbarActions">
            <TaskCenter />
            <span className="userPill">
              <UserRound size={15} />
              {username}
            </span>
            <button className="iconBtn" onClick={onLogout} aria-label="登出">
              <LogOut size={16} />
            </button>
          </div>
        </header>
        <TabPanel tab={tab} />
      </main>
    </div>
  );
}

function TabPanel({ tab }: { tab: Tab }) {
  if (tab.id === 'dashboard') {
    return (
      <section className="panel">
        <DashboardPanel />
      </section>
    );
  }

  if (tab.id === 'scan') {
    return (
      <section className="panel">
        <ScanPanel />
      </section>
    );
  }

  if (tab.id === 'users') {
    return (
      <section className="panel">
        <UsersPanel />
      </section>
    );
  }

  if (tab.id === 'catalog') {
    return (
      <section className="panel">
        <CatalogPanel />
      </section>
    );
  }

  if (tab.id === 'c115') {
    return (
      <section className="panel">
        <C115Panel />
      </section>
    );
  }

  if (tab.id === 'settings') {
    return (
      <section className="panel">
        <SettingsPanel />
      </section>
    );
  }

  if (tab.id === 'schedules') {
    return (
      <section className="panel">
        <SchedulesPanel />
      </section>
    );
  }

  if (tab.id === 'logs') {
    return (
      <section className="panel">
        <LogsPanel />
      </section>
    );
  }

  if (tab.id === 'system') {
    return (
      <section className="panel">
        <SystemPanel />
      </section>
    );
  }

  if (tab.id === 'posters') {
    return (
      <section className="panel">
        <PostersPanel />
      </section>
    );
  }

  if (tab.id === 'zhuigeng') {
    return (
      <section className="panel">
        <ZhuigengGapsPanel mode="zhuigeng" />
      </section>
    );
  }

  if (tab.id === 'gaps') {
    return (
      <section className="panel">
        <ZhuigengGapsPanel mode="gaps" />
      </section>
    );
  }

  if (tab.id === 'cleanup') {
    return (
      <section className="panel">
        <CleanupPanel />
      </section>
    );
  }

  if (tab.id === 'dedup') {
    return (
      <section className="panel">
        <DedupPanel />
      </section>
    );
  }

  if (tab.id === 'manage') {
    return (
      <section className="panel">
        <ManagePanel />
      </section>
    );
  }

  return <FallbackPanel tab={tab} />;
}

function FallbackPanel({ tab }: { tab: Tab }) {
  const [data, setData] = useState<unknown>(null);
  const [error, setError] = useState<string>('');
  const [loading, setLoading] = useState(false);
  const [strmLib, setStrmLib] = useState('电影');

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      const method = tab.endpoint.includes('/scan') || tab.endpoint.includes('/suggest') || tab.endpoint.includes('/detect') ? 'POST' : 'GET';
      const value = await api<unknown>(tab.endpoint, method === 'POST' ? { method, body: JSON.stringify({}) } : {});
      setData(value);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setData(null);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, [tab.id]);

  const loadStrm = async () => {
    setLoading(true);
    setError('');
    try {
      const value = await api<unknown>(`/api/v2/libraries/strm?lib=${encodeURIComponent(strmLib)}&limit=80`);
      setData(value);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setData(null);
    } finally {
      setLoading(false);
    }
  };

  return (
    <section className="panel">
      <div className="panelActions">
        <button className="btn" onClick={load} disabled={loading}>{loading ? '加载中' : '刷新'}</button>
      </div>
      {tab.id === 'scan' && (
        <div className="inlineTool">
          <label>
            <span>strm 库名</span>
            <input className="input" value={strmLib} onChange={(e) => setStrmLib(e.target.value)} />
          </label>
          <button className="btn ghost" onClick={loadStrm} disabled={loading}>列出 strm</button>
        </div>
      )}
      <FeatureMap id={tab.id} />
      {error && <div className="notice warn">{error}</div>}
      {data !== null && <pre className="jsonBlock">{JSON.stringify(data, null, 2)}</pre>}
    </section>
  );
}

function FeatureMap({ id }: { id: string }) {
  const cards = [
    { icon: <LayoutDashboard />, title: 'UI 壳', text: '15 个 tab 已拆成 React 路由入口。' },
    { icon: <Activity />, title: '任务中心', text: '轮询 /api/v2/tasks，支持取消、搜索和全局进度。' },
    { icon: <Database />, title: 'Postgres', text: '配置、任务、调度、undo、catalog 共用数据库。' },
    { icon: <FolderSearch />, title: '业务 Port', text: `${id} 模块按旧版语义迁移，写操作走任务中心跟踪。` }
  ];
  if (id === 'catalog') cards.push({ icon: <Search />, title: 'Catalog', text: '搜索接口已接 Postgres catalog_items。' });
  if (id === 'schedules') cards.push({ icon: <ListChecks />, title: 'Scheduler', text: 'CRUD、立即运行和重叠保护已接入。' });
  if (id === 'settings') cards.push({ icon: <Settings />, title: 'Config', text: '敏感字段脱敏，支持导入导出和密码修改。' });
  if (id === 'users') cards.push({ icon: <UserRound />, title: 'Auth', text: '登录/session/legacy PBKDF2 rehash 已在后端实现。' });
  return (
    <div className="featureGrid">
      {cards.map((card) => (
        <article className="feature" key={card.title}>
          {card.icon}
          <h3>{card.title}</h3>
          <p>{card.text}</p>
        </article>
      ))}
    </div>
  );
}

function CheckingPanel() {
  return (
    <div className="loginPage">
      <section className="loginPanel" aria-label="正在检查登录状态">
        <div className="brand loginBrand">
          <Shield size={24} />
          <div><strong>Emby Manager</strong><span>Rust Preview</span></div>
        </div>
        <p className="loginHint">正在检查登录状态...</p>
      </section>
    </div>
  );
}

function LoginPanel({ onAuthenticated }: { onAuthenticated: (session: AuthSession) => void }) {
  const [username, setUsername] = useState('admin');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const toast = useToast();

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setLoading(true);
    setError('');
    try {
      const session = await apiLogin(username, password);
      onAuthenticated(session);
      toast.push('登录成功', 'ok');
    } catch (e) {
      const message = `登录失败：${errorMessage(e)}`;
      setError(message);
      toast.push(message, 'error');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="loginPage">
      <form className="loginPanel" onSubmit={submit}>
        <div className="brand loginBrand">
          <Shield size={24} />
          <div><strong>Emby Manager</strong><span>Rust Preview</span></div>
        </div>
        <label>
          <span>用户名</span>
          <input className="input" value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" />
        </label>
        <label>
          <span>密码</span>
          <input
            className="input"
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="current-password"
            autoFocus
          />
        </label>
        {error && <div className="notice warn">{error}</div>}
        <button className="btn" disabled={loading || !username.trim() || !password}>
          {loading ? '登录中' : '登录'}
        </button>
      </form>
    </div>
  );
}

function AuthGate() {
  const [auth, setAuth] = useState<AuthState>({ status: 'checking' });
  const toast = useToast();

  useEffect(() => {
    let alive = true;
    const unsubscribe = subscribeAuthSession((session, reason) => {
      if (session.username || session.csrf) {
        setAuth({ status: 'authenticated', username: session.username || '已登录' });
        return;
      }
      setAuth((prev) => {
        if (prev.status === 'authenticated' && reason === 'unauthorized') {
          toast.push('登录已过期，请重新登录', 'warn');
        }
        return { status: 'anonymous' };
      });
    });

    apiMe()
      .then((current) => {
        if (!alive) return;
        if (current.authenticated) {
          setAuth({ status: 'authenticated', username: current.username || '已登录' });
        } else {
          setAuth({ status: 'anonymous' });
        }
      })
      .catch((e) => {
        if (!alive) return;
        setAuth({ status: 'anonymous' });
        if (!(e instanceof ApiError && e.status === 401)) {
          toast.push(`无法确认登录状态：${errorMessage(e)}`, 'warn');
        }
      });

    return () => {
      alive = false;
      unsubscribe();
    };
  }, [toast]);

  const handleAuthenticated = (session: AuthSession) => {
    setAuth({ status: 'authenticated', username: session.username || '已登录' });
  };

  const handleLogout = async () => {
    try {
      await apiLogout();
      toast.push('已登出', 'info');
    } catch (e) {
      toast.push(`登出请求失败，本地会话已清理：${errorMessage(e)}`, 'warn');
    } finally {
      setAuth({ status: 'anonymous' });
    }
  };

  if (auth.status === 'checking') return <CheckingPanel />;
  if (auth.status === 'anonymous') return <LoginPanel onAuthenticated={handleAuthenticated} />;
  return <Shell username={auth.username} onLogout={handleLogout} />;
}

export default function App() {
  return (
    <ToastProvider>
      <AuthGate />
    </ToastProvider>
  );
}
