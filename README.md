# Emby Manager

给非技术亲戚装一台 Emby 后,自己用来「日常运维」的网页工具。基于 Emby HTTP API + NAS 本地文件系统操作,**纯 Python 标准库零依赖** —— 一个 `app.py` 加一个 `index.html`,丢进 NAS 任意目录 `python3 app.py` 就跑。

为 strm 架构(strm 文件 + 115 网盘 CloudDrive2 挂载)而生,但纯 Emby 库也能用大部分功能。

**v3.0** 升级了体验:全局 UI 组件库(Modal/Toast/TaskCenter/Drawer/VirtualList/Combobox),危险操作走 Modal + 打字防误删,长任务进 TaskCenter(顶部进度条 + 🔔 bell + 可取消、跨 tab 不丢、刷新自动恢复),键盘快捷键全覆盖,暗/亮三态主题,撤销系统接入 UI,配置导出/导入。后端能力 100% 暴露,lib 模块保持零循环依赖,300+ 单元测试。

**v3.0.x** 增量:智能清理(多维度评分)/ 一条龙加新资源向导 / 追更扫描 + 缺集汇总 / 全库缺集扫描 / 海报错绑检测 / 仪表盘待办 / 空 folder 扫 / 一键自动去重 / 自动 strm + 增量补扫 / ⏰ 定时任务(每日 / 每周 / 每月,5 种 kind)/ 手机加主屏 + PWA standalone(蓝紫「管」图标)。

---

## 功能(16 个 tabs)

| Tab | 用途 |
|---|---|
| 仪表盘 | Emby 在线状态、库卡片(点击跳管理)、待办清单(无海报 / 重复项 / 无评分 等) |
| 扫描 | 单库 / 全库扫描(异步任务,进度条),按关键词扫指定子目录,集成自动清孤儿 strm |
| 115 转存 | 粘贴 115 分享链接 → snap 列文件 → receive 到指定库 cid;支持多链接批量;**一条龙向导**:转存 → 扫 → 等刮削 → 海报+重复检查 → 报告 |
| 找资源 | 本地 `catalog_115.db` 关键词搜资源,115 分享链走转存,magnet/ed2k/http 直链走 115 离线下载 |
| 追更检查 | 拉 TMDb `status` 看哪部剧已完结 / 在播,标红「应追更但本地没新集」;**一键扫所有在更剧** + **缺集求资源清单** |
| 缺集检查 | 对照 TMDb 季集表,列出本地缺失的集号(支持绝对集号模式);**全库缺集扫描** |
| 海报修复 | 列无海报项 → 调 Emby RemoteSearch 给候选 → 一键 Apply TMDb id;**全自动批量**;**错绑检测**(folder 中文 vs emby name 重合度低的疑似绑错) |
| 字幕 | 扫 `/strm` 树统计外挂字幕覆盖情况,不碰 115 挂载 |
| 去重 | 同 TMDb id 多份的项目对照(分辨率 / 容器 / 大小),勾选删冗余;**一键全自动去重**(只删可逆的清晰胜负);**全替换**(新版替老版) |
| 删除·移动 | 单选 / 多选删除项目,或在库之间移动 strm + nfo + 海报;**智能冲突**(归档时按 strm 集数判断保留方) |
| 智能清理 | 多维度评分(⭐评分低 / 📅入库久 / 👁️没人看 / 💾占空间 / 🖼️元数据残缺),维度可勾选,**只算勾选维度**;评分细则可查;一键删除选中 |
| 系统 | Docker 容器列表、磁盘 / 内存 / 负载、Emby 版本;**健康预警**(容器非 Up / 磁盘紧 / Emby 离线 红色高亮)+ 复制系统报告 |
| ⏰ 定时 | 5 种 kind(扫全库 / 扫追更剧 / 海报自动修 / 无评分剧刷新 / 增量监控补扫),触发模式每日 / 每周X / 每月N日 + HH:MM,启停 / 立即跑 / 改 / 删 |
| 日志 | 应用日志环形缓冲(最近 200 条),按级别过滤;**Undo 子页**:可逆操作记录 |
| 用户 | Emby 用户增删改、禁用、活跃度;**单用户限速**(远程串流码率上限 Mbps)+ **限同时播放数**(并发流) |
| 设置 | 改 Emby 地址 / API Key / 登录密码 / 115 cookie / 115 cid 映射 / 反代信任 IP / 配置导出导入 |

## 推荐 NAS 架构

这套工具按「家用 NAS 上自管 Emby + 115 网盘资源」设计，公开仓库不绑定任何私有域名、账号或路径，但推荐拓扑如下：

