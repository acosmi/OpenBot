# Batch104：OpenAI Responses 官方 recorded trace 回放与来源闭包

日期：2026-09-04

候选原提交：`b50dd1caa08f9fecb57ca48270dae8cc22e4978b`

本分支实现：`cb6e2d69b5ea70d4a046d31aea00c3b2bdd02e77` +
`5becad90ee71fd6b84f1bbc6f47d1e24fdb3a1dc`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§7.3、§16.3、§23、§24 G0/G4、§28.1 R178

固定产品上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

本批 trace 来源：`openai/openai-dotnet@19d0a3cb8e0cf0f3137a5c56c3c70a0c3f6c96f5`

## 1. 本批结论

Batch104 接纳并加固外部候选中的 OpenAI Responses 官方 recorded trace。来源是 OpenAI 官方
`openai-dotnet` 仓库的 `ResponsesToolTests/FunctionToolStreamingWorks` recorded session，而不是手写
JSON、文档片段或本项目 test double。

固定 source record 为
`tests/SessionRecords/ResponsesToolTests/FunctionToolStreamingWorks.json`：

- commit time：`2026-09-02T18:33:36Z`；
- 13,211 bytes；
- Git blob SHA-1：`3272bc4116a32b4472d21f68b41ce23ce363c02c`；
- SHA-256：`ff7fadb3baabc1fd1fc482fe9ec28ba93d5a899b961cce73bf19f8d9c1fe20b8`。

本仓 fixture 是 `Entries[0].ResponseBody` 的36段字符串按顺序逐字节UTF-8拼接：9,070 bytes，
SHA-256 `fe38a7a044f8de33d441d0c9bf426e1291b1401bb9099ef419f62651b50d2927`；主控从固定
source record重新提取后与fixture执行`cmp`，字节完全相同。末尾的`\n\n`是SSE最后一个事件的协议
分隔符，不是编辑器垃圾；`.gitattributes`只对`*.sse`关闭`blank-at-eof`诊断，LF与其它whitespace
检查继续生效。

这只关闭 **OpenAI 1/3 provider recorded-trace 子证据**。Anthropic、Google仍未完成，故G4不勾；
该recorded response只有function call，没有text/reasoning delta，因此既有T-FIX-0013仍todo，也不冒充
live credential调用、供应商最终账单、完整tool/computer旅程或三家trace整关。

## 2. 消毒、许可与可复算来源

官方source record已经把Authorization、Date、organization/project/request id等字段标为
`Sanitized`。本仓进一步只保存response body，并在provenance中保留`content-type`与
`openai-version`两项非身份响应header；不复制RequestHeaders、RequestBody、Variables、prompt、API
key、账号/项目标识、客户数据或可验证secret hash。

新增一对一provenance、SPDX package/`GENERATED_FROM`关系与NOTICE投影：

- `fixtures/provider/openai-responses-function-tool-stream.provenance.json`；
- `provenance/sources.spdx.json`当前56个唯一package、65条relationship；
- OpenAI .NET仓库许可证为MIT，保留`Copyright (c) 2024 OpenAI (https://openai.com)`。

`tools/check-provider-recorded-traces.sh`离线校验closed provenance schema、provider官方GitHub组织、
fixture非symlink、一对一引用、字节数/SHA-256、header allowlist、消毒布尔值与credential-shaped内容。
本轮实得`traces=1; providers=openai`。此guard是后续Anthropic/Google候选的最低来源边界，不会把
“存在一个trace”误判为三家齐全。

## 3. 生产解码兼容与 fail-closed 边界

实录揭示当前OpenAI Responses事件的
`response.function_call_arguments.done`可以不带`name`。解码器现在仅在此前
`response.output_item.added`已提供同一tool的非空有界name时接受缺失或null name；若两处都有name则
必须逐字相等，两处都没有name仍拒绝。可选`output_index`一旦出现必须与开始事件一致。

新增单测同时锁住三条负向路径：

1. done事件把已开始的tool改名；
2. done事件把output index改到另一项；
3. added与done全程都不提供name。

三者均返回本地`InvalidResponse`，vendor prose/body不进入错误、Debug、audit或GUI。

## 4. 回放证据

集成测试不是直接喂decoder，而是loopback HTTP server经production
`OpenAiProvider → SafeDialer → HTTP/SSE → ResponsesDecoder`执行。对同一原始fixture分别使用：

- 整块body；
- `1/2/3/5/8/13/21/34/55/89`非规则chunk；
- 逐字节chunk，并只在测试副本插入一个未知UTF-8扩展事件。

三种分块得到完全相同的normalized event序列：一个response start、一个function-call start、12段
arguments delta、一个完成的`get_weather_at_location`、usage=`85/13/98`与唯一Completed。另以坏body
canary证明只返回`InvalidResponse`且错误输出不回显body。测试明确断言本trace没有TextDelta或
ReasoningDelta。

## 5. 本轮机械证据

- OpenAI provider unit：`7/0/0`；
- OpenAI recorded production replay：`1/0/0`；
- Infra lib unit：`328/0/0`；`cargo test -p openbot-infra`完整命令exit 0，需真PG的ignored项未冒充运行；
- `cargo clippy -p openbot-infra --all-targets --all-features -- -D warnings`；
- `cargo fmt --all -- --check`、`git diff --check`；
- provider recorded-trace guard：`1 trace / openai`；
- SPDX JSON：56个唯一package、65条relationship；
- `cargo xtask parity-check`：`848/862/1710`、fixtures `24/22/46`、0 violation；
- clean pinned upstream `891df72f…` strict recount：`160 passed / 0 mismatch / 0 skipped`；
- `git rev-parse HEAD:grok-bot`仍为`86f5a85f560f721677fa7e587a67ac0ffc036cb5`；非Grok
  `package.json`恰一，未运行npm。

按R63未运行`cargo xtask ci`，未派发GitHub Actions。没有真实provider凭据、PostgreSQL、schema、
native migration、API、UI、env、dependency、Cargo.lock或workflow变化。

复核期间曾把R172外部worktree与R177主控worktree指向同一个临时Cargo target，Cargo复用了不兼容的
旧rmeta并产生假编译错误。删除共享target后，各worktree使用自己的target重跑即通过。后续审计不同
worktree不得共享Cargo target；该错误不能记作产品回归或通过证据。

## 6. 台账与明确剩余

新增`T-FIX-0046 provider-openai-responses-recorded-trace`为done：

- parity=`848 done / 862 todo / 1710`；
- fixtures=`24 done / 22 todo / 46`；
- overlay carry/revalidate/split/superseded=`1299/403/2/6`；
- native latest=`0029`，schema=`47表/478列/342 NOT NULL/269约束/97索引`。

明确剩余：Anthropic与Google recorded trace；T-FIX-0013 text skeleton；真实provider credential trace；
acting Approval完整computer/thread/cancel集成；computer runtime budget；Desktop Local OAuth；
RMCP/computer/file/shell protocol cancel；Plugins/Skills UI；Browser/file/shell；P1 Windows/runsc真机；
ScreenHub/viewer ticket；Desktop真实Wry/正式发行与golden；G2外审/KMS/Windows；G8迁移、签名、外审、
operator-attestation、最终secret retirement及v4其余全部未闭合项。
