# `fixtures/db/` —— 数据库 schema 参照事实

## 这是什么

`schema-0012.json` 是**上游 OpenBot 走完全部 13 条 migration 之后**（即 `0012_truncate_is_not_a_way_around_append_only.sql` 之后）真实 PostgreSQL 库的 schema 事实，导出成规范 JSON。

它是 v3 §24 G1 判据「28 表/13 migration 映射」的**比对目标**：`openbot-infra` 的 baseline DDL 建出来的库，提取同一份事实后必须与本文件逐字段相等。

## 为什么要入库，而不是每次现跑 migration

CI 里没有上游克隆（本仓与上游是两个仓库，且 §16.3 的供应链闸门不允许构建期拉外部源）。把参照事实固化进仓，闸门才有一个不依赖外部网络与外部 checkout 的锚。

上游漂移由 `upstream_commit` 承担：本文件对应 `891df72f1827454d8b353d108fe5dd2313b7e30d`。上游换 commit 就必须重生成本文件并在同一个 PR 里说明差异。

## 内容摘要（可复算，见下）

| 项 | 数量 |
| --- | ---: |
| 表 | 28 |
| 列 | 204 |
| NOT NULL 列 | 153 |
| 约束（不重复计 NOT NULL） | 59 |
| 索引 | 44 |
| 触发器 | 2 |
| enum 类型 | 4 |
| 函数 | 1 |
| extension | 0 |

## 生成过程（2026-08-22 实跑）

1. 取上游固定 commit 的 `server/drizzle/0000..0012` 共 13 个 `.sql`。
2. **两处替换**（本机没有安装 pgvector；下面证明它不影响 0012 终态）：
   - `CREATE EXTENSION IF NOT EXISTS vector;` 整行注释掉；
   - `chunks` 表的 `"embedding" vector(1536) NOT NULL` 改为 `"embedding" text NOT NULL`。

   替换范围经 `diff --strip-trailing-cr` 实证**恰好只有这 2 行**，其余逐字未动。
3. 按序 `psql -v ON_ERROR_STOP=1 -f` 应用到一个全新数据库，13 条全部成功。
4. 用 `crates/openbot-infra/sql/schema_facts.sql` 提取事实，`json.dumps(..., indent=2, sort_keys=True)` 规范化。

### 那两处替换为什么不污染终态（正向对照齐备）

`0010_drop_the_document_index.sql` 会 `DROP TABLE chunks` 并 `DROP EXTENSION IF EXISTS "vector"` —— 也就是说 `chunks` 与 `vector` 在 0012 终态里本来就该不存在。被替换的两处**全部位于 `chunks` 这张注定被删的表上**，`vector` 类型在上游也只服务它一列（`0010` 的注释逐字写着 "`vector` existed for the `embedding` column on `chunks` and for nothing else"）。

实测断言（每条"期望 0"都配了一条同判据的正向对照，否则该断言在"这个查询压根查不到东西"的世界里同样成立）：

| 断言 | 实得 | 正向对照 | 实得 |
| --- | ---: | --- | ---: |
| `information_schema.tables` 里 `chunks` 的条数 | 0 | 同查询查 `users` | 1 |
| `pg_type` 里 `vector` 的条数 | 0 | 同查询查 `role` | 1 |
| `public` 下 extension 数 | 0 | `public` 下表数 | 28 |

另外 `0000` 头部注释点名"必须存活"的 append-only 触发器实测存活两个：`audit_events_append_only` 与 `audit_events_no_truncate`。

## PostgreSQL 17 重验与跨版本归一化（2026-08-23）

v3 §14.1 把数据库钉在 **PostgreSQL 17**。本轮在 PostgreSQL **17.11** 上重跑 baseline 与
`schema_facts.sql`，原先由 PostgreSQL 18.1 生成的 fixture 在 28 张表上都只差约束集合：

- 18.1 fixture：`f=27 / n=153 / p=28 / u=4`，合计 212；
- 17.11 活库：`f=27 / p=28 / u=4`，合计 59；
- 18.1 多出的 153 条 `contype='n'` 全部是 `NOT NULL <列名>`，数量与
  `columns[].notnull=true` 的 153 列逐项相等。

这不是 DDL 语义差异，而是 PostgreSQL 18 把已经存在于列事实里的 NOT NULL 又暴露为
`pg_constraint` 对象。提取器现显式排除 `contype='n'`，NOT NULL 仍由 `columns[].notnull`
逐列验证。把旧 fixture 的 153 条 `n` 对象移除后，与 17.11 活库提取结果整棵 JSON
结构化比较为 `True`（59 / 59）；本文件据此由 PostgreSQL 17.11 重生成。这样既以目标版本为准，
也避免同一事实重复计数。

