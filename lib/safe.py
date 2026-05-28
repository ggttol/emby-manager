"""路径安全:_safe_under(base, name) 防 path traversal。
任何来自请求 body 的文件夹名都必须先过这个,再拼到 STRM/CD 下。
"""
import os


def _safe_under(base, name):
    """确认 name 拼到 base 后仍在 base 树内,防 path traversal。
    返回 realpath;非法时 ValueError。name 来自请求 body 的入口都必须过这个。"""
    if not name or name in (".", "..") or "\x00" in name:
        raise ValueError("非法路径名: %r" % name)
    norm = name.replace("\\", "/")
    if norm.startswith("/") or any(seg in ("", "..") for seg in norm.split("/")[:-1] + [norm.split("/")[-1]] if seg == ".."):
        raise ValueError("非法路径段(含 .. 或绝对路径): %r" % name)
    # 简化:不允许任何 ".." 段或 leading /
    if ".." in norm.split("/") or norm.startswith("/"):
        raise ValueError("非法路径段: %r" % name)
    base_real = os.path.realpath(base)
    full = os.path.realpath(os.path.join(base_real, name))
    if not (full == base_real or full.startswith(base_real + os.sep)):
        raise ValueError("路径越出 %s: %r → %s" % (base, name, full))
    return full
