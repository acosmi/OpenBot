# Batch105：Rust run 取消到 RMCP 协议级取消纵向（R180 纠偏后）

日期：2026-09-04

原始 implementation：`9b03cca38b0841bce46acc6ed1132b2633064cdd`

R180 protocol correction：`3a694658a34ef3b9cbeba60fc32907eba8b50a2b`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§7.4、§8.1、§9.1–§9.2、§13.2、§17.2、§21.3、§24 G4、§28.1 R179–R180

固定产品上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

固定协议实现：`rmcp 3.1.4`，VCS commit
`4a738b9dd99eaca418b614afa433a0cbdaf8d056`

## 1. 当前结论

Batch105 关闭产品运行时中 **Rust-owned run cancellation → RMCP `tools/list` / `tools/call`**
的协议级纵向。private、non-serde、process-local cancellation registry与Agent的5秒合作收口保持
R179实现；但R179把MCP `2026-07-28` Streamable HTTP错误写成发送
`notifications/cancelled`。该wire断言已经由R180明确覆盖，禁止继续引用。

当前按协商协议分流：

- 首选`2026-07-28`：使用无initialize session的`server/discover`生命周期；取消通过关闭该请求
  对应的HTTP/SSE response stream表达，客户端取消notification POST必须为0；
- server不支持`server/discover`时自动降级到`2025-11-25` initialize/session生命周期；旧版取消仍由
  RMCP发送`notifications/cancelled`，携精确request ID与稳定reason；
- 取消已在任何网络前存在时，socket仍为0；
- `tools/call`一旦进入transport future就按可能已产生effect处理：取消结果为Unknown并进入
  reconciliation，绝不冒充NotCommitted。

这只关闭RMCP产品运行时的协议取消。computer/file/shell的进程树/协议取消仍todo；完整
T-FIX-0018还要求固定官方conformance套件对lifecycle/capability/list/call/cancel/timeout/progress全套
100%实跑，本批没有借局部纵向把它勾掉，也没有勾G4整关。

## 2. R180为何必须纠偏

MCP官方`2026-07-28` Streamable HTTP规范明确规定客户端不得经该transport发送JSON-RPC
notifications；客户端取消in-flight请求的信号是关闭该请求对应的SSE response stream。
`notifications/cancelled`只属于旧生命周期/stdio等相应传输语义：

- <https://modelcontextprotocol.io/specification/2026-07-28/basic/transports#streamable-http>
- <https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/transports/streamable-http.mdx>

原R179测试调用`ClientInfo::serve(...)`，实际走的是legacy initialize生命周期；server handler虽然在
`get_info()`声明`2026-07-28`，该声明不能证明wire已经协商现代生命周期。因此“2026 server收到
cancelled notification”的旧测试只证明legacy行为，不能作为现代协议证据。

