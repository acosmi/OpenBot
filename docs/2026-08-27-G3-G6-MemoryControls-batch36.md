# Batch 36：Memory Controls 与 `/settings/memory`

> 日期：2026-08-27。分支：`codex/2026-08-27-G3-memory-controls`。
> base：Batch35交付head `5bd272d5e602162660ea1ade433ee8be8e07d78c`；
> implementation：`bec30ec52e3fbaabe3aa3f08a5de0d1e7bd4f991`。
> 固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批只关闭第一真源 §3.1 条7、§4.3 条8–11要求的用户 Memory Controls/GUI 与其生产依赖。
未运行`cargo xtask ci`，未派发Actions，未生成正式golden，未做actual legacy exporter/
production bundle三次演练；这些边界仍阻止G3/G6整关勾选。既有未跟踪`docs/assets/`未修改、
未暂存、未提交。

## 1. 第一性裁决

R66已经证明六条memory API和PostgreSQL事务，但它们不能替代用户可见GUI；同时，第一真源要求
“整体禁用写入”，此前没有任何权威状态可供GUI、correct和remember tool共同消费。

本批固定以下不变量：

1. 全局开关是runtime数据治理，不是主题/语言渲染偏好，因此不写入`user_ui_preferences`；
2. 开关也不是一条特殊memory，不得进入list、FTS或recall；
3. `user_memory_controls`缺行解释为enabled，旧部署升级后不因没有回填而突然拒绝写入；
4. disabled拒绝所有会保留新content的runtime入口：GUI remember、correct、built-in `remember`
   tool；verified importer是一次性迁移边界，不在runtime开关内；
5. disabled绝不拒绝list、recall、forbid、delete。用户关闭未来写入后仍必须能查看和擦除既有数据；
6. 拒绝稳定投影为`policy_refused` + rule/code `memory_writes_disabled`，不携memory content；
7. 页面不optimistic修改memory row。写成功后重新读取权威页；commit unknown不冒充成功；
8. 没有后台抽取、learning job、document index或pgvector；memory仍只有explicit preference/fact。

## 2. 实施面

### 2.1 native 0022

- 新表`user_memory_controls(tenant_id, actor_user_id, writes_enabled, updated_at)`；
- tenant+actor复合主键、两字段nonempty CHECK、actor→users级联FK；
- `NATIVE_0022_VERSION/NAME/SQL`进入唯一migration数组、checksum、expand-only/idempotency与
  table registry机械闸门，latest从0021推进到0022，native ledger条数从9变10；
- `fixtures/db/schema-0022.json`由PostgreSQL 17.11活库guarded regeneration生成，不手写。

终态事实：44张public表、408列、299个NOT NULL、225个约束、87个索引、4个trigger、
4个enum、1个public函数、0 extension；fixture SHA-256为
`f7dfda29296bb67f08bfa1c514b376a14dd403eb2f87ced26569d19614c5ee25`。

### 2.2 typed authority 与生产写约束

- contracts新增closed `MemoryControl`/`UpdateMemoryControl`、Get/Update command与reply；wire没有
  actor/tenant/time字段；
- application新增authority-only request、use case、service operation与port；actor/tenant只取
  `AuthContext`；
- PostgreSQL read缺行返回enabled，update使用数据库时钟upsert；
- GUI remember、correct、remember tool在各自既有transaction内复读control，关闭时零content写；
- tool executor把拒绝映射为`NotCommitted` + `memory_writes_disabled`；
- Agent、Server channels exhaustive reply与transport parity同步纳入新variant。

### 2.3 HTTP 与 Leptos

- `GET /api/memories/control`：authenticated、closed DTO、`Cache-Control: no-store`；
- `PUT /api/memories/control`：trusted Origin先于body parse，只接受`writesEnabled`；
- 既有memory list/recall/create/correct/forbid/delete响应统一`no-store`；
- `/settings/memory`与AppSidebar Memory destination进入production route set；
- 页面首屏50条，owner-scoped memory-id cursor load-more；展示status、kind、sensitivity、scope、
  source、origin、created time、tags与content-erased；
- active记录可correct；forbid/delete不受writes switch影响；
- correct dialog取消返原按钮。成功后原按钮会因旧记录变superseded而消失，因此等待权威refetch，
  再聚焦新replacement行；forbid/delete完成后聚焦同一变更行；
- DOM id只使用memory id的SHA-256截断映射，不把raw id用作selector；中英占位符集合逐字相等。

### 2.4 确定性浏览器fixture

