# OpenBot P1 runsc Chromium 沙箱证据闭包 Batch55

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-P1-runsc-spike`

基线：Windows probe Batch54 PR #37 已先以 merge commit `f084afb` 合入。

## 1. 结论

本批完成的不是 runsc spike“通过”，而是把 v4 R121 的口头判据变成一条会说“不”的端到端命令：

```text
verified full gVisor release tarball
  → root Ubuntu 24.04 x86_64 OCI (network/host IPC none)
  → pinned Xvfb local display (:99, no TCP/VNC)
  → Rust probe as container init
  → real Electron Browser + Component roles
  → renderer /proc layer-1 + layer-2 evidence
  → frame / listener / orphan / profile-lock closure
  → only then print pin candidate
```

当前机器是 macOS arm64且无runsc/Ubuntu host；本轮只取得 Linux target check/Clippy与macOS回归，**没有运行runsc、没有 `Seccomp`/`NoNewPrivs`/namespace结果、没有版本pin**。P1继续红，P2仍禁止。

## 2. R128 裁决

### 2.1 为什么不先选一个版本

R121明写版本必须由P1 spike实测后钉。先取“latest”日期写进`engine-pins.toml`会把未运行版本伪装成release真源。Batch55因此只接受调用者提供的：

- 完整 `gvisor.tar.bz2` 绝对路径；
- 来自同一官方point release `SHA256SUMS`的64位摘要；
- exact `release-YYYYMMDD.N`；
- prepared Ubuntu24.04 x86_64 rootfs。

命令先用Rust重算archive SHA，再安全检查tar路径、解包并要求top-level `runsc`与恰一个非空`gvisor-bin/`或`gvisor/` sidecar目录，随后要求`runsc --version`逐字命中。只有整个Electron probe PASS才打印候选；工具本身**不编辑pins**，避免“打印到一半/测试后来失败”留下半真源。

gVisor官方安装文档已说明2026-07后release从单binary迁移为含sidecars的tarball，并提醒sidecars必须跟随runsc。只hash单个runsc或允许缺文件时自动补下载都不满足可复现发行。

### 2.2 为什么不用 `runsc do`

gVisor官方源码说明`runsc do`是便捷实验面，默认给sandbox只读访问整个host filesystem；官方rootless文档又说明`--rootless`主要适用于`do`、不支持常规`create`。P1生产判据是Server OCI，因此harness只接受root Ubuntu24.04 x86_64并执行真实OCI `run`，不把rootless/do结果外推。

### 2.3 R129：为什么必须有Xvfb、又为什么它不是VNC

Electron官方长期结论是Linux `BrowserWindow`即使`show:false`仍需图形环境；“真正headless Electron”到2025仍是未实现feature request。当前rootfs没有DISPLAY会在进入Chromium sandbox判据前必然失败，不能靠未经Electron承诺的`--headless`/Ozone flag猜通。

R129因此只允许**runsc容器内部**的Xvfb作本地display backend：probe先校`/usr/bin/Xvfb` SHA-256等于外层authority从readonly rootfs算出的值，再以固定`:99 -screen 0 1280x800x24 -nolisten tcp -noreset -ac`启动；Engine launch只接受typed `with_runsc_probe_display()`并固定设置`DISPLAY=:99`，不接受自由DISPLAY。Xvfb不安装/启动x11vnc、不对外传帧、不替代ScreenHub；其PID不属于Engine树但由probe拥有，必须TCP LISTEN=0，最终kill/wait后容器额外PID=0。

`-ac`意味着同一runsc scope内的进程可连本地X socket，Xvfb本身不作为renderer隔离边界；若renderer进一步攻破Xvfb/main，仍必须由outer runsc阻止扩大scope。未来若引入Xauthority只能收紧，不得拿它替代runsc。Xvfb的rootfs dpkg amd64 package version与binary hash随PASS一并打印，进入Server image provenance后才是pin。

## 3. 外层 OCI authority

`cargo xtask engine runsc-spike`固定：

- host与rootfs都必须是Ubuntu 24.04 x86_64；host EUID必须0；
- `--platform=systrap`、`--network=none`、`--host-uds=none`、`--host-fifo=none`；
- `--sidecar-release-enforcement-policy=ALWAYS` + `--sidecar-usage-policy=STRICT`，sidecar缺失、embedded fallback或release不一致均红；
- `--gvisor-marker-file=true`，容器内`RunscAttestation`必须看到`/proc/gvisor/kernel_is_gvisor`；
- rootfs必须含dpkg登记的amd64 `xvfb`与`/usr/bin/Xvfb`；外层记录package version/binary SHA并传入容器复核；
- OCI root只读；probe与engine仅以只读bind进入；`/tmp`、`/dev`、`/dev/shm`为有界tmpfs；
- capability五集合全空、`noNewPrivileges=true`、pids=64、memory=6GiB、NOFILE=4096；
- 90秒deadline；超时先`runsc kill ... KILL`再杀runtime进程；stdout/stderr落有界批次目录，避免pipe回压死锁；
- rootfs须预建`/opt/openbot/{bin,engine}`等mount destination，但harness不修改输入rootfs。

网络namespace即使存在也不能替代`--network=none`；反之只写network none也不能替代renderer自身Chromium sandbox。

## 4. 容器内真实 Engine probe

`engine-runsc-probe`是`openbot-computer`的`runsc-spike` feature所保护的Cargo example，结构上不属于默认/product bins。它作为OCI init并复用生产P1代码路径：

1. `RunscAttestation::detect`验证gVisor marker、Ubuntu24.04、x86_64；普通Linux host不能调用direct spawn；probe校hash后启动固定Xvfb并等Unix socket；
2. `EngineBundle::open`先校sidecar manifest与全部named file digest；
3. Browser与Component各创建独立profile/UDS/boot token，完成hello→ready→start→1280×800 JPEG→stop→shutdown；
4. Linux上的Electron `ProcessMetric`没有`sandboxed`字段。shim明确发送false，Rust要求它保持false，防止把renderer自报当证据；
5. probe直接读renderer `/proc/<pid>/status`，要求main `Seccomp != 2`作为负对照、renderer `Seccomp:2`、`NoNewPrivs:1`；
6. layer-1定义为renderer相对main同时进入不同PID+network namespace，或进入不同user namespace；两条都不成立即红；
7. renderer必须属于authenticated main process tree；全树 `/proc/<pid>/net/{tcp,tcp6}` LISTEN=0；shutdown后容器内除PID1 probe外的数字PID总数=0、三个profile lock=0（不只检查事前已观测集合）。

main `Seccomp!=2`负对照很重要：若OCI或gVisor把所有process统一显示成2，单看renderer会得到恒真探针，本批会明确拒绝这种结果。

## 5. Ubuntu真机命令

先以普通用户在clean checkout构建Linux Electron与probe（Electron官方zip仍由既有pin校验，零npm）：

```bash
cargo xtask engine fetch
cargo xtask engine bundle
cargo xtask engine verify
cargo build -p openbot-computer --example engine-runsc-probe --features runsc-spike --release --locked
```

prepared rootfs必须是独立Ubuntu24.04 x86_64树，已安装Electron运行所需共享库与dpkg登记的amd64 `xvfb`，`/usr/bin/Xvfb`存在，且以下目录存在：`proc`、`dev/pts`、`dev/shm`、`tmp`、`sys`、`opt/openbot/bin`、`opt/openbot/engine`。不得把host `/`当rootfs；即使readonly也会把host secrets暴露给不可信renderer。

从[gVisor official point release](https://github.com/google/gvisor/releases)下载同一release的`gvisor-x86_64.tar.bz2`（或该release实际命名的完整tarball）与`SHA256SUMS`，人工确认摘要来自同一tag后执行：

```bash
sudo -E env PATH="$PATH" cargo xtask engine runsc-spike \
  --archive /absolute/path/gvisor.tar.bz2 \
  --sha256 <SHA256SUMS中的64位值> \
  --version release-YYYYMMDD.N \
  --rootfs /absolute/path/ubuntu-24.04-rootfs
