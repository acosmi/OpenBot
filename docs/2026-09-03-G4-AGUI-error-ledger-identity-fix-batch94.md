# G4 AG-UI Error Ledger Identity Fix（Batch94）

> 日期：2026-09-03（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §7.5、§19.3、§24、§28.1
> 基线：Batch93 docs，`c273ca3f9b208edc3a5f6e92f77dc9073e2ff1d4`
> correction：`2050fab0369bbc1537f94347eb1f74b75ffb5820`

## 1. 发现的问题

Batch92 的代码、真实 PostgreSQL/SafeDialer/SSE 证据、v4 R166、CLAUDE 与批次文档都明确关闭 `T-EVT-0011 agui-error`。但对原提交做逐文件复核：

```text
git show a46438f2d2f338d3e9473d72f639f8b674b6fcc3 -- parity/events.yaml
```

可见该提交实际把 `T-EVT-0003 agui-tool-call-result` 从 todo 改为 done，并把整段 RUN_ERROR/malformed 证据挂在它下面；真正的 `T-EVT-0011` 未修改，仍是 todo。

这类错误不会被单纯总数捕获：Batch92 的 events done 从35变36，错位后仍是36；Batch93再关闭 reasoning 后仍会得到37。`parity-check` 能校验字段、计数、T-ID唯一性和done_evidence存在性，但不能从自然语言推断一段 error 证据是否属于 tool-result T-ID。

## 2. 修复

- `T-EVT-0003 agui-tool-call-result` 恢复 `status: todo`，移除不相干 error `done_evidence`；
- `T-EVT-0011 agui-error` 改为 `status: done`，挂回原 Batch92 error production evidence；
- R166 与 Batch92 实现结论保留，并显式登记本次 R168 纠正；
- 不改变任何 Rust代码、schema、fixture、API、Cargo依赖或产品行为。

`T-EVT-0003` 只有在 remote tool result 的独立投影与 §8.1 本地唯一执行管线具备 production 证据后才能重新标 done，不能借 error 测试代替。

## 3. 复算

- 定点解析：T-EVT-0003=`todo`且无`done_evidence`；T-EVT-0011=`done`且证据包含 RUN_ERROR/malformed production vertical。
- `cargo xtask parity-check`：parity=`825/879/1704`、events=`37/51/88`、fixtures=`21/22/43`、overlay=`1293/403/2/6`、0 violation。
- done 总数刻意不变；本批修的是 T-ID 身份，不把“计数没变”写成“没有问题”。
- 本批无代码变更，未重跑与本次身份移动无关的 PG/Clippy；Batch92/93 的原测试证据保持。
- strict recount仍因未配置固定上游目录未跑；按R63未运行`cargo xtask ci`，未派发Actions。

## 4. 全量状态

本批不是 v4 完成。当前仍为 parity `825 done / 879 todo / 1704`；`T-EVT-0003`及其它AG-UI、computer runtime budget、三家trace、Browser/file/shell、Desktop OAuth、MCP private egress/admin、外审/KMS、发行与golden等继续未完成。
