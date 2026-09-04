# Anthropic Messages recorded trace 外部交付

日期：2026-09-04

任务：外部任务 A —— Anthropic 官方 recorded trace

分支：`feat/2026-09-04-G4-anthropic-recorded-trace`

独立 worktree：`/Users/fushihua/Desktop/OpenBot-G4-anthropic-recorded-trace`

固定基线：`8a91b2d5606891ee28db744c8ad7909a5a68b96e`（R188）

候选 SHA 由本文件所在的单一候选 commit 与交付回执给出；本文不写自指 SHA。

第一真源：`CLAUDE.md`、v4 §7.3/§16.3/§23/§24 G0/G4/§25/§28.1 R70/R178，以及
`docs/2026-09-04-v4并行实施预留台账.md` §3.A/§4。

## 1. 结论与边界

本候选取得两份 Anthropic 官方仓库公开的 recorded/captured HTTP/SSE 资产，并把原始 response body
逐字节提取为一对一 fixture：

1. Anthropic Go SDK 的 `go-vcr` cassette 第一轮响应，覆盖 text、tool_use、partial JSON、usage 和
   `message_stop`；
2. Anthropic PHP SDK 明确声明为 verbatim captured raw HTTP streaming response 的 thinking 用例，覆盖
   thinking、signature、text、usage 和 `message_stop`。

两份 fixture 均通过 production
`AnthropicProvider → SafeDialer → 本机真实 TCP HTTP/1.1 chunked SSE → SseDecoder → AnthropicDecoder`
离线回放。原始 fixture 自身分别以整块、非规则分块和逐字节分块回放，normalized event 序列逐项相等；
usage 不倒退，terminal 恰一次。真实回放没有暴露 production adapter 兼容缺口，因此本候选没有修改
`crates/openbot-infra/src/provider/anthropic.rs`。

本结论只提供 Anthropic provider recorded-trace 候选证据。它不等于 live Anthropic 调用，不表示
OpenAI/Anthropic/Google 三家齐备，不关闭 T-FIX-0013、G4、G6、v4 或任何中央机器台账；主控独立审计并
择取该单一 commit 后，才可决定中央登记。

## 2. 官方来源、固定字节与确定性提取

### 2.1 Tool-use：Anthropic Go SDK `go-vcr`

- repository：`https://github.com/anthropics/anthropic-sdk-go`；
- exact commit：`e9c104e7e5fb80a26ff26e398c0e4e3fe1fe7f33`；
- commit time：`2026-09-03T22:32:55Z`；
- source record：
  `toolrunner/testdata/cassettes/tool_runner_streaming_all.yaml`；
- source URL：
  `https://github.com/anthropics/anthropic-sdk-go/blob/e9c104e7e5fb80a26ff26e398c0e4e3fe1fe7f33/toolrunner/testdata/cassettes/tool_runner_streaming_all.yaml`；
- recording proof：`internal/testutil/vcr.go` 明确使用 `go-vcr`，默认 replay，
  `ANTHROPIC_LIVE=1` 才 recording；`runner_test.go::TestToolRunner_AllStreaming` 注入该 VCR client；
- source endpoint：`https://api.anthropic.com/v1/messages?beta=true`，即官方 Beta Messages tool runner；
- production replay endpoint：`https://api.anthropic.com/v1/messages`，两者在 provenance 中分列，不混写；
- source record：10,980 B；Git blob SHA-1
  `2085d5a9d2bb3b97992e74206a35fb0c92253ecb`；SHA-256
  `a523fe3e4db93da6e1b8f715e151d448bc9cc2231bae603d5357f2f6583140fe`；
- fixture：`fixtures/provider/anthropic-messages-tool-use-stream.sse`；
- deterministic extraction：解析固定 YAML，取
  `interactions[0].response.body` literal scalar，连同尾部 LF 原样写出；
- fixture/raw response body：3,489 B；SHA-256
  `9e75e3423449cfda1266e73327f43949fa0318b68a1d17293d4d06fe7ecbd783`；
- 独立重提取后 `cmp` 相等。