```text
浏览器 / 手机 PWA
        │ HTTPS
        ▼
NAS 反向代理 / 内网访问入口
        │ http://127.0.0.1:8097
        ▼
emby-manager  单进程 Python HTTP 服务
        │
        ├─ Emby Server API  http://127.0.0.1:8096/emby
        │    ├─ 刷新媒体库 / 删除 Item / RemoteSearch / 用户策略
        │    └─ 媒体库指向 /strm 或 /media 里的实际条目
        │
        ├─ NAS 本地文件系统
        │    ├─ STRM 根目录：默认 /volume1/strm
        │    ├─ CloudDrive2/115 挂载根：默认 /volume1/docker/clouddrive2/CloudNAS/CloudDrive
        │    └─ Docker CLI：默认 /var/packages/ContainerManager/target/usr/bin/docker
        │
        ├─ 115 Web API
        │    ├─ c115_cookie：用于分享链接 snap / receive / 离线下载
        │    └─ c115_cid_map：Emby 库名 → 115 目标文件夹 cid
        │
        └─ 本地运行数据
             ├─ config.json：Emby API Key、115 cookie、路径覆盖、定时任务等
             ├─ undo_log.jsonl：可逆删除/移动记录
             ├─ logs/：运行日志
             └─ catalog_115.db：资源目录数据库，仓库只带空模板
```

几个约定：

- **管理工具和 Emby 放在同一台 NAS 上**：管理工具默认只监听 `127.0.0.1:8097`，通过 NAS 反代或内网访问；Emby API 默认是 `http://127.0.0.1:8096/emby`。
- **媒体路径分两层**：Emby 看到的是容器/服务内路径（例如 `/strm/...`、`/media/...`），NAS 主机上实际路径通常是 `/volume1/strm/...` 和 CloudDrive2 挂载目录。代码里的 `cd` / `strm` / `docker` 都能在 `config.json` 里覆盖，换 NAS 或换挂载点不用改代码，改完重启生效。
- **115 cookie 只用于转存和离线下载**：工具不会内置任何账号；`115 转存` tab 通过用户自己填写的 cookie 调 115 Web API，把分享链接转存到 `c115_cid_map` 指定的目标 cid。
- **资源目录数据库是本地数据**：`catalog_115.db` 用于关键词搜索/一键转存。公开仓库只保留空表结构，真实数据请在自己的 NAS 上导入，不要提交到公开仓库。
- **长任务都在本进程后台线程里跑**：扫库、转存、去重、海报修复、定时任务都会进入任务中心；重启后不会恢复线程本身，但定时任务状态会在启动时做残留修正。
- **HTTPS 交给反向代理**：应用本身是标准库 HTTP 服务，不直接做 TLS。外网访问建议用 NAS/nginx/Caddy 反代加 HTTPS，并配置 `trusted_proxies` 后再读取 `X-Forwarded-For`。

### Emby Server 部署方式

推荐用 Synology Container Manager / Docker Compose 跑 Emby，媒体目录由 CloudDrive2 和本地 `strm` 目录 bind 进容器：

```yaml
services:
  emby:
    image: emby/embyserver:latest
    container_name: emby
    restart: unless-stopped
    environment:
      - UID=<NAS 用户 uid>
      - GID=<NAS users 组 gid>
      - TZ=Asia/Shanghai
      # 可选：给 TMDb / TheTVDB 刮削走内网代理；不用代理可删掉
      - HTTP_PROXY=socks5://<LAN_PROXY_IP>:<PORT>
      - HTTPS_PROXY=socks5://<LAN_PROXY_IP>:<PORT>
      - ALL_PROXY=socks5://<LAN_PROXY_IP>:<PORT>
      - NO_PROXY=localhost,127.0.0.1,::1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12
    volumes:
      - /volume1/docker/emby/config:/config
      - type: bind
        source: /volume1/docker/clouddrive2/CloudNAS/CloudDrive
        target: /media
        read_only: true
        bind:
          propagation: rslave
      - /volume1/strm:/strm
    ports:
      - "8096:8096"
      - "8920:8920"
```

说明：

- `/config` 放 Emby 数据库、metadata、封面缓存、用户、API key 等，必须是 NAS 本地可写目录。
- CloudDrive2 挂载目录 bind 到容器内 `/media`，建议对 Emby **只读**，避免 Emby 或插件误改云盘文件。
- `/volume1/strm` bind 到容器内 `/strm`，由本工具生成/移动/删除 `.strm`、`.nfo`、海报等轻量文件。
- Emby 媒体库可以指向 `/strm/电影`、`/strm/电视剧`、`/strm/动漫` 等；如果你选择直扫 CloudDrive2，也可以指向 `/media/电影` 等，但云盘挂载上大量 ffprobe/实时监控更容易慢或触发风控。
- 云盘媒体库建议关闭「保存图片到媒体文件夹」「NFO 写出」「实时监控」「视频预览缩略图」，让元数据留在 `/config`，减少对 CloudDrive2/115 的写入和扫描压力。
- 如果配置刮削代理，`.NET`/Emby 环境变量用 `socks5://...`；不要写成 `socks5h://...`，部分运行时会忽略。

### CloudDrive2 / 115 挂载架构

CloudDrive2 负责登录 115 并把网盘挂成 NAS 本地 FUSE 目录，推荐路径：

```text
/volume1/docker/clouddrive2/
├─ config/                         # CloudDrive2 配置、token、任务状态等，本地私有数据
└─ CloudNAS/CloudDrive/             # FUSE 挂载点，供 NAS / Emby / 本工具读取
   ├─ 电影/
   ├─ 电视剧/
   └─ 动漫/
```

