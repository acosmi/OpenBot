# Batch105：Rust run 取消到 RMCP 协议级取消纵向

日期：2026-09-04

implementation：`9b03cca38b0841bce46acc6ed1132b2633064cdd`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§7.4、§8.1、§9.1–§9.2、§13.2、§17.2、§21.3、§24 G4、§28.1 R179

固定产品上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

固定协议实现：`rmcp 3.1.4`，VCS commit
`4a738b9dd99eaca418b614afa433a0cbdaf8d056`

## 1. 本批结论

Batch105 关闭产品运行时中 **Rust-owned run cancellation → RMCP `tools/list` / `tools/call`** 的
协议级纵向。此前Agent收到user cancel、absolute deadline或runtime shutdown后只会丢弃
`ApplicationService`工具future；RMCP `RequestHandle::cancel`/`notifications/cancelled`没有调用点，
远端最多因HTTP连接断开被动发现，且本地不能证明在写terminal前先尝试了协议取消。

现在同一run信号通过private、non-serde、process-local registry绑定到Rust铸造的`ToolCallId`：

- sender只存在于Agent host，model/renderer/HTTP/remote Agent没有自报槽；
- Application仍走唯一`ApplicationService → decision+attempt → capability → executor → outcome/audit`
  管线，没有新增旁路；
- Server与Desktop Local从共享application assembly取得同一个registry实例；
- RMCP fresh list与call都使用cancellable request handle；取消通知携exact active request ID与稳定本地
  reason，不含actor、prompt、参数、vendor prose或secret；
- Agent给协议合作停止5秒有界窗口，并继续续租；远端/本地不合作时abort后仍进入reconciliation，
  不会把“已请求取消”冒充“effect未发生”。

这只关闭RMCP产品运行时的protocol cancel。computer/file/shell的进程树/协议取消仍todo；完整
T-FIX-0018还要求固定官方conformance套件对initialize/capability/list/call/cancel/timeout/progress全套
100%实跑，本批没有借局部纵向把它勾掉，也没有勾G4整关。

## 2. 取消身份与时序

`ToolCancellationReason`只含`User / Deadline / LeaseLost / HostDropped`，映射为四个稳定reason code；
first reason wins，后到竞态不能重写。`ToolCancellationRegistry`只接受本地receiver注册，key是gateway
新铸的tool call UUID；外部即使构造同形`ToolInvocation.call_id`也没有向registry插入sender/receiver的
入口。RAII registration在application完成或future被drop时精确删除；重复call ID与counter exhaustion
fail-closed。

串行tool取得全局budget后在独立child中运行。user/deadline到达时顺序固定为：

1. 发布private cancellation reason；
2. 等待child最多5秒，期间继续续lease；
3. RMCP若已发request，先POST `notifications/cancelled`；
4. remote `RequestContext`停止或本地grace耗尽后，才写deadline audit与run reconciliation terminal。

MCP工具由build-owned scheduling始终判serial；parallel-safe仅限既有11个ordinary compiled component，
所以RMCP不会绕开这条合作取消路径。human-decision waiter保持既有detached durable retirement语义，不被
本批误改成vendor effect。

## 3. 三态 commit 语义

闭合fixture：`fixtures/mcp/rmcp-run-cancellation.json`，806 bytes，SHA-256
`23bcd6d0184f729d437c9db08305ce34d9f4f096e90764e1dc5d4466c1d36f0a`。

### 3.1 信号在任何网络前已存在

`call_tool_bound_cancellable`在解析endpoint、connect或list前读取当前reason并返回
`mcp_cancelled_before_call`。loopback listener的accept在50ms内保持0，证明不是“连接后再关闭”。

### 3.2 fresh `tools/list`进行中

per-operation client的fresh list改用RMCP cancellable request与15秒总deadline；deadline到达时server
实际收到`notifications/cancelled`，requestId非空、reason=`run_deadline_exceeded`，list handler的
`RequestContext`停止，后续`tools/call`为0。list timeout仍由RMCP handle发送其固定timeout cancel；本批
没有把单个正向case冒充T-FIX-0018完整timeout/progress conformance。