normalized 输出为：一个 response start；一个 text block 与 5 段 text delta；一个 tool-use block、
一个 `get_weather` start、10 段非空 arguments delta、一个完成的
`{"city":"San Francisco","units":"fahrenheit"}`；usage=`397/89/486`；唯一 Completed。
官方流在 `message_start` 与 `message_delta` 重复报告相同 input usage，production adapter兼容相等值；
负向 mutation 把最终 output usage 从 89 降到 1 时稳定 `InvalidResponse`。

### 2.2 Thinking：Anthropic PHP SDK captured raw HTTP/SSE

- repository：`https://github.com/anthropics/anthropic-sdk-php`；
- exact commit：`93aa419595dceeb7062292e09406b4e2a63b96e1`；
- commit time：`2026-09-01T18:08:37Z`；
- source record：`fixtures/ga/thinking.txt`；
- source URL：
  `https://github.com/anthropics/anthropic-sdk-php/blob/93aa419595dceeb7062292e09406b4e2a63b96e1/fixtures/ga/thinking.txt`；
- capture proof：`fixtures/README.md` 明确称 `.txt` 为 “captured raw HTTP streaming response,
  verbatim”，对应 `curl -N -D - .../v1/messages`，并由正式 streaming entrypoint 回放；
- source record：1,609 B；Git blob SHA-1
  `b6d1f6575606504542fd59bf55b8ef6cbeaa7731`；SHA-256
  `70366a8b43b634eb1bc1e4e1fabbc7c14a8d182cca7561402ff40c07594652ef`；
- fixture：`fixtures/provider/anthropic-messages-thinking-stream.sse`；
- deterministic extraction：在固定 source bytes 的第一个 LF-LF HTTP header separator处分割，
  将其后直至 EOF 的字节原样保留；
- fixture/raw response body：1,415 B；SHA-256
  `d5cf8f848dd95e809110c93c7531d3689331f52a92f0722211ca5c71bbff23d8`；
- 独立重提取后 `cmp` 相等。

normalized 输出为：一个 response start；一个 reasoning block 与 2 段 thinking delta；signature delta
按既有 production 规则忽略；一个 text block与一段 text delta；usage=`15/20/35`；唯一 Completed。
本候选只因为官方 recorded fixture 真实包含 thinking 才报告 thinking 覆盖，没有用 mutation 或手写事件冒充。

## 3. 消毒、ID 非 secret 依据与 canary

Go source cassette公开记录了 request body、测试占位 key、Anthropic organization header、request ID、
rate-limit header等；本仓不保存它们，只保存第一轮 response body。PHP source的示例 request ID和rate-limit
header同样全部丢弃。两份 fixture 均不保存请求 prompt、Authorization/API key、账号/客户标识、客户数据或
可验证 secret hash；仅在 provenance 中保留 allowlisted `content-type`值。

保留的 vendor protocol values：

- Go message ID `msg_01H1pwRRkQxKbUGKi785gT4M`；
- Go tool-use ID `toolu_01RaX2WYWRWCbaeFHssmGJXG`；
- PHP message ID `msg_thinking_x`；
- PHP公开示例 signature delta `abc123sig==`。

上述值已经由 Anthropic 在公共 SDK 测试资产中发布，只用于不可信 trace correlation、streamed tool pairing
或被 decoder 忽略的协议测试，不授予任何 authority，也不是 API credential、session token、customer
identifier或secret。两份 provenance 以结构化 `retained_public_protocol_values`逐项固定该依据，测试要求
每项明确包含“grants no authority”。

测试与目录 guard 同时执行：

- fixture禁止 `Authorization`、`Bearer`、`ANTHROPIC_API_KEY`、`X-Api-Key`、`sk-ant-`、request JSON
  结构、organization UUID、`req_` request ID、URL、email-like `@` 和 canary；
- provenance递归拒绝 `authorization`、`api_key`、`organization_id`、`request_body`、
  `request_headers`、`request_id`、`request_prompt` 等敏感字段，并拒绝 UUID/request-ID/credential/canary
  形状；测试不复制源 organization、request ID、测试 key、请求 prompt 或完整 customer/secret canary 作为
  denylist 字面量，只使用通用前缀与结构形状；
- `tools/check-provider-recorded-traces.sh`验证 provider 官方组织、一对一引用、非symlink、字节/SHA、
  header allowlist、消毒布尔和credential-shaped内容；实得
  `provider recorded trace guard: ok (traces=3; providers=anthropic,openai)`；
