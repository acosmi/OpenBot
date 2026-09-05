# G4 provider 外部交付主控验收 Batch121

日期：2026-09-04（America/Los_Angeles）。第一真源修订：R197。
主控基线 R196 `87d84bb85d0056dfa4dcc2b35be4c2a610a55ae3`；分支 `feat/2026-09-04-G4-provider-delivery-audit`。

## 结论

Anthropic 两份官方发布的 response-body fixture 已通过主控独立验收，新增 T-FIX-0056。
三家 recorded provider 子证据从 OpenAI 1/3 变为 OpenAI+Anthropic **2/3**。
Google 交付是合格的来源调查，所审查的公开 JSON recordings 没有原始 SSE 字节；它没有完成 Google recorded replay。
Google、live 凭据、T-FIX-0013 等完整 corpus 和 G4/Alpha/v4 整关继续未完成。

| 外部任务 | 原候选 | 主控本地择取 | 结果 |
|---|---|---|---|
| A Anthropic | `7903e61e37cd528ecfcb8776b6d9bec62b0e0b55` | `442b6fe297436602dda3859fd9b7c769624bfd96` | 两份 trace + production replay + 来源/消毒验收通过 |
| B Google | `7dcba21e1ac13f29c88ef3329f285ebb4769a5da` | `2d1505174a9ed3dc6eb59a9752794320a866f838` | 仅调查报告；fixture、production replay 未实现 |

两候选均从 R188 `8a91b2d5606891ee28db744c8ad7909a5a68b96e` 出发、基线后恰一个 commit，外部工作树干净。
主控亲读全部新增测试、provenance、报告；候选未改 production adapter、Cargo、中央台账或其它任务。
外部工作树未修改。接收时原出口网关4文件在制稿已按每文件字节/SHA完整备份到主控忽略目录，
恢复 R196 后独立验收；网关稿未提交、未测试，不计任何功能完成。

## 独立来源核验

本轮重新抓取32份官方固定源码/录制材料/提交元数据，不以执行方的成功日志代替来源。

| Anthropic fixture | 官方 source bytes / Git blob | 提取 body bytes / SHA-256 |
|---|---|---|
| tool-use | 10980 / `2085d5a9d2bb3b97992e74206a35fb0c92253ecb` | 3489 / `9e75e3423449cfda1266e73327f43949fa0318b68a1d17293d4d06fe7ecbd783` |
| thinking | 1609 / `b6d1f6575606504542fd59bf55b8ef6cbeaa7731` | 1415 / `d5cf8f848dd95e809110c93c7531d3689331f52a92f0722211ca5c71bbff23d8` |

Go 固定 `e9c104e7e5fb80a26ff26e398c0e4e3fe1fe7f33`，YAML解析 `interactions[0].response.body` 后编码为UTF-8；
PHP 固定 `93aa419595dceeb7062292e09406b4e2a63b96e1`，在首个 LF-LF 分隔符后提取到EOF。
两份独立提取与仓内 fixture 全字节相同；source SHA-256、commit time 与 MIT/copyright 也逐项相同。
原始HTTP chunk boundary不在本项保留范围；重放的三种chunk切分是本项目测试设置。

