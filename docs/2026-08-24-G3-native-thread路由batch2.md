# G3 Native Thread 路由、事务追加与实时 batch 2

> 第一真源修订：v3 §28.1 R64–R65；完成项见 v3 §24.1。R63 继续有效：不运行 GitHub
> Actions / `cargo xtask ci`，只记录本机定向证据。

## 1. 完成项勾选

- [x] `POST /api/threads/mint` 经 Authenticated → typed `AppCommand::MintThreadId` →
  application `ThreadDirectory` → production OS CSPRNG issuer；
- [x] `GET /api/threads/{thread_id}` 恒注册，只读 native PostgreSQL，不含 Intelligence
  client、配置开关或 fallback；
- [x] status 同时绑定 AuthContext 的 deployment / tenant / actor 与 thread membership；
- [x] missing / deleted / 无 membership / 错 scope 统一 `known:false`，存储失败独立 503；
- [x] 非 UUID 在碰 port 前 400；foreign/legacy UUID 不因 `owns=false` 被错误拒绝；
- [x] Axum 与 Desktop typed in-process 持有同一 `Arc<dyn ApplicationService>` 并逐结果对拍；
- [x] tests ledger 8 条、API ledger 2 条同批转 done；
- [x] `BeginThreadRun` 同事务提交 thread/membership/lease/message/running run/started event/
  internal outbox + empty NOTIFY，精确 run-id replay 零写；
- [x] 同 run-id 不同内容稳定 `request_conflict` 409；末段 outbox collision 七 surface 全回滚；
- [x] LISTEN before replay、cursor 补取、双 replica wake、1s lost-notify catch-up、撤权断流；
- [x] authenticated SSE + `Last-Event-ID`，新增 API `T-API-0149`；
- [x] 50ms/8KiB UTF-8 accumulator 纯边界；
- [ ] accumulator 尚未接真实 provider；WebSocket、history、terminal/chunk writer、outbox relay、
  memory/importer 未完成。

## 2. 第一性裁决

固定上游状态路由的业务问题是“Intelligence 是否还能产生该用户的 thread”。§4.1 退出
Intelligence 后，可观察问题不变，但权威答案只能来自 PostgreSQL。由此得到四个不能互换的结论：

1. route 必须恒注册；没有 Intelligence 配置不再是“不存在此接口”；
2. fingerprint 只说明谁铸造 ID，不说明谁现在有访问权，不能代替 ACL；
3. `false` 合并不存在与不可见，避免状态接口成为 thread 枚举器；
4. 数据库不可用不是 `known:false`，也不是已删除 vendor 的 502，而是稳定 503。

事务/实时再增加六条：

1. `run_id` 同时是 durable idempotency key；相同内容回原 receipt，不同内容不能冒充 replay；
2. new thread 只能用当前 deployment minted UUID；已迁入的 legacy/foreign UUID 仍按 ACL 可写；
3. 30s lease 是新增 failover 值，不是 run deadline；过期 takeover 必须推进 fencing；
4. run 一提交即为 `running` 并有 `started` event；provider effect 只能发生在 receipt commit 后；
5. NOTIFY payload 恒空且只 wake；LISTEN 必须先建立，再 replay，断线/丢通知仍按 durable cursor；
6. SSE 用标准 `Last-Event-ID`，Desktop durable/error frame 不可静默丢，满则断开后 replay。

## 3. 本机证据

- `cargo test -p openbot-contracts --locked`：**62/0/0**；
- `cargo test -p openbot-application --locked`：**99/0/0**（chunk 5、thread use case 8、
  ApplicationService execute/subscribe 均含）；
- `cargo test -p openbot-server http::threads::tests --all-features --locked`：**11/0/0**；
- `cargo test -p openbot-server error::tests --all-features --locked`：**12/0/0**；
- `cargo test -p openbot-desktop budget::tests --locked`：**6/0/0**；
- `cargo test -p openbot-testkit --test transport_thread_parity --locked`：**3/0/0**；
- PostgreSQL 17/SCRAM `thread_directory`：**1/0/0**；
- PostgreSQL 17/SCRAM `thread_begin`：**3/0/0**；`thread_live`：**1/0/0**（含强制
  `pg_terminate_backend` 两条 LISTEN、无 NOTIFY 写 event 2、两 replica 重连 replay）；
- `cargo xtask parity-check --json`：API **18/131/149**、tests **169/878**、总 parity
  **280/1366/1646**，violations/warnings 均 0；
- Cargo.lock 新 package **0**；infra/server 只增加既有 `futures-core` direct edge；
- GitHub Actions：**0 个**本分支 run，未派发。

## 4. 未冒充边界

本批已能 durable 创建并开始 native turn，也能从 PostgreSQL replay→SSE live；但它没有真正
消费 `agent_run_dispatch` outbox，尚无 lease renew/stale-running recovery 或 provider/tool reducer
来写 semantic chunk/terminal，纯
accumulator 也没有 producer 调用点。`/api/channels/events` WebSocket parity、空 history facade、
memory user journey 与 Intelligence import checksum 继续 todo，因此 G3 整关保持未勾。