- 候选全树对已丢弃 source identifier 与 secret-shaped canary 的精确反向扫描为 `No matches found`。

网络存在间歇故障，但本轮对上述 exact raw/API URL 的固定抓取成功；没有用手写或缓存拼造内容替代失败抓取。

## 4. Production回放与负向边界

`crates/openbot-infra/tests/anthropic_recorded_trace.rs`完整固定两份来源、fixture、provenance与normalized
事件。每份原始 fixture 均执行：

1. 整块 body；
2. `1/2/3/5/8/13/21/34/55/89`循环的非规则 HTTP chunk；
3. 每个 HTTP body byte 独立一个 chunk。

每次都使用真实 `TcpListener`、HTTP/1.1 chunked framing、production `AnthropicProvider`、唯一
`SafeDialer`和精确 `127.0.0.1/32` allowlist，并完整 drain session 到 `None`。请求侧逐次证明：

- request line恰为 `POST /v1/messages HTTP/1.1`，无query；
- key恰出现一次且只在 `x-api-key` header；
- 无 `Authorization`；
- 固定 `anthropic-version: 2023-06-01`；
- JSON body与URL均不含key；
- 本地 synthetic replay request不复制官方source request prompt。

所有 mutation 只在测试内存副本生成并明确标记 `test-only mutation` 或 `negative mutation`，不写入
recorded fixture：

- 未知 UTF-8扩展事件逐字节分块后不改变 recorded normalized输出；
- malformed SSE canary只形成一个 `InvalidResponse`且Debug不回显；
- in-stream Anthropic error message canary只形成规范化 `ServerUnavailable`且不回显；
- HTTP 500 JSON error body canary不进入 ProviderEvent/Debug；
- usage regression fail-closed；
- 删除最后一个SSE delimiter byte后以唯一 `InvalidResponse`收口；
- 所有正常/失败路径显式计数，Completed与Failed合计恰一且terminal为最后一个normalized event。

## 5. 本轮机械证据

宿主临时原始输出目录（不入仓）：`/tmp/openbot-anthropic-recorded/evidence/`。
不同 worktree 不共享 Cargo target；本任务独占 `CARGO_TARGET_DIR=/tmp/openbot-target-g4-anthropic`。
GUI worker初始非login shell未带Cargo/Homebrew native PATH，首次命令在编译前失败；补入仓库已钉的
`xmlsec1 1.3.12 / libxml2 2.15.3 / OpenSSL 3.6.3`路径后，以下最终命令全部exit 0：

| 证据 | 结果 | 原始输出 |
|---|---:|---|
| `cargo test -p openbot-infra --test anthropic_recorded_trace --locked` | 3 passed / 0 failed / 0 ignored | `01-anthropic-recorded-trace.log` |
| `cargo test -p openbot-infra --lib provider::anthropic::tests --locked` | 5 / 0 / 0；323 filtered | `02-anthropic-provider-unit.log` |
| SafeDialer sensitive-header exact test | 1 / 0 / 0；327 filtered | `03-safe-dialer-anthropic-header.log` |
| `cargo test -p openbot-server anthropic --locked` | lib 1/0/0 + bin 1/0/0，其余target 0 matched | `04-anthropic-server-tests.log` |
| managed rate-card与三provider production factory exact tests | 1/0/0 + 1/0/0 | `05-anthropic-factory-tests.log` |
| `cargo clippy -p openbot-infra --all-targets --all-features --locked -- -D warnings` | exit 0 | `06-openbot-infra-clippy.log` |
| source bytes/SHA/Git blob + 两份独立重提取 `cmp` | exit 0 | `07-source-extraction.log` |
| provider recorded-trace guard | 3 traces；providers=`anthropic,openai`；exit 0 | `08-provider-recorded-guard.log` |
| `cargo fmt --all -- --check` | exit 0 | `09-cargo-fmt-check.log` |
| `cargo test -p openbot-infra --all-targets --all-features --locked` | 374 passed / 0 failed / 169 ignored | `11-openbot-infra-all-targets.log` |

