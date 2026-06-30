import { ClipboardCheck, Copy, Download, Globe2, KeyRound, RefreshCw, Save, SearchCheck, ShieldCheck, Shuffle, Webhook } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { ConfirmDanger } from './Modal';
import { useToast } from './Toast';

type C115AutoCidResponse = components['schemas']['C115AutoCidResponse'];
type AutostrmStatusResponse = components['schemas']['AutostrmStatusResponse'];
type ChangePasswordRequest = components['schemas']['ChangePasswordRequest'];
type ChangePasswordResponse = components['schemas']['ChangePasswordResponse'];
type ConfigImportReport = components['schemas']['ConfigImportReport'];
type ConfigImportRequest = components['schemas']['ConfigImportRequest'];
type ConfigResponse = components['schemas']['ConfigResponse'];
type ConfigUpdateRequest = components['schemas']['ConfigUpdateRequest'];
type EmbyLibrary = components['schemas']['EmbyLibrary'];
type LibrariesResponse = components['schemas']['LibrariesResponse'];

type CidRow = {
  lib: string;
  cid: string;
  hint?: string;
};

const defaultEmbyUrl = 'http://127.0.0.1:8096/emby';
const defaultTmdbBaseUrl = 'https://api.themoviedb.org';
const defaultTgResourceApiBaseUrl = 'http://gaotao.cc:8100';
const knownKeys = new Set([
  'emby_url',
  'api_key',
  'tmdb_base_url',
  'tmdb_url',
  'tmdb_api_key',
  'tmdb_key',
  'tmdb_timeout_secs',
  'tg_resource_api_base_url',
  'tg_resource_api_token',
  'c115_cookie',
  'c115_cid_map',
  'trusted_proxies',
  'auto_strm_enabled',
  'auto_strm_fullauto',
  'cd2_mount_prefix',
  'auto_strm_debounce_sec',
  'cd2_webhook_secret'
]);

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function stringValue(value: unknown, fallback = '') {
  return typeof value === 'string' ? value : value == null ? fallback : String(value);
}

function boolValue(value: unknown) {
  return value === true;
}

function numberString(value: unknown, fallback: number) {
  return typeof value === 'number' && Number.isFinite(value) ? String(value) : String(fallback);
}

function parseCidMapValue(value: unknown): Record<string, string> {
  const raw = typeof value === 'string' ? parseJsonObject(value) : value;
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {};
  return Object.fromEntries(
    Object.entries(raw as Record<string, unknown>)
      .map(([key, item]) => [key, typeof item === 'string' ? item : item == null ? '' : String(item)])
      .filter(([key, item]) => key.trim() && item.trim())
  );
}

function parseJsonObject(value: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    return {};
  }
}

function parseTrustedProxies(value: unknown) {
  if (Array.isArray(value)) return value.map((item) => String(item)).join(', ');
  if (typeof value === 'string') return value;
  return '';
}

function rowsFromConfig(config: ConfigResponse, libraries: EmbyLibrary[]): CidRow[] {
  const map = parseCidMapValue(config.settings.c115_cid_map);
  const names = new Set<string>();
  libraries.forEach((lib) => names.add(lib.name));
  Object.keys(map).forEach((name) => names.add(name));
  if (names.size === 0) {
    ['电影', '电视剧', '动漫'].forEach((name) => names.add(name));
  }
  return Array.from(names)
    .sort((a, b) => a.localeCompare(b, 'zh-CN'))
    .map((lib) => ({ lib, cid: map[lib] || '' }));
}

function sanitizeCidRows(rows: CidRow[]) {
  const out: Record<string, string> = {};
  for (const row of rows) {
    const lib = row.lib.trim();
    const cid = row.cid.trim();
    if (!lib || !cid) continue;
    if (!/^[1-9]\d*$/.test(cid)) {
      throw new Error(`库「${lib}」的 cid 必须是正整数，0 根目录不允许`);
    }
    out[lib] = cid;
  }
  return out;
}

