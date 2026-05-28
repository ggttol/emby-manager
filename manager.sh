#!/bin/sh
# Emby 管理工具 控制脚本(start/stop/restart;开机自启也用它)
# 放 /volume1/docker/emby-manager/manager.sh,并拷一份到 /usr/local/etc/rc.d/emby_manager.sh
PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH
APP=/volume1/docker/emby-manager/app.py
PY=/usr/bin/python3
[ -x "$PY" ] || PY="$(command -v python3)"
kill_all(){ for p in $(ps aux | grep '[e]mby-manager/app.py' | awk '{print $2}'); do kill -9 "$p" 2>/dev/null; done; }
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
