# Google 官方 `streamGenerateContent` recorded trace 外部交付

- 日期：2026-09-04
- 任务：外部任务 B——Google 官方 `streamGenerateContent?alt=sse` recorded trace
- 分支：`feat/2026-09-04-G4-google-recorded-trace`
- 独立工作树：见任务交付回执，不写入 public repository
- 固定基线：`8a91b2d5606891ee28db744c8ad7909a5a68b96e`（R188）

## 1. 结论

**证据不足，不交付 Google recorded SSE fixture，不声称本任务、G4 或三家 provider trace 完成。**

Google 官方公开仓库中可以固定以下两类证据：

1. 官方 SDK 确实以 record/replay 测试调用 `streamGenerateContent?alt=sse`，公开 recording 覆盖 text、`functionCall`、usage 与 `finishReason`；
2. Google `test-server` 确实是 record/replay reverse proxy，并在 record 模式读取厂商 HTTP response body。

但公开 recording 保存的是 recorder 从 SSE `data: ` 行反序列化得到的 JSON `bodySegments`，不是原始 HTTP SSE response body 字节。Python SDK 的独立 replay client同样先消费 parsed segments；可选 `byte_segments` 对普通 HTTP response 每段只保留前 100 字节。公开资产不能证明或恢复原始 `data:` framing、事件空行、行结束符、JSON 原始序列化、非 `data:` 行、压缩前字节或 HTTP chunk boundary。

把 `bodySegments` 用任意 JSON serializer 重写并添加 `data: ...\n\n`，会生成新的字段顺序、空白、转义和 framing，属于合成 golden，不是厂商原始 recorded trace；任务第一真源明确禁止这种做法。因此本分支没有创建：

- `fixtures/provider/google-*.sse`；
- `fixtures/provider/google-*.provenance.json`；
- `crates/openbot-infra/tests/google_recorded_trace.rs`；
- 对 `crates/openbot-infra/src/provider/google.rs` 的修改。

## 2. 来源门槛判定

| 判据 | 结果 | 证据 |
| --- | --- | --- |
| Google 官方仓库 | 满足 | `google/test-server` 与四个 `googleapis/*-genai` 官方仓库 |
| exact commit/blob | 满足 | 见 §3–§7 |
| 可证明的 record/replay 机制 | 满足 | `test-server` README、proxy、store；SDK record/replay 配置与测试 |
| `streamGenerateContent?alt=sse` | 满足 | Kotlin、JS、.NET recording 的 request URL 与 `Content-Type: text/event-stream` |
| text、`functionCall`、usage、finish 语义 | 满足 | 多份官方 `bodySegments` recording |
| 原始 SSE response body 字节 | **不满足** | recorder 在持久化前把 SSE `data:` 行 JSON 解码成 map/object |
| 可逆、确定性的原始字节提取 | **不满足** | framing、空白、顺序、行结束符与非 `data:` 行已丢失 |
| 可安全导入且不改原始字节 | **不满足** | 候选还含 `turnToken`、`thoughtSignature` 等 opaque 字段；删除会改变内容，保留不符合任务消毒约束 |
| production `GoogleProvider` recorded replay | **未执行** | 没有合格原始 fixture，不能用合成 SSE 冒充 |

## 3. Google `test-server`：record 机制与原始字节丢失点

- 官方仓库：<https://github.com/google/test-server>
- 固定 release：`v0.2.9`
- exact commit：`1f97f4f64f8f24a87d6069b20aaed6eefe745208`