挂载关系：

```text
115 网盘
  │  CloudDrive2 登录和挂载
  ▼
NAS: /volume1/docker/clouddrive2/CloudNAS/CloudDrive
  ├─ emby 容器内：/media  只读 bind
  ├─ emby-manager：通过 config.json 的 cd 读取/映射
  └─ 字幕/整理工具：可按需在 NAS 主机侧写入，但要控制并发
```

实践建议：

- NAS 主机侧的 CloudDrive2 挂载可以是读写，方便字幕工具或整理脚本落字幕/改名；但 Emby 容器里仍建议 `read_only: true`。
- 本工具默认 `cd=/volume1/docker/clouddrive2/CloudNAS/CloudDrive`，如果你的 CloudDrive2 挂载点不同，在 `config.json` 里改 `cd` 后重启。
- `strm=/volume1/strm` 是本地轻量索引层；真实视频仍在 115/CloudDrive2。这样 Emby 扫描的是小文件，日常新增/删除/移动也更可控。
- 115/CloudDrive2 对密集读取很敏感，批量生成 strm、复制 metadata、字幕扫描等任务尽量串行或限速，不要在挂载目录上并发跑大量 ffprobe/下载。
- 如果另有字幕下载器（如 chinese-subfinder），让它把字幕写到 CloudDrive2 对应媒体目录旁边，Emby 扫库后即可识别；但仍建议限制扫描频率。

### 推荐库结构

一种简单结构如下：

```text
115 / CloudDrive2
├─ 电影/
├─ 电视剧/
└─ 动漫/

NAS 本地 strm
├─ 电影/
├─ 电视剧/
└─ 动漫/

Emby 媒体库
├─ 电影   -> /strm/电影      ContentType=movies
├─ 电视剧 -> /strm/电视剧    ContentType=tvshows
└─ 动漫   -> /strm/动漫      ContentType=tvshows
```

`115 转存` tab 里的 `c115_cid_map` 建议也按库名映射到 115 里的目标目录 cid，例如「电影」→ 115 的电影目录、「电视剧」→ 电视剧目录。这样一条龙转存、扫库、海报修复、去重、追更扫描都能按库闭环运行。

---

## ⌨️ 键盘快捷键

| 键 | 作用 |
|---|---|
| `/` 或 `Cmd+K` | 全局搜索 / 跳转 tab |
| `?` | 快捷键帮助 |
| `Esc` | 关闭最上层 Modal / Drawer |
| `g` 然后 `d/s/c/z/g/p/r/m/y/l/u/,` | 跳 12 个常用 tab(仪/扫/115/追/缺/海/重/管/系/日/用/设) |

在 input/textarea 输入时整套快捷键自动屏蔽,只允许 Esc blur 输入框。找资源 / 字幕 / 智能清理 / 定时等低频 tab 从 nav 或全局搜索进入。

## 🔔 任务中心

任何长操作(扫库 / 批量删 / 批量移 / 115 批量转存 / 海报批量修复 / 配置自动检测)走 `UI.tasks`:

- **顶部进度条**:任意活跃任务即显示,宽度 = `Σprogress / Σtotal`
- **🔔 Bell 数字**:活跃任务数;点开右侧 Drawer 列每个任务的标签、进度、状态
- **跨 tab 不丢**:切 tab 不影响任务,完成时 toast 提示
- **取消**:Drawer 内每个 running 任务有取消按钮 → `POST /api/task/cancel`
- **刷新自动恢复**:启动时 `UI.tasks.hydrate()` 调 `/api/tasks/list` 把后端还在跑的任务拉回前端

## 🌓 主题

三态:auto(跟系统) / light / dark。点击 header 🌓 切换,持久化到 `localStorage.theme`。所有组件 CSS 走 `:root` 变量,无主题闪烁。

## ⏰ 定时任务

后台 daemon 线程 30s 轮询,到点后会在本周期内补跑一次(防 NTP 校时 / 挂起恢复 / 轮询错拍漏跑)+ **同周期防重入**(daily 同天 / weekly 同 ISO 周 / monthly 同月不重跑)+ **重叠保护**(只在上次任务真实仍在 `TASKS` 中 pending/running 时跳过,避免长跑任务跨周期并发起两份)。命中后走 `run_async` 进 TaskCenter,watch 线程跟到结束 + 写 `last_status` / `last_err` 到 config。watch 线程 24h deadline 防卡死。

5 种内置 kind(`business.SCHEDULE_KINDS`):

| kind | 描述 |
|---|---|
| `scan_all` | 🔍 对每个 strm 库发 Refresh,发现手动加的新 strm |
| `zhuigeng_scan_airing` | 🔄 对所有「在更」剧用剧名扫对应库,拿新集 |
| `fix_posters_all` | 🖼️ 对所有无海报项跑保守自动匹配 |
| `refresh_no_rating_all` | 🔄 对所有无评分剧调 Emby Refresh 重拉 TMDb |
| `monitor_incremental` | 🛰️ autostrm webhook 兜底:只扫 mtime 变新的 top 目录,补漏掉的新内容 |

