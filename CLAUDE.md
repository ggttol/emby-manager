# CLAUDE.md

给后续 Claude Code 操作本仓库时用。**功能描述、UI、接口表都在 [README.md](README.md);本文件只讲"动手怎么不踩雷"。**

## 一句话定位

NAS 上单进程 Python BaseHTTPServer + 单文件 `index.html`,主打**零外部依赖**(只用标准库 + 浏览器原生 API)。功能见 README。

## 常用命令

```sh
# 测试(本机 Mac 或 NAS 都可)
cd /Users/gaotao/code/emby-manager && python3 -m unittest discover tests
# 只跑一个
python3 -m unittest tests.test_scheduler -v

# 本地起服务(开发)
python3 app.py    # 默认 host=127.0.0.1 port=8097,首次启动建 config.json

# 部署到 NAS(无 sftp/rsync — 用 tar over ssh,见下"陷阱")
tar cf - <files…> | sshpass -p "$NAS_PW" ssh -p 5022 -o ControlMaster=no -o ControlPath=none gaotao@gaotao.cc "tar -C /volume1/docker/emby-manager -xf -"

# 重启(NAS 上)
sshpass -p "$NAS_PW" ssh … "echo $NAS_PW | sudo -S /volume1/docker/emby-manager/manager.sh restart"

# 看运行日志
sshpass … ssh … "tail -50 /volume1/docker/emby-manager/logs/app.log"
```

NAS 凭据见 `~/.claude/projects/-Users-gaotao-code-yw/memory/nas1821-synology-access.md`(本会话已加载)。

## 架构

```
app.py (HTTP handler + main)
 ├─ lib/config       CFG dict(全局可变,跨模块共享,绝不 rebind)
 ├─ lib/logger       logger + LOGS 环形 deque + AppError 业务异常
 ├─ lib/safe         _safe_under() path traversal guard
 ├─ lib/auth         PBKDF2 hash + token + CSRF + login rate-limit + XFF 信任
 ├─ lib/tasks        TASKS dict + run_async(kind, fn, *a)/task_set/cancel
 ├─ lib/emby         eget/epost/edelete + fetch_libs + list_users/_noposter + update_user(限速/限同时播放→用户 Policy)
 ├─ lib/c115         115 webapi:cookie 鉴权 + snap/receive(转存)/auto_cid + 离线下载(magnet/ed2k → offline_add)
 ├─ lib/catalog      115 资源目录:catalog_115.db(~32万条转存链接,只读 sqlite,**.gitignore 不入库**)关键词搜 + 一键转存
 ├─ lib/business     业务编排(扫描/去重/移动/清理/追更/字幕概览/...),**最大模块(~2000 行);所有 *_async 都在这里**
 ├─ lib/scheduler    定时调度:30s 轮询 + 同周期防重入 + 重叠保护(看真实任务状态,见雷区 8)
 ├─ lib/undo         删/移/替换 操作的 undo log(jsonl 持久化)
 └─ lib/__init__     占位

index.html (~4000 行,单文件 SPA,内嵌 UI 组件库 window.UI.{modal/toast/tasks/drawer/Combobox/VirtualList/…})
 └─ static/         iOS apple-touch-icon / PWA manifest / favicon
```

依赖**单向**:`app → {business, catalog}`、`business → {emby, tasks, c115, undo}`、`scheduler` 由 `app` 启动 + **lazy 调** `business.SCHEDULE_KINDS`(避免模块级循环)。**违反则启动报 ImportError**。

## 不能踩的雷(从过往 bug 里提炼)

1. **删除 Emby Item 要先 `edelete`,再动磁盘**。`_del_folder` 先发 `/Library/Media/Updated UpdateType=Deleted` 会让 Emby 异步加锁,紧接的 DELETE `/Items/{id}` silent fail。正确顺序见 `business.delete_item`(2026-05-28 修)。空通知(`done=[]`)**不发** epost,同理。

2. **进度跳变陷阱**:wrapper 函数不要复用别的 `*_async`,后者会 `task_set(tid, total=N)` 重置 total 让前端进度条回退。需要复用时抽出 `_xxx_one` core 函数(参见 `_fix_poster_one`),wrapper 自管 total/progress。

3. **CFG 跨模块共享,不 rebind**:`load_cfg()` 用 `CFG.clear() + CFG.update()`,绝不 `CFG = {...}`,否则其他模块持有的旧引用立刻失效。改 CFG 字段 → 调 `save_cfg()`。

4. **SSH 必须 `ControlMaster=no` + `ControlPath=none`**:NAS DSM 的 sshd 长连接会 255 cascade drop。多路复用是惊喜不是优化。**长任务用 `setsid`/`nohup` 后台跑 + 短连接 poll 日志**,别让一条 ssh 连超过几十秒。

5. **NAS sftp/rsync `--server` 都坏**(Synology 限制),只能用 **tar over ssh**:
   ```
   tar cf - <files> | ssh … "tar -C /dst -xf -"
   ```
   用 gaotao 用户(root + 借 key 会乱)。

6. **DSM 自动封 IP**:同源 SSH 失败 ~10 次后 IP 进黑名单。**两次失败重试间隔 ≥10s**,被封了去「控制面板 → 安全性 → 自动封锁」解。

