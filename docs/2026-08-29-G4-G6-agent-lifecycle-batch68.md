# Batch68：用户 Agent lifecycle、customer Authorization 与运行时权限闭环

> 日期：2026-08-29（America/Los_Angeles）
>
> 分支：`feat/2026-08-29-G4-G6-agent-lifecycle`
>
> base：`423b3549fe175de5c6fed8fedcc7e14e55144a03`（PR #50 merge commit）
>
> implementation：`70da77374b59e3ac89bd6113b06d69e85c6464c8`
>
> 第一真源：v4 §3.1条3、§3.2–§3.4、§5.1–§5.3、§6.4、§7.1–§7.5、§8.6、§13.1–§13.3、§15.1–§15.3、§21.1、§24 G4/G6 与 §28.1 R142；GUI 形态仍服从 v2。

## 1. 本批结论

本批关闭的是用户创建 Agent 的 **Server/Application/PostgreSQL/Vault/SafeDialer/runtime backend**：

- `POST /api/agents/test-connection`、`POST /api/agents`、`PATCH /api/agents/{id}`、duplicate、
  hide、unhide、delete 七条既有 API；
- `package_id IS NULL` 的 managed/remote Agent 可进入 provider context，customer `Authorization`
  每个 run 从当前 active Vault row fresh 解封；
- create/update/duplicate/hide/unhide/delete、credential create/rotate/revoke 与 allowlisted hash-chain
  audit 在同一事务完成，audit/Vault 故障不留下业务写；
- private Agent 的 owner/admin/non-owner read/run 判据在 roster、BeginThreadRun 与 provider context 一致；
- Web 侧已有 create/edit/copy/hide/recover/delete/start-channel 表单与 stale-response fencing。

本批**没有**关闭 T-ROUTE-0007、T-UI-0031/0032：in-app Browser 从错误页导航到 loopback fixture 时被
Browser URL policy 明确拒绝，且工具指示不得换表面或绕过；因此正式 `/agents` 浏览器 journey、AX/双视口与
formal golden 记为 **NOT RUN**。Desktop custom protocol 也没有新增 Agent API，不能把 Web Server 证据外推到
Desktop。P1 Windows/runsc 仍红，P2/P3/P4 未进入。

## 2. 固定输入与范围红线

- 固定上游 clone：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`，clean；只读
  Agent route/form/store/policy/audit/endpoint tests 与页面结构，不复制产品文本。
- `grok-bot/` git tree 保持 `86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff 0；
  `grok-inventory --check` 仍为 2,110 文件。
- 非 Grok 恰一个 `crates/openbot-desktop/engine-shim/package.json`；无 package lock、无 npm。
- 没有新增 Grok 产品能力、没有人工 census、没有改 D2、ExecutionRealm、Linux tier-2 或 CSS 预算。
- R63 保持 manual-only：未运行 `cargo xtask ci`，未派发 GitHub Actions。

## 3. 第一性原理裁决（R142）

固定上游有六条行为不能照译，均在 overlay 记 `superseded`：

| 上游测试 | 不照译原因 | 替代证据 |
| --- | --- | --- |
| T-TEST-0307 / 0324 | forged owner/package/permission 字段若静默忽略，transport 接受了不应存在的 authority 输入 | closed serde + Application 校验，未知字段 400 且 port 调用 0 |
| T-TEST-0334 | Rust port 的错误闭集不允许任意异常跨层 rethrow | T-TEST-0333 的穷举稳定 AppError/HTTP 映射 |
| T-TEST-0336 | 是否挂 API 不能取决于可选 store 装配 | canonical route 恒挂，`NoAgentAdministration` fail-closed 503 |
| T-TEST-0379 | v4 要求 authority mutation 与 audit 原子 | 强制 `bot.created` audit trigger 失败，Agent/credential/audit 全回滚 |
| T-TEST-0384 | Vault 是 customer auth 权威源，拒绝不能被吞 | credential insert/rotate/revoke 与 profile/audit 同事务，故障 fail-closed |

