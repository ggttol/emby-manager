#!/bin/sh
# Emby 管理工具 控制脚本(start/stop/restart;开机自启也用它)
# 放 /volume1/docker/emby-manager/manager.sh,并拷一份到 /usr/local/etc/rc.d/emby_manager.sh
PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH
APP=/volume1/docker/emby-manager/app.py
PY=/usr/bin/python3
[ -x "$PY" ] || PY="$(command -v python3)"
kill_all(){
  pids="$(ps aux | grep '[e]mby-manager/app.py' | awk '{print $2}')"
  [ -z "$pids" ] && return 0
  # 先 TERM，让 Python 结束当前磁盘写入；直接 kill -9 可能在 config.json/log rotate 中间截断文件。
  for p in $pids; do kill "$p" 2>/dev/null || true; done
  n=0
  while [ "$n" -lt 8 ]; do
    alive=""
    for p in $pids; do kill -0 "$p" 2>/dev/null && alive="$alive $p"; done
    [ -z "$alive" ] && return 0
    sleep 1; n=$((n + 1))
  done
  # 卡在不可中断的 FUSE/网络 IO 时才兜底强杀，保证 restart 不会永久卡住。
  for p in $pids; do kill -9 "$p" 2>/dev/null || true; done
}
case "$1" in
  stop)
    kill_all; echo "emby-manager stopped";;
  start|restart|"")
    kill_all; sleep 1
    setsid "$PY" "$APP" </dev/null >/tmp/embymgr.log 2>&1 &
    echo "emby-manager started ($PY)";;
  *)
    echo "usage: $0 {start|stop|restart}";;
esac
