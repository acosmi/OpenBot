# Batch 51：HumanLease 与 Browser Input Protocol

> 日期：2026-08-28。分支 `codex/2026-08-28-G5-human-lease-input-protocol`；
> base `bc9fdbb`；WIP `9726447`；implementation
> `9d027b22712982546be9cf18d957c730b10c4f67`；固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

Batch50 后 Desktop sandbox renderer 仍被一个真实前置阻塞：`openbot-computer` 只有 Phase 0 边界说明，
仓内没有 §11.1 的单一 Electron/Chromium engine。直接另做 component-only renderer 会引入第三种生产
渲染器并偏离第一真源。本批先实现所有 browser/component renderer 共用的 HumanLease epoch 栅栏与
封闭输入协议；不在没有 engine 进程、CDP 与帧流实证前关闭 Desktop renderer 或输入执行条目。

## 第一真源裁决

- 固定上游 `control.ts` 的可观察状态机保持：Bot request 只提出请求；human take 后 Bot acting 立即拒绝，
  不排队；release 删除旧 reason；pending secret 只保存 label/ref/document generation，不保存值；
  unanswered help request 恰 10 分钟后在读取时过期，human holder 不被该 help TTL 自动释放；
- v3 §12.5 的新增 HumanLease 绑定 authority-owned actor/computer/tab/computer generation/auth generation、
  epoch 与显式 expires-at。take、transfer、release、expiry、navigation、computer restart 均推进 epoch；
  旧输入即使已在队列里也 fail-closed；
- HumanLease 的有效期数值在第一真源中没有默认值。本批不猜常量：调用方必须传权威 expires-at；
- BrowserInput 是 closed enum：mouse move/down/up、wheel、key down/up、insertText、secret insert；
  没有 IME composition、drag、upload、file chooser 或自由 CDP 变体；
- secret value 使用可清零、不可 Clone、Debug 脱敏的容器，只进入独立 typed command；不进普通 key event、
  control state、日志、frame 或 transcript；
- 本批可以关闭纯状态机与结构性负面条目；CDP 映射、真实 Electron execution、frame/input broker、
  Desktop renderer 与 a11y 豁免仍须真实 engine 证据，不能靠 enum 冒充。

## 实施

- `ControlService` 按 computer/tab/generation 建独立状态：help request、pending secret、take、transfer、
  release、Agent acting refusal、human input ticket、navigation 与 restart 全部在一处；
- help request 恰在 `age > 10 min` 时由读取路径清除，严格保留固定上游边界；human lease 在
  `now >= expires_at` 时释放。二者不是同一个 TTL；
- take 只接受不能由 renderer 反序列化铸造的 `AuthContext`，HumanLease 绑定 actor、auth generation、
  computer、tab、computer generation、epoch 与 expires-at。viewer ticket不含actor/role，authorize每次用
  fresh AuthContext逐字段对拍；
- secret request只存label、field ref与Rust权威`DocumentGeneration`；value装入既有`SecretBytes`，不可
  Clone、drop清当前allocation、Debug只显示`[REDACTED]`。完成只清exact current target；
- `BrowserInput`恰八变体：MouseMove/Down/Up、Wheel、KeyDown/Up、InsertText、SecretInsert；私有payload
  constructor拒绝NaN/Infinity、zero click count、非法modifier与空key/secret。普通text/key Debug同样只
  显示UTF-16长度；receipt字符数按JavaScript UTF-16 code units；
- `BrowserOperation`先固定navigate/snapshot/read/click/type/key/scroll/screenshot/screencast/human input/
  secret/profile十二类，零自由CDP、upload或file chooser成员。它只是下一批engine framing的类型前置，
  本批不把这些operation标为已执行。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| `openbot-computer` tests | **8/0/0**；help TTL、secret、take/transfer/release、actor/auth/scope、expiry/navigation/restart、epoch饱和、closed input与redaction |
| Clippy / format | `openbot-computer --all-targets --all-features` Clippy `-D warnings`；`cargo fmt --all -- --check` 绿 |
| dependency delta | Cargo.lock新package **0**；只给`openbot-computer`增加锁内已有contracts/domain/thiserror/time直接边 |
| parity | browser operations **7/39/46**；components **13/9/22**；总计 **693/993/1686**；fixtures **17/22/39** |
| 机械闸门 | parity-check 0 violation/0 warning；固定上游 strict recount **158/0/0** |

只关闭 `T-BROP-0005`–`0009`、`T-BROP-0045`、`T-BROP-0046`。其中 0005–0009 是真实
control/HumanLease状态行为；0045是closed input union；0046是真epoch fencing。`T-BROP-0037`–`0044`
虽然已有Rust payload类型，但migration rule要求真实CDP映射/执行，继续todo；`T-BROP-0036`还要求engine
对file chooser确定性拒绝并上报，也继续todo。Electron/Chromium进程、authenticated UDS/Named Pipe
framing、CDP conformance、ScreenHub/frame/input broker、viewer ticket、Desktop sandbox renderer与a11y
具名豁免均未实现。

本批没有UI/CSS/bundle变化，不冒充browser/golden证据；未运行`cargo xtask ci`或GitHub Actions，未触碰
`docs/assets/`，未push/建PR。