## `schema-0013.json`：Rust-owned expand 终态

`schema-0013.json` 是在同一 PostgreSQL **17.11** 一次性事实库上按
`baseline_0012.sql → native_0013.sql → schema_facts.sql` 顺序实跑生成的 post-migration
fixture。它不改写 `schema-0012.json`：前者回答“当前 Rust schema 是什么”，后者继续回答
“固定上游 13 条 migration 的终态是什么”。

| 项 | 0013 数量 | 相对 0012 |
| --- | ---: | ---: |
| public 表 | 31 | +3 |
| 列 | 248 | +44（含 `audit_events` 两个 nullable hash 列） |
| 约束（不重复计 NOT NULL） | 93 | +34 |
| 索引 | 53 | +9 |
| 触发器 | 4 | +2 |
| enum 类型 | 4 | 0 |
| public 函数 | 1 | 0 |
| extension | 0 | 0 |

三张新增 public 表恰好是 `audit_checkpoints` / `tool_attempts` / `tool_calls`。
`openbot_internal.schema_migrations` 与 `openbot_internal.prevent_append_only_mutation()` 位于
internal schema，按定义不进只扫描 `public` 的 schema fixture；它们由
`tests/native_0013.rs` 的账本、并发与回滚用例独立核对。

expand-only 不是靠读 SQL 猜：真库测试
`post_0013_fixture_is_exact_and_every_0012_object_survives` 先提取 0012 事实，再逐表断言原列、
约束、索引、触发器以及 enum/function/extension 全是 0013 的结构化子集，最后才与本 fixture
整棵相等。`object_collision_rolls_back_every_0013_change_and_does_not_forge_ledger` 再制造同名异形
表，实证失败事务不会留下 hash 列、checkpoint 表或伪账本。

## `schema-0014.json`：user auth generation

W-3 开工前的调用链审计发现：`AuthContext` 与领域 `AccessChangeRequest` 都要求权威
`auth_generation`，撤权还要求在同一事务把 subject 的 generation 递增；但 0013 之前没有任何
持久化列。只放内存会在重启/多副本后回到旧值，拿 actor 自己的 generation 又会错绑到被管理者。

0014 因此只做一次 expand：`users` 末尾追加 nullable `auth_generation bigint` 与非负 CHECK。
旧行 NULL 在读侧等价于 generation 0，第一次角色/撤权写入用
`coalesce(auth_generation,0)+1` 变成 1；兼容窗口内不 `SET NOT NULL`。

| 项 | 0014 数量 | 相对 0013 |
| --- | ---: | ---: |
| public 表 | 31 | 0 |
| 列 | 249 | +1 |
| 约束 | 94 | +1 |
| 索引 | 53 | 0 |
| 触发器 | 4 | 0 |

真库测试 `post_0014_is_exact_expand_only_and_null_legacy_generation_is_zero_floor` 先与 0013
fixture 相等，再施加 0014，逐旧列证明无改写，并实测 6 个 seed user 均为 NULL、`NULL→1`、
负数命中 `users_auth_generation_nonnegative`。`two_replicas_apply_0013_and_0014_exactly_once`
实得恰好 `Applied + AlreadyApplied`。

## `schema-0015.json`：Rust session 签发代际

上游 `sessions.token` 是可直接使用的明文 token，且 session 行没有“签发时 auth generation”。
第一真源 §6.3 要求旧 Better Auth session 在切换时统一失效，并要求 role/access generation 更新
立即让旧 session/ticket/capability 失效。0015 因此只在 `sessions` 末尾追加 nullable
`auth_generation bigint` 与非负 CHECK：新 Rust session 写当前 generation；旧行保持 NULL，
token 也没有 Rust keyed-hash 前缀，resolver fail-closed 要求重新登录。兼容窗口不回填、不
`SET NOT NULL`。

| 项 | 0015 数量 | 相对 0014 |
| --- | ---: | ---: |
| public 表 | 31 | 0 |
| 列 | 250 | +1 |
| 约束 | 95 | +1 |
| 索引 | 53 | 0 |
| 触发器 | 4 | 0 |

PG17 测试 `post_0015_is_exact_expand_only_and_legacy_sessions_remain_unclaimed` 逐对象证明只有
sessions 多一列/一约束，6 条旧 seed session 全为 NULL，typed 新值 0 可读回，负数命中具名
CHECK；`two_replicas_apply_0013_through_0015_exactly_once` 实得恰好
`Applied + AlreadyApplied`。

## `schema-0016.json`：native thread/realtime/memory base

