import { RefreshCw, Save } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { useToast } from './Toast';

type UserSummary = components['schemas']['UserSummary'];
type UsersResponse = components['schemas']['UsersResponse'];
type UpdateUserPolicyRequest = components['schemas']['UpdateUserPolicyRequest'];
type UpdateUserPolicyResponse = components['schemas']['UpdateUserPolicyResponse'];

type Draft = {
  remoteBitrateMbps: string;
  simultaneousStreamLimit: string;
  disabled: boolean;
};

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function draftFromUser(user: UserSummary): Draft {
  return {
    remoteBitrateMbps: user.remote_bitrate_mbps == null ? '' : String(user.remote_bitrate_mbps),
    simultaneousStreamLimit:
      user.policy.SimultaneousStreamLimit == null ? '' : String(user.policy.SimultaneousStreamLimit),
    disabled: user.disabled
  };
}

function parseNumberField(value: string, label: string) {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed) || parsed < 0) {
    throw new Error(`${label}必须是非负数字`);
  }
  return parsed;
}

function lastActivity(value?: string | null) {
  if (!value) return '未记录';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  });
}

export function UsersPanel() {
  const [users, setUsers] = useState<UserSummary[]>([]);
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  const [loading, setLoading] = useState(true);
  const [savingId, setSavingId] = useState<string | null>(null);
  const [error, setError] = useState('');
  const toast = useToast();

  const sortedUsers = useMemo(() => [...users].sort((a, b) => a.name.localeCompare(b.name, 'zh-CN')), [users]);

  const load = async () => {
    setLoading(true);
    setError('');
    try {
      const data = await api<UsersResponse>('/api/v2/users');
      setUsers(data.users);
      setDrafts(Object.fromEntries(data.users.map((user) => [user.id, draftFromUser(user)])));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      toast.push(`用户列表加载失败：${message}`, 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const patchDraft = (id: string, patch: Partial<Draft>) => {
    setDrafts((prev) => ({
      ...prev,
      [id]: {
        ...(prev[id] || { remoteBitrateMbps: '', simultaneousStreamLimit: '', disabled: false }),
        ...patch
      }
    }));
  };

  const save = async (user: UserSummary) => {
    const draft = drafts[user.id] || draftFromUser(user);
    let payload: UpdateUserPolicyRequest;
    try {
      payload = {
        remote_bitrate_mbps: parseNumberField(draft.remoteBitrateMbps, '远程限速'),
        simultaneous_stream_limit: parseNumberField(draft.simultaneousStreamLimit, '同时播放数'),
        disabled: draft.disabled
      };
    } catch (e) {
      toast.push(errorMessage(e), 'warn');
      return;
    }

    setSavingId(user.id);
    try {
      const data = await api<UpdateUserPolicyResponse>(`/api/v2/users/${encodeURIComponent(user.id)}/policy`, {
        method: 'PUT',
        body: JSON.stringify(payload)
      });
      setUsers((prev) => prev.map((item) => (item.id === user.id ? data.user : item)));
      setDrafts((prev) => ({ ...prev, [user.id]: draftFromUser(data.user) }));
      toast.push(`已保存 ${data.user.name} 的用户策略`, 'ok');
    } catch (e) {
      toast.push(`保存用户策略失败：${errorMessage(e)}`, 'error');
    } finally {
      setSavingId(null);
    }
  };

  return (
    <section className="usersPanel">
      <div className="panelActions usersToolbar">
        <button className="btn ghost" onClick={load} disabled={loading}>
          <RefreshCw size={16} />
          {loading ? '加载中' : '刷新用户'}
        </button>
        <span>{users.length ? `${users.length} 个 Emby 用户` : '等待用户数据'}</span>
      </div>
      {error && <div className="notice warn">{error}</div>}
      <div className="usersTableWrap">
        <table className="dataTable usersTable">
          <thead>
            <tr>
              <th>用户</th>
              <th>远程限速 Mbps</th>
              <th>同时播放</th>
              <th>禁用</th>
              <th>最后活动</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {sortedUsers.map((user) => {
              const draft = drafts[user.id] || draftFromUser(user);
              return (
                <tr key={user.id}>
                  <td>
                    <strong>{user.name}</strong>
                    <small>{user.id}</small>
                  </td>
                  <td>
                    <input
                      aria-label={`${user.name} 远程限速 Mbps`}
                      className="input compactInput"
                      inputMode="decimal"
                      value={draft.remoteBitrateMbps}
                      onChange={(event) => patchDraft(user.id, { remoteBitrateMbps: event.target.value })}
                    />
                  </td>
                  <td>
                    <input
                      aria-label={`${user.name} 同时播放数`}
                      className="input compactInput"
                      inputMode="numeric"
                      value={draft.simultaneousStreamLimit}
                      onChange={(event) => patchDraft(user.id, { simultaneousStreamLimit: event.target.value })}
                    />
                  </td>
                  <td>
                    <label className="switchRow">
                      <input
                        type="checkbox"
                        checked={draft.disabled}
                        onChange={(event) => patchDraft(user.id, { disabled: event.target.checked })}
                      />
                      <span>{draft.disabled ? '禁用' : '启用'}</span>
                    </label>
                  </td>
                  <td>{lastActivity(user.last_activity_date)}</td>
                  <td>
                    <button className="btn compact" onClick={() => save(user)} disabled={savingId === user.id}>
                      <Save size={14} />
                      {savingId === user.id ? '保存中' : '保存'}
                    </button>
                  </td>
                </tr>
              );
            })}
            {!loading && sortedUsers.length === 0 && (
              <tr>
                <td colSpan={6} className="emptyCell">没有读取到 Emby 用户</td>
              </tr>
            )}
            {loading && sortedUsers.length === 0 && (
              <tr>
                <td colSpan={6} className="emptyCell">正在加载用户...</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}