7. **路径前缀**:Emby 返的 Path 用容器路径(`/strm/...`、`/media/...`),host 上是 `/volume1/strm/...` 和 `/volume1/docker/clouddrive2/CloudNAS/CloudDrive/...`(2026-05-26 后,旧 .spk 路径 `/volume1/CloudNAS/CloudDrive2/115open/emby` **已废**)。映射常量在 `lib/config.CD`/`STRM`/`DOCKER` —— 2026-05-29 起这三个**可在 config.json 用 `cd`/`strm`/`docker` 键覆盖**(换机器/换主机不改代码,缺省回落群晖布局),改后需重启生效。

8. **schedule 重叠保护看的是真实任务状态,不是字符串**:`scheduler._fire` 用 `task_get(last_tid).status == "running"` 判断,**不能**退回成只看持久化的 `last_status == "running"` —— 那样进程重启后 config 残留 `running` 会永久卡死该 schedule(连 run_now 都被挡)。`start()` 里有 `_reconcile_on_start()` 兜底重置残留。改 `_fire` 守卫务必保持"查 TASKS 真实状态 + 守卫与置位在同一 CFG_LOCK 临界区"。

9. **115 风控**:任何 `*_async` 都**不要并发 ffprobe / 下载** CloudDrive2 mount 上的文件 → 触发 115 WAF → mount 挂掉。**strm 生成、metadata copy 都串行**,sleep 0.5s 防过快。

10. **Emby 用户 Policy 字段名认准新版**:同时播放数是 `SimultaneousStreamLimit`(**不是** `MaxActiveSessions` —— 后者在本机 Emby 4.9.5 是死字段,写了静默失效);限速是 `RemoteClientBitrateLimit`(单位 **bps**,UI 用 Mbps 要 ×1e6)。两个都免费版可用,只有硬件转码要 Premiere。改 `update_user`/`list_users` 时别写回旧字段名。

11. **限速靠软件转码生效,会吃 CPU**:`RemoteClientBitrateLimit` 在源码率超上限时触发实时转码;叠加 strm/115 拉流,设太低会频繁转码压垮 NAS。这是产品行为,不是 bug,文案/默认值上别诱导用户设极低值。

12. **`/Library/Media/Updated` 通知:消失的路径发 `Deleted`,不是 `Modified`**。`Modified` 清不掉已不存在路径的 Emby 条目 → 留孤儿重复剧集。`replace_folder`(去重/替换)两个改名方向都要按「被删路径→`Deleted`、被新内容占用的路径→`Modified`/`Created`」发(2026-05-29 修;有 `tests/test_replace_folder.py` 守)。

## 常见任务怎么加

### 加新 endpoint
1. 业务函数写到 `lib/business.py`(同步返 dict,或 `*_async(tid, ...)` 长任务用 `task_set`/`task_is_cancelled`)
2. `app.py` 顶部 `from lib.business import …` 加进来
3. `do_GET`/`do_POST` 里加路由分支:同步直接 `return self._json(fn(...))`;长任务 `return self._json({"tid": run_async("kind", fn, ...args)})`
4. 测试:`tests/test_xxx.py`,用 `unittest.TestCase` + 临时 tmpdir + `patch.object(app, "...", ...)` mock 外部依赖

### 加新定时 kind
1. `lib/business.py`:写 `scheduled_xxx_async(tid)`,先一次性算 grand total 防 task_set 跳变
2. 同文件末 `SCHEDULE_KINDS` 字典加一项:`{kind: {"label": "🔍 ...", "desc": "...", "fn": scheduled_xxx_async}}`
3. UI 不用改 —— `/api/schedules` 自动返新 kind 给前端下拉
4. 测试加在 `tests/test_scheduler.py`(只测 kind 已注册;`_fire` 链路不测,需 mock 全栈)

### 加 config 字段
1. `_DEFAULTS`(`lib/config.py`)加默认值
2. 如果需要数据迁移:写 `_mig_to_vN()` + 加到 `MIGRATIONS = [(N, _mig_to_vN), ...]`
3. `CURRENT_SCHEMA = N` 同步加
4. `/api/config` 的 GET 脱敏(`get_config`)如果新字段含 secret,要 mask
5. 测试加到 `tests/test_config_import_export.py`

### 加新 tab
1. `index.html`:nav 加 `<button data-tab="xxx" onclick="tab('xxx')">…</button>`
2. 同文件加 `<section id="xxx" class="hide">…</section>`
3. `function tab(name)` 加分支 `if(name==='xxx') loadXxx()`
4. 写 `async function loadXxx()` 用 `api('/api/...')` 拿数据,渲染到 section
5. 危险操作走 `UI.modal.confirm({requireType:'删除',...})`
6. 长任务用 `UI.tasks.start(tid, {label, onDone})` 而不是手动轮询

### 改 UI 组件库(`<script id="ui-lib">`)
在 index.html 前部 `window.UI = {modal, toast, tasks, drawer, ...}` 区块。改了**所有 tab 都受影响**,慎重 — 加测试覆盖新行为(JS 没单测,只能手测)。

## 提交 / 部署

- **commit message**:`feat(...)` / `fix(...)` / `docs(...)`,中文 body,可以一个 commit 包多个相关 fix(见历史)
- **commit 末尾加** `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- **不在 main 上开 branch**(项目历史就是 main 直推,本地 git + GitHub 私有 remote `ggttol/emby-manager`)
- **commit 后 push**:用户明确 ask 时再 `git push`
- **NAS 部署 ≠ git push**:tar over ssh 传到 `/volume1/docker/emby-manager/` 再 `manager.sh restart`