UI 僻瓜式下拉:每日 / 每周X / 每月N日 + HH:MM。改 / 启停 / 立即跑 / 删 都在卡片上。

## 📱 手机 / PWA

- **viewport + 两段 `@media (max-width:640px)`**:tab nav 缩字 + 横向滚动,Modal/Drawer 在手机全屏,input 16px 防 iOS zoom,库卡片 360px 屏两列
- **iOS 主屏图标**:`/apple-touch-icon.png`(180×180,蓝紫底白「管」,PIL+Hiragino 本地生成)
- **PWA standalone**:Safari → 分享 → 添加到主屏幕 → 全屏运行(无浏览器 UI);Android Chrome → 安装应用走 `/manifest.json`
- 静态资源:`app.py` 白名单 7 条路径(`/apple-touch-icon.png`、`/favicon.png`、`/icon-{192,512}.png`、`/manifest.json` 等),`Cache-Control: max-age=86400`,**不开 `/static/*` 通配防 path traversal**

## 🚦 单用户限速 / 限同时播放数

「用户」tab 每行可设两项(都写 Emby 用户 Policy,**免费版 Emby 即可,无需 Premiere**):

| 控件 | Emby Policy 字段 | 含义 |
|---|---|---|
| **同时播放** | `SimultaneousStreamLimit` | 该账号最多同时开几路播放,0=不限。纯计数检查 |
| **限速 Mbps** | `RemoteClientBitrateLimit` | 远程串流码率上限(UI 用 Mbps,存时 ×1e6 转 bps),0=不限。参考:1080p≈8 / 4K≈25 |

注意:

- **限速靠转码实现**:源码率超过上限时 Emby 实时转码降码率 → 吃 NAS CPU(免费版只有软件转码,`HardwareAccelerationRequiresPremiere`)。源码率本就在上限内则 direct play 不转码。别把上限设得过低,否则频繁触发转码,叠加 strm/115 拉流负担更重。
- **「远程」才生效**:`RemoteClientBitrateLimit` 限的是远程客户端;内网直连默认算本地不受限。
- **历史 bug 修正**:旧版本工具的「并发」写的是 `MaxActiveSessions`(本机 Emby 4.9.5 已无此字段 → 静默失效);现写真实字段 `SimultaneousStreamLimit`(+ `MaxActiveSessions` 兼容旧版 Emby)。

---

## 部署

### 资源目录数据库

仓库内的 `catalog_115.db` 只是一个**空模板数据库**，只包含 `catalog` 表结构，不包含任何 115 分享链接或个人资源数据。`115 转存` tab 里的关键词搜索 / 一键转存需要你自己向这个数据库写入数据；不需要资源目录功能时可以保持为空，不影响其他功能。

表结构如下：

```sql
CREATE TABLE catalog(name TEXT, sheet TEXT, link TEXT, is_pkg INT, link_type TEXT);
CREATE INDEX idx_catalog_link_type on catalog(link_type);
```

部署到 NAS 后，如果你有自己的资源目录数据，请写入同目录下的 `catalog_115.db`。该数据库属于本地运行数据，不建议把真实数据提交到公开仓库。

### 最小步骤

1. 把整个目录拷到 NAS,比如 `/volume1/docker/emby-manager/`。
2. 第一次跑:

   ```sh
   sudo python3 /volume1/docker/emby-manager/app.py
   ```

   首次启动会生成 `config.json`(权限 0o600)。打开 `http://<NAS>:8097`,**默认无密码** —— 立刻进「设置」改密码。

3. 默认绑 `127.0.0.1`(只 NAS 本机能访问)。外网用走 NAS 反代或 iKuai 端口转发(见下)。

### Rust + Docker 预览版

Rust/React 重构版使用 `Dockerfile` + `docker-compose.yml` 启动,默认只把宿主 `127.0.0.1:8098` 映射进容器,不会占用 legacy Python 版的 `8097`。灰度时建议 **Python 8097 继续跑生产流量,Rust 8098 只给自己验收**,两个服务并行,不要直接替换反代入口。

重要原则: **镜像在本地开发机 build,再传到 NAS;NAS 只 `docker load` + `docker compose up --no-build`,不在 NAS 上编译 Rust/React。** 不要在 NAS 上运行 `docker compose up --build`、`docker compose build` 或 `cargo build`。

本地开发机:

```sh
cp .env.example .env
# 编辑 .env: 至少改 EMBY_MANAGER_BOOTSTRAP_PASSWORD / POSTGRES_PASSWORD,
# 如需访问 NAS bind mount,同步填 NAS 上有权限的 uid/gid 和路径。
docker compose build emby-manager-rs
docker save ghcr.io/ggttol/emby-manager:main | gzip > emby-manager-rs-main.tar.gz
```

如果本地开发机不是 amd64,但 NAS 是常见 Synology x86_64/amd64,用 buildx 明确产出 amd64 镜像:

