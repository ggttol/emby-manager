# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目定位

`emby-manager` 是给 NAS 上自管 Emby 使用的单进程管理工具：后端是 Python 标准库 `BaseHTTPServer` 风格 HTTP 服务，前端是单文件 `index.html` SPA，项目目标是**零外部运行依赖**（Python 标准库 + 浏览器原生 API）。功能、UI、接口表和 NAS/Emby/CloudDrive2 部署架构以 `README.md` 为准；本文件只记录后续改代码时需要快速知道的命令、架构边界和容易踩坑的实现约束。

## 常用命令

```sh
# 运行全部单元测试（纯 unittest，零外部依赖）
python3 -m unittest discover tests

# 等价的项目测试脚本
./tests/run.sh

# 跑单个测试文件
python3 -m unittest tests.test_scheduler -v
python3 -m unittest tests.test_http_handler -v

# 直接运行某个 test_*.py
python3 tests/test_qscore.py -v

# 本地开发启动服务；默认 host=127.0.0.1 port=8097，首次启动生成 config.json
python3 app.py

# 部署到 NAS：此项目环境不要用 sftp/rsync，使用 tar over ssh
# 把 <files...> 替换成要部署的文件；远端路径通常是 /volume1/docker/emby-manager/
tar cf - <files...> | ssh -p <SSH_PORT> -o ControlMaster=no -o ControlPath=none <USER>@<HOST> \
  "tar -C /volume1/docker/emby-manager -xf -"

# NAS 上重启服务（根据实际 sudo/manager.sh 配置调整）
ssh -p <SSH_PORT> -o ControlMaster=no -o ControlPath=none <USER>@<HOST> \
  "sudo /volume1/docker/emby-manager/manager.sh restart"
```

没有构建步骤，也没有单独的 lint/format 工具配置；改动后主要靠 `python3 -m unittest discover tests` 验证。JS 没有自动化单测，改 `index.html` 尤其是 UI 组件库时需要手测。

## 高层架构

```text
app.py
  ├─ HTTP handler、静态文件白名单、路由分发、启动入口
  ├─ 启动时加载/迁移 config，启动 scheduler
  └─ 路由层只做鉴权、参数解析、调用业务函数、JSON 响应

lib/config.py
  ├─ 全局 CFG dict、schema migration、配置原子保存
  └─ NAS 路径常量 CD / STRM / DOCKER，可由 config.json 覆盖

lib/auth.py
  ├─ PBKDF2 密码 hash、登录 token、CSRF
  └─ 登录限流、trusted_proxies / X-Forwarded-For

lib/tasks.py
  └─ TASKS 进程内任务表、run_async、task_set、cancel

lib/emby.py
  └─ Emby HTTP API 封装：eget / epost / edelete、库/用户/海报相关 API

lib/c115.py
  └─ 115 Web API：cookie 鉴权、分享 snap、receive 转存、离线下载

lib/catalog.py
  └─ catalog_115.db 资源目录查询；仓库里的数据库只是空模板

lib/business.py
  └─ 最大业务编排层：扫描、去重、删除、移动、清理、追更、海报、strm 等

lib/scheduler.py
  └─ 定时任务 30s 轮询、同周期防重入、重叠保护

lib/undo.py
  └─ 删除/移动/替换的 undo_log.jsonl 持久化

index.html
  ├─ 单文件 SPA
  └─ 内嵌 window.UI 组件库：modal / toast / tasks / drawer / Combobox / VirtualList 等
```

依赖方向保持单向：`app -> {business, catalog}`，`business -> {emby, tasks, c115, undo}`。`scheduler` 由 `app` 启动，并 lazy 读取 `business.SCHEDULE_KINDS`，避免模块级循环 import。改动时不要让底层模块反向 import `app.py`。

## 本地数据和公开仓库约定

- `config.json`、`undo_log.jsonl`、`logs/` 都是本地运行数据，不应提交。
- `catalog_115.db` 在仓库中是**空模板库**，只保留表结构：
  ```sql
  CREATE TABLE catalog(name TEXT, sheet TEXT, link TEXT, is_pkg INT, link_type TEXT);
  CREATE INDEX idx_catalog_link_type on catalog(link_type);
  ```
  真实 115 资源目录数据需要用户在自己的 NAS/本地导入，不要把真实数据提交到公开仓库。
- `catalog_115_validation.db`、WAL/SHM、备份库、日志和批量处理临时产物都属于本地数据。

## 关键实现约束

1. **CFG 不能 rebind**：`CFG` 是跨模块共享的 mutable dict。`load_cfg()` 必须 `CFG.clear(); CFG.update(...)`，不要写 `CFG = {...}`。改配置字段后调用 `save_cfg()`；路径 `cd` / `strm` / `docker` 改完通常需要重启，让已 import 的模块拿到新常量。

2. **长任务进度不要复用会重置 total 的 wrapper**：外层 `*_async` 如果调用另一个会 `task_set(tid, total=N)` 的 `*_async`，前端进度会回退。需要复用时抽出 `_xxx_one` core，外层自己维护 total/progress。

3. **删除 Emby Item 的顺序**：先调用 `edelete` 删除 Emby Item，再动磁盘/strm；不要先发 `/Library/Media/Updated UpdateType=Deleted`，否则 Emby 可能异步加锁导致 DELETE silent fail。空通知列表不要发 `epost`。