候选形成后已归档 `git diff --check`、固定基线到候选的 name-status、工作树状态、Grok tree和
`package.json` inventory，SHA由交付回执给出。首轮独立验证发现测试denylist误把已丢弃的source
organization/request标识写入候选；最终候选已改为结构化UUID/request-field扫描，并以候选全树反向扫描
确认这些值为零命中。没有运行 npm/npx，没有生成node_modules或lockfile，没有运行`cargo xtask ci`，
没有派发Actions，没有push、开PR或合并。

## 6. 中央 NOTICE 需增补的精确文本

外部候选不得直接修改中央 NOTICE。主控来源/许可复核通过后，可在现有OpenAI recorded trace节后加入：

```text
--------------------------------------------------------------------------------
4.3C Anthropic Go/PHP Messages recorded test traces —— MIT
--------------------------------------------------------------------------------

  Go来源仓库 : https://github.com/anthropics/anthropic-sdk-go
  固定 commit: e9c104e7e5fb80a26ff26e398c0e4e3fe1fe7f33
  源记录     : toolrunner/testdata/cassettes/tool_runner_streaming_all.yaml
  源字节     : 10,980 B；Git blob SHA-1
               2085d5a9d2bb3b97992e74206a35fb0c92253ecb；SHA-256
               a523fe3e4db93da6e1b8f715e151d448bc9cc2231bae603d5357f2f6583140fe
  本仓文件   : fixtures/provider/anthropic-messages-tool-use-stream.sse
  派生       : interactions[0].response.body literal scalar逐字节提取，3,489 B；
               SHA-256 9e75e3423449cfda1266e73327f43949fa0318b68a1d17293d4d06fe7ecbd783

  PHP来源仓库: https://github.com/anthropics/anthropic-sdk-php
  固定 commit: 93aa419595dceeb7062292e09406b4e2a63b96e1
  源记录     : fixtures/ga/thinking.txt
  源字节     : 1,609 B；Git blob SHA-1
               b6d1f6575606504542fd59bf55b8ef6cbeaa7731；SHA-256
               70366a8b43b634eb1bc1e4e1fabbc7c14a8d182cca7561402ff40c07594652ef
  本仓文件   : fixtures/provider/anthropic-messages-thinking-stream.sse
  派生       : 固定raw HTTP response首个LF-LF之后的response body逐字节提取，1,415 B；
               SHA-256 d5cf8f848dd95e809110c93c7531d3689331f52a92f0722211ca5c71bbff23d8

  许可证     : MIT
  版权行     : Copyright 2023 Anthropic, PBC.

  Go资产由官方go-vcr recording/replay测试发布；PHP资产由官方README明确声明为verbatim captured
  raw HTTP streaming response并经正式streaming entrypoint回放。本仓只保存response body；请求正文、
  Authorization/API key、organization/request/rate-limit header、账号/客户标识和客户数据均不复制。
  保留的message/tool-use ID与公开示例signature只作不可信trace correlation/协议测试，不授予authority，
  不是credential或secret。许可证全文与本文件§1收录的标准MIT条款相同；分发上述fixture时须同时保留
  Anthropic版权行和MIT条款。
```

主控还需在独立审计后统一增加两个 SPDX source package及`GENERATED_FROM`关系，并新增中央fixture
机器台账项；本候选不预占 T-ID，不擅改 `fixtures/MANIFEST.yaml`、parity、overlay、v4、CLAUDE、README、
移交指南或预留台账。

## 7. 未运行、未完成与证据不足项

- 未使用真实 Anthropic credential，未做 live调用、计费或供应商账单核对；
- 未运行 PostgreSQL ignored纵向；本任务没有改变数据库、schema、native migration、API或UI；
- 未运行完整 workspace、`cargo xtask ci`、strict fixed-upstream recount、供应链全闸门或GitHub Actions；
- 没有修改production adapter，因为两份官方recorded response已按现有production路径通过；
- Google recorded trace仍缺，三家provider gate仍非完整；
- T-FIX-0013、acting Approval完整thread/cancel/computer、完整computer runtime budget、Desktop Local OAuth、
  RMCP/computer/file/shell完整cancel、Plugins/Skills UI、Browser/file/shell、P1 Windows/runsc、完整Screen、
  G2/G6/G8与v4其余项目均保持未完成；
- 候选通过只表示可供主控审计，不代表任何中央T-ID或G4整关已关闭。