`openbot-ui-fixture`新增52条memory、四种状态、user/bot/thread scope、fact source、normal/sensitive、
三种origin、control持久态与correct/forbid/delete语义；fixture只服务本地release WASM验收，不冒充
production PostgreSQL证据。

## 3. 本机机械证据

| 面 | 结果 |
| --- | --- |
| contracts / application / Agent / Server / UI | **77 / 138 / 28 / 203 / 108**，均0失败 |
| infra host | **306 / 0 / 0**（14个loopback/TLS项在受限沙箱被OS拒绝后，以同命令在允许本地绑定环境重跑全绿） |
| Server memory HTTP | **5 / 0 / 0** |
| Axum/in-process transport | 总矩阵 **8 / 0 / 0**；memory专用 **1 / 0 / 0** |
| PostgreSQL 17.11 SCRAM | schema0022 regeneration开/关各 **1 / 0 / 0**；memory全旅程 **3 / 0 / 0** |
| Clippy | contracts/application/Agent/infra/Server/testkit + UI，`-D warnings`全绿 |
| WASM | contracts/UI `wasm32-unknown-unknown`通过 |
| i18n / design / CSS | **452** leaf；**72 Rust / 74 icons**；**215** source class literals |
| release bundle | WASM gzip **870,212 B**；CSS **73,367 B**；fonts **740,216 B**；external/inline **1/0** |
| parity | API **52/115/167**；routes **2/30/32**；UI **85/67/152**；总计 **642/1036/1678** |
| fixtures | **16/22/38** |
| strict fixed-upstream recount | **157 / 157 / 0** |
| parity violations | **0** |

PostgreSQL实例由`initdb`以host `scram-sha-256`创建；实测`server_version=17.11`、
`password_encryption=scram-sha-256`、当前role password为`SCRAM-SHA-256`。迁移测试先施加到0021并与
旧fixture整棵相等，再施加0022；关闭regeneration后再次从空临时库重跑，活库与入库fixture逐字段相等。

memory真库同时证明：缺行enabled；关闭后GUI remember、correct、remember tool均返回
`WritesDisabled`且零新content；delete仍成功并擦除；重新启用后remember恢复；另一tenant的control不串扰。
既有explicit scope/pagination/correct/supersede/forbid/delete/recall与末段event失败回滚三条均保持通过。

## 4. release WASM浏览器

最终fixture的HTML与CSS分别实得200；CSS MIME=`text/css`、73,367 bytes，document加载
`app-d7dc53e5a0e8f74f.css`且`document.styleSheets`有**445**条规则，Inter Variable生效。

- 首屏50条，load-more后52条且52个row id全部唯一；
- switch true→false后47个correct全disabled，48个forbid与49个delete仍enabled；hard reload后false保持；
- 重新启用后correct恢复；correct产生新active replacement，旧行superseded，dialog关闭后焦点落新行；
- cancel dialog焦点精确返回原correct按钮；
- forbid后status=forbidden/content erased，只剩delete；delete后status=deleted/content erased、action=0；
- zh-CN/en切换后heading/description/control/AX switch name均对应当前locale；
- 1440×900、1024×640、900×640、600×640四档：X overflow=0，main/nav/h1各1，
  switch可见，row computed display=grid，duplicate IDs=0，visible alerts=0；
- console error/warn=0。

验收中用户清理了可再生成的编译产物，正在运行的旧fixture仍持有旧index，但`dist`已删除，旧/新CSS
请求均实得404，首次截图因此退化成浏览器默认样式。该状态没有被计作通过：停止fixture后按
`tools/pins.toml`重建四个钉版工具，`cargo fetch --locked`只补回被清理的Cargo.lock源包缓存，再以
`trunk build --release --offline --locked`重建，重启fixture并完成上述CSS 200/445规则及全部交互复验。

## 5. 台账与未完成边界

- 新增并关闭`T-API-0166/0167`、`T-FIX-0038`；关闭既有`T-ROUTE-0032`；
- `T-UI-0152`是正式golden矩阵条目，本批浏览器截图只作目视QA，故继续todo；
- G3仍因actual legacy exporter/production bundle三次演练未闭合；
- AppSidebar仍缺skills/settings-home/admin destinations；完整channel/home Composer、markdown、
  tool boundary、Screen、golden、Tauri binary/window与其余G4/G5/G7/G8保持todo；
- 临时PostgreSQL只监听127.0.0.1；测试后已停止并删除。浏览器tab与fixture进程已关闭。

