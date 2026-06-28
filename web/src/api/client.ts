import type { components } from './openapi';

type Schema<Name extends keyof components['schemas']> = components['schemas'][Name];

export type ApiErrorBody = {
  err: string;
  code?: string;
};

export type AuthSession = {
  csrf?: string;
  username?: string;
};

export type LoginResponse = Schema<'LoginResponse'>;

export type MeResponse = Schema<'MeResponse'>;

export class ApiError extends Error {
  status: number;
  body: ApiErrorBody | null;

  constructor(message: string, status: number, body: ApiErrorBody | null) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.body = body;
  }
}

type SessionReason = 'login' | 'refresh' | 'logout' | 'unauthorized' | 'manual';
type SessionListener = (session: AuthSession, reason: SessionReason) => void;

const SESSION_KEY = 'emby-manager.auth.v1';
const listeners = new Set<SessionListener>();
let authSession = readStoredSession();

function getStorage(): Storage | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage ?? null;
  } catch {
    return null;
  }
}

function readStoredSession(): AuthSession {
  const storage = getStorage();
  if (!storage) return {};
  try {
    const raw = storage.getItem(SESSION_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as AuthSession;
    return {
      csrf: typeof parsed.csrf === 'string' ? parsed.csrf : undefined,
      username: typeof parsed.username === 'string' ? parsed.username : undefined
    };
  } catch {
    storage.removeItem(SESSION_KEY);
    return {};
  }
}

function persistSession(session: AuthSession) {
  const storage = getStorage();
  if (!storage) return;
  const clean = {
    csrf: session.csrf || undefined,
    username: session.username || undefined
  };
  if (!clean.csrf && !clean.username) {
    storage.removeItem(SESSION_KEY);
  } else {
    storage.setItem(SESSION_KEY, JSON.stringify(clean));
  }
}

function emitSession(reason: SessionReason) {
  for (const listener of listeners) listener(getAuthSession(), reason);
}

export function getAuthSession(): AuthSession {
  return { ...authSession };
}

export function setAuthSession(session: AuthSession, reason: SessionReason = 'manual') {
  authSession = {
    csrf: session.csrf || undefined,
    username: session.username || undefined
  };
  persistSession(authSession);
  emitSession(reason);
}

export function clearAuthSession(reason: SessionReason = 'manual') {
  setAuthSession({}, reason);
}

export function subscribeAuthSession(listener: SessionListener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function isMutating(method: string) {
  return !['GET', 'HEAD', 'OPTIONS'].includes(method.toUpperCase());
}

function contentLooksJson(body: BodyInit | null | undefined) {
  return Boolean(body) && !(body instanceof FormData) && !(body instanceof Blob) && !(body instanceof URLSearchParams);
}

function pathnameOf(path: string) {
  try {
    const base = typeof window === 'undefined' ? 'http://localhost' : window.location.origin;
    return new URL(path, base).pathname;
  } catch {
    return path.split('?')[0];
  }
}

function sessionFromLogin(data: LoginResponse): AuthSession {
  return {
    csrf: data.csrf,
    username: data.username
  };
}

function syncAuthFromResponse(path: string, data: unknown) {
  const route = pathnameOf(path);
  if (route === '/api/v2/auth/login' && data && typeof data === 'object') {
    const login = data as Partial<LoginResponse>;
    if (typeof login.token === 'string' && typeof login.csrf === 'string') {
      setAuthSession(sessionFromLogin(login as LoginResponse), 'login');
    }
    return;
  }
  if (route === '/api/v2/auth/me' && data && typeof data === 'object') {
    const me = data as Partial<MeResponse>;
    if (me.authenticated) {
      setAuthSession(
        {
          ...authSession,
          csrf: typeof me.csrf === 'string' ? me.csrf : authSession.csrf,
          username: typeof me.username === 'string' ? me.username : authSession.username
        },
        'refresh'
      );
    } else {
      clearAuthSession('unauthorized');
    }
  }
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const method = (init.method || 'GET').toUpperCase();
  const route = pathnameOf(path);
  const headers = new Headers(init.headers);
  const attachSession = route !== '/api/v2/auth/login';
  if (attachSession && isMutating(method) && authSession.csrf && !headers.has('X-CSRF-Token')) {
    headers.set('X-CSRF-Token', authSession.csrf);
  }
  if (contentLooksJson(init.body as BodyInit | null | undefined) && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }
  const res = await fetch(path, {
    ...init,
    method,
    headers,
    credentials: 'same-origin'
  });
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!res.ok) {
    const body = data && typeof data === 'object' ? (data as ApiErrorBody) : null;
    if (res.status === 401) {
      clearAuthSession('unauthorized');
    }
    throw new ApiError(body?.err || `${res.status} ${res.statusText}`, res.status, body);
  }
  syncAuthFromResponse(path, data);
  return data as T;
}

export async function login(username: string, password: string): Promise<AuthSession> {
  const data = await api<LoginResponse>('/api/v2/auth/login', {
    method: 'POST',
    body: JSON.stringify({ username, password })
  });
  return sessionFromLogin(data);
}

export async function logout(): Promise<void> {
  try {
    await api<{ ok: boolean }>('/api/v2/auth/logout', { method: 'POST' });
  } finally {
    clearAuthSession('logout');
  }
}

export async function me(): Promise<MeResponse> {
  return api<MeResponse>('/api/v2/auth/me');
}