创建/改址 audit 也不记录完整 endpoint。新 allowlist 字段只接受 URL parser 产生的
`scheme://host[:port]`；path/query/userinfo/fragment/Authorization 均无可表达位置。

## 4. 类型与 transport 边界

- contracts 新增 closed `AgentMutationRequest`、write-only `AgentAuthInput`、connection verdict、
  lifecycle receipt；auth 只允许 canonical `Authorization`，Debug 永远 redacted，值由共享 zeroizing
  allocation 持有。
- Application 新增 `AgentAdministrationScope{tenant,actor,admin,auth_generation}`；只从 AuthContext 铸造，
  adapter reply 必须逐字段对应 request/id/state，否则按依赖损坏拒绝。
- `FreshOriginAuthenticated` 在 Json body 前完成 session freshness 与 trusted Origin；普通 owner 写不要求
  admin，但 owner/admin 终判留在 Application/PG。
- connection test 与 runtime 共用 `SafeRemoteAguiTransport`。注册预检只解析/DNS/policy且不开 socket；实际
  POST 和每次 redirect 仍重新解析并绑定校验后的 peer/TLS SNI。`DestinationRejected`、`Unreachable`、
  `Authentication`、`Protocol`、`Inconclusive` 不再混写。

## 5. PostgreSQL、Vault、audit 与竞态

`PostgresAgentAdministration` 每次持久写先锁当前 user generation 并复核 revoke/admin，再锁 Agent/Profile：

- stale AuthGeneration 与非 owner 的 endpoint update 都在 SafeDialer preflight 前拒绝；probe validation
  调用计数不增加；
- package attachment 与 update/delete 真并发时 mutation 阻塞，提交后重读 package 状态并返回 Protected；
- user Agent ID 由 UUIDv7 铸造；duplicate 固定 private/managed，不复制 auth、callback、hidden、channel；
- remote key 只在 `credentials(kind='agent')` 存 v2 ciphertext；configuration 只存 exact UUID reference；
- replace 时旧 key revoked、新 key active；空 key edit 保留当前 key；切 managed/delete 撤销；
- lifecycle event 精确为 created=4、updated=3、duplicated=1、hidden=1、unhidden=1、deleted=2；
  credential created/rotated/revoked 各 1；secret canary 在 config/audit 为 0。

`PostgresAgentContextSource` 把 package join 改为受约束 LEFT JOIN；运行时同时复核 private owner/admin，
对伪造的 non-owner run 返回 Stale。active credential 每 run 重读，撤销后不回落旧 key/环境值。

## 6. Web UI 边界

- `/agents?new=true` 与 `?agent=<id>` 继续 URL-owned；visible 与 hidden roster 分别读同一 typed GET。
- 表单完整覆盖 name/title/standing role/visibility/endpoint/password，connection test 不保存 secret；
  submit/cancel/unmount 清 input state。
- endpoint/auth 改动推进 checked connection generation；旧 probe response 不覆盖新输入。
- profile selection 推进 checked generation；旧 load/duplicate/hide/delete response 不覆盖新 profile。
- hide 后可从 hidden roster 恢复，避免不可达死路；delete 二次确认；start 复用 `/channel/new?agent=`。

这些代码通过 host unit/WASM/release bundle，但没有 Browser journey，所以相关 UI/route ledger 仍 todo。

## 7. 本轮亲跑证据

### 7.1 Rust 与 PostgreSQL

| 证据 | 结果 |
| --- | --- |
| 九 crate full tests | Agent 34；Application 153；Contracts 95；Desktop 81；Domain 370+6+28；Infra 311；Server 216+3+7+8；xtask 93；UI 167；全部 0 fail |
| 九 crate all-target/all-feature Clippy | `-D warnings` 绿；只有已登记的 future-incompat notice |
| PG lifecycle | 1/0/0；generation/revoke、Vault、runtime auth、竞态、audit rollback、secret-free |
| PG thread begin | 1/0/0；non-owner private 404、admin private run 成功 |
| PG Agent directory | 1/0/0；strict remote config/auth UUID 与 owner/admin/hidden/tenant |
| PG audit reader | 1/0/0；secret input 只写 allowlist metadata |
| PG channel activity | 1/0/0；119 个受影响 done target 中对应 7 条 realtime revalidate 共享证据 |
| PG agent runtime | ignored suite 全 8/0/0；package/remote/managed/remember/component/cancel/deadline 全回归 |

