# Batch88：G4 operator-attested run cost upper bound

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G4-run-cost-accounting`
>
> base：`7b4aa2e93a3e17125a69c10468bc819c1b4e467e`（Batch87 PR #70 merge commit）
>
> implementation：`6d46cb0d2ecd6d8d4b03c4c7d5d46ac3e2da91aa`
>
> 第一真源：v4 §7.3–§7.4、§8.4、§14.3、§15.3–§15.4、§24 G3/G4、§28.1 R161/R162。

## 1. 结论

Batch87 已把 normalized input/output/total token 按 run 跨 sampling 持久化，但仓内没有价格、币种、
价格来源或用户费用上限。直接硬编码当前官网价不诚实：价格会变，custom OpenAI-compatible endpoint
与企业合同也不一定按官网结算；runtime 联网抓价又会把预算真源交给可变外部页面。

本批建立可审计的前置：部署方显式提供 provider/model/currency/maximum rates/source URL+document
SHA-256/observed-at；同一 immutable snapshot 随 run 冻结，token usage 在同一 PostgreSQL transaction
里累计成本上界。没有 snapshot 时列保持 NULL，含义是 unpriced，不是零成本。

本批仍不宣称完整费用预算：用户尚无 cap 的 API/UI，counter 也不会因金额主动终止 run；并发 tool 与
computer runtime预算仍缺。它提供下一批费用 cap 唯一可依赖的持久化上界。

## 2. 为什么是“上界”而不是“账单”

三家现有 normalized `ProviderUsage` 只有 input/output/total，没有稳定、跨 vendor 的 cache-read、
cache-write、batch 或其它折扣分项。即使钉住公开 list price，也无法从当前事件重建供应商最终账单。

因此 rate 字段逐字叫 `max_*`：operator 必须填入不低于合同任一适用 cached/uncached 档位的每百万
token 最大费率。用全部 input/output token 乘 maximum rate 得到保守上界：可能高估并在未来 cap
落地后略早停止，但不能低估。文档、类型、列名和 R162 都禁止把它称作 vendor bill。

currency 是必填三位 uppercase code。OpenBot 不做汇率换算；不同 currency 不相加。未来用户 cap
必须与 run snapshot 同币种，否则 fail closed。

## 3. Rate snapshot 输入

Package built-in provider 在既有 `model.yaml` 的可选 `pricing` 对象声明：

```yaml
pricing:
  currency: USD
  max_input_micro_units_per_million_tokens: 1500000
  max_output_micro_units_per_million_tokens: 2000000
  source_url: https://prices.example.test/archive/2026-08-30
  source_sha256: <64 lowercase hex>
  observed_at: 2026-08-30T12:00:00Z