4. **Emby 路径映射要区分容器路径和 NAS 路径**：Emby 返回的 Path 常是 `/strm/...` 或 `/media/...`；NAS 主机上对应 `STRM`（默认 `/volume1/strm`）和 `CD`（默认 `/volume1/docker/clouddrive2/CloudNAS/CloudDrive`）。相关逻辑看 `lib/config.py` 的 `CD` / `STRM`。

5. **115 / CloudDrive2 挂载不要并发重 IO**：任何 `*_async` 都不要在 CloudDrive2 mount 上并发 ffprobe、大量下载或密集扫描；strm 生成、metadata copy、字幕相关任务应串行或限速，必要时 sleep，避免触发 115 风控或挂载异常。

6. **定时任务重叠保护看真实 TASKS 状态**：`scheduler._fire` 需要用 `task_get(last_tid).status == "running"` 判断，而不是只看持久化的 `last_status == "running"`。否则进程重启后旧 config 残留会永久卡死 schedule。守卫与置位要在同一 `CFG_LOCK` 临界区。

7. **Emby 用户 Policy 字段**：同时播放限制用 `SimultaneousStreamLimit`；限速用 `RemoteClientBitrateLimit`，单位是 bps，UI 的 Mbps 要乘 `1e6`。不要只写旧字段 `MaxActiveSessions`。

8. **限速会触发转码**：`RemoteClientBitrateLimit` 在源码率超过上限时让 Emby 实时转码，NAS CPU 压力会增加；UI 文案和默认值不要诱导用户设极低码率。

9. **媒体更新通知语义**：不存在/被删的路径发 `UpdateType=Deleted`，被新内容占用或变更的路径才发 `Modified` / `Created`。`replace_folder` 这类双向改名逻辑尤其要保持这个语义，相关测试在 `tests/test_replace_folder.py`。

10. **HTTP 写接口安全层**：新增 POST/DELETE 路由时要走现有 auth + CSRF 机制；CD2 webhook 这类免登录入口必须有独立 secret，并明确不要走普通 `_auth/_csrf`。

## 常见改动入口

### 新增 API endpoint

1. 业务函数放 `lib/business.py`；同步函数返回 dict，长任务写成 `*_async(tid, ...)` 并使用 `task_set` / `task_is_cancelled`。
2. `app.py` 顶部 import 业务函数。
3. 在 `do_GET` / `do_POST` / `do_DELETE` 对应分支加路由；长任务返回 `{"tid": run_async("kind", fn, ...args)}`。
4. 测试用 `unittest.TestCase`，优先 mock 外部 Emby/115/NAS IO。

### 新增定时任务 kind

1. 在 `lib/business.py` 写 `scheduled_xxx_async(tid)`，先计算 grand total，避免进度 total 跳变。
2. 在 `SCHEDULE_KINDS` 注册 `{kind: {"label": ..., "desc": ..., "fn": scheduled_xxx_async}}`。
3. UI 下拉来自 `/api/schedules`，通常不需要改 `index.html`。
4. 在 `tests/test_scheduler.py` 或相关测试中覆盖注册和调度边界。

### 新增 config 字段

1. `_DEFAULTS` 添加通用默认值；secret 或需要区分“未配置”的字段不要随意放默认非空值。
2. 需要迁移时写 `_mig_to_vN()`，加入 `MIGRATIONS`，并更新 `CURRENT_SCHEMA`。
3. `/api/config` GET / export 要对 secret 字段脱敏。
4. 测试放 `tests/test_config_import_export.py` 或相邻配置测试。

### 新增前端 tab

1. `index.html` nav 加 `<button data-tab="xxx" onclick="tab('xxx')">...`。
2. 加 `<section id="xxx" class="hide">...`。
3. `tab(name)` 增加 `loadXxx()` 分支。
4. `loadXxx()` 使用现有 `api('/api/...')` 和 `window.UI` 组件渲染。
5. 危险操作走 `UI.modal.confirm({requireType: ...})`；长任务走 `UI.tasks.start(tid, {label, onDone})`，不要手写轮询。

### 修改 UI 组件库

`index.html` 前部 `<script id="ui-lib">` 定义全局 `window.UI`。这里影响所有 tab；改动后至少手动验证 modal、toast、task drawer、移动端布局和当前触达的 tab。

## NAS 部署注意事项

- DSM 环境里 SSH 连接不要启用 ControlMaster：命令加 `-o ControlMaster=no -o ControlPath=none`。
- sftp/rsync 在目标 NAS 上可能不可用；部署用 tar over ssh。
- 长任务不要绑在一条 SSH 会话里跑；用 `setsid` / `nohup` 后台跑，再短连接轮询日志。
- 多次 SSH 失败可能触发 DSM 自动封锁；排错时避免密集重试。

## 提交约定

- commit message 用 `feat(...)` / `fix(...)` / `docs(...)` / `chore(...)`，中文 body 可以。
- 当前项目历史习惯在 `main` 直接提交；只有用户明确要求 push 时才 `git push`。
- NAS 部署和 GitHub push 是两件事：push 不会更新 NAS，部署需要 tar over ssh 到 `/volume1/docker/emby-manager/` 并重启服务。