```sh
docker buildx build --platform linux/amd64 -t ghcr.io/ggttol/emby-manager:main --load .
docker save ghcr.io/ggttol/emby-manager:main | gzip > emby-manager-rs-main-amd64.tar.gz
```

把 `emby-manager-rs-main*.tar.gz`、`docker-compose.yml` 和你的 NAS `.env` 传到 NAS 上的灰度目录,例如 `/volume1/docker/emby-manager-rs/`。NAS 上按 **先迁移、再 serve** 的顺序加载并启动:

```sh
cd /volume1/docker/emby-manager-rs
gzip -dc emby-manager-rs-main-amd64.tar.gz | docker load
docker compose --env-file .env up -d --no-build postgres
docker compose --env-file .env run --rm --no-build emby-manager-rs migrate --dry-run
docker compose --env-file .env run --rm --no-build emby-manager-rs migrate --apply
docker compose --env-file .env up -d --no-build emby-manager-rs
```

升级也是同样流程:本地重新 build/save,传 NAS,`docker load`,先跑 `migrate --dry-run` / `migrate --apply`,再 `docker compose up -d --no-build emby-manager-rs`。回滚/停止 Rust 灰度只用 `docker compose down`,**不要加 `-v`**,也不要删除 `postgres-data` volume;Postgres named volume 里有 Rust 版用户、会话、迁移状态等运行数据。

默认挂载:

- legacy 数据: `./legacy` → `/legacy`(只读,供迁移读取 `config.json` / `undo_log.jsonl` / `catalog_115.db`;真实迁移时用 `.env` 指到旧目录)
- 本地数据: `./data` → `/data`
- strm 根目录: `/volume1/strm` → `/strm`(默认只读灰度;Rust 已有真实 STRM 写入、删除、移动、清空目录能力,要写 STRM 或执行 cleanup/delete/move 才设 `EMBY_MANAGER_STRM_MODE=rw`)
- CloudDrive2 媒体根: `/volume1/docker/clouddrive2/CloudNAS/CloudDrive` → `/media`(默认只读;只有确认要让 Rust 对媒体/CD 根执行真实删除或移动时,才设 `EMBY_MANAGER_MEDIA_MODE=rw`)

权限建议:

| 场景 | `/strm` | `/media` / CD 根 |
|---|---|---|
| 只看面板、读配置、扫描验证 | `ro` | `ro` |
| 生成 / 删除 / 移动 STRM、清空 STRM 空目录 | `rw` | `ro` |
| 对 CloudDrive2 媒体根执行真实删除 / 移动 | 按需 `rw` | `rw` |

危险能力必须显式打开:默认 `ro` 是为了灰度时把 Rust 的文件系统写入挡在 Docker bind mount 外。打开 `rw` 前确认目标目录、uid/gid 和操作范围,尤其不要把 CloudDrive2 根误配成可写后直接跑批量删除。

可通过 `.env` 覆盖 `EMBY_MANAGER_BIND_IP`、`EMBY_MANAGER_HTTP_PORT`、`EMBY_MANAGER_LEGACY_DIR`、`EMBY_MANAGER_DATA_DIR`、`EMBY_MANAGER_STRM_ROOT`、`EMBY_MANAGER_STRM_MODE`、`EMBY_MANAGER_MEDIA_ROOT`、`EMBY_MANAGER_MEDIA_MODE`、`EMBY_MANAGER_UID`、`EMBY_MANAGER_GID`、`POSTGRES_PASSWORD` 和 `EMBY_MANAGER_BOOTSTRAP_PASSWORD`。

```env
# 默认是 127.0.0.1,建议经 NAS 反代访问;确需局域网直连再改成 0.0.0.0
EMBY_MANAGER_BIND_IP=127.0.0.1
EMBY_MANAGER_HTTP_PORT=8098
EMBY_MANAGER_BOOTSTRAP_PASSWORD=改成你的强密码
POSTGRES_PASSWORD=改成随机强密码
# 如果 NAS bind mount 不是 world-readable,把这里改成有权读取 /volume1/strm 和 CloudDrive2 的宿主 uid/gid
EMBY_MANAGER_UID=10001
EMBY_MANAGER_GID=10001
# 默认灰度只读。确认 Rust 要写 STRM 后才改 rw
EMBY_MANAGER_STRM_MODE=ro
# 默认灰度只读。确认 Rust 要真实删除/移动媒体根后才改 rw
EMBY_MANAGER_MEDIA_MODE=ro
```

首次启动会创建 Rust 版 `admin` 用户。`EMBY_MANAGER_BOOTSTRAP_PASSWORD` 只在数据库里还没有用户时使用,且 Rust 服务会拒绝缺失、过短或明显弱的 bootstrap 密码。
`POSTGRES_PASSWORD` 也必须在首次启动前改成随机强密码,不要沿用示例或可猜值。Postgres named volume 创建后,再改 `.env` 里的 `POSTGRES_PASSWORD` 不会自动轮换已有数据库用户密码;需要换密码时请在数据库内执行 `ALTER USER` 后再同步更新 `.env`。
如果 `/strm` 或 CloudDrive2 bind mount 读不到,优先在 `.env` 里把 `EMBY_MANAGER_UID` / `EMBY_MANAGER_GID` 对齐到 NAS 上拥有这些目录权限的用户/组,而不是把容器改回 root。

