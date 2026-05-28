# emby-manager 告警脚本套件

主动监控 emby-manager 工具、Emby、115 cookie 三层是否健康。状态翻转时推 Bark / 微信 / Telegram,避免每 6h 骚扰。

## 文件

| 文件 | 作用 |
| --- | --- |
| `alert.sh` | 主探活脚本,cron 跑。三层探活 + 状态机 + 推送 |
| `alert.conf.example` | 配置模板,`cp` 一份为 `alert.conf` 后填写 |
| `install-cron.sh` | 把 `alert.sh` 加进 crontab(`0 */6 * * *`) |

依赖:`sh`、`curl`、`python3`(只用标准库)。DSM 6/7 自带,无需额外装。

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
