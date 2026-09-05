# Batch108：BrowserInput → CDP 输入计划纯核心

日期：2026-09-04

外部候选：`1ca08993f725310e97c16c9bb77cdd14f1f61a4e`

审计修正后 implementation：`b9e464013072b4d190e204aebc0f8afd861ba223`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§11.2、§12.5–§12.6、§19.1 P2、§24 G5/G7、§28.1 R183

固定产品上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 结论与严格边界

Batch108把已存在的closed `BrowserInput`转换为不含自由method的`CdpInputPlan`：move/down/up/wheel
只能生成对应`Input.dispatchMouseEvent`参数，key down/up只能生成`Input.dispatchKeyEvent`参数，普通文本
只能生成`Input.insertText`参数；`SecretInsert`构造性拒绝进入普通计划。

本批只关闭**纯参数映射与source-identity fixture**。它没有修改engine wire或shim，没有让
`EngineProcess`执行CDP，没有证明页面收到事件，也没有接HumanLease/generation拒绝、坐标变换、ScreenHub
或viewer ticket。因此T-BROP-0037–0044保持todo，P1仍红且P2没有进入，G5A/G7均不勾。

## 2. 外部候选复核与修正

候选的类型边界、普通输入/SecretInsert隔离与Debug脱敏方向正确，原10条测试也能通过；但测试把两处
固定上游偏差写成了期望，故不能原样接纳：

1. 候选把`VIRTUAL_KEY_CODES`写成16项，理由是空格可由通用分支得到32；固定上游表本身明确有17项，
   包含`" " → 32`。行为碰巧相同不能替代source identity逐项相等。
2. 候选让`F1`、`Meta`、非BMP键等多UTF-16单元未知键返回`UnknownKey`；固定上游对非单单元键返回0
   并继续发送。v4没有批准这种收窄，故必须preserve而不是实现期替换。

主控从R182用`cherry-pick --no-commit`导入候选；合并冲突只来自Batch106已增加的`eviction`模块，
最终同时保留`eviction`与`cdp_input`。候选未作为错误中间commit落库，修正后一次提交为`b9e4640…`。

## 3. 固定上游证据与映射语义

官方仓库`agent-computer/src/screencast.ts`在固定commit上为：

- Git blob：`9bc27c11fc1b4cd296f7fc9df412aea0bedbbb22`；
- bytes：`6906`；
- SHA-256：`be79bde5007f03f37e3b99a1ef1388ba672d684ba32ec2b9090c417f9f47f566`。

最终纯映射固定：

- mouse moved=`mouseMoved/clickCount0`；pressed/released=`mousePressed|mouseReleased`且clickCount非零；
- wheel=`mouseWheel`并保留坐标、delta X/Y与modifier；
- key down有非空text=`keyDown`，无text=`rawKeyDown`，key up=`keyUp`；
- 17项具名键码逐项相等，含空格32；其它单UTF-16单元按uppercase结果首unit；未知多单元为0；
- native/windows virtual key code相等；key/code/text只通过getter交engine，Debug只显示UTF-16长度；
- `InsertText`保留空串与Unicode正文但Debug不显示；SecretInsert返回稳定
  `cdp_input_secret_requires_typed_path`，不借普通CDP计划越过pending target/generation authority。

`fixtures/computer/cdp-input-plan.json`为1315 bytes，SHA-256
`e6cf14b947d86cd2bc458adc27b75272b619cc90c4f41558c0a3b1438424463f`；其
`productEngineWire/liveCdpEffect/screenHub`三项固定false，防止后续把pure core扩大解释。

## 4. 实跑证据

| 检查 | 结果 |
|---|---|
| `cargo test -p openbot-computer --lib --locked browser::cdp_input::tests` | `11/0/0` |
| `cargo test -p openbot-computer --all-features --locked` | lib=`54/0/0`，host conformance=`0/0/2 ignored` |
| `cargo clippy -p openbot-computer --all-targets --all-features --locked -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` / `git diff --check` | 通过 |
| `cargo xtask parity-check --json` | parity=`861/849/1710`，browser-operations=`7/43/50`，fixtures=`27/22/49`，overlay=`1283/419/2/6`，0 violation/warning |
| `cargo xtask recount` | `71 passed / 0 mismatch / 89 skipped`；未配置固定上游目录，strict未跑 |

fixture test首次编译曾因比较`serde_json::Value`与`Vec<Value>`报类型错误；改成显式JSON Array后上述
11条与完整54条均重跑通过，首次失败不计通过。

本批无schema/native/API/route/UI/bundle/dependency/Cargo.lock/env/npm/Grok/workflow变化；Cargo package仍
829。没有运行R63禁止的`cargo xtask ci`，没有派发GitHub Actions。

## 5. 下一步

真正关闭T-BROP-0037–0044必须在P1两平台证据满足后，把同一closed plan接入authenticated engine，逐项
记录实际CDP method/参数与页面observable effect，并同时证明stale generation、错误lease、SecretInsert
普通通道、engine伪造scope都在effect前拒绝。坐标映射、拖拽序列、IME完成文本、慢消费者与ScreenHub
属于P2/G7后续批次，不能混入本pure core结论。