### 开机自启(Synology DSM)

```sh
# 控制脚本
cp /volume1/docker/emby-manager/manager.sh /usr/local/etc/rc.d/emby_manager.sh
chmod +x /usr/local/etc/rc.d/emby_manager.sh
# 立即启
/usr/local/etc/rc.d/emby_manager.sh start
```

DSM 会在开机时自动跑 `/usr/local/etc/rc.d/*.sh start`。`manager.sh` 用 `setsid` 守护,日志到 `/tmp/embymgr.log`。

### 外网访问

**不要**直接把 `host` 改成 `0.0.0.0` 然后裸暴露 —— 没 HTTPS。推荐方式:

- **NAS 反代:** DSM 控制面板 → 应用程序门户 → 反向代理 → 加一条 `https://mgr.你域名/` → `http://127.0.0.1:8097`,DSM 自动配 Let's Encrypt。
- **iKuai 端口转发:** 仅在你信任的源 IP 上转发(白名单),且仍建议在 NAS 上反代加 HTTPS。

如果一定要 bind `0.0.0.0`:`config.json` 把 `host` 改成 `"0.0.0.0"`,**前提是已经设过登录密码**(没密码时启动会拒绝监听公网,见 `migrate_cfg`)。

---

## 配置(config.json)

文件与 `app.py` 同目录,首次启动自动建,权限固定 0o600。

| 字段 | 类型 | 说明 |
|---|---|---|
| `emby_url` | str | Emby API 基址,默认 `http://127.0.0.1:8096/emby` |
| `api_key` | str | Emby API Key(Emby 控制台 → 高级 → API Keys 生成) |
| `password_hash` | str | 登录密码 PBKDF2-SHA256 hash(自动生成,不要手填明文) |
| `host` | str | 监听地址,默认 `127.0.0.1`;改 `0.0.0.0` 才能外网直连 |
| `port` | int | 监听端口,默认 `8097` |
| `c115_cookie` | str | 115 网盘 cookie(整段,含 `UID=...; CID=...; SEID=...`);仅 115 转存 tab 用 |
| `c115_cid_map` | dict | 库显示名 → 115 目标文件夹 cid 的映射,例如 `{"电影": "1234567"}` |
| `trusted_proxies` | list | 受信任反代 IP 列表;默认 `[]` 不读 XFF。配 `["192.168.2.1"]` 让 X-Forwarded-For 生效(防反代后所有用户共享 IP 限流) |
| `last_password_change_at` | int/null | 最近改密 unix timestamp;`null` = grace 模式,允许一次无旧密码改密(首次升级 v3.0 后用) |
| `username` | str | header 显示用户名,默认 `admin`(单用户系统硬编码) |
| `schema_version` | int | 配置 schema 版本,启动时 `migrate_cfg` 自动升(当前 = 5) |

旧字段 `password`(明文)在启动时会被 `migrate_cfg` 自动转成 `password_hash` 并删除。

---

## 安全

### Legacy Python

- **密码:** PBKDF2-SHA256 + per-user salt + 200000 iter,只存 hash。
- **登录限流:** 同源 IP 5 分钟内 5 次失败 → 429。
- **Token:** 登录成功后发 token(HttpOnly cookie),7 天过期,后台 `token-reaper` 线程定期回收。
- **CSRF:** 改写类请求(POST / DELETE)校验 CSRF token。
- **CSP:** 响应头限制外部资源,脚本 / 样式允许当前单文件架构需要的 inline(`unsafe-inline`),禁 frame / object / 外站 connect。
- **默认监听:** `127.0.0.1`(loopback),不主动暴露公网。
- **config 权限:** 写入时原子 rename + `chmod 0600`,只 owner 可读(护住 cookie / api_key / hash)。
- **Path traversal:** 所有暴露给前端的文件操作参数都过 `_safe_under` —— 拒绝 `..`、绝对路径、超出 base 的 realpath。
- **危险操作 Modal + requireType:** 删除 / 移动 / 删用户 / 配置导入 / 容器重启都走 UI.modal.confirm,批量 ≥5 项或不可逆操作要求**打字「删除」或库名/用户名**才能确认。
- **API Key + 115 Cookie 默认遮罩:** input `type="password"` + 「👁️ 显示」toggle,防止截图泄露。
- **配置导出剔密:** `/api/config/export` 返回的 JSON 把 `password_hash` 和 `c115_cookie` 替换为 `"<redacted>"`,导入时这些字段保留原值。
- **反代 X-Forwarded-For:** 配 `trusted_proxies` 白名单后,login 限流按真实客户端 IP 而非反代 IP,防止所有用户被同时锁。
- **Cookie / API Key 明文存:** 见下「已知未做」。

### Rust Preview

