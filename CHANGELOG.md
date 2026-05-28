# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 风格,版本号按 [SemVer](https://semver.org/lang/zh-CN/)。

## [3.0.0] - 2026-05-28

商业化标准 + 体验细化:把 v2.0 的「功能集合」做成「产品体验」,统一组件库 + 危险操作 modal + 全局任务管理 + 后端能力 100% 暴露 + 键盘/主题/移动端打磨。零外部依赖,纯 vanilla。

### Added
- 全局 UI 组件库 `window.UI.*`,内嵌 `index.html`,零外部依赖:
  - **Modal**:`UI.modal.confirm` / `alert` / `form`,`requireType` 打字确认防误删。
  - **Toast**:`UI.toast(msg, {type, action})`,4 种类型 + 可挂「撤销 / 查看 / 重试」action 按钮。
  - **TaskCenter**:全局任务总线,bell 数字 = 活跃任务数,顶部进度条 = `Σprogress / Σtotal`,Drawer 抽屉看详情,跨 tab 不丢。
  - **Drawer**:右/左侧滑出抽屉。
  - **Loading / Skeleton**:`UI.loading.wrap` 包异步 fn,skeleton 替代裸「加载中…」。
  - **EmptyState**:列表空 / 搜索无结果统一占位。
  - **Pagination**:客户端分页,海报 / 缺集等用。
  - **VirtualList**:固定行高 + IntersectionObserver,管理 tab 2 万项不卡。
  - **Combobox**:可搜下拉,替代 500+ 项 `<select>`(缺集 / 管理目标库)。
  - **Banner**:inline 提示条(如「115 cookie 过期,去设置」)。
- Header 顶部右栏:🔍 全局搜索 / 🌓 三态主题切换(auto / light / dark,prefers-color-scheme + 手动 toggle 持久化)/ 🔔 任务 bell(数字 = 活跃数,点击拉 Drawer)/ 👤 用户菜单(改密 / 退出登录)。
- 键盘快捷键全局生效:
  - `/` 或 `Cmd+K` 聚焦全局搜索
  - `?` 弹快捷键帮助
  - `Esc` 关最上层 Modal / Drawer
  - `g` + `(d/s/c/z/g/p/r/m/y/l/u/,)` 跳 12 个 tab(input/textarea focus 时除 Esc 全部屏蔽)
- URL hash 路由:`#tab=manage&lib=动漫&q=xxx`,刷新恢复 tab 与状态,改 tab 用 `history.replaceState`,旧书签兼容。
- 后端 11 个新 endpoint:
  - `POST /api/zhuigeng?async=1` — 追更异步化 + 进度
  - `GET /api/strm_list?lib=&folder=` — 去重区「查看文件列表」展开
  - `POST /api/fixposter_batch?async=1` — 海报批量自动匹配(保守策略:取 name 包含原 name 关键词且有 img 的第一个候选)
  - `POST /api/manage/delete_batch?async=1` — 批量删 + 可取消
  - `POST /api/manage/move_batch?async=1` — 批量跨库移动
  - `POST /api/dedup/exec_batch?async=1` — 多 tmdb 批量去重
  - `POST /api/c115/test_candidate` — 不写 CFG 测候选 cookie
  - `POST /api/c115/auto_cid?async=1` — 自动检测 cid 异步化
  - `GET /api/tasks/list?limit=20` — 列最近任务(系统页 + TaskCenter 恢复用)
  - `GET /api/config/export` — 下载 redacted config json
  - `POST /api/config/import` — schema 校验 + atomic apply
