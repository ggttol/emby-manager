#!/bin/sh
# emby-manager 告警脚本
# 三层探活:工具进程 / Emby / 115 cookie。状态翻转才推送,避免每 6h 骚扰。
# 兼容 DSM 6/7 + 普通 Linux,纯 POSIX sh + curl + python3 标准库。
#
# 用法: sh alert.sh [-v]    -v 打印调试信息
# 退出码: 0=全 ok  1=有告警  2=配置错
#
# 部署: 见 scripts/README.md

set -u
VERBOSE=0
[ "${1:-}" = "-v" ] && VERBOSE=1

# ---- 路径自适应:不论 cron 在哪 cwd 跑都 OK ----------------------------
# readlink -f 在部分 BusyBox 上不可用,降级到原始路径
SCRIPT_PATH="$0"
if command -v readlink >/dev/null 2>&1 && readlink -f "$0" >/dev/null 2>&1; then
    SCRIPT_PATH="$(readlink -f "$0")"
fi
SCRIPT_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
CONF="$SCRIPT_DIR/alert.conf"
STATE_FILE="/tmp/embymgr_alert_state"
LOG_FILE="/tmp/embymgr_alert.log"
LOG_MAX_LINES=200

# ---- 日志 -------------------------------------------------------------
log() {
    ts="$(date '+%Y-%m-%d %H:%M:%S')"
    echo "[$ts] $*" >> "$LOG_FILE"
    [ "$VERBOSE" = "1" ] && echo "[$ts] $*" >&2
}
# 启动时把日志截到 LOG_MAX_LINES,避免无限增长
if [ -f "$LOG_FILE" ]; then
    tail -n "$LOG_MAX_LINES" "$LOG_FILE" > "$LOG_FILE.tmp" 2>/dev/null && mv "$LOG_FILE.tmp" "$LOG_FILE"
fi

# ---- 读配置 -----------------------------------------------------------
if [ ! -f "$CONF" ]; then
    msg="alert.conf 不存在,请: cp $SCRIPT_DIR/alert.conf.example $CONF 并填写"
    echo "$msg" >&2
    log "$msg"
    exit 2
fi
# shellcheck disable=SC1090
. "$CONF"

# 默认值兜底
EMBYMGR_URL="${EMBYMGR_URL:-http://127.0.0.1:8097}"
EMBY_URL="${EMBY_URL:-http://127.0.0.1:8096/emby}"
EMBYMGR_PW="${EMBYMGR_PW:-}"
BARK_URL="${BARK_URL:-}"
SCT_URL="${SCT_URL:-}"
TG_URL="${TG_URL:-}"

# 至少一个推送通道?
if [ -z "$BARK_URL" ] && [ -z "$SCT_URL" ] && [ -z "$TG_URL" ]; then
    log "未配置任何推送通道(BARK_URL/SCT_URL/TG_URL 都空),只 print 不推送"
fi

# ---- HTTP 工具:curl 包一层不让任何 curl 失败把脚本带崩 ----------------
# 用法: http_get URL [extra-args...]   echo body, 返回 HTTP code 在 $HTTP_CODE
HTTP_CODE=0
http_get() {
    _url="$1"; shift
    _tmp="$(mktemp 2>/dev/null || echo /tmp/_alertcurl.$$)"
    _code="$(curl -sS --max-time 5 -o "$_tmp" -w '%{http_code}' "$@" "$_url" 2>/dev/null || echo 000)"
    HTTP_CODE="$_code"
    cat "$_tmp" 2>/dev/null
    rm -f "$_tmp"
}
# 用法: http_post URL JSON [extra-args...]
http_post() {
    _url="$1"; _body="$2"; shift 2
    _tmp="$(mktemp 2>/dev/null || echo /tmp/_alertcurl.$$)"
    _code="$(curl -sS --max-time 5 -o "$_tmp" -w '%{http_code}' \
        -H 'Content-Type: application/json' \
        --data "$_body" "$@" "$_url" 2>/dev/null || echo 000)"
    HTTP_CODE="$_code"
    cat "$_tmp" 2>/dev/null
    rm -f "$_tmp"
}