PG 全部使用本机 PostgreSQL 17.11、TCP SCRAM、独立临时数据库；临时集群均已停止并删除。主动注入的
`forced agent lifecycle audit failure` 是 rollback 正向证据，不是产品错误。

### 7.2 GUI、Engine 与台账

| 闸门 | 最终结果 |
| --- | --- |
| UI WASM | `cargo check -p openbot-ui --target wasm32-unknown-unknown --locked` 绿 |
| release UI | Trunk 0.21.14，`--release --offline --locked` 绿；首次 `NO_COLOR=1` 参数失败后显式 `true` 重跑 |
| i18n/design/CSS | 771 leaf keys；102 Rust files/74 icons；354 class literals |
| bundle | wasm gzip `1,792,687/3,670,016`；CSS `113,662/131,072`；fonts `740,216/819,200`；scripts `1/0` |
| tools | Tailwind 4.3.3、Trunk 0.21.14、Binaryen 132、wasm-bindgen 0.2.127 独立 verify 绿 |
| Engine | Electron 43.3.0 zip `ee939d…`；ASAR 17,306 B、release epoch 1、protocol 1；sandbox 内首次 `--version exit None`，宿主原命令 verify 绿 |
| parity | `808/886/1694`，0 violation/0 warning |
| overlay | carry/revalidate/split/superseded=`1450/236/2/6`，diff-required 119 |
| strict recount | fixed upstream `891df72f…`，`159/0/0`；首次四个 expect 漂移后按机械实际更新并重跑 |
| Grok/shim | inventory 2,110；tree未改；shim 3 files、405/600 LOC、protocol hash match |

### 7.3 浏览器证据边界

本批前段 release fixture 的宿主 HTTP 矩阵实得 list 200、create 201、update 200、probe 200、duplicate
201、hide/unhide/delete 204、deleted get 404，response 无 auth 值。随后尝试 in-app Browser 时，初始服务未启动
得到 connection refused；服务启动后 Browser URL policy 禁止从错误页导航到 loopback，并明确要求不得绕过或
改用另一浏览器表面。因此 **formal browser = NOT RUN**，上述 curl 只算 HTTP，不算视觉/AX/用户旅程。

## 8. 台账变化

- API：`73/97 → 80/90`，关闭 T-API-0021–0026、0029；
- tests：`395/652 → 457/590`，关闭：
  - T-TEST-0269–0297；
  - T-TEST-0304、0306、0308–0318；
  - T-TEST-0321–0323、0325–0327、0330–0331、0333、0335；
  - T-TEST-0372–0374、0376–0378、0380–0383；
- superseded：T-TEST-0307、0324、0334、0336、0379、0384；
- routes/UI 不变：routes 23/9，UI 87/65；T-ROUTE-0007、T-UI-0031/0032 仍 todo；
- 总 parity：`739/955 → 808/886`；fixtures 17/22 不变；G2 子集变为 165/69/234。

## 9. 明确未做与下一步

- 未运行 `cargo xtask ci`、GitHub Actions、live vendor credential、Windows/runsc；
- 未实现 Desktop custom-protocol Agent list/lifecycle；
- 未取得 `/agents` formal Browser/AX/双视口/golden；
- 未补 AG-UI interrupt/resume 与其余事件完整 durable/UI projection；
- 未关闭 G4/G6 整关；P1 仍等待 Windows 真机与 Ubuntu runsc/Xvfb，P2/P3/P4 未进入。

本批合并后仍按 v4 §19.1：P1 外部平台证据未到时，不越门进入 P2；可以继续并行处理不依赖 P1 的
G2/G3/G4/G6/G8 剩余独立判据，但每批必须维持 R63、零 npm、Grok 规格先行吸收与机械台账证据。
