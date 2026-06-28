import { RefreshCw, Save, SearchCheck, ShieldCheck } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useToast } from './Toast';

type C115AutoCidResponse = components['schemas']['C115AutoCidResponse'];
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
const knownKeys = new Set([
  'emby_url',
  'api_key',
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

function extraSettings(config: ConfigResponse) {
  const extra = Object.fromEntries(
    Object.entries(config.settings).filter(([key]) => !knownKeys.has(key))
  );
  return JSON.stringify(extra, null, 2);
}

export function SettingsPanel() {
  const [config, setConfig] = useState<ConfigResponse>({ settings: {} });
  const [libraries, setLibraries] = useState<EmbyLibrary[]>([]);
  const [embyUrl, setEmbyUrl] = useState(defaultEmbyUrl);
  const [apiKey, setApiKey] = useState('');
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
  const toast = useToast();

  const maskedApiKey = config.settings.api_key === '***';
  const sortedRows = useMemo(() => cidRows, [cidRows]);

  const hydrate = (nextConfig: ConfigResponse, nextLibraries: EmbyLibrary[]) => {
    setConfig(nextConfig);
    setLibraries(nextLibraries);
    setEmbyUrl(stringValue(nextConfig.settings.emby_url, defaultEmbyUrl));
    setApiKey(stringValue(nextConfig.settings.api_key));
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
    const settings: Record<string, unknown> = {
      ...safeExtraJson(extraJson),
      emby_url: embyUrl.trim() || defaultEmbyUrl,
      api_key: apiKey.trim() || (maskedApiKey ? '***' : ''),
      c115_cid_map: sanitizeCidRows(cidRows),
      trusted_proxies: splitTrustedProxies(trustedProxies),
      auto_strm_enabled: autoEnabled,
      auto_strm_fullauto: autoFullauto,
      cd2_mount_prefix: cd2Prefix.trim() || '/CloudNAS/CloudDrive',
      auto_strm_debounce_sec: debounce
    };
    if (c115Cookie.trim()) settings.c115_cookie = c115Cookie.trim();
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

  return (
    <section className="settingsPanel">
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
              placeholder={maskedApiKey ? '已设置，留空会清空；保持 *** 可保留原值' : 'Emby 控制台生成的 API Key'}
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
    </section>
  );
}
