# emby-manager 告警脚本套件

主动监控 emby-manager 工具、Emby、115 cookie 三层是否健康。状态翻转时推 Bark / 微信 / Telegram,避免每 6h 骚扰。

## 文件

| 文件 | 作用 |
| --- | --- |
| `alert.sh` | 主探活脚本,cron 跑。三层探活 + 状态机 + 推送 |
| `alert.conf.example` | 配置模板,`cp` 一份为 `alert.conf` 后填写 |
| `install-cron.sh` | 把 `alert.sh` 加进 crontab(`0 */6 * * *`) |
| `update_catalog_from_qqdocs.py` | 从已登录 Chrome 加载过的腾讯文档响应里重建 `catalog_115.db` |
| `validate_catalog_115_links.py` | 对 `catalog_115.db` 里的 115 分享链接做有效性验证,结果写旁路 SQLite |

依赖:`sh`、`curl`、`python3`(只用标准库)。DSM 6/7 自带,无需额外装。

## 更新资源库 catalog_115.db

腾讯文档地址:

```text
https://docs.qq.com/sheet/DZWtEeFFGZW9XUkJo?tab=000001
```

这份文档的导出权限被关掉,且部分表格块是腾讯自定义 protobuf + zlib/base64 格式。现在已经把解析流程固化成脚本:

```sh
cd /Users/gaotao/code/emby-manager

# 1. 先用已登录 Chrome 打开腾讯文档,并让所有 sheet 至少加载一次。
#    Codex 可用 Chrome 正常切 tab;不要读 cookie/localStorage,只需要页面把 docs.qq.com 响应写进缓存。

# 2. 先 dry-run 看统计,低于 10 万条会拒绝写库
python3 scripts/update_catalog_from_qqdocs.py --dry-run

# 3. 确认后替换 catalog_115.db;脚本会自动生成 catalog_115.db.bak-YYYYmmdd-HHMMSS
python3 scripts/update_catalog_from_qqdocs.py
```

脚本要点:

- 数据源只认 Chrome 缓存里的 `docs.qq.com/dop-api/opendoc` / `get/sheet` 在线响应,不拿旧 `catalog_115.db` 混数据。
- 默认只看最近 24 小时的 Chrome 缓存,避免旧块把已删除资源混回来;需要全量扫缓存可加 `--since-hours 0`。
- 默认只看最近 24 小时缓存,再按最终 `(name,sheet,link,is_pkg,link_type)` 记录级去重;如果刚加载完表格,这是最稳的更新路径。
- 解析 `related_sheet` 的 base64 + zlib + protobuf;支持普通字符串、富文本、单元格超链接里的 `115` / `magnet` / `ed2k`。
- 写库前有 `--min-rows` 保护(默认 100000),抽取异常时拒绝替换。
- 当前观察:`高清影视之家 1` 这类 sheet 只下发展示文本,没有可用链接时不会写入资源行。

常用参数:

```sh
# 指定 Chrome cache 目录
python3 scripts/update_catalog_from_qqdocs.py --cache-dir "$HOME/Library/Caches/Google/Chrome/Profile 2/Cache/Cache_Data"

# 临时放宽/收紧保护线
python3 scripts/update_catalog_from_qqdocs.py --min-rows 200000
```

## 验证 catalog_115.db 里的 115 分享链接

验证走 115 `share/snap` 接口,会触发真实网络请求。资源库里通常有十万级分享链接,必须限速、断点续跑:

```sh
# 先只统计唯一 115 分享链接数量
python3 scripts/validate_catalog_115_links.py --dry-run

# 小批试跑 20 条;Cookie 可来自 config.json、--cookie-file 或 C115_COOKIE 环境变量
C115_COOKIE='UID=...; CID=...; SEID=...' python3 scripts/validate_catalog_115_links.py --limit 20

# 全量慢跑,默认跳过 30 天内已验证过的链接,结果写 catalog_115_validation.db
C115_COOKIE='UID=...; CID=...; SEID=...' python3 scripts/validate_catalog_115_links.py --sleep 1.0
```

结果表:

