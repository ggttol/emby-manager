"""Pure helpers for media duplicate analysis.

This module intentionally has no CFG / filesystem / Emby dependencies, so
tests can lock scoring and episode parsing behavior without importing the
large business orchestration module.
"""
import collections
import re


def qscore(s):
    """Filename/content path -> quality score used by duplicate sorting."""
    p = (s or "").lower()
    sc = 0
    if re.search(r"2160p|\buhd\b|\b4k\b|2160", p):
        sc += 4000
    elif "1080p" in p or "1080i" in p:
        sc += 2000
    elif "720p" in p:
        sc += 1000
    elif "480p" in p or "dvdrip" in p:
        sc += 300
    if "remux" in p:
        sc += 800
    elif "bluray" in p or "blu-ray" in p or "bdrip" in p:
        sc += 400
    elif "web-dl" in p or "webdl" in p or "webrip" in p or ".web." in p:
        sc += 200
    elif "hdtv" in p:
        sc += 100
    # Bare "dv" must be token-like; dvd/dvdrip/advengers are not Dolby Vision.
    if re.search(r"(?<![a-z])dv(?![a-z])|杜比视界|dovi|dolby.?vision", p):
        sc += 60
    if "hdr" in p:
        sc += 30
    return sc


_EXTRA_RE = re.compile(
    r"花絮|预告|片花|彩蛋|特典|菜单|making[ ._-]?of|sample|trailer|preview|featurette"
    r"|(?:^|[ ._/\-])(?:ncop|nced)\d{0,2}(?=[ ._\-]|\.[a-z0-9]+$|$)"
    r"|(?:^|[ ._/\-])(?:sp|op|ed)\d{1,3}(?=[ ._\-]|\.[a-z0-9]+$|$)",
    re.IGNORECASE,
)


def _is_extra(name):
    """Whether a media path looks like trailer/sample/special content."""
    return bool(_EXTRA_RE.search(name or ""))


def episode_set(media_paths):
    """Extract {(season, episode)} pairs from SxxExxxx style names."""
    out = set()
    for path in media_paths:
        m = re.search(r"s(\d{1,2})e(\d{1,4})", (path or "").lower())
        if m:
            out.add((int(m.group(1)), int(m.group(2))))
    return out


def compact_ints(values):
    """Sorted integer sequence -> compact ranges like ['1-3', '5']."""
    if not values:
        return []
    xs = sorted(values)
    out = []
    start = prev = xs[0]
    for x in xs[1:]:
        if x == prev + 1:
            prev = x
            continue
        out.append(str(start) if start == prev else "%d-%d" % (start, prev))
        start = prev = x
    out.append(str(start) if start == prev else "%d-%d" % (start, prev))
    return out


def fmt_eps(episodes):
    """{(s,e),...} -> 'S01 · E1-2,5' or 'S01E1 · S02E1'."""
    if not episodes:
        return ""
    by_s = collections.defaultdict(list)
    for season, ep in episodes:
        by_s[season].append(ep)
    if len(by_s) == 1:
        season = next(iter(by_s))
        return "S%02d · E%s" % (season, ",".join(compact_ints(by_s[season])))
    return " · ".join(
        "S%02dE%s" % (season, ",".join(compact_ints(by_s[season])))
        for season in sorted(by_s)
    )
