# 外部任务 F：Ubuntu runsc/Xvfb 真机验收

先读同目录 `2026-09-04-v4第二轮外派任务-总则.md` 和列出的第一真源，全部共同约束适用。
固定基线 `87d84bb85d0056dfa4dcc2b35be4c2a610a55ae3`；分支 `feat/2026-09-04-P1-ubuntu-runsc-runtime-audit`。
执行环境必须是 Ubuntu 24.04 x86_64，可运行真实 runsc OCI；macOS/Linux target check 不算运行。

## 任务与范围

按 v4 R121、R128–R129 及 `2026-08-29-P1-runsc-harness-batch55.md`，运行既有 `engine runsc-spike`。
目标是验证真实 Engine 两 role 与 Chromium 双层 sandbox，并提交经完整 PASS 才成立的 pin 候选。
不得先把任意 latest 版本写入生产 pin；使用完整官方 point-release tarball、同 release SHA256SUMS 与 sidecars。

允许修改：

- `crates/openbot-testkit/src/xtask/engine_runsc.rs`。
- `crates/openbot-computer/examples/engine-runsc-probe.rs`。
- `crates/openbot-computer/src/engine/process.rs` 的 Linux 专属实现。
- 新增 `tools/qa/prepare-runsc-audit-rootfs.sh`：只准备任务自有目录，拒绝 host `/`，不改宿主全局服务/网络，不安装 Docker Desktop。
- 新增 `docs/2026-09-04-Ubuntu-runsc真机-外部交付.md` 与 `fixtures/computer/runsc-runtime-audit/` 下无 secret JSON 报告。

共享协议、shim、Engine pin、Cargo、Windows 分支与主控出口网关不在范围内。

## 验收

1. 核实 host/rootfs 的 Ubuntu 24.04 x86_64 身份、独立 rootfs 来源/摘要、官方 runsc 整包版本/摘要/sidecars、Xvfb dpkg 版本与 binary SHA；下载失败和未经实际验证的候选单列。
2. 普通用户构建当前 epoch 4 `engine fetch|bundle|verify` 与 `engine-runsc-probe` release binary；隔离 rootfs 只安装必要运行库和 Xvfb，不把 host 根或用户 home 当输入。
3. 按 Batch55 参数执行真实 OCI spike；root 仅用于这项已明确需要的隔离运行。保留 `network=none`、host UDS/FIFO none、只读 root/bind、零 capability、sidecar ALWAYS/STRICT、gVisor marker、pids/memory/deadline。
4. Xvfb 固定容器内 `:99`、1280×800×24、`-nolisten tcp`；不得通过 VNC、host display 或 `--headless` 猜测替代。
5. 两 role 分别证明 main `Seccomp != 2` 负对照、renderer `Seccomp:2` 与 `NoNewPrivs:1`；PID+network namespace 或 user namespace 的 layer-1 正向、renderer tree ownership、真实帧、无 listener/orphan/profile lock。
6. 失败只能修 runsc 版本/配置或明确平台缺陷后重跑，不能关 Chromium sandbox、删除 renderer 检查或把 main 统一 Seccomp=2 当正向。
7. 只有全部 PASS 才在交付文档提供精确 pin/第一真源增补候选；主控复核后统一落入 `tools/engine-pins.toml` 和 R 行。工具打印 pin candidate 不等于已完成生产 pin。
8. 跑 Linux 定向测试、相关 Clippy、fmt、diff-check，列出每条实际命令/退出码及机器证据的 SHA。普通 host Linux 运行不能替代 runsc 内结果。

本任务不接真实 browser egress，不关闭完整 G5/G7/G8；也不把 probe 的内部 display 声称为产品 Screen。
没有合适主机或运行权限时明确记录阻塞，继续只做可验证的准备，不派发 Actions。
