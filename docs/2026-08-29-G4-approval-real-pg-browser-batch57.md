# OpenBot G4 Approval 真实 PostgreSQL 浏览器竖切 Batch57

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G4-approval-pg-browser`

基线：Batch56 PR #39 已以 merge commit `57e50163114668e2b0a642804b1a003b3b56b255` 合入 `main`。

implementation：`2cecfeab7e225cb0ad3bd4ae134c26193aa0c30b`

## 1. 结论

本批关闭 v4 §24.1 G4 中“approval 真实 PostgreSQL 浏览器端到端”子项，但不把 testkit auth、单次acting approval或一个run外推成完整thread集成/G4整关：

```text
fresh PG17.11 SCRAM database
  → production baseline + native 0013–0023
  → real user/roles/Agent/thread/member/run/lease
  → PostgresToolApprovalCoordinator::request_and_wait
  → durable pending + requested audit
  → production ApplicationService + Axum + approval WebSocket/GET/POST
  → release Leptos/WASM browser card
  → human click Grant
  → granted row + summaries cleared + granted audit + waiter released
  → hard reload remains empty
```

产品代码/API/ledger均未新增；变化只在`required-features=testkit`的既有`openbot-ui-fixture`。默认内存fixture保持原样，只有显式设置`OPENBOT_UI_APPROVAL_DATABASE_URL`才进入PG模式。

## 2. 为什么扩展既有 fixture

Batch15/56 的内存fixture能证明真实WASM、Axum framing、WebSocket、DOM与交互，但不能证明：

- browser card确实来自native0020查询；
-点击会提交真实state CAS而非改一个Mutex；
- resolved summary真的清NULL；
- requested/granted audit与状态同一durable路径；
- `request_and_wait`真的被浏览器决定释放；
- hard reload不会从内存假状态复活。

另起product route或把seed代码放production main都会扩大面。既有fixture本来就是GUI first-source的testkit host，复用它可保留同一release bundle与ServerBuilder，同时把approval port切到真实`PostgresToolApprovalCoordinator`；其它sidebar/auth数据继续固定，避免本批偷带整个登录系统。

## 3. 构造性防误写

PG模式不会接受任意数据库URL：

- host必须逐字等于`127.0.0.1`；
- dbname必须以`openbot_ui_approval_fixture_`开头；
- dbname最多63字节，字符只允许`[a-z0-9_]`；
- URL只从环境变量读取，不进argv、仓库或普通日志；
- 默认未设置env时不连接数据库，PG proof API也不挂载；实测`/api/__fixture/approval-pg-proof`为404。

最终证据使用全新临时cluster中的专用库`openbot_ui_approval_fixture_batch57`。setup在事务中seed固定scope；fixture identity、request与SQL由一个pure unit双向检查，防止actor/Bot/thread/run常量漂移。

## 4. proof 的闭合口径

`/api/__fixture/approval-pg-proof`只在PG testkit模式挂载，查询同一临时库并只返回计数/闭集waiter状态，不返回approval id、参数、hash、数据库文本或凭据。

浏览器打开前实得：

```json
{"approvals":1,"granted":0,"grantedAudits":0,"mode":"postgres","pending":1,"requestedAudits":1,"summariesCleared":0,"waiter":"waiting"}
```

浏览器批准后实得：

```json
{"approvals":1,"granted":1,"grantedAudits":1,"mode":"postgres","pending":0,"requestedAudits":1,"summariesCleared":1,"waiter":"granted"}
```

这组相等式同时堵住“只改UI”“只改DB但waiter未醒”“audit缺失”“摘要残留”四种假绿色。

## 5. 最终真实运行

本轮最终实例：

- PostgreSQL `17.11 (Homebrew)`；
- `password_encryption=scram-sha-256`；role password前缀检查为真；
- 只监听`127.0.0.1:55457`；host/local均SCRAM；
- Axum只监听`127.0.0.1:39057`；fixture打印`OPENBOT_UI_APPROVAL_MODE=postgres`；
- browser card含权威target `workspace/reports/q4.txt`与`[redacted]`，批准按钮可用；
- click后article=`0`、status=`已提交批准。`、console warn/error=`0`；
- server打印`OPENBOT_UI_APPROVAL_WAITER=granted`；
- hard reload后article仍`0`；
- Ctrl-C后PG fast shutdown，`pg_isready`无响应，data/pwfile/script路径均零残留。

浏览器使用的release/offline/locked bundle与Batch56 implementation完全相同；本批没有UI源码/CSS/i18n变化。auth仍是testkit FixedAuthResolver，但actor/deployment/tenant/AuthGeneration与PG seed/request逐字段相等；因此本批关闭的是“真实PG approval browser state/decision竖切”，不冒充production session-cookie浏览器矩阵。

## 6. 本轮机械证据

| 面 | 结果 |
| --- | --- |
| `cargo test -p openbot-server --bin openbot-ui-fixture --features testkit --locked` | `1/0/0`；request/seed/proof state与DB guard闭合 |
| fixture bin Clippy `-D warnings` | exit 0 |
| memory default mode | `OPENBOT_UI_APPROVAL_MODE=memory`；既有probe200；PG proof API 404 |
| real PG browser | 初始/终态proof逐字如§4；card1→0、status、hard reload、console0 |
| `cargo xtask parity-check` | `695/999/1694`；overlay `1600/92/2/0`；本branch required revalidate=0；0违反 |
| fmt/diff | `cargo fmt --check`与`git diff --check`通过 |

## 7. 明确未做

- 未把testkit proof endpoint或database env接入production server/main；没有新产品API/T-ID。
- 未完成production PostgreSQL session-cookie浏览器登录矩阵；本批auth仍是固定测试身份。
- 未完成approval与完整thread transcript/cancel/computer generation的同一浏览器旅程。
- 未运行`cargo xtask ci`或Actions（R63 manual-only）。
- 未推进Windows/runsc P1 runtime，未进入P2。
- 未修改`grok-bot/`，未新增Grok产品能力，未使用npm。