# 用 python3 stdlib 抽 JSON 字段(不依赖 jq)
# 用法: json_field <key> <<<"$body"   只支持顶层 string/bool/number
json_field() {
    _key="$1"
    python3 -c "
import sys, json
try:
    d = json.loads(sys.stdin.read())
    v = d.get('$_key')
    if isinstance(v, bool): print('true' if v else 'false')
    elif v is None: print('')
    else: print(v)
except Exception:
    pass
" 2>/dev/null
}

# URL encode(给 Server酱/Telegram 用)
urlenc() {
    python3 -c "import sys, urllib.parse; print(urllib.parse.quote(sys.stdin.read(), safe=''))" 2>/dev/null
}

# ---- 推送通道 ---------------------------------------------------------
# notify <title> <body>
notify() {
    _title="$1"; _body="$2"
    log "NOTIFY: $_title | $_body"

    if [ -n "$BARK_URL" ]; then
        _t="$(printf '%s' "$_title" | urlenc)"
        _b="$(printf '%s' "$_body" | urlenc)"
        # 兼容用户填带/不带末尾 / 的 BARK_URL
        _u="$(printf '%s' "$BARK_URL" | sed 's:/*$::')"
        http_get "$_u/$_t/$_b?group=emby-manager" >/dev/null
        log "  bark http=$HTTP_CODE"
    fi
    if [ -n "$SCT_URL" ]; then
        _t="$(printf '%s' "$_title" | urlenc)"
        _b="$(printf '%s' "$_body" | urlenc)"
        # SCT_URL 可能本身已带 ?, 也可能没有
        case "$SCT_URL" in
            *\?*) _sep='&' ;;
            *)    _sep='?' ;;
        esac
        http_get "${SCT_URL}${_sep}title=${_t}&desp=${_b}" >/dev/null
        log "  sct http=$HTTP_CODE"
    fi
    if [ -n "$TG_URL" ]; then
        _msg="$(printf '%s\n%s' "$_title" "$_body" | urlenc)"
        # TG_URL 已经带 chat_id=...,我们只追加 &text=
        case "$TG_URL" in
            *\?*) _sep='&' ;;
            *)    _sep='?' ;;
        esac
        http_get "${TG_URL}${_sep}text=${_msg}" >/dev/null
        log "  tg http=$HTTP_CODE"
    fi
}

# ---- 状态机:只在翻转时通知 -------------------------------------------
# 状态文件每行: KEY=ok|fail
# KEY ∈ {tool, emby, c115}
load_prev() {
    _k="$1"
    if [ -f "$STATE_FILE" ]; then
        grep "^${_k}=" "$STATE_FILE" 2>/dev/null | tail -n1 | cut -d= -f2
    fi
}
save_state() {
    _tool="$1"; _emby="$2"; _c115="$3"
    {
        echo "tool=$_tool"
        echo "emby=$_emby"
        echo "c115=$_c115"
        echo "updated=$(date '+%Y-%m-%d %H:%M:%S')"
    } > "$STATE_FILE"
}

# transition <key> <new> <fail-title> <fail-body> <ok-title> <ok-body>
transition() {
    _key="$1"; _new="$2"
    _ftit="$3"; _fbody="$4"; _otit="$5"; _obody="$6"
    _prev="$(load_prev "$_key")"
    # 首次跑没历史:只在当前 fail 时报警(避免开机刷屏 ok)
    if [ -z "$_prev" ]; then
        if [ "$_new" = "fail" ]; then
            notify "$_ftit" "$_fbody"
        else
            log "  $_key: 首跑且 ok,不通知"
        fi
        return
    fi
    if [ "$_prev" = "$_new" ]; then
        log "  $_key: 状态未变($_new),不通知"
        return
    fi
    if [ "$_new" = "fail" ]; then
        notify "$_ftit" "$_fbody"
    else
        notify "$_otit" "$_obody"
    fi
}

