# OpenBot G4 Approval Critical Realtime Batch56

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G4-approval-realtime`

基线：Batch55 PR #38 已以 merge commit `fe04fdc19491d9c7f1981337185c49a0f80175c2` 合入 `main`。

implementation：`971f28af086cee5c5205623663fdd882505fadf1`

## 1. 结论

本批关闭 v4 §24.1 G4 中独立登记的 **approval critical realtime** 子项，但不把它外推成 G4 整关、真实 PostgreSQL 浏览器端到端或完整 thread 集成：

- 新增 actor-scoped、same-origin、read-only `openbot.tool-approvals.v1` WebSocket；
- frame 只有 `pendingCount`，没有 approval id、tool、target、arguments/change summary；它只是失效提示，权威状态仍只来自 `GET /api/tool-approvals`；
- PostgreSQL coordinator 在同进程 `Notify` 或 1 秒 durable poll 后整页重读，页面任一变化才发 hint；多 replica 不依赖 NOTIFY payload；
- 每次重读先验证当前 user AuthGeneration、role 与 revoke；撤权后现有流发稳定错误并终止，新 handshake 在 headers 前拒绝；
- Desktop delivery class 把 approval data/error 都归为 `Critical`，满队列不得静默 shed；
- Leptos 从 1 秒 GET polling 改为 socket hint + authoritative GET；连接/重连前先 GET，无 replay cursor；另保留 30 秒 bounded fallback；
- 每次 GET 使用 checked `u64` epoch，只有最新请求能提交结果，旧响应不能复活已批准卡片；
- decision/refresh worker 固定到 `ApprovalPage` owner，移除卡片不会取消成功状态或后续权威刷新。

P1 仍因 Windows 真机和 Ubuntu/runsc/Xvfb runtime/pin 为红，P2 未进入；本批属于 v4 §19.1 明确允许与 Engine 线并行的 G4 余项。

## 2. 为什么 socket 不是第二状态存储

§7.5 对 durable run event 要求 cursor/replay；approval 列表已经由 PostgreSQL `tool_approvals` 与 typed GET 承担 durable projection。本批没有伪造一份 approval event journal：

1. `ToolApprovalActivityEvent` 只带同一 100-row projection 的 `pendingCount`；
2. 客户端不拿 count 增删卡片，每个合法/坏/error frame都只触发 GET；
3. socket 建立前、每次重连前都先 GET，因此断线区间不靠 replay 猜状态；
4. 30 秒 fallback 只在 socket/hint失效时界定最长陈旧窗口，不回到旧的每秒轮询；
5. PostgreSQL比较完整 `PendingToolApprovals`，即使旧项解决与新项创建后 count 同为 1，仍会发 hint；wire仍不泄露哪一项变化。

因此这是与既有 channel activity 同类的低延迟 invalidation surface，不是 durable run-event规则的例外扩张。

## 3. Authority 与失效

`ApplicationService::subscribe` 新增封闭 `SubscriptionRequest::ToolApprovalActivity`，只把 transport 已验证的 `AuthContext` 传给 `ToolApprovalAdministration`。默认 port 继续 fail-closed；没有 production coordinator 时返回 `tool_approval` dependency unavailable。

生产 coordinator 的每轮顺序固定为：

```text
same deployment/tenant
  → current users.auth_generation
  → at least one current role
  → no revoked_access row
  → authoritative pending projection
  → compare full prior projection
  → emit count-only hint or keep waiting
```

依赖错误或撤权都发一个 stable-code `ToolApprovalStreamError` 后终止。Axum随后以1011关闭；client Text/Binary一律1008，Ping/Pong不构成业务输入。exact subprotocol与trusted Origin都在upgrade前验证。

## 4. WASM owner 缺陷与修复

第一次真实浏览器点击捕获到一个单元测试没有暴露的问题：POST 已成功且卡片消失，但 `role=status` 没有保留。根因不是后端，而是 `spawn_local_scoped_with_cancellation` 绑定在卡片 owner；成功路径先删除自己的 `<ApprovalCard>`，owner dispose 会取消同一 future 的尾部。

本批没有用延迟或不取消任务掩盖，而是沿用仓内 Composer / Connected Accounts 已验证模式：`ApprovalPage` 捕获稳定 `Owner`，所有 decision 与 refresh worker都在该 owner 下启动。修后真实浏览器实得卡片 `1→0`、status=`已提交批准。`，console warn/error 0。

## 5. PostgreSQL 17.11 / SCRAM 真证据

本机两次使用独立 `/private/tmp/openbot-batch56-pg.*` 集群；最终一次实得：

- `server_version = 17.11 (Homebrew)`；
- `password_encryption = scram-sha-256`；
- `pg_authid.rolpassword` 前缀检查为真；
- 只监听 `127.0.0.1:55456`，host/local均SCRAM；
- 用例后 `pg_ctl -m fast` 停止，data/pwfile删除，`pg_isready`无响应且临时路径零残留。

最终命令：

```bash
OPENBOT_TEST_DATABASE_URL=… \
  cargo test -p openbot-infra --test tool_approval_runtime \
  durable_wait_grant_reuse_deny_expire_and_generation_cancel_are_real \
  --locked -- --include-ignored --exact --nocapture