| 文件 | Git blob | 字节 | SHA-256 | 用途 |
| --- | --- | ---: | --- | --- |
| [`README.md`](https://github.com/google/test-server/blob/1f97f4f64f8f24a87d6069b20aaed6eefe745208/README.md) | `a3590a8a40c6e8a70151c114e1b68d64b8254ff8` | 1,953 | `a149b2db68a5677d131da62ea97e1bae41ad698796c0460feaa567fd70b54b98` | 定义 record/replay reverse proxy、record/replay 命令与 header redaction |
| [`internal/record/recording_https_proxy.go`](https://github.com/google/test-server/blob/1f97f4f64f8f24a87d6069b20aaed6eefe745208/internal/record/recording_https_proxy.go) | `4becf1a5bbd5bc872a556d4836ca3674e164b116` | 9,998 | `b7e8ef492f5f5094721b7f1a75950cc6182f25da4359c24f67d38b9f06ae9a5d` | `proxyRequest` 调用真实 HTTPS upstream，并以 `io.ReadAll(resp.Body)` 取得完整 response body；随后交给 `NewRecordedResponse` |
| [`internal/store/store.go`](https://github.com/google/test-server/blob/1f97f4f64f8f24a87d6069b20aaed6eefe745208/internal/store/store.go) | `2241e8921ce2237001fa38fbc33a3106d77b4a8c` | 6,982 | `9dc2cdca67ed1af91ad15b1bfc59f36622476c3310f46c2649e582a044a60f99` | `NewRecordedResponse` 的持久化转换，是来源门槛失败点 |
| [`LICENSE`](https://github.com/google/test-server/blob/1f97f4f64f8f24a87d6069b20aaed6eefe745208/LICENSE) | `d645695673349e3947e8e5ae42332d0ac3164cd7` | 11,358 | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` | Apache-2.0 |

`RecordingHTTPSProxy::proxyRequest` 把 upstream response body 原样返回客户端，但 `recordResponse` 在写 JSON recording 前调用 `store.NewRecordedResponse`。后者执行：

1. gzip response 先解压；
2. 若整 body 不是单个 JSON object，则用 `bufio.Scanner` 按行扫描；
3. 跳过空行；
4. 只接受字面前缀 `data: `；
5. 对后续字节做 `json.Unmarshal` 到 `map[string]any`；
6. redaction 后只写入 `RecordedResponse.BodySegments`。

这一步不可逆地丢失：

- `data:` 的原始空格形式和 SSE 空行分隔；
- LF/CRLF 与文件结尾；
- JSON key 顺序、数字/字符串原始写法、空白与转义；
- `event:`、`id:`、`retry:`、comment 及未知行；
- HTTP content-encoding 前的原始字节；
- 网络 read boundary 与 HTTP chunk boundary。

因此 `bodySegments` 能证明“语义对象来自真实录制”，不能证明“公开文件保存了原始 SSE response bytes”。

许可与版权：上述 Go 文件声明 `Copyright 2025 Google LLC`，按 Apache License 2.0 发布。由于本任务没有复制任何上游源码或 response fixture 到仓库，不需要修改中央 NOTICE；也没有把 Apache-2.0 内容重许可为本项目内容。

## 4. Kotlin SDK：语义覆盖充分，但只有 `bodySegments`

- 官方仓库：<https://github.com/googleapis/kotlin-genai>
- exact commit：`b2f0983654122f7b16b1137a8e3e35dcf8eb743f`

### 4.1 record/replay 机制

| 文件 | Git blob | 字节 | SHA-256 |
| --- | --- | ---: | --- |
| [`BaseTestServer.kt`](https://github.com/googleapis/kotlin-genai/blob/b2f0983654122f7b16b1137a8e3e35dcf8eb743f/src/commonTest/kotlin/com/google/genai/kotlin/BaseTestServer.kt) | `6b6728045e858f90e163fc86209b09d69c7be81a` | 5,423 | `6897dab4bf2eaa12b7626ea3bbc4b05173f19df0e36e443112eca822715133cb` |
| [`ModelsTest.kt`](https://github.com/googleapis/kotlin-genai/blob/b2f0983654122f7b16b1137a8e3e35dcf8eb743f/src/commonTest/kotlin/com/google/genai/kotlin/ModelsTest.kt) | `2ff1747067a39f2aa1505d4f367a3012fb7840fa` | 38,620 | `40d11ee181adce6abf3be70447db5f757dbc760a1ca243e90d186bc080cdb86c` |

`BaseTestServer` 明确配置 recording directory、record/replay mode、redaction secrets，并把 client 指向本地 test-server。`ModelsTest.testGenerateContentStreamSimple` 与 `ModelsTest.testGenerateContentStreamFunctionCall` 通过 `generateContentStream` 消费对应 recording。

### 4.2 recording

| 官方 recording | Git blob | 字节 | SHA-256 | 可证明语义 | 原始 SSE 字节 |
| --- | --- | ---: | --- | --- | --- |
| [`ModelsTest.testGenerateContentStreamSimple.mldev.json`](https://github.com/googleapis/kotlin-genai/blob/b2f0983654122f7b16b1137a8e3e35dcf8eb743f/src/commonTest/resources/recordings/ModelsTest.testGenerateContentStreamSimple.mldev.json) | `ac6f3f52ff61c8a764441887e86fbc5b04b57c02` | 21,985 | `3500405edbd0dcb21c87f71ed8c5117a8a0cd05c1781c4cc95208dff822f9298` | `text/event-stream`、多段 text、usage 单调增长、`STOP` | 否；response 是 JSON `bodySegments` |
| [`ModelsTest.testGenerateContentStreamFunctionCall.mldev.json`](https://github.com/googleapis/kotlin-genai/blob/b2f0983654122f7b16b1137a8e3e35dcf8eb743f/src/commonTest/resources/recordings/ModelsTest.testGenerateContentStreamFunctionCall.mldev.json) | `d25f3d52d56e72d026c2c864b0adb24c485ff395` | 5,203 | `5e7a51e1c6ee3704170003057cf21df5429552a2b5bcc2fb4d362f9a8f1416b7` | `text/event-stream`、`functionCall`、usage、`STOP` | 否；response 是 JSON `bodySegments` |

两份 response body 都含 opaque `turnToken` 与 `thoughtSignature` 字段。本任务没有复制这些字段值，也没有为其计算单独 hash。删除这些字段再重包 SSE 会改变语义记录；原样保留又不满足任务“不能保留 token”的消毒约束。无论哪种方式都不能产出合格原始 fixture。

仓库级 [`LICENSE`](https://github.com/googleapis/kotlin-genai/blob/b2f0983654122f7b16b1137a8e3e35dcf8eb743f/LICENSE)：Git blob `7a4a3ea2424c09fbe48d455aed1eaa94d9124835`，11,357 字节，SHA-256 `58d1e17ffe5109a7ae296caafcadfdbe6a7d176f0bc4ab01e12a689b0499d8bd`，Apache-2.0；测试源码声明 `Copyright 2026 Google LLC`。

## 5. JavaScript SDK：锁定 test-server 0.2.9，recording 仍非原始字节

- 官方仓库：<https://github.com/googleapis/js-genai>
- exact commit：`aeb745302671ac0b70d48a4b4415952f12970a76`

| 文件 | Git blob | 字节 | SHA-256 | 证据 |
| --- | --- | ---: | --- | --- |
| [`package.json`](https://github.com/googleapis/js-genai/blob/aeb745302671ac0b70d48a4b4415952f12970a76/package.json) | `319757b76b8268708dfdfb8dc74b00f80a7c1b97` | 6,665 | `64e3cd991d6b4749cd4b3afe46c8a0ad2f5d983f33df94081343797693fdbc81` | `test-server-tests:record` 命令与 `test-server-sdk ^0.2.9` |
| [`package-lock.json`](https://github.com/googleapis/js-genai/blob/aeb745302671ac0b70d48a4b4415952f12970a76/package-lock.json) | `52ecc13d7069044ab63d2d1515018317a3b08509` | 381,976 | `d946a563c7fd4cd0a11334355d5695a53962dcbb55da310b0615e651634bb510` | exact `test-server-sdk 0.2.9` |
| [`Client_Tests_generateContentStream_ML_Dev_should_stream_generate_content_with_specified_parameters.json`](https://github.com/googleapis/js-genai/blob/aeb745302671ac0b70d48a4b4415952f12970a76/test/system/recordings/Client_Tests_generateContentStream_ML_Dev_should_stream_generate_content_with_specified_parameters.json) | `ae7334aa6f25b4a3dc1a8034602d1c842aa14601` | 5,556 | `2ff7218a81dbef0064fa9edbf57373bf6aefcf305ae5a0e808916143fe8704d3` | text、usage；finish 为 `MAX_TOKENS`；response 是 `bodySegments` |
| [`Chats_Tests_chats_function_calling_Google_AI_with_function_calling_stream.json`](https://github.com/googleapis/js-genai/blob/aeb745302671ac0b70d48a4b4415952f12970a76/test/system/recordings/Chats_Tests_chats_function_calling_Google_AI_with_function_calling_stream.json) | `6d6a867c0aa63790bede3f4c2c0f77be7fa9bb9d` | 7,978 | `3f860077eda84683eb8540180af390ed7e6a016922be35d5b4589b7d8247fc16` | `functionCall`、usage、`STOP`；response 是 `bodySegments` |

该仓库直接锁定 §3 审计的 `test-server-sdk 0.2.9`，因此其 recording 的 `bodySegments` 与 `test-server` 持久化转换一致，不能反推原始 SSE bytes。

仓库级 [`LICENSE`](https://github.com/googleapis/js-genai/blob/aeb745302671ac0b70d48a4b4415952f12970a76/LICENSE)：Git blob `d645695673349e3947e8e5ae42332d0ac3164cd7`，11,358 字节，SHA-256 `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`，Apache-2.0。

## 6. .NET SDK：官方 record/replay JSON 同样只有 `bodySegments`

- 官方仓库：<https://github.com/googleapis/dotnet-genai>
- exact commit：`076e8f4812eb0617f12827dc040674fbab4903e5`

| 文件 | Git blob | 字节 | SHA-256 | 证据 |
| --- | --- | ---: | --- | --- |
| [`Google.GenAI.E2E.Tests/README.md`](https://github.com/googleapis/dotnet-genai/blob/076e8f4812eb0617f12827dc040674fbab4903e5/Google.GenAI.E2E.Tests/README.md) | `c6fc5605e429fd2a228aeb8b6073eba46fbdfc30` | 2,854 | `5a1a8e95a771ce1bf5ec872e64a3e730b91044e677a5227240f1da97a22d8f80` | 明确说明 `record`/`replay` 两种模式和 `Recordings` JSON 目录 |
| [`Directory.Packages.props`](https://github.com/googleapis/dotnet-genai/blob/076e8f4812eb0617f12827dc040674fbab4903e5/Directory.Packages.props) | `e4c838c5ee170002a3a4bcc2f041961156184336` | 1,926 | `94ab865db49175b2973872ed7ef5715d30e40ef327677083aa8a54bdc73510b6` | `TestServerSdk 0.1.5` |
| [`Google.GenAI.E2E.Tests/packages.lock.json`](https://github.com/googleapis/dotnet-genai/blob/076e8f4812eb0617f12827dc040674fbab4903e5/Google.GenAI.E2E.Tests/packages.lock.json) | `b5d29a1847a9b8671da4ba7c8e121c22ee347a47` | 12,539 | `3cd252f44aba0a888f7e43e1c80720195f094d137957e30a1b7190ae3b4a3384` | 锁定 `TestServerSdk 0.1.5` |
| [`GenerateContentStreamSimpleTest.GenerateContentStreamSimpleTextGeminiTest.json`](https://github.com/googleapis/dotnet-genai/blob/076e8f4812eb0617f12827dc040674fbab4903e5/Google.GenAI.E2E.Tests/Recordings/GenerateContentStreamSimpleTest.GenerateContentStreamSimpleTextGeminiTest.json) | `eda272c6b839297eb8840e26302692c35e325eb9` | 4,081 | `247b762a0b1d12e79b05c4c29091951486786e6a4140aa78a7c99fc5c94fc651` | text、usage、`STOP`；response 是 `bodySegments` |
| [`GenerateContentStreamToolsTest.GenerateContentStreamManualFunctionCallGeminiTest.json`](https://github.com/googleapis/dotnet-genai/blob/076e8f4812eb0617f12827dc040674fbab4903e5/Google.GenAI.E2E.Tests/Recordings/GenerateContentStreamToolsTest.GenerateContentStreamManualFunctionCallGeminiTest.json) | `e22a9fa1f38829b4d480d3464c3f199b5d113014` | 3,665 | `2629841efffa9a535fabed0df1108f45e4d967018de81ea5074c74b43b2485a2` | `functionCall`、usage、`STOP`；response 是 `bodySegments` |

即使不依赖具体 SDK 版本内部实现，公开 recording 本身也只提供 JSON object array，未提供 SSE framing 或原始 response body。它不能通过确定性提取满足任务来源门槛。

仓库级 [`LICENSE`](https://github.com/googleapis/dotnet-genai/blob/076e8f4812eb0617f12827dc040674fbab4903e5/LICENSE)：Git blob `7a4a3ea2424c09fbe48d455aed1eaa94d9124835`，11,357 字节，SHA-256 `58d1e17ffe5109a7ae296caafcadfdbe6a7d176f0bc4ab01e12a689b0499d8bd`，Apache-2.0。

## 7. Python SDK replay client：parsed segment 与截断 byte segment

- 官方仓库：<https://github.com/googleapis/python-genai>
- exact commit：`28430799e32430265a1f8012383739a481d94629`

| 文件 | Git blob | 字节 | SHA-256 |
| --- | --- | ---: | --- |
| [`google/genai/_replay_api_client.py`](https://github.com/googleapis/python-genai/blob/28430799e32430265a1f8012383739a481d94629/google/genai/_replay_api_client.py) | `90b0a807844986a6991b5a7504a9400d981ca13b` | 26,423 | `a1664d7f1c1093edd162b41e17e23da7585f945506192f3c843547bac1b93b0c` |
| [`LICENSE`](https://github.com/googleapis/python-genai/blob/28430799e32430265a1f8012383739a481d94629/LICENSE) | `d645695673349e3947e8e5ae42332d0ac3164cd7` | 11,358 | `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30` |

`ReplayResponse` 持有 `body_segments` 与可选 `byte_segments`。普通 `HttpResponse` 的录制逻辑是：

- `body_segments=list(http_response.segments())`；
- `byte_segments=[seg[:100] for seg in http_response.byte_segments()]`。

同步与异步 stream record 路径都先迭代 parsed segment，再执行 `json.dumps(segment)` 重建 response stream；replay 也对 `body_segments` 逐段 `json.dumps`。这证明该机制面向 SDK 解析对象重放，而不是保存完整原始 HTTP SSE body。每段前 100 字节的可选样本也不能证明完整 response，更不能恢复超出 100 字节的部分或原始 framing。

对该 exact checkout 的公开 `*.json` 搜索未发现包含 `body_segments` 或 `byte_segments` 的 checked-in replay 文件；同一字段搜索在 `_replay_api_client.py` 有正向命中。因此没有可进一步审计的完整 Python raw recording。

该文件声明 `Copyright 2025 Google LLC`，按 Apache-2.0 发布。

## 8. 原始字节与“提取后”身份

本任务没有合法的原始 SSE source bytes，因此不存在可报告的“原始 response body 字节数/SHA-256”或“提取后 fixture 字节数/SHA-256”。§3–§7 表中的字节数/SHA-256只标识官方源码与官方 JSON recording 文件本身，**不把 recording JSON 的 hash 冒充 SSE body hash**。

合法提取规则判定为：

```text
输入：官方 recording 的 response.bodySegments
输出：拒绝
理由：缺失信息使原始 SSE bytes 不可逆；任何 serializer + "data:" 包装都是新生成内容
```

因此：

- 原始 SSE bytes：不可得；
- 提取后 SSE bytes：未生成；
- `cmp` 身份：不可执行；
- SSE fixture SHA-256：不存在；
- 一对一 provenance：不创建伪记录。

## 9. 消毒、凭据与 canary

- 没有调用 live Google provider；
- 没有读取或提交真实 API key；
- 没有把客户请求写入仓库；
- 没有复制官方 recording 中 opaque token 类字段的值；
- 没有为 API key、token、customer content 生成可验证 secret hash；
- 文档只记录公开文件级 SHA-256、Git blob 与不含秘密的字段名；
- `x-goog-api-key` 只作为协议/header 名称出现，不含值；
- 无 fixture，因而没有把消毒后的 mutation 冒充原始录制。

最终 canary/secret 扫描结果见 §10。

## 10. 验收结果

来源门槛失败没有被测试绿色覆盖。`google_recorded_trace` test target 未在无合格 fixture 时伪造，因此任务指定命令按事实报告失败。

| 命令 | 退出码 | passed/failed/ignored | 结果 | 私有交付日志文件名（不入仓） |
| --- | ---: | --- | --- | --- |
| `cargo test -p openbot-infra --test google_recorded_trace --locked` | 101 | 未进入 test harness | `openbot-infra` 不存在该 test target；这是来源门禁未通过后的明确未完成项 | `openbot-g4-google-recorded-trace-required-test.log` |
| `cargo test -p openbot-infra google --locked` | 0 | 13/0/0 | Infra unit 9/0/0；`google_drive_runtime` 4/0/0；其它 integration binary 只被过滤，不计作通过 | `openbot-g4-google-existing-tests.log` |
| `cargo clippy -p openbot-infra --all-targets --all-features --locked -- -D warnings` | 0 | N/A | 通过，无 warning | `openbot-g4-google-recorded-trace-clippy.log` |
| `cargo fmt --all -- --check` | 0 | N/A | 通过 | `openbot-g4-google-recorded-trace-fmt.log` |
| `git diff --no-index --check -- /dev/null docs/2026-09-04-Google-recorded-trace-外部交付.md` | 1 | N/A | 新文件非空差异的预期退出码，stdout/stderr 为空，表示无 whitespace error；包装 gate 接受 rc=0/1 且要求诊断为空，退出 0。候选提交创建后再对固定基线运行 `git diff --check` | 会话 Git 回执 |

范围与安全审计：

- 工作树只新增本交付文档；没有 production、fixture、provenance、Cargo、parity 或中央台账改动；
- `grok-bot` tree 仍为 `86f5a85f560f721677fa7e587a67ac0ffc036cb5`；
- 非 Grok `package.json` 仍恰一份：`crates/openbot-desktop/engine-shim/package.json`；
- `Cargo.lock`、`package.json`、npm lock 与 `node_modules` diff 均为空；
- 对本交付文档执行本机路径、常见 Google credential、bearer/private-key、opaque token value 与 URL query credential canary，命中为 0；
- 未运行 npm/npx、`cargo xtask ci` 或 GitHub Actions；未调用 live provider。

## 11. 明确未完成项

1. 未取得 Google 官方公开的完整原始 `streamGenerateContent?alt=sse` response body 字节；
2. 未创建 Google recorded SSE fixture 或 provenance；
3. 未创建 production `GoogleProvider` recorded replay 测试；
4. 未验证整块、非规则、逐字节 HTTP chunk 对同一 Google recorded input 的等价输出；
5. 未由 recorded trace 验证 usage 单调与 terminal 恰一次；
6. 未由 recorded trace 验证 header-only key、URL 无 key、坏帧/error body 的 Display/Debug/normalized failure 隔离；
7. 未修改 `GoogleProvider`，因为没有真实原始 replay 暴露兼容缺口；
8. 未关闭 Google trace、G4、G6、v4 或三家 provider recorded trace；
9. 未运行 `cargo xtask ci`，未派发 GitHub Actions，未调用 live provider。

## 12. 后续解除阻断所需证据

只有获得以下任一官方资产后，才能重新启动 fixture 与 production replay 实施：

1. Google 官方仓库中逐字节保存的完整 HTTP SSE response body，且有 exact commit/blob、record 机制与许可证；或
2. Google 官方 record 工具产生的原始 body sidecar，能以确定性规则逐字节提取并与 source `cmp` 相等。

仅有 `bodySegments`、SDK 对象、JSON array、非流式 response、手写 golden、网络抓包转述或重新序列化的 `data:` 行仍不合格。