- 撤销系统 UI:日志 tab 新增「Undo 记录」子页,move 操作可一键反向撤销(`POST /api/undo`),delete 提示去 115 还原。
- 删除/移动 tab:VirtualList 全量(2 万项不卡)+ 过滤前缀语法(`tmdb:` `年:2024` `路径:S01`)+ 批量异步 + 撤销 toast(7s)。
- 海报修复:批量自动匹配按钮(走 `/api/fixposter_batch`),候选缩略图 92×138 + lazy load。
- 追更检查:异步 + 进度条 + 「扫所有在更」批量;每行加「刷新元数据」+ 「去 TMDb 看」icon。
- 缺集检查:Combobox 选剧 + 集号点击复制剪贴板 + 「复制缺集列表」按钮。
- 115 转存:文件列表 checkbox 多选/反选(>200 用 VirtualList)、失败行重试、转完成功后「去 X 库扫一下」跳扫描 tab、cookie 失效 Banner 引导。
- 设置:配置导出/导入(敏感字段 redacted)、`trusted_proxies` 列表编辑 UI、API Key + Cookie 默认 `type="password"` 遮罩 + 显示 toggle、cookie 粘贴即时验证。
- 系统页:30 秒自动刷新(可暂停)+ 最近 5 任务卡片;Docker 容器加「重启」按钮(requireType 二次确认)。
- 用户管理:`GET /api/users?withActivity=1` 显示用户最后活跃时间(`LastActivityDate`);新建用户 Modal form 含密码强度。
- 移动端:Modal/Drawer 全屏 + 底部 sheet 模式;Combobox input 16px(防 iOS zoom);TaskCenter bell 在移动端折叠进汉堡菜单。
- 可访问性:所有 button 加 `aria-label`,badge 颜色区分加符号(✓/✗/⚠️/●/○),`:focus-visible` 蓝色描边。
- 新增 CSS 变量:`--warn` / `--danger-bg` / `--ok-bg` / `--skeleton-bg` / `--overlay` / `--focus`。

### Changed
- 所有 `window.confirm()` 替换为 `UI.modal.confirm` Modal(10+ 处),Modal 内显示要影响什么的完整清单;批量 / 不可逆操作 `requireType` 打字确认(批量 >5 项强制)。
- 所有 toast 闪过升级为 `UI.toast`,可带 action 按钮(撤销 / 查看 / 重试);错误 toast 带「详情」→ Modal 显示完整 err / errid 可复制,不再 2 秒丢。
- 批量操作进度从 toast 闪改为全局 TaskCenter(跨 tab 不丢、可取消、bell 数字提示)。
- `/api/scan_all` 前端从同步 loop 改为异步 TaskCenter(注册 tid + 顶部进度条)。
- 扫描 tab:扫描中按钮变「取消」→ `/api/task/cancel`;大库(>500)无 keyword 弹 Modal 警告。
- 改密码强制旧密码校验(`set_config` 校验);grace 模式:首次升级允许一次无旧密码改密(`last_password_change_at` 字段不存在时)。
- `/api/me` 多返 `{username:"admin"}`。
- 所有响应加 `X-Server-Version` header。
- schema v3 → v4:加 `last_password_change_at` 字段;`_mig_to_v4` 首次升级 noop。
- 仪表盘:库卡片加 type icon / 最近扫描时间 / 点击跳管理 tab;`excluded` 库挪到底部 Drawer;顶部加 `/health` 卡片(Emby / 115 / 挂载 / Uptime)。
- 日志 tab:级别 chip(WARN/ERROR 红)+ 搜索框过滤 + 自动刷新 toggle + 下载 .log 按钮 + 「操作日志」「Undo 记录」双子页。

### Security
- 危险操作 Modal `requireType` 打字防误触:批量删 / 删用户 / 配置导入 / Docker 容器重启 / 去重「确认去重」均要求打字「删除」或库名 / 用户名才能确认。
- API Key + 115 Cookie input 默认 `type="password"` 遮罩,显示需手动 toggle。
- `/api/config/export` 剔除 `password_hash` 和 `c115_cookie` raw,替换为 `"<redacted>"`。
- `/api/config/import` schema 校验 + atomic apply;不接受被替换为 `<redacted>` 的字段(保留原值);schema 不匹配时拒绝并显示完整 diff。
- 反代信任 IP 白名单(`trusted_proxies`):只信白名单内源的 `X-Forwarded-For` 头(防伪造客户端 IP 绕限流)。

---

## [2.0.0] - 2026-05-28