# ---- 探测 1:工具 ------------------------------------------------------
# 优先 /health,没有就退化到 / (拿 HTML 也算活)
check_tool() {
    body="$(http_get "$EMBYMGR_URL/health")"
    if [ "$HTTP_CODE" = "200" ]; then
        log "tool /health 200"
        echo ok; return
    fi
    # 退化:根路径,200 即活(app.py 的 / 返回 index.html 或登录页)
    http_get "$EMBYMGR_URL/" >/dev/null
    if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "302" ] || [ "$HTTP_CODE" = "401" ]; then
        log "tool / http=$HTTP_CODE (降级判活)"
        echo ok; return
    fi
    log "tool 不可达 http=$HTTP_CODE"
    echo fail
}

# ---- 探测 2:Emby ------------------------------------------------------
check_emby() {
    body="$(http_get "$EMBY_URL/System/Info/Public")"
    if [ "$HTTP_CODE" = "200" ]; then
        log "emby Info/Public 200"
        echo ok; return
    fi
    log "emby 不可达 http=$HTTP_CODE"
    echo fail
}

# ---- 探测 3:115 cookie(需登录工具拿 token)---------------------------
# 返回 ok / fail / skip (skip = 缺密码或工具本身挂了,不算 cookie 失效)
check_c115() {
    _toolstate="$1"
    if [ "$_toolstate" != "ok" ]; then
        log "c115 跳过(工具不可达)"
        echo skip; return
    fi
    if [ -z "$EMBYMGR_PW" ]; then
        log "c115 跳过(未配 EMBYMGR_PW)"
        echo skip; return
    fi
    # 1) 登录拿 token
    login_body="$(http_post "$EMBYMGR_URL/api/login" "{\"pw\":\"$EMBYMGR_PW\"}")"
    if [ "$HTTP_CODE" != "200" ]; then
        log "c115 登录失败 http=$HTTP_CODE body=$login_body"
        echo fail; return
    fi
    token="$(printf '%s' "$login_body" | json_field token)"
    if [ -z "$token" ]; then
        log "c115 登录响应无 token: $login_body"
        echo fail; return
    fi
    # 2) 调 /api/c115/test
    test_body="$(http_get "$EMBYMGR_URL/api/c115/test" -H "X-Token: $token")"
    if [ "$HTTP_CODE" != "200" ]; then
        log "c115 test 接口异常 http=$HTTP_CODE body=$test_body"
        echo fail; return
    fi
    ok_field="$(printf '%s' "$test_body" | json_field ok)"
    if [ "$ok_field" = "true" ]; then
        log "c115 ok"
        echo ok; return
    fi
    log "c115 cookie 失效 body=$test_body"
    echo fail
}

# ---- 主流程 -----------------------------------------------------------
host="$(hostname 2>/dev/null || echo nas)"
log "==== run start host=$host ===="

tool_state="$(check_tool)"
emby_state="$(check_emby)"
c115_state="$(check_c115 "$tool_state")"

# 状态翻转推送
transition tool "$tool_state" \
    "🔴 emby-manager 工具离线" \
    "host=$host url=$EMBYMGR_URL 探活失败,请 ssh 上去看 /tmp/embymgr.log" \
    "🟢 emby-manager 工具已恢复" \
    "host=$host url=$EMBYMGR_URL 已重新可达"

transition emby "$emby_state" \
    "🔴 Emby 离线" \
    "host=$host url=$EMBY_URL 探活失败,Docker 可能挂了" \
    "🟢 Emby 已恢复" \
    "host=$host url=$EMBY_URL 已重新可达"

# c115 的 skip 不参与状态机,保持上次状态
if [ "$c115_state" != "skip" ]; then
    transition c115 "$c115_state" \
        "🔴 115 cookie 失效" \
        "host=$host 工具拿不到 115 数据,去网页 设置 → 115 Cookie 重新粘贴" \
        "🟢 115 cookie 已恢复" \
        "host=$host 115 接口又能用了"
fi

# 持久化(c115 skip 时沿用旧状态)
prev_c115="$(load_prev c115)"
[ "$c115_state" = "skip" ] && c115_state="${prev_c115:-skip}"
save_state "$tool_state" "$emby_state" "$c115_state"

log "==== run end tool=$tool_state emby=$emby_state c115=$c115_state ===="

# 退出码
if [ "$tool_state" = "fail" ] || [ "$emby_state" = "fail" ] || [ "$c115_state" = "fail" ]; then
    exit 1
fi
exit 0
