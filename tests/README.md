# emby-manager 单元测试

纯标准库 `unittest`,零外部依赖。

## 跑法

```sh
# 全部跑(推荐)
./tests/run.sh
# 或等价
cd /Users/gaotao/code/emby-manager && python3 -m unittest discover tests -v

# 跑单个文件
python3 -m unittest tests.test_qscore -v
python3 -m unittest tests.test_password -v
# ...

# 直接跑(每个 test_*.py 自带 __main__)
python3 tests/test_qscore.py -v
```

全部用例预期 < 3 秒(PBKDF2 ~12 次 hash 占大头,~0.8s)。

## 覆盖范围

每个 test 文件对应 app.py 里一组**纯函数**(不依赖 NAS / Emby / 115 网络):

| 文件 | 函数 | 用例数 |
|---|---|---|
| `test_import_smoke.py` | `import app` + 关键符号存在性 | 3 |
| `test_qscore.py` | `qscore(s)` 文件名画质打分 | 10 |
| `test_path_guard.py` | `_safe_under(base, name)` 防 path traversal | 14 |
| `test_password.py` | `_hash_password` / `_verify_password` (PBKDF2) | 12 |
| `test_c115_parse.py` | `c115_parse_url(url, pwd)` 解析 115 分享链接 | 14 |
| `test_c115_internals.py` | `c115_list_dirs` / `c115_snap_full` 解析逻辑(mock `_c115_req`) | 11 |
| `test_compact_eps.py` | 区间压缩 + episode 解析 / 格式化 | 21 |

合计 **85 个用例**,跑全程 < 1s。

## 不测什么

下列功能**不在单元测试范围**(都依赖外部 IO,跑测时拿不到):

- NAS 文件系统操作(`scan_lib`、`_del_folder`、`move_item`)
- Emby HTTP API(`eget` / `epost` / `edelete` / `fetch_libs` / `apply_match`)
- 115 网盘真实请求(`_c115_req`、`c115_save_to_lib`)
- HTTP server / handler(`H` / `do_GET` / `do_POST`)
- 系统命令(`docker ps`、`ps aux`、`/proc/meminfo`)

这些通过手动 / 集成测试(在 NAS 上跑实际服务)验证。

## 注意

- `_safe_under` 在 macOS 上 realpath 会把 `/var/folders/...` 解为 `/private/var/folders/...`,
  断言写成 `out.startswith(realpath(base) + sep)` 兼容。
- `_hash_password` 用 200 000 次迭代,单次约 65ms。12 个用例总耗时 < 1s,可接受。
  **不要 patch 迭代次数 ——** 生产用的也是 200k,patch 了 test 就跟生产行为不一致。
- `test_compact_eps.py` 里 `_ref_compact` / `_ref_eps` / `_ref_fmt_eps` 是从 app.py
  的闭包**抄出来的等价实现**,因为当前函数嵌在 `analyze_dups` / `series_gaps` 内,
  没法 import。**等 H-5 模块拆分把它们提到顶层(或拆 lib/)之后,把那三个
  `_ref_*` 换成 `from app import ...`** —— 文件顶部有 TODO 注释。
- c115 测试用 `unittest.mock.patch.object(app, '_c115_req', ...)` 拦在 `_c115_req`
  这一层,**不打** `urllib.request.urlopen` —— 因为我们的契约是"_c115_req 返回 dict",
  打那层失去了对解析层的覆盖。

## 测试期间发现的 bug

写测试时发现 `analyze_dups.eps` 的正则是 `r's(\d{1,2})e(\d{1,3})'` —— **episode
最多识别 3 位**。海贼王这种已经播到 s01e1163 的剧,文件名会被错误解析成 (s=1, e=116),
后面 "3" 被吞。`test_compact_eps.test_absolute_three_digit_episode` 现在断言的是
**当前 buggy 行为**(锁住基线),并在注释里标注了修复方法:把 `{1,3}` 改成 `{1,4}`
或更松。修复后顺手把那个 case 的期望改成 `{(1, 1163)}`。
