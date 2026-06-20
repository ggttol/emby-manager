"""日志:RotatingFileHandler + stderr 双输出,UI 可读持久化尾部,业务异常类 AppError。
全局 logger 单例,任何模块 `from lib.logger import logger, log`,不要重复 addHandler。
"""
import collections, logging, logging.handlers, os, sys, time
from lib.config import HERE

START_TIME = time.time()
LOGS = collections.deque(maxlen=400)

LOG_DIR = os.path.join(HERE, "logs")
os.makedirs(LOG_DIR, exist_ok=True)
LOG_FILE = os.path.join(LOG_DIR, "app.log")

logger = logging.getLogger("embymgr")
# 防重复 addHandler(模块被重复 import 时 handler 会叠加 → 日志重复)
if not logger.handlers:
    logger.setLevel(logging.INFO)
    _log_handler = logging.handlers.RotatingFileHandler(
        LOG_FILE, maxBytes=2_000_000, backupCount=5, encoding="utf-8")
    _log_handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s [%(name)s] %(message)s"))
    logger.addHandler(_log_handler)
    # 同时把 WARNING+ 输出到 stderr 让 manager.sh 的 /tmp/embymgr.log 也能看到
    _stderr = logging.StreamHandler(sys.stderr); _stderr.setLevel(logging.WARNING)
    _stderr.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(message)s"))
    logger.addHandler(_stderr)


class AppError(Exception):
    """业务异常:status=4xx,detail 安全可暴露给用户。"""
    def __init__(self, message, status=400):
        super().__init__(message); self.status = status; self.user_msg = message


def log(msg):
    """业务操作日志:写 UI 的 LOGS deque + RotatingFileHandler。"""
    e = time.strftime("%m-%d %H:%M:%S") + "  " + msg
    LOGS.appendleft(e)
    logger.info(msg)


def list_recent_logs(limit=200):
    """返回最新 N 条持久化日志(新到旧)。

    LOGS 只覆盖本次进程；这里直接读取当前轮转文件，让 UI 重启后仍能看到
    操作轨迹。单文件最大 2MB，整读后切尾比实现脆弱的 seek-by-line 更可靠，
    同时严格限制返回条数，避免日志页把大文件塞进浏览器。
    """
    try:
        limit = max(1, min(500, int(limit)))
    except (TypeError, ValueError):
        limit = 200
    try:
        with open(LOG_FILE, "rb") as f:
            rows = f.read().splitlines()
        return [row.decode("utf-8", "replace") for row in rows[-limit:][::-1]]
    except FileNotFoundError:
        return list(LOGS)[:limit]
    except Exception:
        logger.exception("读取持久化操作日志失败")
        return list(LOGS)[:limit]
