# OpenBot P1 Windows Engine 边界 Batch54

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-P1-platform-spikes`

基线：P1 macOS Batch53 PR #36 已先以 merge commit 合入，分支起点 `13f8b82`。

## 1. 结论

本批把 v4 §19.1 P1 的 Windows 部分从“平台占位拒绝”推进为**可在 Windows 真机执行的探针代码与命令**，但没有 Windows 机器在环，因此不把交叉编译写成 spike 通过，也不勾 P1：

- Windows Engine host 已接入 current-user+Restricted-Code/low-label 双 Named Pipe、PID + exact 100 ns process creation FILETIME、medium-integrity write-restricted LUA token、suspended → Job → resume、profile/temp ACL、renderer Job membership与 `Degraded` fidelity；
- Windows bundle 已能将官方 Electron executable 改为明确 fixture 文件名，组装 shim ASAR、写九 fuse，并以 Win32 resource transaction 写入、data-file 模式重读 `Integrity` / `ElectronAsar` 官方 JSON；
- Windows target 的 boundary/computer/xtask check 与 Clippy 绿；Windows runtime tests、真实 bundle、两个 role conformance **未跑**；
- Ubuntu 24.04 x86_64 + runsc spike仍未实现/未运行，`tools/engine-pins.toml` 仍不得猜 runsc 版本；P2 继续禁止。

`grok-bot/` 没有参与实现，也没有任何改动；本批没有 npm、Node 构建链、Grok 产品能力、Docker Desktop或 Actions dispatch。

## 2. R127：为什么是第 11 个 crate

十个核心 crate 的 workspace lint 是 `unsafe_code = deny`，而 Rust std/Tokio 的安全 API不能完成 restricted-token process、exact Named Pipe peer PID、Job attribute、low-integrity ACL或 PE resource transaction。v4 §5.1 只允许四种新 crate 理由；这里同时命中“独立安全边界”和“明显不同的 feature graph”。因此：

- `openbot-windows-sandbox` 是唯一 `#![allow(unsafe_code)]` 的窄 crate；owner 固定为 `openbot-computer::engine`；
- raw handle/pointer 永不穿出 public API；每个 handle 都由 RAII 单一所有；失败路径 terminate/reap/discard；
- 原十个 crate、`ApplicationService` 业务入口与 parity `owner` 封闭域均不变；xtask 的 workspace 自检要求“十个 parity owner + 恰一个 R127 例外”。

安全说明与允许的 Win32 调用清单在 `crates/openbot-windows-sandbox/SECURITY.md`。

审计时拒绝了 `sandboxrs-windows 0.1.1`：其 AppContainer fallback 会递归尝试给 `SystemRoot` / `ProgramFiles` 加 ACL，删除 profile 时没有对应 ACL 回收；这会令一次 Engine 启动持久改变宿主权限。该 crate及其 `rappct` / `flatbuffers` 依赖没有进入 `Cargo.lock`。普通 spawn 后再 `AssignProcessToJobObject` 也因 child 可在间隙产生逃逸后代而拒绝。

## 3. Windows 边界

### 3.1 进程与 Job

1. authority先创建/规范化 profile 与 temp；两目录写 protected current-user + SYSTEM + Restricted Code DACL，并设置可继承 low mandatory label；
2. 从当前 primary token创建 `DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED` token，restricting SID固定为 Windows Restricted Code并强制验证 integrity仍为medium；token default DACL加入 Restricted Code，避免子进程新建对象后无法继续写自己创建的对象；main保持medium也让renderer low-integrity证据不会被外层token伪造；
3. Job在 spawn 前配置 `KILL_ON_JOB_CLOSE | ACTIVE_PROCESS | JOB_MEMORY`，P1 ceiling 固定 32 processes / 4 GiB；不设置任何 breakaway flag；
4. `STARTUPINFOEX` handle list恰含 child stdin和两个 `NUL` handle；Job list在 child suspended 创建时附着，随后才 `ResumeThread`；
5. child/drop只以 Job终止整棵树，不退化成只杀 root；环境剥离 `ELECTRON_RUN_AS_NODE` / `NODE_OPTIONS` / `NODE_EXTRA_CA_CERTS` / `ELECTRON_ENABLE_LOGGING`，TEMP/TMP指向受控目录；
6. Chromium仍只收到固定 loopback black-hole proxy flags；Windows fidelity明确是 `Degraded`，这些机制不冒充网络/可执行路径 allowlist，也不抵抗同 UID 恶意进程。