此外，固定`rmcp 3.1.4`在现代HTTP请求尚未收到首个SSE event时，transport worker会同步等待POST，
导致公开`RequestHandle`取消命令无法及时处理；上游已登记
[`rust-sdk#1193`](https://github.com/modelcontextprotocol/rust-sdk/issues/1193)。本仓不静默升级依赖，
而是在唯一SafeDialer适配层仅对header已协商为`2026-07-28`的`tools/list`/`tools/call`请求同时等待
run cancellation；命中后drop精确的in-flight request future，从而关闭该response stream。协商、
lifecycle notification与session cleanup绝不被这条路径取消；旧版仍交给RMCP正常发送notification。

## 3. 取消身份、时序与commit语义

`ToolCancellationReason`只含`User / Deadline / LeaseLost / HostDropped`，first reason wins。
`ToolCancellationRegistry`只接受本地receiver注册，key是gateway新铸的tool call UUID；外部即使构造
同形`ToolInvocation.call_id`也没有插入sender/receiver的入口。RAII registration在application完成或
future被drop时精确删除；重复call ID与counter exhaustion fail-closed。

串行tool取得全局budget后在独立child中运行。user/deadline到达时顺序固定为：

1. 发布private cancellation reason；
2. 等待child最多5秒，期间继续续lease；
3. 协议层按已协商版本关闭modern response stream或发送legacy cancellation notification；
4. remote `RequestContext`停止或本地grace耗尽后，才写deadline audit与run reconciliation terminal。

三类稳定结果为：

- network前：`mcp_cancelled_before_call`，NotCommitted；
- modern fresh list进行中：关闭list response stream，后续tool call为0，仍为
  `mcp_cancelled_before_call`；
- tool call进入transport边界后：`mcp_cancelled_after_call`，
  `tool_attempts.status=reconciliation_required`、`commit_state=unknown`、
  `mcp.call_failed=1`、`mcp.call_succeeded=0`。

transport-aware cancellation无法确认时统一使用`mcp_cancel_signal_unknown`，同样保持Unknown；不再用
只描述notification的旧名`mcp_cancel_notification_unknown`。

## 4. 闭合fixture与真实协议证据

闭合fixture：`fixtures/mcp/rmcp-run-cancellation.json`，1,111 bytes，SHA-256
`ec9644f41e8b0773e0983efa52179eef1e32387b2bbd0ec69101afd4874e6b56`。

fixture v2固定四态：

1. pre-network cancellation：accepted socket=0；
2. modern fresh list：protocol=`2026-07-28`、signal=`close_response_stream`、
   cancellation notification POST=0、tool call=0；
3. modern tool call：protocol=`2026-07-28`、signal=`close_response_stream`、
   cancellation notification POST=0、commit unknown；
4. legacy fallback tool call：protocol=`2025-11-25`、signal=`notifications/cancelled`、
   notification POST=1、request ID非空、reason=`run_cancelled`。

钉版RMCP 3.1.4官方`StreamableHttpService`/`LocalSessionManager`作为真实server。测试同时记录
`mcp-method`、`mcp-protocol-version`与server `RequestContext.protocol_version()`，不再用
`get_info()`声明代替实际协商。modern list/call都在server尚无首个event时取消并于1秒内停止handler；
legacy server显式拒绝`server/discover`后，client才自动回落initialize并发送精确notification。

本机临时PostgreSQL 17.11集簇使用TCP SCRAM-SHA-256；完整`mcp_protocol`实得`11/0/0`。PG纵向继续
证明call取消后attempt/audit/registry残留分别为
`reconciliation_required/unknown/mcp_cancelled_after_call`、`1/0`、`0`。临时集簇已fast-stop并删除。

## 5. 本轮机械证据

- RMCP无PG=`8/0/3 ignored`；PG17.11+official RMCP完整=`11/0/0`；
- Infra lib=`328/0/0`；Agent=`57/0/0`；
- Infra/Agent all-target/all-feature locked Clippy `-D warnings`；
- `cargo fmt --all -- --check`、`git diff --check`；
- RMCP、SafeDialer、MCP OAuth、Application assembly、Tauri background、SAML guards；
- `cargo xtask parity-check --json`：parity=`848/862/1710`，fixtures=`25/22/47`，
  overlay=`1299/403/2/6`，0 violation/warning；
- R179既有Application/Server/Desktop/assembly/transport测试证据不因本次协议纠偏失效；
- `grok-bot` Git tree、零npm、唯一非Grok `package.json`均未改变。

按R63未运行`cargo xtask ci`，未派发GitHub Actions。完整官方MCP conformance未运行；当前官方runner
依赖Node工具链，本仓零npm约束下不能把临时`npx`结果冒充正式gate。无schema/native migration、产品
API/route/UI、env、dependency、Cargo.lock或workflow变化。

## 6. 台账与明确剩余

`T-FIX-0047 mcp-run-protocol-cancellation`保持done，但done evidence由fixture v2与R180取代R179旧wire
断言；既有T-FIX-0018保持todo：

- parity=`848 done / 862 todo / 1710`；
- fixtures=`25 done / 22 todo / 47`；
- overlay carry/revalidate/split/superseded=`1299/403/2/6`；
- native latest=`0029`，schema=`47表/478列/342 NOT NULL/269约束/97索引`。

明确剩余：RMCP完整官方conformance；computer/file/shell协议取消与process tree；Anthropic/Google
recorded trace和三家live credential；acting Approval完整computer/thread旅程；computer runtime budget；
Desktop Local OAuth；Plugins/Skills UI；Browser/file/shell；P1 Windows/runsc真机；ScreenHub/viewer ticket；
Desktop真实Wry、正式发行/golden；G2外审/KMS/Windows；G8迁移、签名、外审、operator-attestation、最终
secret retirement及v4其余全部未闭合项。
