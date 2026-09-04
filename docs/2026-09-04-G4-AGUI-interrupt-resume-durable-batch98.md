# G4 AG-UI Interrupt/Resume Durable Production Vertical（Batch98）

> 日期：2026-09-04（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §7.2、§7.5、§8.4、§8.6、§13.1–§13.3、§14.3、§15.1–§15.3、§17.2、§24、§25、§28.1
> GUI 真源：`docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md` §3、§6.4、§8、§9、§10、§15
> 基线：R171 / Batch97，`7e13562c477f90c8512c039f60212bb8e1d49797`
> implementation：`9164ac03775e0618712fcdd5648939679a9c04e5`

## 1. 本批关闭什么

Batch97只固定官方0.0.57 wire。本批把同一remote Agent run真正闭合为：

```text
first SafeDialer request
  → RUN_FINISHED outcome=interrupt
  → native0028 pending + requested audit（同事务）
  → Agent AwaitingHuman（lease/cancel/deadline仍生效）
  → actor GET/PUT through typed ApplicationService
  → answer + resolved/cancelled audit（同事务）
  → fresh context/authority reload
  → new protocol run id + exact parentRunId/resume[]
  → second SafeDialer request
  → local durable terminal + interrupt content scrub（同事务）
```

因此`T-EVT-0010`由todo转done；AG-UI十一条event family机器台账均为done。完整G4仍不勾：三家recorded/live provider trace、acting Approval完整thread/computer等独立判据未闭合。

## 2. Native authority

native0028新增`public.remote_agent_interrupts`，不复用`component_human_decisions`：后者的结果是本地compiled component tool exchange，前者的结果是下一次remote protocol invocation，两者不能共用状态或身份。

每行绑定：

- Rust铸造canonical UUIDv7 `request_id`；remote `interrupt_id`不能单独成为URL/control authority；
- deployment、tenant、thread、local run、actor、Bot、AuthGeneration；
- producing protocol run、0..255 ordinal与known-field-only descriptor；
- closed `pending|resolved|cancelled|expired|retired`、closed response status与下一protocol run id；
- DB `requested_at/expires_at/resolved_at`。

remote `expiresAt`只留在untrusted descriptor。本地权威TTL固定30分钟并用PostgreSQL clock判定。named constraints拒绝非UUIDv7 handle、未知descriptor key、错类型、越界position、坏state/response/time与same protocol resume。

schema0028真实结果：47表、477列、342 NOT NULL、268约束、97索引、4触发器；fixture 5600行，SHA-256 `7c6c2351a113423357705e67412679fede1dcdde59cfe19dc77ef8cd2e6d4a2f`，native ledger恰16。regeneration开启与关闭各`1/0/0`。

## 3. Fresh authority与audit-before-resume

`PostgresRemoteInterruptCoordinator`每次request/wait/resolve均重新核：

- run/thread/Bot/actor与当前fencing；
- runtime owner、lease未过期、run仍running；
- actor current AuthGeneration、有role、未revoked；
- direct-thread或channel当前membership。

request rows与`agent.remote_interrupt_requested`在同一个serializable事务；resolved/cancelled answer与对应audit同事务；DB-clock expiry与expired audit同事务。audit payload为空，target只用权威local run id，不记录remote message/schema/metadata或人的answer。exact request/answer replay不重复audit，异值重答409。

真实PG coordinator测试同时证明：other actor list为空且不能拿已知handle回答；两个item按原ordinal返回resolved/cancelled；第二个batch被DB clock推到expired后返回cancelled resume；五条audit顺序与空payload精确。

## 4. Runtime lineage与无界循环防护

`BuiltInAgentRuntime`收到typed interrupt后从Sampling进入AwaitingHuman，等待期间复用统一`await_run_child`，所以user cancel、absolute deadline与lease heartbeat没有旁路。answer commit后回Preparing并重新加载context；credential、assertion、role与grants可以刷新，但以下必须逐字不变：

- local durable run；
- thread；
- Bot；
- remote endpoint。

endpoint在等待期间变化会在第二次provider effect前以closed invalid-response终止，人的payload不会发送给replacement endpoint。单测用两个不同HTTPS endpoint证明provider start次数仍为1。