### 3.3 `tools/call`已经开始

user cancel时server实际收到唯一`notifications/cancelled`，requestId非空、reason=`run_cancelled`，
handler停止。即使通知成功，这只能证明remote获知“结果将不再使用”，不能证明非幂等effect未提交；
因此application同事务写：

- `tool_attempts.status=reconciliation_required`；
- `commit_state=unknown`；
- `error_code=mcp_cancelled_after_call`；
- `mcp.call_failed=1`、`mcp.call_succeeded=0`。

通知POST本身失败时使用独立`mcp_cancel_notification_unknown`，同样`commit_state=unknown`，不会把失败
伪装成已送达。只有call尚未发送时才允许NotCommitted。

## 4. 真实 PostgreSQL 与 RMCP 证据

本机临时PostgreSQL 17.11集簇使用TCP SCRAM-SHA-256。钉版RMCP 3.1.4官方
`StreamableHttpService`/`LocalSessionManager`作为真实server；完整`mcp_protocol` 9项串行实得
`9/0/0`，同时回归：

- initialize/capability/list/call/normalization/per-operation close；
- catalog disappearance/schema/effect/transport drift与grant suspension；
- custom private CIDR、TLS与credential retirement；
- no-grant、vendor schema、CEL/content、acting approval、decision+attempt/capability/outcome/hash-chain；
- 新增pre-network/list/call三态取消。

取消call的本地registry在application返回后残留0。共享PostgreSQL application assembly另实跑
`1/0/0`，证明新增registry没有生成第二个ApplicationService。临时集簇已fast-stop并删除。

## 5. 本轮机械证据

- Application=`166/0/0`；Agent=`57/0/0`；Infra lib=`328/0/0`；
- Server lib=`222/0/0`；Desktop all-feature lib=`131/0/3 ignored`；
- RMCP无PG=`6/0/3 ignored`；PG17.11+official RMCP完整=`9/0/0`；
- shared application assembly PG=`1/0/0`；transport parity=`8/0/0`；
- Application/Agent/Infra/Server/Desktop all-target/all-feature locked Clippy `-D warnings`；
- `cargo fmt --all -- --check`、`git diff --check`；
- RMCP、SafeDialer、MCP OAuth、Application assembly、Tauri background/dependency、SAML guards；
- `cargo xtask parity-check`：parity=`848/862/1710`，fixtures=`25/22/47`，0 violation；
- fixed-upstream strict recount：`160 passed / 0 mismatch / 0 skipped`；
- `grok-bot` Git tree、零npm、唯一非Grok `package.json`均未改变。

Batch104后首次重跑RMCP/SAML guard时，两者因仍钉SPDX package总数55而判红；OpenAI recorded source已
使真值变成56。修复同时锁定OpenAI SPDX ID、exact commit、MIT许可、56个ID唯一，而非只放宽数字；
两个guard复跑均绿。

按R63未运行`cargo xtask ci`，未派发GitHub Actions。无schema/native migration、产品API/route/UI、
env、dependency、Cargo.lock或workflow变化。

## 6. 台账与明确剩余

新增`T-FIX-0047 mcp-run-protocol-cancellation`为done；既有T-FIX-0018保持todo：

- parity=`848 done / 862 todo / 1710`；
- fixtures=`25 done / 22 todo / 47`；
- overlay carry/revalidate/split/superseded=`1299/403/2/6`；
- native latest=`0029`，schema=`47表/478列/342 NOT NULL/269约束/97索引`。

明确剩余：RMCP完整官方conformance；computer/file/shell协议取消与process tree；Anthropic/Google
recorded trace和三家live credential；acting Approval完整computer/thread旅程；computer runtime budget；
Desktop Local OAuth；Plugins/Skills UI；Browser/file/shell；P1 Windows/runsc真机；ScreenHub/viewer ticket；
Desktop真实Wry、正式发行/golden；G2外审/KMS/Windows；G8迁移、签名、外审、operator-attestation、最终
secret retirement及v4其余全部未闭合项。
