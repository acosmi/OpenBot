# G3 Run Runtime 与 WebSocket batch 4

> 第一真源修订：v3 §28.1 R67；R63 继续有效，只运行本机定向测试，不派发 Actions / 不跑
> `cargo xtask ci`。

## 1. 完成项

- [x] 非 serde `RunExecutionLease` / `ClaimedRunDispatch`，身份不能从 transport 字节铸造；
- [x] `RunRuntime` application port 与 production PostgreSQL adapter；
- [x] outbox claim commit reconciliation、attempt CAS、consumer reserve→durable ack→activate/revoke；
- [x] expected run-local sequence 的 semantic chunk/terminal 精确幂等；
- [x] owner + fencing + lease expiry 每次写验证，旧 writer fail-closed；
- [x] terminal 同事务写 run/event/thread cursor、聚合 text chunks 为一条 assistant message、NOTIFY；
- [x] pending/未受理的过期 dispatch 可 fencing+1 重绑；delivered stale run 只进 reconciliation；
- [x] production `RunRelay` 已在 Server main 启停；G4 缺席时写明确 failed terminal，不造假回复；
- [x] `DurableTextRun` 把 50ms/8KiB accumulator 接到 expected-sequence durable writer；
- [x] `GET /api/threads/{thread_id}/ws?cursor=...`：trusted Origin、固定 subprotocol、只读、1KiB cap；
- [x] WebSocket/SSE/typed in-process 共用同一 `ApplicationService::subscribe` 与 PostgreSQL cursor；
- [x] Axum ws 三包 delta guard、deny/audit/vet 本机闭合。

## 2. 关键裁决

1. Outbox `delivered` 只表示 runtime 已 durable 接受，不表示 Agent/provider 已完成；consumer 必须先
   幂等 reserve，ack 后才 activate，ack 失败必须 revoke。
2. 每次 chunk/terminal 都带 expected run-local sequence。commit unknown 后只能用同 sequence + 同
   payload 核对；不同内容稳定 conflict，禁止追加重复文本。
3. 未 delivered 的 outbox 证明尚无已受理 effect，lease 过期后可推进 fencing 并重派；delivered
   只证明可能已开始 effect，因此过期只能 terminal 为 `reconciliation_required`，不能自动重放。
4. `NoRunDispatchConsumer` 是生产 fail-closed sentinel，不是假 Agent：它只写
   `agent_runtime_unavailable`，不产生 assistant 内容，不勾 G4。
5. WebSocket 不是第二个 realtime producer：它只 frame 与 SSE 相同的 `AppEventStream`。浏览器 cookie
   会随跨站握手发送，因此 Origin 必须在 subscription 前验证；client data 没有业务语义，一律 1008。
6. Relay poll 100ms、claim 10s（30s lease 的 1/3）、busy retry 100ms→6.4s capped exponential
   均是新增运行参数，不冒充上游 parity；后续配置化须另写 §15.4 修订。

## 3. 本机证据

- application run runtime：**3/0/0**；
- PG17/SCRAM run runtime：**4/0/0**；
  - claim/ack exact replay，chunk tamper conflict，terminal/message/event/cursor 唯一；
  - pending lease takeover 1→2，旧 writer 拒写；delivered stale takeover 2→3 + reconciliation；
  - rejected terminal 与 outbox ack 末段 trigger failure 整事务回滚；
  - production relay + fail-closed consumer 不留下假 running run；
- Server thread：**16/0/0**；其中真实 loopback TCP/WebSocket：**2/0/0**；
- `bash tools/check-websocket-dependencies.sh`：通过；
- `cargo deny check`：advisories/bans/licenses/sources ok；
- `cargo audit --deny warnings --ignore RUSTSEC-2023-0071`：通过；
- `cargo vet --locked`：**15 fully audited / 403 explicit exemptions**；
- API：**26 done / 130 todo / 156**；tests：**169/878**；
- 总 parity：**288/1365/1653**，violations/warnings 0；fixtures **10/22/32**；
- GitHub Actions：本分支未派发。

## 4. WebSocket 供应链 delta

| package | version | Cargo.lock checksum | build.rs | unsafe token |
| --- | --- | --- | ---: | ---: |
| sha1 | 0.10.7 | `a978451301f4db1d02937a4ab3ccce137717b81826e79b7d49ffe3244a13c3b8` | 0 | 4 |
| tokio-tungstenite | 0.29.0 | `8f72a05e828585856dacd553fba484c242c46e391fb0e58917c942ee9202915c` | 0 | 0 |
| tungstenite | 0.29.0 | `6c01152af293afb9c7c2a57e4b559c5620b421f6d133261c60dd2d0cdb38e6b8` | 0 | 5 |

三条 Cargo Vet exemption 都是 exact version，含 `owner=security` 与 `not a full source audit`；
SHA-1 仅由 RFC6455 `Sec-WebSocket-Accept` handshake 图使用，第一方 Rust 源码零 `sha1::/Sha1`
调用。它们不是“已完成源码安全审计”，G2/G8 外审仍必须进行。

## 5. 未冒充边界

本批没有真实 provider adapter、pure Agent reducer、tool loop 或 remote AG-UI consumer。
`DurableTextRun` 已是 production writer，但没有真实 provider producer 调用它；Server 当前只用
fail-closed consumer 收口为 failed。`remember` tool、Memory GUI、Intelligence importer/checksum
也仍未实现，所以 G3/G4 整关均不勾。

## 6. Git 恢复点

- 实施提交：`87e6c061319f256ac0754cf3a565e799a8672980`；
- 分支：`feat/2026-08-24-G3-run-recovery-terminal`；
- 堆叠 PR：**#21**，base=`feat/2026-08-24-G3-history-memory`，创建后
  `OPEN/CLEAN/MERGEABLE`；
- 实施 head 的 GitHub Actions run 数：**0**。