provider sampling index同时是全run continuation budget：initial加最多8次tool-result或interrupt continuation。即使部署显式关闭wall-clock deadline，remote也不能制造无界human loop/native rows。

## 5. Server、Desktop与GUI

新增：

- `GET /api/me/remote-interrupts`；
- `PUT /api/me/remote-interrupts/{request_id}`。

GET只投影server handle、权威run/Agent、显式`untrustedReason/untrustedMessage/untrustedResponseSchema`与本地DB时间。PUT path只接受canonical UUIDv7；body只接受`resolved`+可选≤64KiB JSON，或`cancelled`+无payload。Server trusted Origin在body parse前，响应no-store。

Desktop custom protocol映射同两条typed command，不复制业务判定。共享Infra composition root只构造一个PostgreSQL coordinator，同时注入ApplicationService与Agent runtime。

conversation UI按current active run过滤pending，remote reason/message只作为Leptos escaped text并放在`data-untrusted-remote-content`边界；response schema保留为untrusted DTO且不执行代码、不产生权限，首版以通用JSON输入承载其值。回答成功后本地移除，1秒权威poll继续纠正状态；中英键集合806逐字相等。此定向/WASM/bundle证据不冒充完整G6 browser golden/AX。

## 6. Terminal retention

所有run terminal路径在既有reasoning/projection scrub旁，同一transaction把本run interrupt行统一改为retired，并清：

- descriptor（含remote message/schema/metadata）；
- response payload；
- resume protocol id；
- resolved actor引用。

保留server request id、scope identity、remote pairing identity、closed response status与时间，供审计关联；不把逻辑清除冒充WAL/backup/replica物理擦除，后者仍属G8。

真实SafeDialer纵向在terminal后联合扫描messages、run_events、audit_events、descriptor与response payload，remote message、伪authority与answer三个canary均为0。

## 7. 实跑证据

- native0028 regeneration on/off：各`1/0/0`；
- PostgreSQL coordinator actor/cancel/expiry：`1/0/0`；
- PostgreSQL shared ApplicationService assembly：`1/0/0`；
- PostgreSQL run-runtime regression：`5/0/0`；
- PostgreSQL Agent完整矩阵：`10/0/0`；
- Agent unit：`55/0/0`；UI unit：`182/0/0`；Server/ Desktop framing定向各`1/0/0`；
- Contracts/Domain/Application/Agent/Infra/Server/Desktop/UI/Testkit all-target/all-feature Clippy `-D warnings`：通过；
- `cargo fmt --all -- --check`、UI wasm32、tools verify、design-lint、css-check、i18n-check：通过；
- offline locked Trunk release连续两次五个top-level artifact SHA逐字相同；
- bundle：WASM gzip `1,897,481 / 3,670,016`，CSS `115,524 / 131,072`，fonts `740,216 / 819,200`，external/inline scripts `1/0`；
- parity=`838/872/1710`，events=`48/44/92`，API=`84/90/174`，fixtures=`22/22/44`，overlay=`1299/403/2/6`，0 violation；
- recount=`71/0/89 skipped`；89条全部因未设置`OPENBOT_UPSTREAM_DIR`，strict没有冒充通过；
- `grok-inventory --check`=2110 files，Git tree仍`86f5a85f560f721677fa7e587a67ac0ffc036cb5`；非Grok `package.json`恰1，零npm；Cargo/workflow无变化。

临时PostgreSQL已fast stop并删除精确目录，127.0.0.1:55498无响应。

## 8. 未声称完成

- G4整关仍缺三家recorded/live provider trace、acting Approval完整thread/computer集成、computer runtime budget等；
- G6仍缺完整routes/components/AppSidebar/Composer、正式Desktop/Wry与golden/AX；
- strict recount、`cargo xtask ci`与Actions未跑；R63继续有效；
- 未push/建PR：GitHub CLI既有token失效；
- v4 G0/G2/G3/G4/G5/G6/G7/G8其余红项均不受本批“AG-UI event family全done”措辞覆盖。