```

PASS输出的runsc version/archive hash与Xvfb package version/binary hash都只是候选。随后必须在**同一PR**把point-release URL、全部hash、rootfs/image provenance、完整命令与两role JSON报告写入§1.2、`tools/engine-pins.toml`、R行和批次文档，再从clean checkout重跑；不能只复制PASS字符串。

## 6. 本轮实跑证据

| 命令 | 结果 |
| --- | --- |
| `cargo test -p openbot-testkit --features xtask --bin xtask --locked` | `93/0/0`；含OCI闭集与报告负向单测 |
| Linux target probe Clippy `-D warnings` | exit 0 |
| Linux target xtask Clippy `-D warnings` | exit 0 |
| Windows target boundary/computer all-target/all-feature Clippy `-D warnings` | exit 0；Linux重构未破坏Batch54编译面 |
| `cargo xtask electron-shim-check` | 3文件、405/600 LOC、唯一package、protocol hash match |
| `cargo xtask engine bundle` | macOS app.asar 17,306 B；header `fb3d17c6…09f4`；fuse sentinel=1 |
| `cargo xtask engine verify`（宿主权限） | raw `v43.3.0` + macOS bundle verify绿 |
| macOS双role host conformance（宿主权限） | `2/0/0`；Linux分支未破坏既有基线 |
| macOS调用`engine runsc-spike` | 在读取artifact前exit 1：`requires a native Linux x86_64 host`，fail-closed正向对照 |
| `cargo xtask parity-check` | parity `693/999/1692`；overlay `1674/16/2/0`；本branch要求revalidate=7；0 violation |
| `cargo xtask recount` | repo `71/0`；未设置`OPENBOT_UPSTREAM_DIR`而如实跳过88；总159 |
| `grok-inventory --check` / `git rev-parse HEAD:grok-bot` | 2,110文件同步；Grok tree仍`86f5a85f…`且diff空 |
| `bash tools/check-deny-release-targets.sh` | bans + 六发行target bans/sources全绿 |
| TOML机械读取`engines.runsc` | `false`；未偷填版本 |

## 7. 明确未做

- 未运行runsc/Xvfb，未选择/下载/钉任何runsc release或Xvfb package/hash；`engine-pins.toml`仍无runsc engine条目。
- 未取得Ubuntu host/rootfs、renderer `Seccomp`/`NoNewPrivs`/namespace或两role runtime JSON。
- 未运行Windows runtime；Batch54红线不变。
- 未派发Actions、未运行`cargo xtask ci`（R63 manual-only）。
- 未进入P2，未新增任何Grok产品能力，`grok-bot/`零改动。

官方依据：[gVisor installation](https://gvisor.dev/docs/user_guide/install/)、[OCI quick start](https://gvisor.dev/docs/user_guide/quick_start/oci/)、[rootless limits](https://gvisor.dev/docs/user_guide/rootless/)、[Chromium Linux sandbox layers](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/linux/sandboxing.md)、[X.Org Xserver `-nolisten`](https://xorg.freedesktop.org/archive/X11R7.5/doc/man/man1/Xserver.1.html)、[Electron headless/Xvfb evidence](https://github.com/electron/electron/issues/228)。