```

结果 `1/0/0`。同一完整用例继续覆盖既有 once-per-run reuse、grant/deny/expire/cancel、摘要清除与hash-chain audit，并新增证明：

- owner/other初始count均0；
- 另一 coordinator replica 创建pending后owner最迟1秒收到count1；other actor连续1.2秒零frame；
- grant后owner收到count0；
- AuthGeneration 1→2后现有owner流收到stable `not_visible`，下一poll立即EOF；
- 同一旧AuthContext不能再打开新流；
- durable waiter也观察generation变化并取消，未扩大既有effect面。

## 6. 真实浏览器与 fixture 边界

使用最新 release/offline/locked Trunk bundle与 `required-features=testkit` 的 `openbot-ui-fixture`；fixture新增的计数探针只存在于该测试二进制，不进入production Router或API台账。

全新fixture进程 + 全新浏览器tab实得：

- 页面初始approval article=1、批准按钮enabled；
- probe初始 `listCalls=3 / subscriptionCalls=1`：initial GET、connect前GET与initial hint GET，只有一条socket；
- 连续六个1秒采样中的前六个读数均保持 `3/1`，旧式每秒GET不存在；30秒fallback仍由源码常量与unit test界定；
- 批准后article=0、status=`已提交批准。`、console warn/error=0；
- 点击后的GET为有限批次，不形成周期风暴；checked epoch只允许最后完成的GET提交。

fixture仍是明确的内存替身，不能证明PG事务或跨replica；上一节真PG证明后端，当前节证明真实WASM/Axum/WebSocket/DOM。**真实PG浏览器同一进程端到端仍todo**，两段证据不合并偷换口径。

## 7. 本轮最终实跑证据

| 命令/面 | 结果 |
| --- | --- |
| 六crate `cargo test`（contracts/application/infra/server/UI/Desktop，all-features） | exit 0；核心unit分别 `88 / 150 / 307 / 213 / 133 / 80`，PG ignored不冒充已跑；目标PG另见§5 |
| approval Axum/WebSocket定向 | `4/0/0`；same-origin/protocol、actor typed frame、read-only1008、stable error+1011、原GET/POST均绿 |
| PG17.11 SCRAM approval runtime | `1/0/0` |
| 六crate all-target/all-feature Clippy `-D warnings` | exit 0 |
| UI `wasm32-unknown-unknown` check | exit 0 |
| release/offline/locked Trunk build | exit 0 |
| `tools verify` | Tailwind4.3.3 / Trunk0.21.14 / Binaryen132 / wasm-bindgen0.2.127 exact |
| i18n/design/CSS | 560 leaf keys；89 Rust files+74 icons；292 class literals，全绿 |
| bundle-budget | wasm gzip `1,384,716/3,670,016`；CSS `97,848/131,072`；fonts `740,216/819,200`；external/inline script=`1/0` |
| `cargo xtask parity-check` | API `67/103/170`；events `34/53/87`；总 `695/999/1694`；overlay carry/revalidate/split/superseded=`1600/92/2/0`；0违反 |
| strict recount | `159/0/0`，固定上游commit `891df72f…`，跳过0 |
| Grok/package/shim guard | Grok tree `86f5a85f…`、diff0、inventory2110同步；非Grok package.json恰1；shim3文件/405 LOC |
| fmt/diff | `cargo fmt --check`与`git diff --check`通过 |

初次六crate测试曾因磁盘只余127MiB而在rustc输出阶段报`No space left on device`；本轮只删除可再生成的Cargo target（`cargo clean` 24.2GiB），随后所有最终命令从当前源码重跑。两次临时PG setup在server启动前分别因`--pwfile=-`不受支持、pwfile误放data目录而被`initdb`拒绝；trap均清理，最终两次真正启动的SCRAM实例均绿。失败尝试不计入通过数字。

## 8. 明确未做

- 未运行 `cargo xtask ci`，未派发GitHub Actions（R63 manual-only）。
- 未运行Windows真机或Ubuntu/runsc/Xvfb，P1仍红，未进入P2。
- 未使用live vendor credential，三家recorded trace仍0/3。
- 未完成真实PG浏览器单一竖切、approval完整thread集成、browser/file/shell executor或其cancel面。
- 未修改 `grok-bot/`，未新增Grok产品能力，未复制/翻译Grok文本。
- 未使用npm/Node构建链；前端工具只经pins与Rust xtask获取/校验。