加固版:从「能用」到「能给亲戚部署」。

### Security
- Path traversal guard:所有暴露给前端的文件操作参数过 `_safe_join`,拒绝 `..` / 绝对路径 / 超出 base 的 realpath。
- 密码 PBKDF2-SHA256 + per-user salt + 120k iter,只存 hash;旧版明文 `password` 字段启动时自动迁移并删除。
- 登录 IP 限流:同源 5 分钟 10 次失败 → 429。
- 默认 bind `127.0.0.1`,不主动暴露公网;`host=0.0.0.0` 且无密码时启动拒绝监听。
- `config.json` 原子 rename + `chmod 0o600`,只 owner 可读。
- HttpOnly cookie + CSRF token + CSP header(脚本源限 `'self'`,内联走 nonce)。

### Added
- 异步任务管理器:scan_all / c115_snap / c115_save 异步化,前端进度条 + 可取消。
- `/health` 端点:Emby 可达性 + 磁盘 + 任务管理器状态。
- Undo log:删除 / 移动操作可撤销。
- Config schema 迁移(`migrate_cfg`):启动时自动升 `schema_version`。
- 单元测试:覆盖纯函数(path 安全、cfg migrate、TMDb 解析)。

### Changed
- 模块拆分:emby / c115 / tasks / auth / business 各成模块。
- stdlib `logging` 替代散落的 `print`,分级 + 文件 + 内存环形缓冲。

---

## [1.12.0] - 2026-05-28

### Added
- 删除 / 移动支持多选。

## [1.11.2] - 2026-05-28

### Added
- 115 转存支持多链接批量。

## [1.11.1] - 2026-05-28

### Fixed
- 115「分享内无可转存文件」:文件夹要用 `cid` 不是 `fid`,之前 API 调用拿错字段导致空列表。

## [1.11.0] - 2026-05-28

### Added
- 115 转存 tab:粘贴分享链接 → snap 列文件 → receive 到指定库 cid。

## [1.10.0] - 2026-05-28

### Added
- 去重 review 区可单行删除。
- 显示集号差异(本地 vs TMDb)。

## [1.9.0] - 2026-05-28

### Fixed
- 去重三 bug:
  - 排序加 `-n`(自然数序,避免 `S10E01` 排在 `S2E01` 前)。
  - 追更库项目也进 review,不再被静默跳过。
  - 集数倒挂保护:本地集数 > TMDb 已知集数时不误判为冗余。

## [1.8.0] - 2026-05-28

### Added
- scan 集成自动清孤儿 strm(115 源文件没了的 strm 自动删)。

## [1.7.0] - 2026-05-27

### Added
- 绝对集号模式(动漫常用,跨季连续编号)。

### Fixed
- tab 切换 bug:切回已加载 tab 不再丢状态。
- scan 兼容已知文件夹:重复扫不再报「文件夹已存在」。

## [1.6.0] - 2026-05-27

### Added
- 缺集检查:对照 TMDb 季集表列本地缺失集号。

## [1.5.0] - 2026-05-27

### Added
- 追更检查:拉 TMDb `status` 标红「应追更但本地没新集」。

## [1.4.0] - 2026-05-27

### Added
- 用户管理 tab(Emby 用户增删改、maxsessions、disabled)。
- 设置 tab(`config.json` 持久化,可改 Emby / API Key / 密码)。

## [1.3.0] - 2026-05-27

### Added
- 库列表动态从 Emby `VirtualFolders` API 读,不再硬编码。
- 开机自启脚本(`manager.sh` + `rc.d/emby_manager.sh`)。

## [1.2.0] - 2026-05-27

### Added
- 系统健康页(Docker 容器、磁盘、内存、负载)。
- 一键扫全库按钮。

## [1.1.0] - 2026-05-27

### Added
- Emby API 反代:前端 `/emby/*` 透传到后端 Emby,避免跨域。

## [1.0.0] - 2026-05-27

### Added
- 初版,7 tab:仪表盘 / 扫描 / 海报修复 / 去重 / 删除·移动 / 系统 / 日志。
