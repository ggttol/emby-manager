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

当前全量约 300 个用例,预期 < 3 秒(PBKDF2 hash 占一部分时间,其余多为 mock / 临时目录 / 本地 HTTP server)。

## 覆盖范围

测试仍然保持纯标准库 `unittest`,但已经不只覆盖纯函数。当前覆盖大致分成几类:

| 范围 | 代表文件 |
|---|---|
| import smoke / 前端静态约束 | `test_import_smoke.py`, `test_frontend_static.py` |
| 密码、XFF、CSRF、HTTP handler、安全头、body 限制 | `test_password.py`, `test_xff.py`, `test_http_handler.py` |
| 配置迁移、原子保存、导出导入、导入白名单和值校验 | `test_config_import_export.py`, `test_analysis_fixes.py` |
| path traversal、strm 列表、挂载探针、扫描保险丝、权限修复 | `test_path_guard.py`, `test_strm_list.py`, `test_analysis_fixes.py`, `test_autostrm.py` |
| qscore、集号压缩、去重/替换/智能归档防误删 | `test_qscore.py`, `test_compact_eps.py`, `test_replace_folder.py`, `test_review_fixes.py` |
| 115 解析、snap 分页、离线下载、catalog 查询和校验脚本 | `test_c115_parse.py`, `test_c115_internals.py`, `test_offline.py`, `test_catalog.py`, `test_validate_catalog_115_links.py` |
| 任务队列、取消、pending 语义、scheduler CRUD / due / 重叠保护 | `test_tasks.py`, `test_scheduler.py` |
| 追更异步、业务回归修复、日志持久化 | `test_zhuigeng_async.py`, `test_review_fixes.py`, `test_logger.py` |

## 不测什么

下列功能**不做真实外部端到端**(都依赖 NAS / Emby / 115 实例):

- NAS 文件系统操作(`scan_lib`、`_del_folder`、`move_item`)
- Emby 真实 HTTP API(`fetch_libs` / `RemoteSearch` / `Apply` 等)
- 115 网盘真实请求(`_c115_req`、`c115_save_to_lib`)
- 系统命令(`docker ps`、`ps aux`、`/proc/meminfo`)

这些通过 mock / 临时目录 / 本地 fake HTTP server 覆盖协议和边界,真实 NAS 行为仍需部署后手测。

## 注意

- `_safe_under` 在 macOS 上 realpath 会把 `/var/folders/...` 解为 `/private/var/folders/...`,
  断言写成 `out.startswith(realpath(base) + sep)` 兼容。
- `_hash_password` 用 200 000 次迭代,单次约 65ms。12 个用例总耗时 < 1s,可接受。
  **不要 patch 迭代次数 ——** 生产用的也是 200k,patch 了 test 就跟生产行为不一致。
- `test_compact_eps.py` 直接测试 `lib.dedup` 的真实 helper,不要再复制业务闭包逻辑;否则测试会和生产实现分叉。
- c115 测试用 `unittest.mock.patch.object(app, '_c115_req', ...)` 拦在 `_c115_req`
  这一层,**不打** `urllib.request.urlopen` —— 因为我们的契约是"_c115_req 返回 dict",
  打那层失去了对解析层的覆盖。

## 回归测试来源

不少测试是事故回归用例:挂载死时不能删光 strm、配置损坏要从 `.bak` 恢复、schedule 不能被
重启残留的 `running` 永久卡死、Emby 删除要先 `edelete` 再动磁盘、115 snap 要分页、配置导入
不能覆盖安全字段等。新增功能时优先把类似“以后不能再踩”的行为写成小测试。
