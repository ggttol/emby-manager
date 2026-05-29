"""配置:CFG 全局 dict + schema migration + 常量路径。
CFG 是跨模块共享的 mutable dict —— 任何模块都 `from lib.config import CFG` 后直接改 key,
**绝不 rebind**(load_cfg 用 clear()+update() 保证)。
"""
import json, os, threading, time

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # 项目根 = lib/ 上一层
CD = "/volume1/docker/clouddrive2/CloudNAS/CloudDrive"   # 115 挂载根
STRM = "/volume1/strm"
DOCKER = "/var/packages/ContainerManager/target/usr/bin/docker"
VE = (".mkv", ".mp4", ".ts", ".m2ts", ".avi", ".iso", ".mov", ".flv", ".wmv", ".rmvb")
CONFIG_FILE = os.path.join(HERE, "config.json")

# _DEFAULTS 只放"任何安装都通用"的兜底;**host 和 schema_version 不放**——
# 它们要让 migrate_cfg 真看到 "key 不在 CFG" 才能区分新装 vs 旧装(旧装沿用 0.0.0.0,新装走 127.0.0.1)
_DEFAULTS = {"password_hash": "", "emby_url": "http://127.0.0.1:8096/emby",
             "api_key": "0faf87b4f47148c9b92cb9d580d4e734", "port": 8097}
WEAK_PWS = {"gaotao369", "celeron", "123456", "12345678", "password", "admin", "qwerty", "111111", "abc123"}

CURRENT_SCHEMA = 4

CFG = {}        # 占位,load_cfg() 填充
CFG_LOCK = threading.RLock()  # 保护 CFG 并发读改写
CONFIG_EXISTED = False  # load_cfg 时若 config.json 已存在 → True;migrate_cfg 据此区分新/旧装

def load_cfg():
    """清掉旧内容、读 config.json 合并 defaults。**不 rebind CFG**(跨模块共享)。"""
    global CONFIG_EXISTED
    CFG.clear()
    CFG.update(_DEFAULTS)
    try:
        with open(CONFIG_FILE, encoding="utf-8") as f:
            CFG.update(json.load(f))
        CONFIG_EXISTED = True
    except Exception:
        CONFIG_EXISTED = False

def save_cfg():
    try:
        # 原子写:先 tmp 再 rename,避免半写;chmod 0600 限只 owner 可读(护 cookie/api_key/密码)
        tmp = CONFIG_FILE + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(CFG, f, ensure_ascii=False, indent=1)
        os.chmod(tmp, 0o600)
        os.replace(tmp, CONFIG_FILE)
    except Exception:
        pass

# 模块加载即填充 CFG —— 其他模块 import 时 CFG 已就绪
load_cfg()


def _mig_to_v2():
    """v1 → v2:明文 password 改 password_hash;补 host 字段(旧装沿用 0.0.0.0+告警,新装走 127.0.0.1);schema_version 标记。"""
    # lazy import 避免循环(auth 依赖 config)
    from lib.auth import _hash_password
    from lib.logger import logger
    if CFG.get("password") and not CFG.get("password_hash"):
        plain = CFG.pop("password")
        CFG["password_hash"] = _hash_password(plain)
        logger.warning("已把明文密码迁移为 PBKDF2 hash%s", " (⚠️ 弱密码,强烈建议改)" if plain in WEAK_PWS else "")
    if "host" not in CFG:
        if CONFIG_EXISTED:
            CFG["host"] = "0.0.0.0"
            logger.warning("旧 config 无 host 字段,沿用 0.0.0.0(外网可访问)。建议改为 127.0.0.1 走内网/反代")
        else:
            CFG["host"] = "127.0.0.1"
            logger.info("新装默认 host=127.0.0.1(只 loopback)。要外网请改 config.json 的 host=0.0.0.0")

def _mig_to_v3():
    """v2 → v3:加 trusted_proxies 字段(默认空 list = 不读 XFF,保持当前行为)"""
    from lib.logger import logger
    if "trusted_proxies" not in CFG:
        CFG["trusted_proxies"] = []
        logger.info("已加 trusted_proxies=[] 字段。要支持反代,在 config.json 填反代 IP 如 [\"192.168.2.1\"]")

def _mig_to_v4():
    """v3 → v4:加 last_password_change_at 字段;加 username 默认值 admin。
    旧装(已有 config)给 None = 允许一次无旧密码改密 grace(老用户升级体恤);
    新装直接戳真实时间戳 = 一开始就要求旧密码,不留永久 grace 窗口(安全:见 review M2)。"""
    from lib.logger import logger
    if "last_password_change_at" not in CFG:
        # grace(None)只在"还没有密码"时给 —— 此时本就没有旧密码可验,允许设一个。
        # 已经有密码的装(老/新)一律戳时间戳 → 改密必须输旧密码,杜绝"永久免旧密码改密"漏洞(review M2)。
        if CONFIG_EXISTED and not CFG.get("password_hash"):
            CFG["last_password_change_at"] = None
            logger.info("加 last_password_change_at=None(无密码老装:允许首次设密码)")
        else:
            CFG["last_password_change_at"] = int(time.time())
            logger.info("加 last_password_change_at=now(已有密码/新装:改密需旧密码)")
    if "username" not in CFG:
        CFG["username"] = "admin"

# 注册表:版本 → 升级函数。新加字段往 _DEFAULTS 加;改语义在这里写新 migration 函数。
MIGRATIONS = [(2, _mig_to_v2), (3, _mig_to_v3), (4, _mig_to_v4)]

def migrate_cfg():
    """按 schema_version 顺序跑所有 pending migration,完整迁移后写回。"""
    from lib.logger import logger
    cur = CFG.get("schema_version", 1)
    changed = False
    for target_version, fn in MIGRATIONS:
        if cur < target_version:
            logger.info("schema migration: %d → %d", cur, target_version)
            fn()
            CFG["schema_version"] = target_version
            cur = target_version
            changed = True
    # 安全自检
    if CFG.get("host", "127.0.0.1") == "0.0.0.0" and not CFG.get("password_hash"):
        logger.error("⚠️ 监听 0.0.0.0 但无登录密码 hash!立刻在「设置」配密码")
    if changed:
        save_cfg()
