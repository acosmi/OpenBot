# G4 AG-UI Tool Result Production Projection（Batch96）

> 日期：2026-09-03（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §7.5、§8.1、§15.3、§16.5、§24、§25、§28.1
> 基线：R169 / Batch95，`f765ef258df46b3a19756115664e55fb0b940c4d`
> evidence：`f7988cabf4fcb11a6296aaf020f0d7c855c17494`

## 1. 为什么R169没有顺手关闭T-EVT-0003

Batch95已经实现`ProviderRemoteProjectionKind::ToolResult`，并有unit正向、unoffered负向和terminal retention，但没有用fake `AgentContextSource`冒充production。`T-EVT-0003`要求START/ARGS/END/RESULT完整协议与§8.1唯一执行管线的边界都被证明：remote声称的result不能自动变成本地tool effect。

本批不增加production代码，只补真实production catalogue/context/SafeDialer/SSE/Agent/PostgreSQL证据后再关闭T-ID。

## 2. Offered tool来自production真源

测试在隔离PostgreSQL 17.11中：

1. 用`PostgresComponentAdministration::sync_catalogue`同步正式compiled component manifest；
2. 用`PostgresAgentContextSource::with_components`为真实remote Agent run加载当前component definitions；
3. remote HTTP端逐字段确认RunAgentInput的`tools`含`showQuote`及object schema，`forwardedProps.openbotDeploymentTools`也含同名tool；
4. 原有bot/run/actor签名scope、SafeDialer loopback allowlist、5-byte SSE分片与package-provider=0判据保持。

因此正向不是手写`ProviderRequest`或只测decoder。

## 3. RESULT只能是显示投影

remote SSE按顺序发送：

```text
TOOL_CALL_START(showQuote, remote-result-call)
TOOL_CALL_ARGS(valid quote object)
TOOL_CALL_END(remote-result-call)
TOOL_CALL_RESULT(remote-result-message, remote-result-call, canary)
```

`RemoteAguiSession`先确认call已完成且tool name确实位于本次offered set，再拒绝重复result，最后才创建R169的`source=remote_ag_ui/untrusted=true` projection。unit负向证明unoffered call在projection之前转`provider_invalid_response`。

RUN_FINISHED前真实PG观察到第10条active projection：family=`tool_result`、`untrustedKey=remote-result-call`、`untrustedType=remote-result-message`、正文canary恰1。RUN_FINISHED后marker由9增10，canary降0。

关键反证：该run使用`NoAgentToolInvoker`仍正常completed；`messages WHERE role='tool'`与`public.tool_calls WHERE run_id='run-remote'`合计为0。也就是说remote result没有转换成`ProviderEvent::ToolCallCompleted`，没有本地decision/attempt/capability/execution/outcome，不能伪造“Rust执行成功”。remote未发RESULT时，offered call仍走既有唯一§8.1本地管线。

## 4. 验证

- 真实PG17.11 + production component catalogue/context + SafeDialer/SSE + Agent：`1/0/0`。
- active projection顺序新增tool_result并由9→10；tool-result binding/canary=`1`。
- terminal projection marker=`10`、reasoning marker=`1`、remote canary=`0`、local tool effects=`0`、invoked=`3`。
- Testkit默认：`17/0/9 ignored`；上述ignored用例已单独真跑。
- Testkit all-target/all-feature Clippy `-D warnings`与`cargo fmt --check`：通过。
- parity=`831/873/1704`、events=`43/45/88`、fixtures=`21/22/43`、overlay=`1293/403/2/6`、0 violation。
- recount=`71/0/89 skipped`；strict因未配置固定上游目录未跑。
- Grok inventory=2,110 files，Git tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`；非Grok package.json恰1，零npm，Actions manual-only。
- 临时PG已fast stop并删除精确data/socket根，55496无listener。

## 5. 未声称完成

- 只关闭`T-EVT-0003`；AG-UI `T-EVT-0010 interrupt-resume`仍todo。
- remote result不是callback执行证明，也不取代callback token/assertion/tool-grant管线。
- 无production代码、schema/native/API/env/Cargo/UI/bundle变化；沿用Batch95已验证bundle。
- 没有关闭完整AG-UI/G4/G6/G8，也没有运行R63禁止的`cargo xtask ci`或派发Actions。
- GitHub CLI token仍失效，本地提交尚未推送/建PR，不伪称远端已完成。
