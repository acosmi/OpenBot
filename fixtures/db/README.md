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
| 约束 | 212 |
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

## 已知差异：PostgreSQL 版本

v3 §14.1 把数据库钉在 **PostgreSQL 17**；本文件是在本机 **PostgreSQL 18.1** 上生成的（`x86_64-windows, compiled by msvc`）。

这条差异**未消除**，如实登记：本文件里的 `format_type()` 输出、`pg_get_constraintdef()` / `pg_get_indexdef()` 的文本形态理论上可能随服务端版本变化。解除条件 = 在 PostgreSQL 17 上重跑一次生成过程并 diff；差异为空则把本节改成"已在 17 与 18.1 上各生成一次，结果相同"。在那之前，任何"本文件等价于 17 上的结果"的说法都是未验证的。

## 复算命令

```bash
# 摘要表里的七个数
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('tables',len(d['tables']))"                                    # 28
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('columns',sum(len(t['columns']) for t in d['tables']))"       # 204
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('constraints',sum(len(t['constraints']) for t in d['tables']))" # 212
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('indexes',sum(len(t['indexes']) for t in d['tables']))"       # 44
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('triggers',sum(len(t['triggers']) for t in d['tables']))"     # 2
python3 -c "import json,io;d=json.load(io.open('fixtures/db/schema-0012.json',encoding='utf-8'));print('enums',len(d['enums']),'functions',len(d['functions']),'ext',d['extensions'])"  # 4 1 []

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