Go 的[固定 go-vcr helper](https://github.com/anthropics/anthropic-sdk-go/blob/e9c104e7e5fb80a26ff26e398c0e4e3fe1fe7f33/internal/testutil/vcr.go)
与对应 runner test 支持 record/replay 来源；旧 recording model 与后来 SDK test 的 model 不混写。
PHP 的[固定 fixtures README](https://github.com/anthropics/anthropic-sdk-php/blob/93aa419595dceeb7062292e09406b4e2a63b96e1/fixtures/README.md)
及 `MessageAccumulatorTest` 明确声明 captured raw HTTP streaming response。
这是发布者的来源声明加本轮字节核验，不能写成本项目亲历了厂商捕获或验证了其内部生成器。

只复制公开 response body，不复制原始 request、组织/request/rate-limit headers 或账号信息。
保留的公开 message/tool ID 只作不可信关联，PHP 示例 signature 被 production decoder 忽略；它们不授予 authority。
NOTICE §4.3C 与两个 SPDX source package/GENERATED_FROM 关系同步，SDK 本身未成为运行依赖。

Google 报告的22项文件身份（bytes/Git blob/SHA）全部独立相符，核对
[固定 test-server store](https://github.com/google/test-server/blob/1f97f4f64f8f24a87d6069b20aaed6eefe745208/internal/store/store.go)
确实先把 SSE data 行 JSON 解析成 map 再写 BodySegments；
[固定 Python replay client](https://github.com/googleapis/python-genai/blob/28430799e32430265a1f8012383739a481d94629/google/genai/_replay_api_client.py)
也使用 parsed segments，普通 response 的 byte segment 截为每段前100字节。
这些资产不能逆推出原始 framing、空白和完整body；重新加 `data:` 会创建合成SSE。
本轮未穷尽其它/未来官方资产，因此结论限定为“所审来源不足”，不宣称不存在任何合格 Google trace。

## 主控实跑

| 命令或检查 | 本轮结果 |
|---|---|
| `CARGO_INCREMENTAL=0 cargo test -p openbot-infra --test anthropic_recorded_trace --locked` | 3 passed / 0 failed / 0 ignored；宿主 loopback，0.11秒测试执行 |
| `cargo test -p openbot-infra --lib provider:: --locked` | 32 / 0 / 0；含现有 Anthropic、Google、OpenAI、SSE 回归 |
| `cargo clippy -p openbot-infra --all-targets --all-features --locked -- -D warnings` | exit0 |
| `cargo fmt --all -- --check` | exit0 |
| `bash tools/check-provider-recorded-traces.sh` | exit0；3 traces，providers=anthropic,openai |
| `cargo xtask parity-check` | 0违反；parity886/826/1712，fixtures36/20/56，overlay1234/470/2/6 |
| `OPENBOT_UPSTREAM_DIR=/private/tmp/openbot-v4-upstream-891df72 cargo xtask recount --require-upstream` | 160通过 / 0失配 / 0跳过 |
| `cargo xtask grok-inventory --check` | 2110文件同步；固定tree不变 |
| `cargo xtask electron-shim-check` | 3文件、595/600 LOC、protocol hash匹配、非Grok package.json恰1 |
| `git diff --check` | exit0 |

运行中均设置 `CARGO_INCREMENTAL=0`。隔离审计输出位置：`target/qa/provider-delivery-audit/`，
含source inventory、独立identity结果和provider/Clippy/fmt/guard/parity/recount/Grok/shim原始输出。
source原件留本地忽略目录且私有权限，不提交其请求/响应头。

production replay 的三项测试实际覆盖：每份whole/irregular/bytewise都经过唯一SafeDialer与真实HTTP chunked SSE；
固定text/tool/thinking/usage序列、唯一最后terminal；UTF-8扩展mutation、malformed/error body、usage回退与截断末尾拒绝。
这些mutation只在内存中，不能冒充新的官方录制。没有做 live 请求、PG/UI、真实账单、Windows/runsc或全供应链验收。
Google不存在新test target，不把“运行不存在的测试失败”当成新的实现或测试数。

## 后续与外派

当前主控继续 Browser/Computer runtime 与出口网关，原生控制仅在副本处理，不改源仓。
第二轮 E–K 任务书按R196固定基线预留；用户新增要求的I/J/K分别是坏帧漂移、断流恢复和Composer候选。
I/K明确无需联网调研，J只需本机loopback/PG。所有新任务尚未收到启动回执，不预记实施或通过。
Personal Skills仍待不可变候选，不重复派发。严格依既有远端上传审批边界保持本地；未派发Actions、未合并PR。

本批是检查点，总目标保持active，§24未通过的整关和§25十条DoD继续保留完整范围。
