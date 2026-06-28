import { RefreshCw, Save, Trash2, UserPlus } from 'lucide-react';
import { useEffect, useMemo, useState, type FormEvent } from 'react';
import { api } from '../api/client';
import type { components } from '../api/openapi';
import { ConfirmDanger } from './Modal';
import { useToast } from './Toast';

type UserSummary = components['schemas']['UserSummary'];
type UsersResponse = components['schemas']['UsersResponse'];
type UpdateUserPolicyRequest = components['schemas']['UpdateUserPolicyRequest'];
type UpdateUserPolicyResponse = components['schemas']['UpdateUserPolicyResponse'];
type CreateUserRequest = components['schemas']['CreateUserRequest'];
type CreateUserResponse = components['schemas']['CreateUserResponse'];
type DeleteUserResponse = components['schemas']['DeleteUserResponse'];

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

function parseIntegerField(value: string, label: string) {
  const parsed = parseNumberField(value, label);
  if (parsed === undefined) return undefined;
  if (!Number.isInteger(parsed)) {
    throw new Error(`${label}必须是非负整数`);
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
  const [newName, setNewName] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [creating, setCreating] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<UserSummary | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
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

  const createUser = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = newName.trim();
    if (!name) {
      toast.push('用户名不能为空', 'warn');
      return;
    }
    const payload: CreateUserRequest = {
      name,
      password: newPassword.length > 0 ? newPassword : null
    };

    setCreating(true);
    try {
      const data = await api<CreateUserResponse>('/api/v2/users', {
        method: 'POST',
        body: JSON.stringify(payload)
      });
      setUsers((prev) => {
        const exists = prev.some((item) => item.id === data.user.id);
        return exists ? prev.map((item) => (item.id === data.user.id ? data.user : item)) : [...prev, data.user];
      });
      setDrafts((prev) => ({ ...prev, [data.user.id]: draftFromUser(data.user) }));
      setNewName('');
      setNewPassword('');
      toast.push(`已创建 ${data.user.name}，复制用户名${payload.password ? '和密码' : ''}给亲友即可`, 'ok');
    } catch (e) {
      toast.push(`创建用户失败：${errorMessage(e)}`, 'error');
    } finally {
      setCreating(false);
    }
  };

  const save = async (user: UserSummary) => {
    const draft = drafts[user.id] || draftFromUser(user);
    let payload: UpdateUserPolicyRequest;
    try {
      payload = {
        remote_bitrate_mbps: parseNumberField(draft.remoteBitrateMbps, '远程限速'),
        simultaneous_stream_limit: parseIntegerField(draft.simultaneousStreamLimit, '同时播放数'),
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
      setUsers((prev) => prev.map((item) => (item.id === user.id ? { ...item, ...data.user } : item)));
      setDrafts((prev) => ({ ...prev, [user.id]: draftFromUser(data.user) }));
      toast.push(`已保存 ${data.user.name} 的用户策略`, 'ok');
    } catch (e) {
      toast.push(`保存用户策略失败：${errorMessage(e)}`, 'error');
    } finally {
      setSavingId(null);
    }
  };

  const deleteUser = async () => {
    if (!deleteTarget || deletingId) return;
    const target = deleteTarget;
    setDeletingId(target.id);
    try {
      const data = await api<DeleteUserResponse>(`/api/v2/users/${encodeURIComponent(target.id)}`, {
        method: 'DELETE'
      });
      if (!data.ok) {
        throw new Error(`删除失败（${data.code}）`);
      }
      setUsers((prev) => prev.filter((item) => item.id !== target.id));
      setDrafts((prev) => {
        const next = { ...prev };
        delete next[target.id];
        return next;
      });
      setDeleteTarget(null);
      toast.push(`已删除 ${target.name}`, 'ok');
    } catch (e) {
      toast.push(`删除用户失败：${errorMessage(e)}`, 'error');
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <section className="usersPanel">
      <div className="panelActions usersToolbar">
        <form className="userCreateForm" onSubmit={createUser}>
          <input
            aria-label="新用户用户名"
            className="input userCreateName"
            placeholder="用户名"
            value={newName}
            onChange={(event) => setNewName(event.target.value)}
            autoComplete="off"
          />
          <input
            aria-label="新用户密码"
            className="input userCreatePassword"
            placeholder="密码可空"
            type="password"
            value={newPassword}
            onChange={(event) => setNewPassword(event.target.value)}
            autoComplete="new-password"
          />
          <button className="btn compact" type="submit" disabled={creating}>
            <UserPlus size={14} />
            {creating ? '创建中' : '新建用户'}
          </button>
        </form>
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
                    <div className="userActions">
                      <button className="btn compact" onClick={() => save(user)} disabled={savingId === user.id}>
                        <Save size={14} />
                        {savingId === user.id ? '保存中' : '保存'}
                      </button>
                      {!user.admin && (
                        <button
                          className="btn ghost compact dangerText"
                          onClick={() => setDeleteTarget(user)}
                          disabled={deletingId === user.id}
                          aria-label={`删除 ${user.name}`}
                        >
                          <Trash2 size={14} />
                          删除
                        </button>
                      )}
                    </div>
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
      {deleteTarget && (
        <ConfirmDanger
          title="删除用户"
          body={(
            <div className="dangerCopy">
              <p>删除「{deleteTarget.name}」后不可恢复。</p>
              <code>{deleteTarget.name}</code>
            </div>
          )}
          confirmText={`确认删除 ${deleteTarget.name}`}
          onCancel={() => setDeleteTarget(null)}
          onConfirm={deleteUser}
        />
      )}
    </section>
  );
}
