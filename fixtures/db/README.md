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