`schema-0016.json` 在同一 PostgreSQL **17.11** 隔离 SCRAM 实例上按
`baseline_0012.sql → native_0013.sql → native_0014.sql → native_0015.sql → native_0016.sql →
schema_facts.sql` 机械生成，SHA-256
`3a9ca0e2292e25171785c526047c279291c1671a357195b603efa2b998616877`。

| 项 | 0016 数量 | 相对 0015 |
| --- | ---: | ---: |
| public 表 | 41 | +10 |
| 列 | 351 | +101 |
| NOT NULL 列 | 268 | +83 |
| 约束 | 181 | +86 |
| 索引 | 80 | +27 |
| 触发器 | 4 | 0 |
| enum / public 函数 / extension | 4 / 1 / 0 | 0 / 0 / 0 |

十张新表集合恰为 `threads` / `thread_memberships` / `messages` / `runs` / `run_events` /
`thread_leases` / `outbox` / `memories` / `memory_events` / `intelligence_import_cursors`。
`tool_calls` 只增加一个 `NOT VALID` 的 `run_id → runs` FK：它约束迁移后的新写，但不扫描并
伪造历史 run；导入/backfill 完成后再由独立 migration `VALIDATE CONSTRAINT`。

真库测试 `post_0016_is_exact_expand_only_and_tool_fk_is_staged_not_validated` 先固定 0015 事实，
再逐旧列证明未改写并与本 fixture 整棵相等；行为测试覆盖 foreground partial unique、terminal
event exactly-once、fencing takeover、replay、outbox replay-safe/claim/delivery、memory scope/source/
删除清空与新 tool FK；双 replica 仍实得 `Applied + AlreadyApplied`。40 个具名 repository 由
`all_forty_current_repositories_touch_their_real_tables` 逐个触表，不以空 struct 占名。

## 复算命令

```bash
# 0012 摘要表里的八个数
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('tables',len(d['tables']))"                                    # 28
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('columns',sum(len(t['columns']) for t in d['tables']))"       # 204
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('notnull',sum(c['notnull'] for t in d['tables'] for c in t['columns']))" # 153
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('constraints',sum(len(t['constraints']) for t in d['tables']))" # 59
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('indexes',sum(len(t['indexes']) for t in d['tables']))"       # 44
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('triggers',sum(len(t['triggers']) for t in d['tables']))"     # 2
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('enums',len(d['enums']),'functions',len(d['functions']),'ext',d['extensions'])"  # 4 1 []

# post-0013 八个数
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0013.json',encoding='utf-8'));print('tables',len(d['tables']),'columns',sum(len(t['columns']) for t in d['tables']),'constraints',sum(len(t['constraints']) for t in d['tables']),'indexes',sum(len(t['indexes']) for t in d['tables']),'triggers',sum(len(t['triggers']) for t in d['tables']),'enums',len(d['enums']),'functions',len(d['functions']),'ext',len(d['extensions']))"  # 31 248 93 53 4 4 1 0

# post-0014
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0014.json',encoding='utf-8'));print(len(d['tables']),sum(len(t['columns']) for t in d['tables']),sum(len(t['constraints']) for t in d['tables']),sum(len(t['indexes']) for t in d['tables']),sum(len(t['triggers']) for t in d['tables']))"  # 31 249 94 53 4

# post-0015
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0015.json',encoding='utf-8'));print(len(d['tables']),sum(len(t['columns']) for t in d['tables']),sum(len(t['constraints']) for t in d['tables']),sum(len(t['indexes']) for t in d['tables']),sum(len(t['triggers']) for t in d['tables']))"  # 31 250 95 53 4

# post-0016
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0016.json',encoding='utf-8'));print(len(d['tables']),sum(len(t['columns']) for t in d['tables']),sum(c['notnull'] for t in d['tables'] for c in t['columns']),sum(len(t['constraints']) for t in d['tables']),sum(len(t['indexes']) for t in d['tables']),sum(len(t['triggers']) for t in d['tables']))"  # 41 351 268 181 80 4

# 表名集合与 parity/tables.yaml 的上游表条目逐字相等（双向差集都必须为空）
python3 -c "
import json,io,yaml
db={t['name'] for t in json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'))['tables']}
led={e['target'].split('::db::tables::')[1] for e in yaml.safe_load(io.open('parity/tables.yaml',encoding='utf-8'))['entries']
     if e['label']=='parity' and '::db::tables::' in e['target']}
print(len(db), len(led), sorted(db-led), sorted(led-db), db==led)
"   # 28 28 [] [] True
```

对不上 = 上游 schema 漂了、或本文件被手改过 → 按上面的生成过程重生成，并在同一个 PR 里说明差异。**不要手工编辑本文件**。