32 / 4 GiB 是 R118 component最多 8 个 256 MiB renderer（2 GiB）外加 main/GPU/network/crashpad与 P1 browser headroom后的 process-tree ceiling，不替代 P2/P3 每 session预算；policy未来只能收紧。

### 3.2 boot identity

两条 pipe名由 128-bit boot token派生，`PIPE_REJECT_REMOTE_CLIENTS`、instance=1、non-inheritable；DACL的正常访问检查只放当前 user，restricting检查另要求 Restricted Code SID，并带 low label。每条连接独立执行：

1. `GetNamedPipeClientProcessId` 必须等于 Rust spawn identity PID；
2. 重新 `OpenProcess` 后 `GetProcessTimes` 的 100 ns FILETIME必须与 spawn handle捕获值逐位相等；
3. Electron ready的 `ProcessMetric.creationTime` 必须等于该 FILETIME转换出的 Unix epoch milliseconds；
4. renderer的 PID/creationTime 还必须由 `IsProcessInJob` 证明属于同一 Job；任一不符都拒绝。

Electron文档说明 creationTime是 epoch milliseconds且应与 PID一起防复用；Microsoft文档说明 `GetProcessTimes` creation FILETIME是从 1601 起的 100 ns单元。Electron源码实际调用 Chromium `Time::FromFileTime(...).InMillisecondsFSinceUnixEpoch()`；Chromium先把 100 ns ticks截到微秒再输出浮点毫秒，本实现逐步采用同一算法，不整毫秒截断，也不通过 wall clock估算。

### 3.3 Windows ASAR Integrity

payload恰为：

```json
[{"file":"resources\\app.asar","alg":"sha256","value":"<lowercase header sha256>"}]
```

它写入 string type=`Integrity`、name=`ElectronAsar`、language-neutral的 PE resource；失败时 `EndUpdateResource(..., discard=true)`，成功后用 `LoadLibraryEx(...AS_DATAFILE_EXCLUSIVE | AS_IMAGE_RESOURCE)` + `FindResource`重读并逐字节比较。Electron 43 的 `archive_win.cc` 正是以这两个 string identifier读取并按 file/alg/value解析。ASAR内部每文件 whole hash + 4 MiB blocks、header hash、fuses与 sidecar digest仍沿用 Batch53同一实现。

本批没有声称完成最终 Windows Authenticode或 G8品牌 VERSIONINFO/icon；fixture只改 executable filename。发布签名/最终品牌仍是后续独立证据，不能由 PE integrity resource替代。

## 4. 本轮实跑证据