- **默认灰度端口:** compose 默认只映射 `127.0.0.1:8098`,容器内监听 `0.0.0.0` 仅用于 Docker 网络;要局域网直连必须显式设置 `EMBY_MANAGER_BIND_IP=0.0.0.0`。
- **首次管理员:** 必须显式设置 `EMBY_MANAGER_BOOTSTRAP_PASSWORD`,服务会拒绝缺失、过短、明显弱或示例 placeholder 密码。
- **会话:** 登录发 HttpOnly cookie,React 前端只保存 CSRF/用户名,不把 Bearer token 存入 localStorage;Bearer 保留给后续显式 API/CLI 场景。
- **CSRF:** cookie 模式下所有 POST/PUT/PATCH/DELETE 要带 `X-CSRF-Token`;未登录的 `/api/v2/*` 业务接口默认 401。
- **运行用户:** Docker runtime 使用非 root 用户;`/media` 和默认 `/strm` 都按只读挂载,写 STRM/cleanup 或真实删除/移动能力需要显式 opt-in。
- **NAS 权限:** Docker build/runtime 默认 uid/gid 是 `10001:10001`;Synology bind mount 保留宿主权限,可用 `.env` 的 `EMBY_MANAGER_UID` / `EMBY_MANAGER_GID` 对齐到有权读取媒体目录的 NAS 用户。
- **Postgres 数据:** Rust 版状态在 compose named volume `postgres-data`;停止灰度用 `docker compose down`,不要 `down -v` 或手工删 volume。
- **仍在预览:** Rust 版还没有 legacy 的登录限流、trusted proxy/XFF、CSP 全量头和 16 tab 功能齐平,上线前必须通过灰度验收。

## 已知未做(诚实清单)

- **`config.json` 是明文 JSON** —— `c115_cookie` 和 `api_key` 没加密。原因:不想引 `cryptography` pip 依赖(项目主打零依赖)。靠 `chmod 0600` + NAS 本机访问兜底。要更安全自己挂 KMS / `pass` / 系统 keychain。
- **单用户:** 只一个登录密码,没多账号 / 角色 / 权限分层。亲戚用 Emby 自己账号,管理工具只你一人用。
- **115 cookie 1-2 周过期:** 115 没刷新 token 机制,过期了「设置 → 115 Cookie」粘新的即可。每次过期工具会在 115 tab 标红。
- **无 HTTPS:** HTTP 服务,加密交给反代(NAS 反代 / nginx / Caddy)。
- **日志保留有限:** 「日志」页会读取当前 `logs/app.log` 的最近 200 条，文件按 2MB × 5 份轮转；没有 ELK / Loki、跨设备集中审计或长期归档。
- **undo log 局部:** 删除 / 移动 / 替换 / 海报改绑有 undo log 或回滚引导;115 转存本身不记录 undo。
- **测试覆盖:** 300+ case 覆盖 path 安全 / cfg migrate / TMDb 解析 / c115 内部 / 密码 / HTTP handler / 配置导入导出 / 配置落盘回滚 / 导入白名单和值校验 / 追更异步任务 / 写接口 CSRF / HTTP 慢连接护栏 / strm 列表 / qscore / XFF / scheduler / 任务队列 / 挂载探针 / 持久化日志 / Emby fake HTTP。**Emby/115 实活 HTTP 端到端未覆盖**(真实 NAS 上手测)。

---

## 接口摘要

所有 `/api/*` 都要登录(除 `/api/login` 自身和 `/health`)。修改类请求要 CSRF token。

| Method | Path | 用途 |
|---|---|---|
| GET | `/` | 单页 HTML |
| GET | `/health` | 健康检查(Emby 可达 + 磁盘 + 任务管理器状态) |
| POST | `/api/login` | 登录,body `{pw}`,返回 token cookie |
| GET | `/api/libraries` | Emby 在线状态 + 全部库 |
| GET | `/api/system` | 主机磁盘 / 内存 / 负载 / Docker 容器列表 |
| GET | `/api/items?lib=` | 列指定库所有项目 |
| GET | `/api/noposter` | 列无海报项目 |
| GET | `/api/dups` | 同 TMDb id 重复项目分析 |
| GET | `/api/zhuigeng` | 追更状态(对照 TMDb status) |
| GET | `/api/gaps?id=` | 单剧缺集列表 |
| POST | `/api/refreshseries` | 触发单剧元数据刷新,body `{id}` |
| GET | `/api/search?id=&name=&type=` | Emby RemoteSearch 候选 |
| GET | `/api/users` | 列 Emby 用户 |
| GET | `/api/config` | 读 config(脱敏:cookie/key 只回 mask) |
| GET | `/api/log` | 最近 200 条应用日志 |
| GET | `/api/task?tid=` | 查异步任务进度 / 结果 |
| GET | `/api/c115/test` | 测 115 cookie 是否有效 |
| GET | `/api/c115/auto_cid` | 自动按库名匹配 115 cid |
| POST | `/api/scan` | 单库扫描,body `{lib, keyword?}` |
| POST | `/api/scan_all` | 全库异步扫描,返回 `tid` |
| POST | `/api/fixposter` | 应用 TMDb id 修海报,body `{id, tmdb, type, name}` |
| POST | `/api/dedup` | 删冗余项,body `{tmdb, remove: [ids]}` |
| POST | `/api/move` | 跨库移动,body `{from, folder, to, id}` |
| POST | `/api/createlib` | 创建 strm 库,body `{name, ctype}` |
| POST | `/api/users/new` | 新建 Emby 用户 |
| POST | `/api/users/update` | 改 Emby 用户:`maxsessions`(同时播放数→SimultaneousStreamLimit)/ `bitrate_mbps`(限速→RemoteClientBitrateLimit,Mbps×1e6)/ `disabled` |
| POST | `/api/config` | 改 config(密码 / Emby / API Key / 115) |
| POST | `/api/c115/snap` | 115 分享 → snap 列文件(支持批量 + async) |
| POST | `/api/c115/save` | 115 receive 到库(支持批量 + async) |
| POST | `/api/task/cancel` | 取消异步任务,body `{tid}` |
| DELETE | `/api/item` | 删项目,body `{lib, folder, id}` |
| DELETE | `/api/user` | 删 Emby 用户,body `{id}` |

