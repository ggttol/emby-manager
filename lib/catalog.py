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


def catalog_search(q, limit=80, link_type=None):
    """按名称 LIKE 多关键词(空格分隔,AND)搜,可选 link_type 过滤(share115/magnet/ed2k)。
    返 {items:[{name,sheet,link,is_pkg,link_type,transfer,share,rc}], total, truncated}。"""
    q = (q or "").strip()
    if not q:
        return {"items": [], "total": 0}
    if not catalog_available():
        return {"err": "资源库未部署(catalog_115.db 不在)"}
    terms = [t for t in re.split(r"\s+", q) if t][:6]  # 最多 6 个关键词,防滥用
    # 最小词长:单字符 term('%' 或 'a')会全表扫 15.7 万行 + filesort,挡掉
    if not terms or all(len(t) < 2 for t in terms):
        return {"items": [], "total": 0, "hint": "关键词太短,至少 2 个字符"}
    conds = ["name LIKE ? ESCAPE '\\'"] * len(terms)
    args = ["%" + _like_escape(t) + "%" for t in terms]
    # 可选按类型过滤(share115 / magnet / ed2k)
    if link_type in ("share115", "magnet", "ed2k"):
        conds.append("link_type = ?"); args.append(link_type)
    where = " AND ".join(conds)
    try:
        with sqlite3.connect("file:%s?mode=ro" % CATALOG_DB, uri=True) as con:
            # link_type 列可能旧库没有 → 兼容查
            cols = "name,sheet,link,is_pkg,link_type" if _has_type_col(con) else "name,sheet,link,is_pkg"
            # 多取一条判断是否截断;115 分享链(秒传稳)优先、非整包优先、名字短靠前
            order = "is_pkg ASC, length(name) ASC"
            if _has_type_col(con):
                order = "(link_type='share115') DESC, is_pkg ASC, length(name) ASC"
            rows = con.execute("SELECT %s FROM catalog WHERE %s ORDER BY %s LIMIT %d"
                               % (cols, where, order, limit + 1), args).fetchall()
    except Exception as e:
        return {"err": "查询失败: " + str(e)}
    truncated = len(rows) > limit
    rows = rows[:limit]
    items = []
    for r in rows:
        name, sheet, link, is_pkg = r[0], r[1], r[2], r[3]
        lt = r[4] if len(r) > 4 else _infer_type(link)
        share, rc = _parse(link)
        items.append({"name": name or "", "sheet": sheet or "", "link": link or "",
                      "is_pkg": int(is_pkg or 0), "link_type": lt,
                      "transfer": (lt == "share115"),   # True=可转存(秒传);False=磁力/ed2k 走离线下载
                      "share": share, "rc": rc})
    log("资源库搜索「%s」→ %d 条%s" % (q, len(items), "(已截断)" if truncated else ""))
    return {"items": items, "total": len(items), "truncated": truncated}


def _has_type_col(con):
    try:
        return any(c[1] == "link_type" for c in con.execute("PRAGMA table_info(catalog)").fetchall())
    except Exception:
        return False


def _infer_type(link):
    l = (link or "").lower()
    if "/s/" in l and ("115cdn.com" in l or "115.com" in l or "anxia.com" in l): return "share115"
    if l.startswith("magnet:"): return "magnet"
    if l.startswith("ed2k:"): return "ed2k"
    return "other"


def _parse(link):
    """从分享链抽 share_code + receive_code(115cdn/115/anxia 的 /s/xxx?password=yyy)。"""
    link = link or ""
    m = re.search(r"/s/([0-9a-zA-Z]+)", link)
    share = m.group(1) if m else None
    m = re.search(r"[?&](?:password|pwd)=([^&#\s]+)", link)
    rc = m.group(1) if m else None
    return share, rc