以下都由本轮当前源码亲自运行：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p openbot-windows-sandbox -p openbot-computer --all-features --locked` | boundary pure `2/0/0`；computer `18/0/0`；host tests默认 `0/0/2 ignored` |
| `cargo test -p openbot-testkit --features xtask --bin xtask --locked` | `91/0/0` |
| `cargo check -p openbot-computer -p openbot-windows-sandbox --all-targets --target x86_64-pc-windows-msvc --locked` | exit 0 |
| `cargo check -p openbot-testkit --features xtask --bin xtask --target x86_64-pc-windows-msvc --locked` | exit 0 |
| Windows-target Clippy（boundary+computer all-target；xtask bin）`-D warnings` | 两条命令均 exit 0 |
| `cargo clippy -p openbot-windows-sandbox -p openbot-computer -p openbot-testkit --all-targets --all-features --locked -- -D warnings` | macOS host exit 0 |
| `cargo xtask engine bundle` | macOS bundle绿；ASAR 17,196 B，header `c01ee82f…a932`，fuse sentinel 1 |
| `cargo xtask engine verify`（宿主权限；默认文件沙箱会令 GUI binary SIGABRT） | raw zip/`v43.3.0`/bundle verify全绿 |
| `cargo test -p openbot-computer --test engine_conformance --locked -- --ignored --nocapture --test-threads=1`（宿主权限） | macOS Browser + Component `2/0/0`，证明跨平台重构未破坏 Batch53 |
| `cargo xtask parity-check` | parity `693/999/1692`；overlay carry/revalidate/split/superseded=`1674/16/2/0`；0 violation |
| `cargo xtask recount` | repo `71/0`，upstream因本轮未设置 `OPENBOT_UPSTREAM_DIR` 如实跳过 88；总 159 |
| `cargo xtask electron-shim-check` / `engine protocol --check` / `grok-inventory --check` | 全绿；shim 404 LOC；Grok inventory 2,110 文件且同步 |
| `git rev-parse HEAD:grok-bot` / `git diff --name-only -- grok-bot` | `86f5a85f…` / 空；Grok tree零改动 |
| unsafe/package/rejected dependency复核 | 含 `unsafe {` 的仓内 Rust文件仅 `openbot-windows-sandbox/src/windows.rs`；非Grok `package.json`恰1；`sandboxrs-windows`/`rappct`/`flatbuffers`不在lock |
| `bash tools/check-deny-release-targets.sh` | bans + 六发行target bans/sources全绿 |

交叉编译只证明类型/feature graph，不证明 Win32 API在真机上的运行语义。

## 5. Windows 真机必须原样补跑

在 clean Windows x64 checkout（不运行完整 workspace；避免已登记的 `openssl-sys` / `samael` 阻塞）执行：

```powershell
cargo test -p openbot-windows-sandbox --locked -- --nocapture
cargo test -p openbot-windows-sandbox --locked restricted_write_process_writes_profile_but_not_medium_outside -- --ignored --nocapture
cargo xtask engine fetch
cargo xtask engine bundle
cargo xtask engine verify
cargo test -p openbot-computer --test engine_conformance --locked -- --ignored --nocapture --test-threads=1
cargo clippy -p openbot-windows-sandbox -p openbot-computer --all-targets --locked -- -D warnings
cargo clippy -p openbot-testkit --features xtask --bin xtask --locked -- -D warnings
cargo xtask parity-check
```

验收必须同时看到：

- boundary default Pipe exact identity与 PE resource round-trip绿；ACL negative先在 profile写成功、再在 medium outside写失败；
- `engine bundle|verify` 真实生成 `windows-x64` bundle并读取同一 PE resource；
- Browser/Component各一次 start→1280×800 JPEG→stop→shutdown；fidelity=`Degraded`、renderer `sandboxed=true`；
- main/renderer creationTime exact绑定、renderer属于 Job；全部 PID TCP LISTEN=0；退出后全部 PID/profile lock=0。

任何失败都只修代码/Win32配置后重跑；不得删 restricted token、Job、renderer sandbox，不能加 `--no-sandbox`，也不能把失败记为平台不适用。

## 6. 明确未做

- 未在 Windows运行任何二进制；Windows P1证据仍为红。
- 未下载、钉版或运行 runsc；没有 Ubuntu 24.04 x86_64的 layer-1、`Seccomp:2`、`NoNewPrivs:1` 证据。
- 未运行 `cargo xtask ci`、完整 workspace tests或 GitHub Actions（R63 manual-only）。
- 未进入 P2；Engine T-ID继续 todo。
- 未修改 `grok-bot/`，未做 census人工分类，未复制/翻译任何文本。

官方依据：[Electron ASAR Integrity](https://www.electronjs.org/docs/latest/tutorial/asar-integrity)、[Electron Windows resource reader](https://github.com/electron/electron/blob/main/shell/common/asar/archive_win.cc)、[Microsoft restricted tokens](https://learn.microsoft.com/en-us/windows/win32/secauthz/restricted-tokens)、[GetProcessTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)。
