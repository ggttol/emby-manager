"""本地 115 资源目录搜索:catalog_115.db(从用户金山文档抓的 ~15.7万 条 115 分享链)。
纯标准库 sqlite3,只读。表 catalog(name, sheet, link, is_pkg)。
FTS5 对中文分词差,统一用 LIKE 子串搜(多关键词空格分隔 = AND)。"""
import os, re, sqlite3

from lib.config import HERE
from lib.logger import log

CATALOG_DB = os.path.join(HERE, "catalog_115.db")


def catalog_available():
    return os.path.exists(CATALOG_DB)


def catalog_stats():
    if not catalog_available():
        return {"available": False}
    try:
        with sqlite3.connect("file:%s?mode=ro" % CATALOG_DB, uri=True) as con:
            n = con.execute("SELECT COUNT(*) FROM catalog").fetchone()[0]
            npkg = con.execute("SELECT COUNT(*) FROM catalog WHERE is_pkg=1").fetchone()[0]
        return {"available": True, "total": n, "packages": npkg}
    except Exception as e:
        return {"available": False, "err": str(e)}


def _like_escape(t):
    r"""转义 LIKE 元字符 % _,否则真实片名里的 % / _ 会被当通配符(如「100%」)。配合 ESCAPE '\'。"""
    return t.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")


def catalog_search(q, limit=80):
    """按名称 LIKE 多关键词(空格分隔,AND)搜。返 {items:[{name,sheet,link,is_pkg,share,rc}], total, truncated}。
    share/rc 预解析出来,前端可直接展示 + 转存。"""
    q = (q or "").strip()
    if not q:
        return {"items": [], "total": 0}
    if not catalog_available():
        return {"err": "资源库未部署(catalog_115.db 不在)"}
    terms = [t for t in re.split(r"\s+", q) if t][:6]  # 最多 6 个关键词,防滥用
    # 最小词长:单字符 term('%' 或 'a')会全表扫 15.7 万行 + filesort,挡掉
    if not terms or all(len(t) < 2 for t in terms):
        return {"items": [], "total": 0, "hint": "关键词太短,至少 2 个字符"}
    where = " AND ".join(["name LIKE ? ESCAPE '\\'"] * len(terms))
    args = ["%" + _like_escape(t) + "%" for t in terms]
    try:
        with sqlite3.connect("file:%s?mode=ro" % CATALOG_DB, uri=True) as con:
            # 多取一条判断是否截断;非整包优先(单片比整包好转),其次名字短的靠前
            rows = con.execute(
                "SELECT name,sheet,link,is_pkg FROM catalog WHERE %s ORDER BY is_pkg ASC, length(name) ASC LIMIT %d"
                % (where, limit + 1), args).fetchall()
    except Exception as e:
        return {"err": "查询失败: " + str(e)}
    truncated = len(rows) > limit
    rows = rows[:limit]
    items = []
    for name, sheet, link, is_pkg in rows:
        share, rc = _parse(link)
        items.append({"name": name or "", "sheet": sheet or "", "link": link or "",
                      "is_pkg": int(is_pkg or 0), "share": share, "rc": rc})
    log("资源库搜索「%s」→ %d 条%s" % (q, len(items), "(已截断)" if truncated else ""))
    return {"items": items, "total": len(items), "truncated": truncated}


def _parse(link):
    """从分享链抽 share_code + receive_code(115cdn/115/anxia 的 /s/xxx?password=yyy)。"""
    link = link or ""
    m = re.search(r"/s/([0-9a-zA-Z]+)", link)
    share = m.group(1) if m else None
    m = re.search(r"[?&](?:password|pwd)=([^&#\s]+)", link)
    rc = m.group(1) if m else None
    return share, rc
