# 外部任务 E：Windows Engine 交付与主控复核

日期：2026-09-05。文件名保留任务书指定路径。

**E 尚未满足合并验收条件。** 已取回候选的测试修复与阻断调查，但 3 项真实 Engine 用例失败。
主控补强的 Windows 探针尚待原生复跑，不能用 macOS 测试或 Windows 交叉编译代替。

## 来源与版本

- 远端：`acosmi/OpenBot`，分支 `feat/2026-09-04-P1-windows-runtime-audit`。
- 准备提交：`de8d0ecc7d71e88e41e9b3b6bf7884c9d89c53fd`；代码基线 R196：
  `87d84bb85d0056dfa4dcc2b35be4c2a610a55ae3`。
- 原候选：`6c4cb00863dd669219deb7c636cdd920079327fc`，准备提交后恰 1 个 commit、3 个文件。
- 主控集成基线：`ec7a6c425713e10d15e1f81f4325328858945476`（已含 R199 环境隔离）。
- 原报告 SHA-256：`ec35a72c479549ec2098ddd9942c9e7030aa51f6d83a2bac1390345813047a74`。
- 原 JSON SHA-256：`02336f8f7fa117211cd88fb7167e113e87756716ef85b32e87084ca3eb1d1f0a`。
- `grok-bot` tree 保持 `86f5a85f560f721677fa7e587a67ac0ffc036cb5`。

原报告可由 `git show 6c4cb00:docs/2026-09-04-Windows-Engine真机-外部交付.md` 复取。
本文件是复核后的口径；JSON 的 `commands` 是外部执行者在旧基线报告的结果，`review` 单独记录主控结论。

## 外部执行者报告的 Windows 结果

环境为 Windows 11 x64 原生、build 26200、Rust 1.98.0 / MSVC；不是 Wine/WSL。
这些结果来自候选报告，原始日志与 bundle 字节未随候选提交，因此主控不能声称已在异机独立重放。

| 项目 | 执行者报告 |
|---|---|
| Engine fetch / bundle / verify / protocol / shim | 均 exit 0，epoch/protocol 4，shim 595/600 |
| Windows boundary 默认 | 5 passed / 0 failed / 3 ignored |
| 原 ACL 写入探针 | 1 passed / 0 failed |
| 原 NUL 复现探针 | 1 passed / 0 failed；表示复现阻断，不表示 Engine 成功 |
| Computer 默认 | lib 66 passed；conformance 5 passed / 3 ignored |
| Computer 含 ignored 的真机运行 | exit 101，5 passed / **3 failed** |
| Windows Clippy / xtask Clippy / fmt / diff | exit 0 |
| 非 strict recount | 71 passed / 0 mismatch / **89 skipped** |

3 个失败分别是 Browser role、Component role和 viewer pause/resume；全部停于 `ConnectTimeout`。
帧、输入、ACK、renderer Job membership、真实 sandbox 状态、无监听及进程树退出均未由该次运行验证。

记录的 ZIP SHA-256 与仓内 pin 一致；ASAR/header 声明与 R190 记录一致，只是摘要字段对拍。
未取得 ZIP/PE/ASAR 原始字节时，不把它称为主控完成了 artifact 校验。

## 已确认的修复与复核修正

原候选只改 `windows.rs` 的测试模块：使用原生分隔符构造 `cmd.exe`；重定向使用工作目录相对路径，
避开 CRT 引号与 cmd 解析差异；删除 `&` 前会进入 echo 输出的空格。生产 `encode_command_line` 不改。

主控发现 NUL 负向测试只检查结果文件缺失，命令未执行也可通过，因此补上：

1. 同一命令在正常权限下必须成功，结果字节必须匹配。
2. 清除正向标记后，受限进程必须写出执行前标记，再以非零退出码结束，结果标记必须缺失。
3. ACL 测试也先证明正常进程可写两个目标，再验证受限进程只可写 profile。
4. 根目录必须新建成功，旧文件不能冒充本轮结果；等待有界，超时回收子进程。

主控实际执行：macOS boundary 4/0/0、Windows boundary+Computer all-target Clippy `-D warnings`、
`cargo fmt --all --check`、`git diff --check` 与 parity-check（0 违反）均通过。
上述新增对照通过 Windows 目标编译检查，**尚未在 Windows 执行**。
`windows.rs` 的 `#[cfg(test)]` 之前与集成基线逐字节相等；R199 封闭环境、token、Job、ACL均未更改。

原报告另有三处需要收窄：

- `named_pipe_peer_identity_binds_pid_and_exact_creation_time` 的客户端与服务端在同一进程；
  它验证 kernel peer 读取，不是实际 Engine spawn/握手，更没有两 role 的运行证明。
- 源码未启 breakaway、Job 先附着再 resume 是代码层观察；运行期约束仍须真实负对照。
- `--no-stdio-init` 后仍超时不能单独推出“stdin boot capability 没有到达”；需要原生观察才能定位。

## 启动阻断与下一步

[Electron 固定版本 NodeBindings::Initialize](https://github.com/electron/electron/blob/v43.3.0/shell/common/node_bindings.cc)
在 Windows 默认初始化路径调用 NUL 检查；
[IsNulDeviceEnabled](https://github.com/electron/electron/blob/v43.3.0/shell/common/platform_util_win.cc)
尝试以读写方式打开设备。
[Microsoft CreateRestrictedToken](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-createrestrictedtoken)
说明 WRITE_RESTRICTED 在写访问时应用 restricting SID 检查。

这些一手来源支持“受限 token 与 NUL 访问存在兼容问题”的调查方向；候选未提供可重放的设备 ACL、
有效 token 与 syscall 错误码材料，故不把具体 ACL 归因当作已独立证实。保留 restricted token、Job、
ACL 与原协议；不因诊断报告删除限制、修改全局设备 ACL 或启用未验证的启动 flag。

原机复跑应使用主控修正分支并带入 R199。先运行两条 paired probe，记录消毒后的成功/失败与 marker
判据；再修复真正的 Engine 启动问题，重跑全部含 ignored 的 conformance。确认两个 role、输入/帧/
停流、隔离负对照及退出清理均成立后，才能重新请求 E 的完整验收。

```powershell
cargo test -p openbot-windows-sandbox --locked -- --nocapture
cargo test -p openbot-windows-sandbox --locked restricted_write_process_ -- --ignored --nocapture --test-threads=1
cargo test -p openbot-computer --test engine_conformance --locked -- --include-ignored --nocapture --test-threads=1
```

本交付没有改中央 manifest/parity 状态，没有关闭 P1、G5、Alpha 或 Windows 发行范围。
Windows fidelity 仍为 Degraded。原有 Credential Manager ignored 测试也没有获得本次运行证明。