### v3.0 新增 endpoint

| Method | Path | 用途 |
|---|---|---|
| GET | `/api/me` | 返当前 csrf + username |
| POST | `/api/logout` | 登出,清 cookie + token |
| GET | `/api/tasks/list?limit=20` | 任务总览,前端 hydrate 用 |
| GET | `/api/undo_log` | 撤销日志列表 |
| POST | `/api/undo` | 执行撤销,body `{id}` |
| GET | `/api/strm_list?lib=&folder=` | 列指定 folder 下所有 strm 文件 |
| GET | `/api/config/export` | 配置导出(敏感字段 `<redacted>`) |
| POST | `/api/config/import` | 配置导入,body `{cfg, confirm:true}` |
| POST | `/api/c115/test_candidate` | 候选 cookie 不写 CFG 直接验证,body `{cookie}` |
| POST | `/api/zhuigeng` `{async:true}` | 追更检查异步,返 tid |
| POST | `/api/fixposter_batch?async=1` | 海报批量自动匹配,body `{ids, type}` |
| POST | `/api/manage/delete_batch?async=1` | 批量删,body `{lib, items}` |
| POST | `/api/manage/move_batch?async=1` | 批量移,body `{from, to, items}` |
| POST | `/api/dedup/exec_batch?async=1` | 批量去重,body `{groups}` |
| POST | `/api/c115/auto_cid?async=1` | 自动检测 cid 异步 |
| Header | `X-Server-Version` | 所有响应附带工具版本号(VERSION 文件) |

### v3.0.x 增量 endpoint

| Method | Path | 用途 |
|---|---|---|
| GET | `/api/dash/todo` | 仪表盘待办(无海报数 / 重复数 / 无评分数等) |
| GET | `/api/system/health` | 系统健康预警(容器非 Up / 磁盘紧 / Emby 离线) |
| POST | `/api/dedup/replace` | 全替换:删 lose folder + win 改名,body `{lib, win_folder, lose_folder}` |
| POST | `/api/dedup/replace_batch?async=1` | 批量替换,body `{items}` |
| POST | `/api/dedup/auto_all` | 一键全自动去重(只删可逆胜负) |
| POST | `/api/zhuigeng/scan_airing` | 一键扫所有在更剧,返报告 |
| POST | `/api/zhuigeng/gaps_summary` | 汇总所有在更剧的缺集 → 求资源清单 |
| POST | `/api/cleanup/suggest` | 智能清理建议,body `{lib, top, min_score, dimensions:[rating,age,idle,size,meta]}` |
| POST | `/api/cleanup/empty_folders` | 扫某库的 115 上无视频文件的空 folder |
| POST | `/api/cleanup/refresh_no_rating` | 触发某库无评分剧的 emby 元数据刷新 |
| POST | `/api/gaps/scan_lib` | 全库缺集扫描,body `{lib}` |
| POST | `/api/poster/detect_mismatch` | 检测疑似绑错 tmdbid(folder vs name 重合度低) |
| POST | `/api/wizard/add_new` | 一条龙加新资源:转存 → 扫 → 等刮削 → 海报+重复检查 → 报告 |
| GET | `/api/schedules` | 列所有定时任务(含 next_run / 状态 / kinds map) |
| POST | `/api/schedules/new` | 新建,body `{name, kind, schedule:{mode,hour,minute,weekday?,day?}, enabled}` |
| POST | `/api/schedules/update` | 改(只接受 name/params/schedule/enabled,**kind 不可改**) |
| POST | `/api/schedules/delete` | 删,body `{id}` |
| POST | `/api/schedules/run` | 立即跑(绕过 is_due 判定),返 tid |
| GET | `/apple-touch-icon.png` 等 | 公开静态资源(iOS 图标 / favicon / manifest);白名单 7 路径 + 1 天缓存 |

---

## License

MIT
