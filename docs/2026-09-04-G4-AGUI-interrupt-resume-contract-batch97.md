# G4 AG-UI Interrupt/Resume Typed Contract（Batch97）

> 日期：2026-09-04（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §2.4、§7.2、§7.5、§13.1–§13.3、§15.3、§24、§25、§28.1
> 基线：R170 / Batch96，`a56042bb7e696648f76c668b8416b759439d3192`
> implementation：`c90aba9fc0dfcdf5c370ed8ec1fee5ed26814a74`

## 1. 固定一手协议

本批没有凭名称猜“resume”。复核AG-UI官方core源码后固定：

- `RUN_FINISHED.outcome={type:"interrupt",interrupts:[...]}`表示本次protocol run因human input暂停；
- Interrupt已知字段为`id`、`reason`、可选`message/toolCallId/responseSchema/expiresAt/metadata`；
- 下一次`RunAgentInput`使用新`runId`，`parentRunId`指向产生interrupt的protocol run；
- `resume[]`每项为`interruptId`、`status=resolved|cancelled`与可选`payload`。

一手来源：

- AG-UI official TypeScript core [`types.ts`](https://github.com/ag-ui-protocol/ag-ui/blob/main/sdks/typescript/packages/core/src/types.ts)
- AG-UI official lifecycle [`events.ts`](https://github.com/ag-ui-protocol/ag-ui/blob/main/sdks/typescript/packages/core/src/events.ts)

这一步只确认wire，不把当前main未来扩展字段自动带入固定0.0.57。Rust decoder只归一化仓内已审的七个字段，任何额外`authority`等键直接丢弃。

## 2. Typed ownership与边界

- `AguiInterrupt`替代`Vec<Value>`跨decoder输出，只保留known fields。
- `ProviderRemoteInterrupt`与batch字段私有、non-serde：id/reason非空且无NUL，schema/metadata只能object，batch 1..=256、id唯一、整体≤1MiB。
- `ProviderRemoteResumeStatus`只含Resolved/Cancelled。
- resume entry id非空无NUL，payload≤64KiB；batch 1..=256、id唯一、整体≤1MiB。
- message/schema/metadata/payload都不进Debug。

这些类型不含actor、tenant、grant、capability或local effect槽。remote descriptor仍是不可信presentation，不是权限。

## 3. Local run与protocol run分离

`RemoteAguiRoute`现在分别持有：

- 永不变化的local durable run id；
- 当前AG-UI protocol run id；
- resume时的parent protocol run id与boxed resume owner。

`with_resume`只接受parent与当前protocol id逐字相同、new id不同的batch。Box只用于稀有resume数据，避免所有ProviderRoute常态值膨胀；首轮Clippy实际捕获未Box时enum至少272B，修后无需allow即绿。

`encode_run_agent_input_with_resume`只在parent与resume同时存在、resume的parent/new id与参数完全一致时写`parentRunId/resume[]`；非resume旧请求继续由原wrapper生成，shape不变。单测让decoder以新run id接RUN_STARTED/RUN_FINISHED，证明不是只改JSON而忘记响应身份。

## 4. 当前刻意保持的红线

`RemoteAguiSession`已把interrupt outcome转成typed `ProviderEvent::Interrupted`，但`BuiltInAgentRuntime`仍将其按原行为fail-closed为`provider_generation_failed`。这是有意的阶段边界：

- 尚无native interrupt表和跨副本wait真源；
- 尚无fresh actor resolve、hash-chain audit-before-resume；
- 尚未接Agent AwaitingHuman的lease/cancel/deadline；
- 尚无第二次真实SafeDialer请求与UI pending/resolve；
- 缺这些证据时不得把T-EVT-0010改done。

## 5. 验证

- Application：`163/0/0`。
- Agent：`53/0/0`。
- Application/Agent/Infra all-target/all-feature Clippy `-D warnings`：通过。
- `cargo fmt --all -- --check`：通过。
- parity仍=`831/873/1704`、events=`43/45/88`、fixtures=`21/22/43`、overlay=`1293/403/2/6`，T-EVT-0010仍todo。
- 无schema/native/API/UI/dependency/Cargo/npm/Grok/workflow变化；不需重建Batch95 bundle。
- strict recount未配置上游目录；按R63未运行`cargo xtask ci`，未派发Actions。
- GitHub CLI token仍失效，本地提交未push/建PR。

## 6. 下一闭环

后续必须在同一纵向完成native persistence、actor-scoped pending/resolve、audit、AwaitingHuman wait/cancel/expiry、第二次RunAgentInput.resume、终态与UI，并用真实PG+SafeDialer两次请求证明；在此之前完整AG-UI和G4继续未完成。
