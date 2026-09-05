# 外部任务 E：Windows 真机 Engine 验收

先读同目录 `2026-09-04-v4第二轮外派任务-总则.md` 和列出的第一真源，全部共同约束适用。
固定基线 `87d84bb85d0056dfa4dcc2b35be4c2a610a55ae3`；分支 `feat/2026-09-04-P1-windows-runtime-audit`。
远程准备提交与实现候选的计数按本分支总则“E任务远程交接补充”；先读 `2026-09-04-E任务Windows远程交接.md`。
本任务必须在 Windows x64 原生运行；macOS cross-check、Wine、WSL 内 Linux 运行不能替代。

## 任务与范围

已有 Windows 桌面控制和 OpenBot Win32 Engine 边界代码，缺的是 OpenBot 的真实运行证据。
依据 v4 R127、R184–R190 和 `2026-08-29-P1-windows-boundary-batch54.md`，对当前 epoch 4 Engine
执行真实 bundle、Browser/Component 两 role、受限 token/Job/ACL/pipe、renderer sandbox 与退出清理验收。
旧 Batch54 的 epoch、shim LOC、测试数仅为历史；按当前生成器重建，不拿旧 bundle 运行新协议。

允许修改：

- `crates/openbot-windows-sandbox/src/windows.rs`、`command_line.rs`、`lib.rs`：只修本次真机复现的 Win32 缺陷及必要回归。
- `crates/openbot-windows-sandbox/SECURITY.md`：只同步已修复边界。
- `crates/openbot-computer/src/engine/process.rs` 的 Windows 专属分支。
- `crates/openbot-computer/tests/engine_conformance.rs` 的 Windows 探针；不得改变跨平台断言、放宽阈值或跳过失败。
- `crates/openbot-testkit/src/xtask/engine_bundle.rs` 的 Windows PE/bundle 专属修复。
- 新增 `docs/2026-09-04-Windows-Engine真机-外部交付.md`；必要的无 secret JSON 报告放 `fixtures/computer/windows-runtime-audit/`，不改中央 manifest。

超出上述范围的缺陷交最小复现，不自行改协议/shim、网络策略、公共进程控制或整个 unsafe 边界。

## 验收

1. 记录 Windows 版本/build、架构、Rust 版本、固定 HEAD、Engine 包/ASAR/manifest 摘要；不记录用户名、机器名、SID 字面量。
2. 按当前代码执行 `engine fetch`、`engine bundle`、`engine verify`、`engine protocol --check`、`electron-shim-check`。
3. boundary 默认测试与 `restricted_write_process_writes_profile_but_not_medium_outside -- --ignored --nocapture`：同一 child 在受控 profile 写成功、medium outside 写失败；双 pipe 的 PID/creation-time 绑定和 PE resource 读回成立。
4. `cargo test -p openbot-computer --test engine_conformance --locked -- --include-ignored --nocapture --test-threads=1`：执行适用于 Windows 的全部当前用例，逐条列出运行与跳过原因。两个 role 的真实帧、ordinary input、latest/ACK、viewer 生命周期均记录实际结果。
5. Job 在 suspended 创建阶段附着、无 breakaway、renderer 属于相同 Job；真实 renderer sandbox、main/renderer 身份、TCP listener、关闭后全进程树/profile lock 清理要有正反证据。
6. 如实保留 Windows `Degraded` fidelity；不能由本批声称 kernel 网络 allowlist、原生 Desktop 产品旅程、签名发行或 Windows golden 已通过。
7. 跑 boundary/computer 本机定向测试、all-target Clippy `-D warnings`、xtask 相关 Clippy、fmt、diff-check。原始退出码和测试数入报告，不用“全部通过”代替。

禁止 `--no-sandbox`、去掉 restricted token/Job/ACL、放宽 peer PID、只杀 root 或将失败改成 ignore。
无 Windows 主机时只交环境阻塞与准备结果，本任务不得记真机通过，也不派发 Actions。
