# W-6 `AUDIT_RETENTION_DAYS` 迁移预检

> 第一真源修订：v3 §28.1 R51。本文是实施与运维证据，不另起一套架构真源。

## 1. 闭合的问题

固定上游 `server/src/config.ts::auditRetentionDays` 的注释说配置值不会被强转，但真实路径是：

```text
optional(environment, "AUDIT_RETENTION_DAYS")
→ Number(raw)
→ Number.isInteger(days) && days >= 1
```

因此旧部署可接受若干不是十进制整数字面量的写法。Rust 侧按 v3 §8.6 保持更窄的审计控制：
只接受十进制正整数，且不把不认识的值折成“永久保留”。如果不做切换前扫描，这类部署会在
新 server 启动时才第一次失败。

本轮新增：

- `openbot-migrate preflight-audit-retention`，只读当前进程环境；
- `openbot_server::config::preflight_audit_retention(&EnvMap)` 纯函数；
- 稳定问题码 `canonical_decimal_required` / `exceeds_supported_range`；
- 可无歧义替换时给 `replacementDays`，但绝不输出环境原值；
- exit 0 = 此项无需迁移动作，exit 2 = cutover 前必须处理，exit 64 = 命令用法错误；
- `openbot_domain::text::trim_ecmascript` 成为上游 `trim()` 对齐路径的唯一空白表。

## 2. Oracle 与边界

本机 Node 20.19.6 对固定上游条件逐项实跑：

| 旧版结果 | 样本 |
| --- | --- |
| 接受 | `+7`、`0x10` / `0X10`、`0b101` / `0B101`、`0o10`、`1e3` / `1E+3`、`7.0`、`1.`、ECMAScript 空白包围的 `+7`、`4294967296` |
| 拒绝 | `0`、`-1`、`7.5`、`abc`、`Infinity`、`1_000`、`0b2` |

生产 Rust parser 与预检复用同一 `openbot_domain::audit::retention::parse_retention_days`；预检
只在“旧版接受、Rust 拒绝”时报告迁移差异。旧版本来也拒绝的坏值不伪装成版本差异。

空白语义按 ECMAScript `TrimString`，不是 Rust `char::is_whitespace`：U+FEFF 必须裁掉，
U+0085 必须保留。该规则同时服务 server/infra 环境解析、retention、email、地址与 HTTP
`parseInt` parity，避免第二次 Rust `trim()` 把已保留的 U+0085 又错误裁掉。

## 3. 运维接口

兼容配置：

```json
{
  "migrationCompatible": true,
  "findings": []
}
```

可规范替换的配置，例如旧版把十六进制写法解释为 16 天：

```json
{
  "migrationCompatible": false,
  "findings": [
    {
      "variable": "AUDIT_RETENTION_DAYS",
      "code": "canonical_decimal_required",
      "replacementDays": 16
    }
  ]
}
```

超过 Rust/数据库控制面 `u32` 范围时不提供替代值，必须由操作员选择新的保留策略，不能截断、
饱和或取模。报告不携带原始值；即使原值未来来自 secret-bearing 配置源，也不会被 stdout、
日志收集或 support bundle 顺手带走。

建议切换流程：

1. 在与旧部署相同的权威环境注入下运行 `openbot-migrate preflight-audit-retention`；
2. exit 2 时按 `replacementDays` 改为规范十进制，或对超范围项作人工策略裁决；
3. 重跑到 exit 0；
4. 再进入完整 migration/readiness 流程。

## 4. 本轮证据

- 固定上游源码回读：生产实现存在 `Number(raw)`，全部固定上游测试对
  `AUDIT_RETENTION_DAYS` 零命中；
- `cargo test -p openbot-domain audit::retention`：12 passed / 0 failed / 0 ignored；
- `cargo test -p openbot-domain identity::email`：10 / 0 / 0；
- `cargo test -p openbot-infra auth::config`：36 / 0 / 0；
- `cargo test -p openbot-server config`：61 / 0 / 0；
- `cargo test -p openbot-server --test migration_preflight_cli`：2 / 0 / 0；
- 真实二进制手动探针：`0x10` → exit 2 / replacement 16；`30` → exit 0。
- workspace 机械汇总：998 passed / 0 failed / 64 ignored；
- PostgreSQL 17.11/SCRAM + 本机 TLS/XMLDSig：456 / 0 / 0（infra 314 + server 142）；
- `cargo xtask ci`：7/7 全绿，固定上游 recount 148 / 148、失配 0、跳过 0；
- `cargo xtask parity-check`：0 违反；contracts WASM check、R44/R50 两个 dependency guard 均 exit 0；
- `cargo deny check` 四段 ok；`cargo audit` 扫描 425 个依赖且除 R44 单 ID 外为 0 告警；
  `cargo vet --locked` = 15 fully audited / 400 explicit exempted。

执行环境注记：受限沙箱内首次 workspace 总跑有 8 个 safe-dialer 单测在创建 loopback listener 时
统一得到 `Operation not permitted`；允许本机 loopback 后以**同一完整命令**重跑为 998/0/64，
没有把环境失败删掉或只挑 8 条补跑。

## 5. 明确不宣称

这个子命令只闭合 W-6 已知的审计留存环境差异。它不扫描数据库、policy 双引擎、旧共享 callback
token、deployment id、tenant package、IdP mapping 或 Intelligence export，也不证明 §24 G8 migration
rehearsal 通过。`openbot-migrate` 已有发行物落点，但完整 PostgreSQL/import migration binary 仍须随
后续迁移工作单逐项增加可复算子命令。