function splitTrustedProxies(value: string) {
  return value.trim() ? value.split(/[,，\s]+/).filter(Boolean) : [];
}

function safeExtraJson(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return {};
  const parsed = JSON.parse(trimmed) as unknown;
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error('高级 JSON 必须是对象');
  }
  return parsed as Record<string, unknown>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function parseImportPayload(value: string): Pick<ConfigImportRequest, 'settings' | 'cfg'> {
  const trimmed = value.trim();
  if (!trimmed) throw new Error('请先粘贴或导出配置 JSON');
  const parsed = JSON.parse(trimmed) as unknown;
  if (!isRecord(parsed)) throw new Error('配置 JSON 必须是对象');
  if ('settings' in parsed || 'cfg' in parsed) {
    if (isRecord(parsed.settings)) return { settings: parsed.settings };
    if (isRecord(parsed.cfg)) return { cfg: parsed.cfg };
    throw new Error('settings 或 cfg 必须是对象');
  }
  return { settings: parsed };
}

function extraSettings(config: ConfigResponse) {
  const extra = Object.fromEntries(
    Object.entries(config.settings).filter(([key]) => !knownKeys.has(key))
  );
  return JSON.stringify(extra, null, 2);
}

function formatList(items: string[], empty: string) {
  return items.length ? items.join('、') : empty;
}

function count(value: number | null | undefined) {
  return new Intl.NumberFormat('zh-CN').format(value ?? 0);
}

function dateText(value: string | null | undefined) {
  if (!value) return '无';
  try {
    return new Intl.DateTimeFormat('zh-CN', {
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit'
    }).format(new Date(value));
  } catch {
    return value;
  }
}

