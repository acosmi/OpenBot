# OpenBot P1 Engine macOS 基线 Batch53

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-28-P1-engine-minimal`

基线：P0 PR #35 已先以 merge commit 合入，分支起点 `0f0a5c4`。

## 1. 结论与边界

本批完成 v4 §19.1 P1 在 **macOS arm64** 可独立复核的 Engine 最小闭环，但不勾 P1 整阶段：Windows Named Pipe/Job/restricted-token spike 与 Ubuntu 24.04 x86_64 + runsc Chromium sandbox spike 尚无真机证据。两者完成前不进入 P2。

没有使用 `grok-bot/` 源码或文本。shim 只依据 Electron 官方公开 API、[ASAR 格式](https://github.com/electron/asar#format)、[fuse wire](https://www.electronjs.org/docs/latest/tutorial/fuses)、[ASAR Integrity](https://www.electronjs.org/docs/latest/tutorial/asar-integrity) 与 [ProcessMetric](https://www.electronjs.org/docs/latest/api/structures/process-metric) clean-room 实现。

## 2. 本批产物

### 2.1 协议与 shim

- `crates/openbot-contracts/engine-protocol-v1.json` 是 language-neutral 协议描述；Rust 常量单测逐字段 join。
- `cargo xtask engine protocol` 机械生成 `engine-shim/generated/protocol.mjs` 与 Rust-owned SHA-256；`--check` 逐字核对。
- shim allowlist 仍恰为 `package.json` / `main.mjs` / `generated/protocol.mjs`，当前 **404 非空 LOC / 600**。
- `package.json` 只有五个允许键，零 dependencies/devDependencies/scripts；仓内无第二个工作区 package manifest、无 Node/npm lockfile。
- control 只有 `start / stop / shutdown`；role、scope digest、computer/generation、operation ID 与一次性 token 全由 Rust 铸造。无 actor/policy/intent/free URL/free CDP/free dispatcher。

### 2.2 Rust Engine host

- `EngineRole::{BrowserComputer, SandboxedComponent}` 与完整 scope typed model；只有闭合 role tag + opaque SHA-256 scope digest 进入 shim。
- 4 KiB 单行 stdin boot capability；control/frame 两个随机 `0600` UDS；16-byte OS CSPRNG token。
- 两条 UDS 都要求 peer PID = live spawned child；持有 `Child` 并复核仍存活，构造性排除 PID reuse 窗口。
- hello / frame hello / ready / command / graceful shutdown 分段 deadline；连接后沉默不能无限挂起。
- 独立 binary JPEG framing绑定 protocol/role/computer/generation/tab/sequence/dimensions；8 MiB 上限；magic/format/stale generation/wrong scope/replay/坏 JPEG 全拒绝。
- release manifest 的 SHA-256 由调用方从签名元数据提供；manifest 与 executable/fuse/app.asar 三摘要在 spawn 前验证，路径禁止 absolute/parent traversal。

### 2.3 macOS confinement

- Electron main 由 `/usr/bin/sandbox-exec -f <Rust 生成 profile>` 启动；写入仅 profile/temp/runtime + `/dev/null`，出站仅 UDS/loopback，exec 默认拒绝。
- main executable 以精确 literal 继承父 Seatbelt profile；只有四个 Electron Helper 与 crashpad 五个精确 executable literal 允许 `(with no-sandbox)`，再由 `app.enableSandbox()` + `sandbox=true` 施加 Chromium 自身 renderer sandbox。Squirrel/ShipIt 与其它 bundle executable 不在 allowlist。直接继承双层 Seatbelt 会令 renderer abort，本批以 allow-default 正向对照和逐层恢复规则实测定位，最终收窄 profile 下双 role 均绿。
- renderer `app.getAppMetrics()` 必须命中同 PID、`sandboxed=true`、creationTime 非零；Rust逐字段拒绝缺失/false/非有限值。main PID另绑定 UDS peer与 creationTime，不把 Electron 对外层 Seatbelt 不投影为 main `sandboxed` 的行为误写成失败。
- helper `with no-sandbox` 只解除父 profile继承，不等于 Chromium `--no-sandbox`；bundle/argv/static gate 仍禁止该 flag，renderer ProcessMetric 正向证明自身 OS sandbox。

### 2.4 Rust-only bundle

- `cargo xtask engine bundle` 从官方校验后 zip 复制当前平台树，零 npm。
- ASAR 按官方 Chromium Pickle 结构生成；每文件 whole SHA-256 + 4 MiB block hashes；plist 使用 JSON header SHA-256。
- Electron 43 v1 九 fuse 严格写为 `000011001`：RunAsNode/NodeOptions/inspect 关，embedded integrity/OnlyLoadAppFromAsar 开，未裁决的 cookie encryption 保持关闭，WasmTrapHandlers 保持开启；wire 长度变化直接判红。
- macOS 外层 app、main executable 与 bundle ID 使用明确的 P1 fixture identity；最终产品品牌尚未清查，helper/framework 内部名按 Electron 官方“可选 rebrand”规则保留到 G8，不冒充最终品牌。
- `default_app.asar` 与 unpacked `app/` 删除；`ElectronAsarIntegrity` 写入；修改后 ad-hoc deep sign 并 `codesign --verify --deep --strict`。
- sidecar manifest固定 platform/arch/Electron/release epoch/protocol/product/bundle/fuse wire 与三份摘要；runtime digest-before-spawn。

## 3. 真进程 conformance

`cargo test -p openbot-computer --test engine_conformance --locked -- --ignored --nocapture --test-threads=1` 本轮实跑：

- Browser role：start → 1280×800 JPEG frame → stop → shutdown，绿；
- Component role：独立进程/临时 partition，同一闭环，绿；
- 两 role 均验证 main PID/creationTime、renderer PID/creationTime/OS sandbox、Node/require 不可见、内部固定 origin；
- main 与全部后代逐 PID `lsof`：TCP LISTEN = 0；
- shutdown 后 main 与所有已观测后代 PID = 0，`SingletonLock/Socket/Cookie` = 0。

本轮实测暴露并修复两个只有真进程才会出现的根因：

1. ES module top-level `await app.whenReady()` 会阻止 bootstrap 完成，表现为 hello 成功但 ready 永久沉默；改为异步 bootstrap 后模块先完成求值。
2. Rust `UnixStream::into_split()` 后丢弃 frame 写半端会向 Node 发 FIN；Node 默认 `allowHalfOpen=false`，自动关闭自身写端，首帧永远等不到 drain。`EngineProcess` 现在持有 frame 写半端到整个生命周期结束。

所有临时 stderr 阶段标记均已删除；最终生产默认 stdout/stderr = null，stable error 只回封闭 control code。

## 4. 本轮机械证据

| 命令 | 结果 |
| --- | --- |
| `cargo xtask engine protocol --check` | version 1；generated module/hash 逐字一致 |
| `cargo xtask electron-shim-check` | 3 files；404/600 LOC；唯一 package.json；API/import/protocol hash 绿 |
| `cargo xtask engine bundle` | macOS ASAR/fuses/rebrand/integrity/signature/manifest 绿 |
| `cargo xtask engine verify` | raw v43.3.0 + macOS bundle verify 绿 |
| `cargo test -p openbot-contracts --locked` | 88/0/0 |
| `cargo test -p openbot-computer --locked` | 18/0/0；2 条 host conformance 默认显式 ignored |
| host conformance（上节命令） | 2/0/0 |
| xtask bin tests | 90/0/0 |
| computer + xtask 定向 Clippy `-D warnings` | 绿 |
| `cargo fmt --all -- --check` | 绿 |
| `cargo xtask parity-check` | 0 违反；parity 693/999/1692；overlay 1674/16/2/0 |

## 5. 明确未做

- 未派发 GitHub Actions、未运行 `cargo xtask ci`（R63 manual-only）。
- 未运行完整 workspace test；本批按 contracts/computer/xtask/engine 变更面定向实跑。
- 未实现 Windows Named Pipe peer PID+creationTime、Job Object/restricted token/profile ACL 与 PE `ElectronAsar` Integrity resource。
- 未下载/钉版/运行 runsc；没有 Ubuntu 24.04 x86_64 的 `Seccomp:2` / `NoNewPrivs:1` / layer-1 证据。
- 未进入 P2，不提供 navigate/snapshot/click 等真实 CDP 产品映射；当前页面只用于内部 P1 conformance。
- 未把 P0 新增的 Engine T-ID 改 done；跨平台 P1/G5A/G5E/G5F 证据未齐，历史状态保持 todo。
- `grok-bot/` 零改动，tree 仍为 `86f5a85f560f721677fa7e587a67ac0ffc036cb5`。

下一批仍是 P1 平台 spike；两项真机证据补齐前，阶段门不允许进入 P2。