- `share_checks`:按 `(share_code, receive_code)` 去重保存最新验证结果,`ok=1` 表示有效。
- `runs`:每次运行的统计,中断后已写入的结果会保留。

## 配置推送通道(至少一个)

### Bark(iOS)
1. App Store 装 Bark App,首页有「服务器地址」一栏
2. 复制完整 URL(形如 `https://api.day.app/<key>`)
3. 填到 `alert.conf` 的 `BARK_URL=""`

### Server 酱 Turbo(微信)
1. 去 [sct.ftqq.com](https://sct.ftqq.com/) GitHub 登录,扫码绑微信
2. 拿到 `SendKey`,拼成 `https://sctapi.ftqq.com/<SendKey>.send`
3. 填到 `SCT_URL=""`

### Telegram bot(可选)
1. Telegram 找 `@BotFather`,`/newbot` 拿 token
2. 给你自己创建的 bot 发任意一条消息
3. 访问 `https://api.telegram.org/bot<TOKEN>/getUpdates`,在返回里找 `chat_id`
4. 拼: `https://api.telegram.org/bot<TOKEN>/sendMessage?chat_id=<CHATID>`
5. 填到 `TG_URL=""`(脚本自动补 `&text=...`)

## 第一次部署

```sh
# 1. 把整个 scripts 目录传到 NAS
#    (假设 emby-manager 在 /volume1/docker/emby-manager/)
scp -P 5022 -r scripts/ gaotao@gaotao.cc:/volume1/docker/emby-manager/

# 2. SSH 上去
ssh -p 5022 gaotao@gaotao.cc
cd /volume1/docker/emby-manager/scripts

# 3. 复制配置,填字段
cp alert.conf.example alert.conf
chmod 600 alert.conf
vi alert.conf       # 至少填一个推送通道 + EMBYMGR_PW

# 4. 先手动跑一次,确认推送能收到
sudo sh alert.sh -v

# 5. 装 cron(每 6 小时一次)
sudo sh install-cron.sh
```

## 抑制重复告警(状态机原理)

状态文件 `/tmp/embymgr_alert_state` 记录上次每项的状态(ok/fail)。本次跑时:

- 状态没变 → 不通知,只写日志
- ok → fail → 发红色告警(🔴 ...)
- fail → ok → 发绿色恢复(🟢 ... 已恢复)
- 第一次跑(无状态文件)且当前 ok → 不通知,避免每次开机刷屏

这样每 6h 一次的 cron,只有真出事 / 真恢复 才会响。

## 排错

```sh
# 手动跑一次 + 调试输出
sudo sh alert.sh -v

# 看最近告警日志(脚本会自动截到最近 200 行)
tail -50 /tmp/embymgr_alert.log

# 看当前状态
cat /tmp/embymgr_alert_state

# 强制重置状态(下次跑会按首跑规则:仅 fail 才通知)
rm /tmp/embymgr_alert_state

# 看 cron 安装情况
sh install-cron.sh --print

# 卸载 cron
sudo sh install-cron.sh --uninstall
```

## 已知限制

- **6 小时粒度**: cron 默认 `0 */6 * * *`,最坏情况 6h 才发现。要近实时报警请上 [Uptime Kuma](https://github.com/louislam/uptime-kuma) 之类的专用监控。改 `install-cron.sh` 顶部的 `CRON_SCHED` 可以加密(注意 6h→5min 期间推送方限频)。
- **/health 端点**: app.py 当前没有 `/health`,脚本会自动降级到 `GET /`(任何 2xx/3xx/401 都算活)。一旦 M-4 加上 `/health`,无需改本脚本即可生效。
- **115 cookie 依赖工具**: 检测 115 需要先登录工具拿 token,所以工具挂了 cookie 状态不更新(`skip`,沿用上次)。
- **DSM 重启 crond**: `install-cron.sh` 已自动 `synoservicectl --restart crond`,但若 DSM 版本 / 权限异常,可能要手动 `sudo /etc/init.d/synoschedtask reload` 或 DSM「控制面板 → 任务计划」里看一下。
- **推送通道无重试**: 任一通道暂时挂了就这一轮丢一次。下一轮(6h 后)状态没变就不再推,所以告警可能丢。冗余配 2 个通道更稳。
