"""日志:RotatingFileHandler + stderr 双输出,UI 用 LOGS deque,业务异常类 AppError。
全局 logger 单例,任何模块 `from lib.logger import logger, log`,不要重复 addHandler。
"""
import collections, logging, logging.handlers, os, sys, time
from lib.config import HERE

START_TIME = time.time()
LOGS = collections.deque(maxlen=400)

LOG_DIR = os.path.join(HERE, "logs")
os.makedirs(LOG_DIR, exist_ok=True)

logger = logging.getLogger("embymgr")
# 防重复 addHandler(模块被重复 import 时 handler 会叠加 → 日志重复)
if not logger.handlers:
    logger.setLevel(logging.INFO)
    _log_handler = logging.handlers.RotatingFileHandler(
        os.path.join(LOG_DIR, "app.log"), maxBytes=2_000_000, backupCount=5, encoding="utf-8")
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