```

Managed provider 使用六个 all-or-none 环境字段：`BOT_PRICE_CURRENCY`、两条
`BOT_PRICE_MAX_*_MICRO_UNITS_PER_MILLION_TOKENS`、`BOT_PRICE_SOURCE_URL`、
`BOT_PRICE_SOURCE_SHA256`、`BOT_PRICE_OBSERVED_AT`。任一缺失或非法都成为启动配置错误；没有默认费率。

`ProviderRateCard` 还要求：model 1..256 bytes；rate 可表示为 PostgreSQL bigint；source 必须 HTTPS、
有 host、无 userinfo/password/query/fragment、≤2048 bytes；digest 恰64 lowercase hex；observed-at
不早于 Unix epoch。production context 用 DB clock 拒绝未来 snapshot，writer 再防御一次，因而不能
先调用 provider、后才发现 provenance 来自未来。

Package/managed route分别只拿自己的 snapshot；remote AG-UI 固定 unpriced。snapshot 不进入 vendor
request body，也不进入 prompt/tool arguments。

## 4. 无浮点 cost upper-bound counter

`ProviderCostUpperBound` 不使用浮点，也不逐 sampling ceil。它保存：

```text
whole micro currency units
+ millionths-of-one-micro-unit remainder (0..999999)
```

每轮将 `input_tokens × max_input_rate + output_tokens × max_output_rate + old_remainder` 用 checked
`u128` 计算，再做一次除法/余数。示例 rate 1.5/2.0 micro-unit per token 时，两轮各1 input+1 output：
第一轮为 `3 + 500000/1000000`，第二轮精确进位成 `7 + 0`；不会逐轮ceil成8。只有未来与用户 cap
比较时才在 aggregate 边界做保守 ceil。

## 5. native 0025

native0025只给 `public.runs` 追加10列和1个具名shape CHECK：currency/provider/model、maximum
input/output rate、source URL/SHA、observed-at，以及whole/remainder cost upper bound。全部NULL或全部合法，
不存在“有价格但没cost”半形。旧行和未定价run保持全NULL。

`record_provider_usage` 已有的active lease/fencing、continuous sampling index与last exact replay继续是
唯一写门；本批再要求snapshot与首轮逐字段相等。rate/currency/source/time任一漂移都conflict；超token
ceiling的sampling仍先记录token与cost upper bound，再返回原BudgetExceeded。future snapshot写0。

真实PostgreSQL 17.11 fixture：45表、447列、320 NOT NULL、247约束、91索引、4触发器、4 enum、
1 public function、0 extension；5,241行，SHA-256=
`f72fe00b7bf2690786d7810bfa0d9481da0ea325a6f896ff56f152a14d8094cc`。regeneration开/关各
`1/0/0`，0024逐旧列保持子集，ledger恰13。

## 6. 本轮亲跑证据

| 证据 | 最终结果 |
| --- | --- |
| Application / Agent / Infra / Server lib | `157/0/0`、`36/0/0`、宿主`323/0/0`、`217/0/0` |
| Desktop / Server bin | `130/0/3`、`7/0/0` |
| PostgreSQL context/native24/native25/run | `1+1+1+5 / 0 / 0`；package/managed exact route、future双拒、priced/unpriced、replay/drift/remainder/overage |
| schema0025 | regeneration开/关各`1/0/0`；45/447/320/247/91，ledger13 |
| Clippy | Application/Agent/Infra/Server/Desktop all-target/all-feature `-D warnings`绿 |
| dependency guards | SafeDialer、Tauri、六target cargo-deny release guard绿；Cargo package `825→825` |
| format / diff | `cargo fmt --all -- --check`、`git diff --check`绿 |
| parity | `819/881/1700`；env=`55/25/80`；fixtures=`19/22/41`；overlay=`1289/403/2/6`；0 violation/warning |
| recount | 本仓`71 passed / 0 mismatch`；未设`OPENBOT_UPSTREAM_DIR`，89 skipped；strict未跑 |
| Grok/npm/workflow | tree与inventory不变；非Grok package.json恰1；manual-only，Actions未派发 |

`Cargo.lock`只给Application记录既有`url 2.5.8` direct edge、Agent dev记录既有`time 0.3.55`；package
仍825，没有下载或许可新增。没有UI/CSS/i18n/bundle变化，未跑Browser/golden。没有运行
`cargo xtask ci`。

## 7. 首跑失败与修正

- 第一次 `--locked` compile在代码前正确拒绝新增direct edge；改用offline解析更新lock，机械确认
  package仍825，之后所有命令恢复`--locked`。
- 首个rate-drift Agent用例超时，定位到Batch87遗留状态机缺陷：第二轮context漂移发生在Preparing，
  旧代码却发Sampling专用`ProviderFailed`，既不继续也不terminal。改为`ContextFailed`，output-cap与
  rate drift共用正确路径，用例最终绿。
- 初稿称“exact cost”且固定USD；复核三家usage后确认cache细分缺失、合同币种也无第一真源，遂改为
  currency-aware maximum-rate cost upper bound，不把估算写成账单。
- secret scanner要求两条含`token`的rate列逐项说明公开计数；误加的`cost_source_sha256`豁免因不命中
  词根被dead-exemption闸门拒绝，删除后Infra最终323/0/0。
- SafeDialer guard仍只认识Batch67 threads loopback测试，漏了后来已存在的approvals/channels/Desktop
  sidecar三处test-only caller。更新为四文件精确集合，逐文件要求唯一caller严格位于唯一`cfg(test)`后；
  production client仍只有SafeDialer，guard最终绿。

## 8. 未闭合边界与下一批

- 没有用户费用上限的持久化偏好、typed ApplicationService、Server/Desktop transport或Settings UI；
- cost upper bound尚未触发terminal，只做可信计数；
- remote AG-UI没有本地可证明的provider/model contract rate，保持unpriced；
- 没有cache细分就不能显示“实际账单”；若未来扩展usage，必须新增vendor recorded trace与schema裁决；
- 所有production tool当前仍`parallel_safe=false`，并发tool budget无正向生产路径；browser/computer executor
  未落，computer runtime budget同样不能假闭合；
- 不关闭完整budget/G4，不修改`grok-bot/`，不新增Grok产品能力、npm、UI或API。

下一批应以本批snapshot/counter为唯一真源，新增actor-scoped、currency-bound per-run费用cap及
GET/PUT/API/Desktop/Settings UI；cap缺rate或币种不匹配时必须在provider effect前拒绝。