function generateSecret() {
  const bytes = new Uint8Array(24);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  }
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 18)}`;
}

function webhookUrl(secret: string) {
  const base = `${window.location.origin}/api/v2/autostrm/webhook`;
  return secret.trim() ? `${base}?key=${encodeURIComponent(secret.trim())}` : base;
}

function ImportReport({ report }: { report: ConfigImportReport }) {
  const hasWarnings = report.warnings.length > 0 || report.rejected.length > 0;
  return (
    <div className={`notice ${hasWarnings ? 'warn' : ''} whitespaceNotice`}>
      <strong>{report.dry_run ? 'dry-run 预检结果' : '导入结果'}</strong>
      <div>accepted: {formatList(report.accepted, '无')}</div>
      {!report.dry_run && <div>applied: {formatList(report.applied, '无')}</div>}
      {report.rejected.length > 0 && (
        <div>
          rejected: {report.rejected.map((item) => `${item.key}(${item.reason})`).join('、')}
        </div>
      )}
      {report.warnings.length > 0 && <div>warnings: {report.warnings.join('、')}</div>}
    </div>
  );
}

export function SettingsPanel() {
  const [config, setConfig] = useState<ConfigResponse>({ settings: {} });
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [embyUrl, setEmbyUrl] = useState(defaultEmbyUrl);
  const [apiKey, setApiKey] = useState('');
  const [tmdbBaseUrl, setTmdbBaseUrl] = useState(defaultTmdbBaseUrl);
  const [tmdbApiKey, setTmdbApiKey] = useState('');
  const [tmdbTimeout, setTmdbTimeout] = useState('8');
  const [tgResourceApiBaseUrl, setTgResourceApiBaseUrl] = useState(defaultTgResourceApiBaseUrl);
  const [tgResourceApiToken, setTgResourceApiToken] = useState('');
  const [c115Cookie, setC115Cookie] = useState('');
  const [c115CookieSet, setC115CookieSet] = useState(false);
  const [cidRows, setCidRows] = useState<CidRow[]>([]);
  const [trustedProxies, setTrustedProxies] = useState('');
  const [autoEnabled, setAutoEnabled] = useState(false);
  const [autoFullauto, setAutoFullauto] = useState(false);
  const [cd2Prefix, setCd2Prefix] = useState('/CloudNAS/CloudDrive');
  const [autoDebounce, setAutoDebounce] = useState('8');
  const [cd2Secret, setCd2Secret] = useState('');
  const [extraJson, setExtraJson] = useState('{}');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [detectResult, setDetectResult] = useState<C115AutoCidResponse | null>(null);
  const [autostrmStatus, setAutostrmStatus] = useState<AutostrmStatusResponse | null>(null);
  const [loadingAutostrmStatus, setLoadingAutostrmStatus] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [importJson, setImportJson] = useState('');
  const [importReport, setImportReport] = useState<ConfigImportReport | null>(null);
  const [dryRunSignature, setDryRunSignature] = useState('');
  const [confirmImport, setConfirmImport] = useState(false);
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [changingPassword, setChangingPassword] = useState(false);
  const toast = useToast();

  const maskedApiKey = config.settings.api_key === '***';
  const maskedTmdbApiKey = config.settings.tmdb_api_key === '***' || config.settings.tmdb_key === '***';
  const maskedTgResourceApiToken = config.settings.tg_resource_api_token === '***';
  const webhookSecretSet = Boolean(stringValue(config.settings.cd2_webhook_secret));
  const maskedWebhookSecret = config.settings.cd2_webhook_secret === '***';
  const copyableWebhookUrl = webhookUrl(cd2Secret);
  const sortedRows = useMemo(() => cidRows, [cidRows]);
  const importSignature = importJson.trim();
  const canApplyImport = Boolean(
    importReport?.dry_run &&
    importReport.accepted.length > 0 &&
    dryRunSignature === importSignature &&
    importSignature
  );

  const hydrate = (nextConfig: ConfigResponse, nextLibraries: EmbyLibrary[]) => {
    setConfig(nextConfig);
    setLibraries(nextLibraries);
    setEmbyUrl(stringValue(nextConfig.settings.emby_url, defaultEmbyUrl));
    setApiKey(stringValue(nextConfig.settings.api_key));
    setTmdbBaseUrl(stringValue(nextConfig.settings.tmdb_base_url ?? nextConfig.settings.tmdb_url, defaultTmdbBaseUrl));
    setTmdbApiKey(stringValue(nextConfig.settings.tmdb_api_key ?? nextConfig.settings.tmdb_key));
    setTmdbTimeout(numberString(nextConfig.settings.tmdb_timeout_secs, 8));
    setTgResourceApiBaseUrl(stringValue(nextConfig.settings.tg_resource_api_base_url, defaultTgResourceApiBaseUrl));
    setTgResourceApiToken('');
    setC115Cookie('');
    setC115CookieSet(Boolean(stringValue(nextConfig.settings.c115_cookie)));
    setCidRows(rowsFromConfig(nextConfig, nextLibraries));
    setTrustedProxies(parseTrustedProxies(nextConfig.settings.trusted_proxies));
    setAutoEnabled(boolValue(nextConfig.settings.auto_strm_enabled));
    setAutoFullauto(boolValue(nextConfig.settings.auto_strm_fullauto));
    setCd2Prefix(stringValue(nextConfig.settings.cd2_mount_prefix, '/CloudNAS/CloudDrive'));
    setAutoDebounce(numberString(nextConfig.settings.auto_strm_debounce_sec, 8));
    setCd2Secret('');
    setExtraJson(extraSettings(nextConfig));
  };

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      const [configData, libraryResult] = await Promise.all([
        api<ConfigResponse>('/api/v2/config'),
        api<LibrariesResponse>('/api/v2/libraries').catch(() => ({ libraries: [] }))
      ]);
      hydrate(configData, libraryResult.libraries);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`配置加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const patchCidRow = (index: number, patch: Partial<CidRow>) => {
    setCidRows((prev) => prev.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  };

  const addCidRow = () => {
    setCidRows((prev) => [...prev, { lib: '', cid: '' }]);
  };

  const removeCidRow = (index: number) => {
    setCidRows((prev) => prev.filter((_, i) => i !== index));
  };

  const buildPayload = (): ConfigUpdateRequest => {
    const debounce = Number(autoDebounce);
    if (!Number.isInteger(debounce) || debounce < 1 || debounce > 120) {
      throw new Error('防抖窗口必须是 1 到 120 的整数秒');
    }
    const timeout = Number(tmdbTimeout);
    if (!Number.isInteger(timeout) || timeout < 1 || timeout > 60) {
      throw new Error('TMDb 超时必须是 1 到 60 的整数秒');
    }
    const settings: Record<string, unknown> = {
      ...safeExtraJson(extraJson),
      emby_url: embyUrl.trim() || defaultEmbyUrl,
      api_key: apiKey.trim() || (maskedApiKey ? '***' : ''),
      tmdb_base_url: tmdbBaseUrl.trim() || defaultTmdbBaseUrl,
      tmdb_api_key: tmdbApiKey.trim() || (maskedTmdbApiKey ? '***' : ''),
      tmdb_timeout_secs: timeout,
      tg_resource_api_base_url: tgResourceApiBaseUrl.trim() || defaultTgResourceApiBaseUrl,
      c115_cid_map: sanitizeCidRows(cidRows),
      trusted_proxies: splitTrustedProxies(trustedProxies),
      auto_strm_enabled: autoEnabled,
      auto_strm_fullauto: autoFullauto,
      cd2_mount_prefix: cd2Prefix.trim() || '/CloudNAS/CloudDrive',
      auto_strm_debounce_sec: debounce
    };
    if (c115Cookie.trim()) settings.c115_cookie = c115Cookie.trim();
    if (tgResourceApiToken.trim()) settings.tg_resource_api_token = tgResourceApiToken.trim();
    else if (maskedTgResourceApiToken) settings.tg_resource_api_token = '***';
    if (cd2Secret.trim()) settings.cd2_webhook_secret = cd2Secret.trim();
    return { settings };
  };

  const save = async () => {
    let payload: ConfigUpdateRequest;
    try {
      payload = buildPayload();
    } catch (e) {
      toast.push(errorMessage(e), 'warn');
      return;
    }
    setSaving(true);
    setError('');
    try {
      const data = await api<ConfigResponse>('/api/v2/config', {
        method: 'PUT',
        body: JSON.stringify(payload)
      });
      hydrate(data, libraries);
      toast.push('配置已保存', 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`保存配置失败：${message}`, 'error');
    } finally {
      setSaving(false);
    }
  };

  const detectCid = async () => {
    setDetecting(true);
    setDetectResult(null);
    setError('');
    try {
      const data = await api<C115AutoCidResponse>('/api/v2/c115/auto-cid', {
        method: 'POST',
        body: JSON.stringify({ max_depth: 2 })
      });
      setDetectResult(data);
      setCidRows((prev) =>
        prev.map((row) => {
          const hits = data.matches[row.lib] || [];
          if (hits.length === 1 && !row.cid.trim()) {
            return { ...row, cid: hits[0].cid, hint: hits[0].path };
          }
          if (hits.length > 1) return { ...row, hint: hits.map((hit) => `${hit.path}=${hit.cid}`).join(' | ') };
          return row;
        })
      );
      toast.push(`已扫描 ${data.scanned} 个目录`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`自动检测 cid 失败：${message}`, 'error');
    } finally {
      setDetecting(false);
    }
  };

  const refreshAutostrmStatus = async () => {
    setLoadingAutostrmStatus(true);
    try {
      const status = await api<AutostrmStatusResponse>('/api/v2/autostrm/status');
      setAutostrmStatus(status);
      toast.push('Autostrm 状态已刷新', 'ok');
    } catch (e) {
      toast.push(`Autostrm 状态加载失败：${errorMessage(e)}`, 'error');
    } finally {
      setLoadingAutostrmStatus(false);
    }
  };

  const copyWebhookUrl = async () => {
    if (!cd2Secret.trim()) {
      toast.push(maskedWebhookSecret ? '后端只返回脱敏密钥；输入或生成新密钥后再复制 URL' : '先输入或生成 webhook 密钥', 'warn');
      return;
    }
    try {
      await navigator.clipboard.writeText(copyableWebhookUrl);
      toast.push('Webhook URL 已复制', 'ok');
    } catch (e) {
      toast.push(`复制 webhook URL 失败：${errorMessage(e)}`, 'error');
    }
  };

  const setImportText = (value: string) => {
    setImportJson(value);
    setImportReport(null);
    setDryRunSignature('');
  };

  const buildImportPayload = (apply: boolean): ConfigImportRequest => ({
    ...parseImportPayload(importJson),
    mode: apply ? 'apply' : 'dry_run',
    dry_run: !apply,
    apply,
    confirm: apply
  });

  const exportConfig = async () => {
    setExporting(true);
    setError('');
    try {
      const data = await api<ConfigResponse>('/api/v2/config/export');
      const text = JSON.stringify(data, null, 2);
      setImportText(text);
      let copied = false;
      try {
        if (navigator.clipboard?.writeText) {
          await navigator.clipboard.writeText(text);
          copied = true;
        }
      } catch {
        copied = false;
      }
      toast.push(copied ? '配置已导出到文本框并复制' : '配置已导出到文本框', 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`导出配置失败：${message}`, 'error');
    } finally {
      setExporting(false);
    }
  };

  const copyImportJson = async () => {
    if (!importJson.trim()) {
      toast.push('没有可复制的配置 JSON', 'warn');
      return;
    }
    try {
      await navigator.clipboard.writeText(importJson);
      toast.push('配置 JSON 已复制', 'ok');
    } catch (e) {
      toast.push(`复制失败：${errorMessage(e)}`, 'error');
    }
  };

  const dryRunImport = async () => {
    let payload: ConfigImportRequest;
    try {
      payload = buildImportPayload(false);
    } catch (e) {
      toast.push(errorMessage(e), 'warn');
      return;
    }
    setImporting(true);
    setError('');
    try {
      const report = await api<ConfigImportReport>('/api/v2/config/import', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setImportReport(report);
      setDryRunSignature(importSignature);
      const tone = report.rejected.length || report.warnings.length ? 'warn' : 'ok';
      toast.push(`预检完成：接受 ${report.accepted.length}，拒绝 ${report.rejected.length}`, tone);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`导入预检失败：${message}`, 'error');
    } finally {
      setImporting(false);
    }
  };

  const requestApplyImport = () => {
    if (!canApplyImport) {
      toast.push('请先 dry-run 预检当前 JSON，确认 accepted 后再导入', 'warn');
      return;
    }
    setConfirmImport(true);
  };

  const applyImport = async () => {
    let payload: ConfigImportRequest;
    try {
      payload = buildImportPayload(true);
    } catch (e) {
      setConfirmImport(false);
      toast.push(errorMessage(e), 'warn');
      return;
    }
    setConfirmImport(false);
    setImporting(true);
    setError('');
    try {
      const report = await api<ConfigImportReport>('/api/v2/config/import', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setImportReport(report);
      setDryRunSignature(importSignature);
      toast.push(`已导入 ${report.applied.length} 个字段`, report.rejected.length ? 'warn' : 'ok');
      await load();
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`导入配置失败：${message}`, 'error');
    } finally {
      setImporting(false);
    }
  };

  const changePassword = async () => {
    const current = currentPassword;
    const next = newPassword;
    if (!current) {
      toast.push('先输入当前密码', 'warn');
      return;
    }
    if (next.length < 8) {
      toast.push('新密码至少需要 8 个字符', 'warn');
      return;
    }
    if (next !== confirmPassword) {
      toast.push('两次输入的新密码不一致', 'warn');
      return;
    }
    const payload: ChangePasswordRequest = {
      current_password: current,
      new_password: next
    };
    setChangingPassword(true);
    setError('');
    try {
      const result = await api<ChangePasswordResponse>('/api/v2/auth/password', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setCurrentPassword('');
      setNewPassword('');
      setConfirmPassword('');
      toast.push(`密码已更新，已退出其他 ${result.invalidated_sessions} 个会话`, 'ok');
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`修改密码失败：${message}`, 'error');
    } finally {
      setChangingPassword(false);
    }
  };

  return (
    <section className="settingsPanel">
      {confirmImport && importReport && (
        <ConfirmDanger
          title="确认导入配置"
          confirmText="确认导入"
          onCancel={() => setConfirmImport(false)}
          onConfirm={applyImport}
          body={(
            <div className="dangerCopy">
              <p>将只应用 dry-run accepted 的配置字段；rejected 会由后端跳过。</p>
              <code>accepted {importReport.accepted.length} · rejected {importReport.rejected.length}</code>
            </div>
          )}
        />
      )}
      <div className="settingsToolbar">
        <button className="btn ghost" onClick={load} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新配置'}
        </button>
        <button className="btn" onClick={save} disabled={saving}>
          <Save size={16} />
          {saving ? '保存中' : '保存全部'}
        </button>
      </div>

      {error && <div className="notice warn whitespaceNotice">{error}</div>}

      <div className="settingsGrid">
        <section className="settingsBlock">
          <h2>Emby 连接</h2>
          <label>
            <span>地址</span>
            <input className="input" aria-label="Emby 地址" value={embyUrl} onChange={(event) => setEmbyUrl(event.target.value)} />
          </label>
          <label>
            <span>API Key</span>
            <input
              className="input"
              aria-label="Emby API Key"
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder={maskedApiKey ? '已设置，留空保留；输入新值会替换' : 'Emby 控制台生成的 API Key'}
            />
          </label>
        </section>

        <section className="settingsBlock">
          <h2>TMDb</h2>
          <label>
            <span>地址</span>
            <input
              className="input"
              aria-label="TMDb 地址"
              value={tmdbBaseUrl}
              onChange={(event) => setTmdbBaseUrl(event.target.value)}
            />
          </label>
          <label>
            <span>API Key</span>
            <input
              className="input"
              aria-label="TMDb API Key"
              value={tmdbApiKey}
              onChange={(event) => setTmdbApiKey(event.target.value)}
              placeholder={maskedTmdbApiKey ? '已设置，留空保留；输入新值会替换' : '用于追更检查和缺集对照'}
            />
          </label>
          <label>
            <span>超时 秒</span>
            <input
              className="input"
              aria-label="TMDb 超时秒数"
              inputMode="numeric"
              value={tmdbTimeout}
              onChange={(event) => setTmdbTimeout(event.target.value)}
            />
          </label>
        </section>

        <section className="settingsBlock">
          <h2>TG Resource API</h2>
          <div className="settingsStatus">
            <Globe2 size={16} />
            {maskedTgResourceApiToken ? 'Token 已设置，保存空输入不会覆盖' : '默认无需认证'}
          </div>
          <label>
            <span>地址</span>
            <input
              className="input"
              aria-label="TG Resource API 地址"
              value={tgResourceApiBaseUrl}
              onChange={(event) => setTgResourceApiBaseUrl(event.target.value)}
            />
          </label>
          <label>
            <span>Token</span>
            <input
              className="input"
              aria-label="TG Resource API Token"
              value={tgResourceApiToken}
              onChange={(event) => setTgResourceApiToken(event.target.value)}
              placeholder={maskedTgResourceApiToken ? '已设置，留空保留；输入新值会替换' : 'API_TOKEN 启用时填写'}
            />
          </label>
        </section>

        <section className="settingsBlock">
          <h2>115 Cookie</h2>
          <div className="settingsStatus">
            <ShieldCheck size={16} />
            {c115CookieSet ? '已设置，保存空输入不会覆盖' : '未设置'}
          </div>
          <label>
            <span>Cookie</span>
            <textarea
              className="input settingsSecret"
              aria-label="115 Cookie"
              value={c115Cookie}
              onChange={(event) => setC115Cookie(event.target.value)}
              placeholder="UID=...; CID=...; SEID=..."
            />
          </label>
        </section>
      </div>

      <section className="settingsBlock">
        <div className="settingsBlockHead">
          <h2>115 库目录 cid 映射</h2>
          <div>
            <button className="btn ghost compact" onClick={addCidRow}>添加行</button>
            <button className="btn ghost compact" onClick={detectCid} disabled={detecting}>
              <SearchCheck size={14} />
              {detecting ? '检测中' : '自动检测'}
            </button>
          </div>
        </div>
        <div className="cidRows">
          {sortedRows.map((row, index) => (
            <div className="cidRow" key={`${row.lib}-${index}`}>
              <input
                className="input"
                aria-label={`库名 ${index + 1}`}
                value={row.lib}
                onChange={(event) => patchCidRow(index, { lib: event.target.value })}
                placeholder="库名"
              />
              <input
                className="input"
                aria-label={`${row.lib || `第 ${index + 1} 行`} cid`}
                inputMode="numeric"
                value={row.cid}
                onChange={(event) => patchCidRow(index, { cid: event.target.value })}
                placeholder="115 cid"
              />
              <button className="btn ghost compact" onClick={() => removeCidRow(index)}>移除</button>
              {row.hint && <small>{row.hint}</small>}
            </div>
          ))}
        </div>
        {detectResult && <div className="settingsHint">自动检测扫描 {detectResult.scanned} 个目录，单匹配且空 cid 的行已填入。</div>}
      </section>

      <div className="settingsGrid">
        <section className="settingsBlock">
          <h2>自动 strm</h2>
          <div className="settingsStatus">
            <Webhook size={16} />
            {webhookSecretSet ? 'Webhook 密钥已设置，保存空输入不会覆盖' : 'Webhook 密钥未设置'}
          </div>
          <label className="switchRow settingsSwitch">
            <input type="checkbox" checked={autoEnabled} onChange={(event) => setAutoEnabled(event.target.checked)} />
            <span>启用自动 strm</span>
          </label>
          <label className="switchRow settingsSwitch">
            <input type="checkbox" checked={autoFullauto} onChange={(event) => setAutoFullauto(event.target.checked)} />
            <span>全自动入库</span>
          </label>
          <label>
            <span>CD2 挂载前缀</span>
            <input className="input" aria-label="CD2 挂载前缀" value={cd2Prefix} onChange={(event) => setCd2Prefix(event.target.value)} />
          </label>
          <label>
            <span>防抖窗口 秒</span>
            <input className="input" aria-label="自动 strm 防抖秒数" inputMode="numeric" value={autoDebounce} onChange={(event) => setAutoDebounce(event.target.value)} />
          </label>
          <label>
            <span>webhook 密钥</span>
            <input className="input" aria-label="CD2 webhook 密钥" value={cd2Secret} onChange={(event) => setCd2Secret(event.target.value)} placeholder="留空不修改已设置密钥" />
          </label>
          <div className="settingsActionRow">
            <button type="button" className="btn ghost compact" onClick={() => setCd2Secret(generateSecret())}>
              <Shuffle size={14} />
              生成密钥
            </button>
            <button type="button" className="btn ghost compact" onClick={copyWebhookUrl} disabled={!cd2Secret.trim()}>
              <Copy size={14} />
              复制 URL
            </button>
            <button type="button" className="btn ghost compact" onClick={refreshAutostrmStatus} disabled={loadingAutostrmStatus}>
              <RefreshCw size={14} />
              {loadingAutostrmStatus ? '刷新中' : '刷新状态'}
            </button>
          </div>
          <code className="settingsUrl">{cd2Secret.trim() ? copyableWebhookUrl : `${window.location.origin}/api/v2/autostrm/webhook?key=...`}</code>
          <div className="settingsHint">CloudDrive2 webhook 使用 POST；密钥可放 query `key`，也可放 `X-Webhook-Secret` header。</div>
          {autostrmStatus && (
            <div className="miniStats settingsMiniStats">
              <span>seen <strong>{count(autostrmStatus.seen.total)}</strong></span>
              <span>seen 库 <strong>{count(autostrmStatus.seen.libraries)}</strong></span>
              <span>unmatched <strong>{count(autostrmStatus.unmatched.total)}</strong></span>
              <span>最近 seen <strong>{dateText(autostrmStatus.seen.last_seen_at)}</strong></span>
            </div>
          )}
        </section>

        <section className="settingsBlock">
          <h2>账户与安全</h2>
          <label>
            <span>反代信任 IP</span>
            <input
              className="input"
              aria-label="反代信任 IP"
              value={trustedProxies}
              onChange={(event) => setTrustedProxies(event.target.value)}
              placeholder="192.168.2.1, 10.0.0.1"
            />
          </label>
          <div className="notice settingsInlineNotice">
            Docker 版路径由 `.env` 控制，例如 `EMBY_MANAGER_STRM_ROOT` 和 `EMBY_MANAGER_MEDIA_ROOT`。这些运行时路径改完需要重建容器。
          </div>
          <div className="settingsDivider" />
          <div className="settingsBlockHead">
            <h2>修改登录密码</h2>
            <KeyRound size={16} />
          </div>
          <label>
            <span>当前密码</span>
            <input
              className="input"
              type="password"
              aria-label="当前密码"
              autoComplete="current-password"
              value={currentPassword}
              onChange={(event) => setCurrentPassword(event.target.value)}
            />
          </label>
          <label>
            <span>新密码</span>
            <input
              className="input"
              type="password"
              aria-label="新密码"
              autoComplete="new-password"
              value={newPassword}
              onChange={(event) => setNewPassword(event.target.value)}
            />
          </label>
          <label>
            <span>确认新密码</span>
            <input
              className="input"
              type="password"
              aria-label="确认新密码"
              autoComplete="new-password"
              value={confirmPassword}
              onChange={(event) => setConfirmPassword(event.target.value)}
            />
          </label>
          <button
            type="button"
            className="btn ghost compact"
            onClick={changePassword}
            disabled={changingPassword || !currentPassword || !newPassword || !confirmPassword}
          >
            <KeyRound size={14} />
            {changingPassword ? '修改中' : '更新密码'}
          </button>
        </section>
      </div>

      <section className="settingsBlock">
        <h2>高级 JSON</h2>
        <textarea
          className="input settingsJson"
          aria-label="高级 JSON"
          value={extraJson}
          onChange={(event) => setExtraJson(event.target.value)}
        />
      </section>

      <section className="settingsBlock">
        <div className="settingsBlockHead">
          <h2>配置导出 / 导入</h2>
          <div>
            <button className="btn ghost compact" onClick={exportConfig} disabled={exporting}>
              <Download size={14} />
              {exporting ? '导出中' : '导出'}
            </button>
            <button className="btn ghost compact" onClick={copyImportJson} disabled={!importJson.trim()}>
              <Copy size={14} />
              复制
            </button>
          </div>
        </div>
        <textarea
          className="input settingsJson"
          aria-label="导入配置 JSON"
          value={importJson}
          onChange={(event) => setImportText(event.target.value)}
          placeholder='粘贴 {"settings": {...}}，或点击导出把当前配置放到这里'
        />
        <div className="settingsBlockHead">
          <div className="settingsHint">
            <ClipboardCheck size={16} />
            dry-run 只预检，不会写入；确认导入会再次显式确认。
          </div>
          <div>
            <button className="btn ghost compact" onClick={dryRunImport} disabled={importing || !importJson.trim()}>
              {importing ? '处理中' : 'dry-run 预检'}
            </button>
            <button className="btn danger compact" onClick={requestApplyImport} disabled={importing || !canApplyImport}>
              确认导入
            </button>
          </div>
        </div>
        {importReport && <ImportReport report={importReport} />}
      </section>
    </section>
  );
}
