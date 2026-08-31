# Batch87：G4 run-wide normalized token accounting

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G4-run-wide-budgets`
>
> base：`077934ceb4f7465bf72b1c88b898be537090195e`（Batch86 PR #69 merge commit）
>
> implementation：`7592caeae379ef78fefff06259c6ecfbd7614c34`
>
> 第一真源：v4 §7.2、§7.4、§8.4、§14.3、§15.3、§24 G3/G4、§28.1 R70/R161。

## 1. 结论

R70 已把 `OPENBOT_PROVIDER_MAX_OUTPUT_TOKENS` 接到每次 sampling 的 vendor request、normalized
usage 与 host 双重校验；Batch71 后真实 tool loop 可以连续多次 sampling，但此前没有 run-wide usage
真源。进程内相加不能安全跨 commit-unknown、重启或多副本，也无法证明同一次 provider usage 没被重复
计费。

本批关闭这一子项：每个 run 现在以 PostgreSQL 为唯一真源，按连续 sampling index 原子累计
normalized input/output/total token；同一最后 sampling 可精确回放，异值、跳号、lease/fencing 失效与
ceiling 漂移全部 fail closed。超预算 sampling 已经真实消耗，先如实记账，再以稳定
`failed/run_token_budget_exceeded` 收口；per-sampling 超限同样不丢失真实 usage。

本批没有把这项写成“完整 run-wide budget”：仓内没有权威的 provider/model/version/生效时点价格
provenance，也没有已裁决的用户费用上限输入。并发 tool 与 computer runtime 预算也尚未接线。硬编码
当天 vendor 价格会制造会漂移的第二真源，因此明确留作后续，不猜值、不伪造完成。

## 2. 不引入新配置的 derived ceiling

首个 authoritative context 的 per-sampling output cap 在 run 内固定；后续 context 若漂移，provider
启动前即按 invalid response 收口。当前唯一可由既有裁决机械推出的 run output 上界是：

```text
per-sampling max output × (初始 sampling + 最多 8 个 tool step) = cap × 9
```

它只是 consistency ceiling：在 provider 遵守每次上限且 8-step reducer 正常时不应被突破。它不是
用户费用预算，不限制 input token，也不替代未来的价格 snapshot/cost counter。把这个区别写进 R161，
避免后续把一个数学上界误报成产品费用控制。

## 3. native 0024 与原子协议

`native_0024.sql` 只在 `public.runs` 末尾追加 9 列，不建表、不 drop/rename/改类型：

- immutable `budget_max_output_tokens`；
- `usage_input_tokens / usage_output_tokens / usage_total_tokens` aggregate；
- `usage_next_sampling`；
- last sampling index 与 input/output/total 三元组，用于 commit-unknown 后的 exact replay。

三个具名 CHECK 固定 ceiling 正数、aggregate 非负且 total 覆盖已知分量、next/last shape。计数列名虽
命中 secret scanner 的 `token` 词根，但它们只是 `bigint` 数量，不含 prompt/reply/credential token；
七列逐一写入带理由的 `SECRET_SCAN_EXEMPTIONS`，没有关闭防漏扫描。

`PostgresRunRuntime::record_provider_usage` 在同一 transaction 内：

1. 验 normalized usage 自洽、u64→i64 可表示、ceiling 非零；
2. `FOR UPDATE` 锁 run/thread/lease，复核 run identity、owner、fencing、running 与 DB-clock expiry；
3. 固定 ceiling，要求 sampling index 恰等于 next；
4. last index+同 usage 精确 replay，旧异值或跳号 conflict；
5. checked 累加并写 aggregate/next/last；
6. 若新 aggregate 越界，commit 已消费 usage 后返回 `BudgetExceeded(new aggregate)`；精确重放仍返回
   同一 `BudgetExceeded`，不会误继续。

native 0024 的 PostgreSQL 17.11 schema fixture 由真实 `pg_catalog` 机械生成：45 表、437 列、320
NOT NULL、246 约束、91 索引、4 触发器、4 enum、1 public function、0 extension；文件 5,166 行，
SHA-256=`349ff3f9573ee97cd588d717ea107d3b3ed1c5cf677ff75dc637b4e104de1031`。regeneration 开/关各
`1/0/0`，旧 0023 每列保持子集，ledger 恰 12。历史 `native_0023` fixture 改为 apply-through 0023，
防止 latest=0024 后旧 oracle 被悄悄扩写。

## 4. Agent 接线

Agent runtime 为每个 run 维护 sampling index 与首次 context 的 derived ceiling。收到 `Usage` 时先做
既有重复/total/per-sampling shape 校验，再调用 durable usage port：

- recorded/replayed 且每 sampling 未越界：继续；
- recorded/replayed 但每 sampling 越界：`provider_token_budget_exceeded`；
- durable aggregate 越界：`run_token_budget_exceeded`；
- stale lease：走 lease-lost；
- DB unavailable/corrupt/conflict/commit-unknown：走 reconciliation，不猜写入结果。

tool exchange durable commit 后 sampling index 才 checked 前进。测试 fake 同样累计两轮 usage，证明
`3/2/5 + 5/1/6 = 8/3/11`，而不是只检查最后一轮；另有强制 budget receipt 验证稳定 terminal。报告
per-sampling 33>32 的用例同时断言 durable usage 仍为 `10/33/43`。

## 5. 本轮亲跑证据

| 证据 | 最终结果 |
| --- | --- |
| Agent / Application / Domain / Infra lib | `35/0/0`、`153/0/0`、`371/0/0`、宿主 `323/0/0` |
| Infra integration compile | 全部 integration test binaries `--no-run` 绿 |
| PostgreSQL run runtime | 完整 `5/0/0`；含usage exact replay/conflict/overage/terminal code/stale lease |
| PostgreSQL native history | native0016=`3/0/0`、native0023=`1/0/0`、native0024=`1/0/0` |
| schema0024 generation | regeneration 开/关各 `1/0/0`；ledger 12、旧列子集 |
| Clippy | Domain/Application/Agent/Infra all-target/all-feature `-D warnings` 绿 |
| format / diff | `cargo fmt --all -- --check`、`git diff --check` 绿 |
| parity | `813/881/1694`；fixtures=`18/22/40`；overlay=`1283/403/2/6`；0 violation/warning |
| recount | 本仓 `71 passed / 0 mismatch`；未设 `OPENBOT_UPSTREAM_DIR`，88 skipped；strict 未跑 |
| Grok/package/workflow | tree=`86f5a85f…`、inventory 2,110；非Grok `package.json` 恰1；workflow仅 `workflow_dispatch` |
| Cargo/UI | `Cargo.lock` 0 diff；无UI/CSS/i18n/bundle变化，未重跑Browser/golden |

没有运行 `cargo xtask ci`，没有派发 GitHub Actions；R63 manual-only 保持。

## 6. 首跑失败与修正

- 首版在 budget exceeded 时不更新 aggregate；复核发现这会低报已经消耗的 provider usage。改为先原子
  记账，再返回 `BudgetExceeded`，并让 exact replay重复同一超限结论。
- 首次 schema fixture 生成时本轮全特性构建把磁盘占满，报 `ENOSPC`；按用户授权执行 `cargo clean`，
  删除 8.2 GiB 可再生构建物后只重建定向目标。该次失败不计通过。
- 首次四 crate 全目标测试在 sandbox 内有 15 个 loopback socket 用例因 `Operation not permitted`
  失败；宿主重跑 Infra 最终 `323/0/0`，没有把 sandbox 限制写成代码失败。
- Clippy 首跑报 provider-event helper 10 参数；收成 typed `ProviderSamplingContext` 后全绿，没有加 allow。
- 最终 lib 首跑由 secret-column scanner 报 7 个新 token-count 列未分类；逐列确认是数值计数并写理由
  豁免，重跑 `323/0/0`。没有扩宽词根或禁用闸门。
- latest 提升后发现历史 native0023 test 调用了 apply-latest；改为 apply-through 0023，并以真实PG
  `1/0/0`证明旧 fixture仍独立。
- domain enum diff机械命中6条已done profile-policy target；先跑完整Domain `371/0/0`，再按R124加入
  T-TEST-0298–0303 revalidate，parity从6 violation回到0。

## 7. 未闭合边界与下一批

- 没有价格 snapshot/source/version/effective-at/币种语义，不能计算或强制用户费用上限；
- 没有 run-wide concurrent-tool semaphore/resource-lock budget，也没有 computer runtime counter；
- derived output ceiling是现有上限的机械一致性界，不是新产品配置，正常路径下本就不应触发；
- 三家 recorded/live vendor trace仍0/3，本批只消费既有 normalized Usage；
- remote AG-UI 是否进入同一 durable cost/token counter、computer/file/shell协议级cancel与完整Approval
  thread/cancel/computer旅程仍需后续闭合；
- 不关闭 G4 整关，不改`grok-bot/`，不新增Grok产品能力、npm、API、UI或T-ID。

下一批应先从权威配置/持久化模型定义“用户费用上限 + 带时点价格provenance”，或独立关闭并发tool/
computer runtime budget；在没有可审计价格来源前不得硬编码当前官网价格，也不得用output token数冒充费用。
