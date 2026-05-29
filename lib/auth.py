"""鉴权:PBKDF2 密码、token 池(TTL + reaper)、login 频控、CSRF 工具、SAFE_METHODS 常量。
TOKENS 跨模块共享(HTTP handler 校验、reaper 清理),全部进入要持 TOKENS_LOCK。
"""
import collections, hashlib, hmac, secrets, threading, time

from lib.logger import logger

TOKENS = {}            # token -> {created, last_seen, ip, csrf}
TOKENS_LOCK = threading.Lock()
TOKEN_TTL = 7 * 24 * 3600          # 7 天没用就失效(滑动空闲窗)
ABS_TOKEN_TTL = 30 * 24 * 3600     # 绝对寿命上限:不管多活跃,30 天必须重登(防泄露 token 被无限 keep-alive)

LOGIN_FAIL = collections.defaultdict(list)  # ip -> [失败时间戳]
LOGIN_FAIL_LOCK = threading.Lock()
LOGIN_WINDOW = 300   # 5 分钟
LOGIN_MAX_FAIL = 5   # 窗内 ≥5 次 → 429

SAFE_METHODS = ("GET", "HEAD", "OPTIONS")


def _hash_password(plain):
    salt = secrets.token_bytes(32)
    h = hashlib.pbkdf2_hmac("sha256", plain.encode("utf-8"), salt, 200000)
    return "pbkdf2_sha256$200000$%s$%s" % (salt.hex(), h.hex())


def _verify_password(plain, stored):
    if not plain or not stored: return False
    try:
        scheme, iters, salt_hex, hash_hex = stored.split("$")
        if scheme != "pbkdf2_sha256": return False
        h = hashlib.pbkdf2_hmac("sha256", plain.encode("utf-8"), bytes.fromhex(salt_hex), int(iters))
        return hmac.compare_digest(h.hex(), hash_hex)
    except Exception:
        return False


def _token_new(ip):
    t = secrets.token_urlsafe(32)
    csrf = secrets.token_urlsafe(32)
    with TOKENS_LOCK:
        TOKENS[t] = {"created": time.time(), "last_seen": time.time(), "ip": ip, "csrf": csrf}
    return t, csrf


def _token_drop(t):
    with TOKENS_LOCK: TOKENS.pop(t, None)


def _token_csrf(t):
    with TOKENS_LOCK:
        return TOKENS.get(t, {}).get("csrf")


def _token_valid(t, ip=None):
    """校验 token。ip 给定时:可选 IP 绑定(config bind_token_ip=true 则换 IP 失效;否则只记审计日志)。"""
    if not t: return False
    now = time.time()
    with TOKENS_LOCK:
        rec = TOKENS.get(t)
        if not rec: return False
        # 空闲窗 + 绝对寿命双重过期
        if now - rec["last_seen"] > TOKEN_TTL or now - rec.get("created", now) > ABS_TOKEN_TTL:
            TOKENS.pop(t, None); return False
        # IP 绑定 / 审计:token 记录的登录 IP 与当前请求来源不一致时
        if ip and rec.get("ip") and ip != rec["ip"]:
            try:
                from lib.config import CFG
                strict = bool(CFG.get("bind_token_ip"))
            except Exception:
                strict = False
            if strict:
                TOKENS.pop(t, None)
                logger.warning("token IP 变更 (%s→%s) 且开启绑定 → 失效", rec["ip"], ip)
                return False
            logger.warning("token 来源 IP 变更:登录 %s → 当前 %s(未开 bind_token_ip,放行)", rec["ip"], ip)
        rec["last_seen"] = now
        return True


def _login_allowed(ip):
    with LOGIN_FAIL_LOCK:
        now = time.time()
        LOGIN_FAIL[ip] = [t for t in LOGIN_FAIL[ip] if now - t < LOGIN_WINDOW]
        return len(LOGIN_FAIL[ip]) < LOGIN_MAX_FAIL


def _login_record_fail(ip):
    with LOGIN_FAIL_LOCK:
        LOGIN_FAIL[ip].append(time.time())


def client_ip_for_login(remote_addr, xff_header, trusted_proxies):
    """决定限流应该用哪个 IP 作为 key。
    - remote_addr: TCP 直连 IP(self.client_address[0])
    - xff_header: 请求 X-Forwarded-For header 原始字符串(可空)
    - trusted_proxies: 受信任反代 IP 列表(config.json 配),为空则不读 XFF

    规则:remote_addr 必须在 trusted_proxies 列表里才认 XFF,否则用 remote_addr(防客户端伪造 XFF)。
    XFF 是逗号分隔的 IP 链,最右侧是直连本机的(应该 = remote_addr),最左侧是最初的 client。
    返回:最右侧"非 trusted_proxies"的 IP,即客户端真实 IP。

    Examples:
        client_ip_for_login("1.2.3.4", "", []) -> "1.2.3.4"
        client_ip_for_login("192.168.2.1", "8.8.8.8", ["192.168.2.1"]) -> "8.8.8.8"
        client_ip_for_login("192.168.2.1", "8.8.8.8", []) -> "192.168.2.1"  # 没配信任反代不读 XFF
        client_ip_for_login("1.2.3.4", "8.8.8.8", ["192.168.2.1"]) -> "1.2.3.4"  # 直连不在信任列表
        client_ip_for_login("192.168.2.1", "1.1.1.1, 2.2.2.2", ["192.168.2.1"]) -> "2.2.2.2"  # 最右非 trusted
        client_ip_for_login("192.168.2.1", "1.1.1.1, 2.2.2.2, 192.168.2.1", ["192.168.2.1"]) -> "2.2.2.2"  # 剥右侧 trusted
    """
    remote_addr = (remote_addr or "").strip()
    if not remote_addr or not trusted_proxies or not xff_header:
        return remote_addr or "?"
    if remote_addr not in trusted_proxies:
        return remote_addr   # 直连本机的不是受信代理 → XFF 不可信
    # 从 XFF 最右往左剥,跳过 trusted_proxies,取第一个非信任的
    chain = [s.strip() for s in xff_header.split(",") if s.strip()]
    for ip in reversed(chain):
        if ip not in trusted_proxies:
            return ip
    # 链全是 trusted(异常但兼容)→ 用 remote
    return remote_addr


def _token_reaper():
    """后台线程每 30 分钟清过期 token + 清空 LOGIN_FAIL 里的陈旧 IP 项,避免两个 dict 无限增长。"""
    while True:
        time.sleep(1800)
        try:
            now = time.time()
            cutoff = now - TOKEN_TTL
            with TOKENS_LOCK:
                dead = [t for t, r in TOKENS.items()
                        if r["last_seen"] < cutoff or now - r.get("created", now) > ABS_TOKEN_TTL]
                for t in dead: TOKENS.pop(t, None)
            if dead: logger.info("token reaper 清 %d 个过期 token", len(dead))
            # LOGIN_FAIL 同样回收:删掉窗口内已无失败记录的 IP key(否则被 IP 轮换撑大)
            with LOGIN_FAIL_LOCK:
                stale = [ip for ip, ts in LOGIN_FAIL.items()
                         if not [t for t in ts if now - t < LOGIN_WINDOW]]
                for ip in stale: LOGIN_FAIL.pop(ip, None)
            if stale: logger.info("login-fail reaper 清 %d 个陈旧 IP", len(stale))
        except Exception:
            logger.exception("token reaper 异常")
