#!/bin/sh
# 把 alert.sh 安装到 cron(每 6 小时跑一次)
# 兼容 DSM 6/7 + 普通 Linux,需要 sudo / root 权限来改 /etc/crontab。
#
# 用法: sudo sh install-cron.sh             # 安装/更新
#       sudo sh install-cron.sh --uninstall # 卸载
#       sh install-cron.sh --print          # 只打印当前状态,不改

set -u
SCRIPT_PATH="$0"
if command -v readlink >/dev/null 2>&1 && readlink -f "$0" >/dev/null 2>&1; then
    SCRIPT_PATH="$(readlink -f "$0")"
fi
SCRIPT_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
ALERT="$SCRIPT_DIR/alert.sh"
CONF="$SCRIPT_DIR/alert.conf"
CRON_TAG="# emby-manager-alert"
CRON_SCHED="0 */6 * * *"   # 每天 0/6/12/18 点跑

ACTION="${1:-install}"

# ---- 预检 -------------------------------------------------------------
if [ ! -f "$ALERT" ]; then
    echo "找不到 $ALERT" >&2
    exit 1
fi
chmod +x "$ALERT" 2>/dev/null || true

if [ "$ACTION" = "install" ] || [ "$ACTION" = "" ]; then
    if [ ! -f "$CONF" ]; then
        echo "!! 没有 $CONF" >&2
        echo "   先 cp $SCRIPT_DIR/alert.conf.example $CONF 并填写,然后再跑本脚本" >&2
        exit 2
    fi
fi

# ---- 平台识别 ---------------------------------------------------------
IS_DSM=0
if [ -f /etc.defaults/VERSION ] || [ -d /usr/syno ]; then
    IS_DSM=1
fi

# ---- DSM 路径: 直接编辑 /etc/crontab,再 synoservicectl --restart crond
install_dsm() {
    CT=/etc/crontab
    [ -w "$CT" ] || { echo "需要 root 权限改 $CT,请用 sudo 跑" >&2; exit 1; }
    # 先剔掉旧的同 tag 行
    grep -v "$CRON_TAG" "$CT" > "$CT.tmp" && mv "$CT.tmp" "$CT"
    # DSM 的 /etc/crontab 格式: minute hour dom month dow user command
    printf '%s\troot\t%s %s\n' "$CRON_SCHED" "sh $ALERT" >> "$CT"
    printf '%s\n' "$CRON_TAG" >> "$CT"
    if command -v synoservicectl >/dev/null 2>&1; then
        synoservicectl --restart crond >/dev/null 2>&1 \
            && echo "已 synoservicectl --restart crond" \
            || echo "!! synoservicectl 重启 crond 失败,手动 /etc/init.d/synoschedtask reload 或重启" >&2
    fi
    echo "DSM 安装完成,任务:"
    grep -E "$CRON_TAG|alert.sh" "$CT" || true
}

uninstall_dsm() {
    CT=/etc/crontab
    [ -w "$CT" ] || { echo "需要 root 改 $CT" >&2; exit 1; }
    grep -v "alert.sh" "$CT" | grep -v "$CRON_TAG" > "$CT.tmp" && mv "$CT.tmp" "$CT"
    command -v synoservicectl >/dev/null 2>&1 && synoservicectl --restart crond >/dev/null 2>&1
    echo "DSM 卸载完成"
}

# ---- Linux 路径: 用 crontab -l / crontab - --------------------------
install_linux() {
    # 用当前用户的 crontab(通常 sudo 跑 → root crontab)
    cur="$(crontab -l 2>/dev/null || true)"
    new="$(printf '%s\n' "$cur" | grep -v "alert.sh" | grep -v "$CRON_TAG")"
    new="$(printf '%s\n%s %s %s\n' "$new" "$CRON_SCHED" "sh $ALERT" "$CRON_TAG")"
    printf '%s\n' "$new" | crontab -
    echo "Linux 安装完成,当前 crontab:"
    crontab -l | grep -E "$CRON_TAG|alert.sh" || true
}

uninstall_linux() {
    cur="$(crontab -l 2>/dev/null || true)"
    new="$(printf '%s\n' "$cur" | grep -v "alert.sh" | grep -v "$CRON_TAG")"
    printf '%s\n' "$new" | crontab -
    echo "Linux 卸载完成"
}

print_status() {
    echo "脚本: $ALERT"
    echo "配置: $CONF $([ -f "$CONF" ] && echo OK || echo 缺失)"
    echo "平台: $([ "$IS_DSM" = "1" ] && echo DSM || echo Linux)"
    echo "----"
    echo "DSM /etc/crontab 中相关行:"
    grep -E "alert.sh|$CRON_TAG" /etc/crontab 2>/dev/null || echo "  (无)"
    echo "----"
    echo "用户 crontab 中相关行:"
    crontab -l 2>/dev/null | grep -E "alert.sh|$CRON_TAG" || echo "  (无)"
}

# ---- 分发 -------------------------------------------------------------
case "$ACTION" in
    install|"")
        if [ "$IS_DSM" = "1" ]; then install_dsm; else install_linux; fi
        echo ""
        echo "下一步:"
        echo "  1) 手动跑一次验证: sudo sh $ALERT -v"
        echo "  2) 看日志: tail -f /tmp/embymgr_alert.log"
        ;;
    --uninstall|uninstall)
        if [ "$IS_DSM" = "1" ]; then uninstall_dsm; else uninstall_linux; fi
        ;;
    --print|print|status)
        print_status
        ;;
    *)
        echo "用法: $0 [install|--uninstall|--print]" >&2
        exit 2
        ;;
esac
