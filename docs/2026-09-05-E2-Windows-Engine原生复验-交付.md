# E2：Windows Engine 启动阻断修复与原生复验 —— 交付

日期：2026-09-05（本机 America/Los_Angeles）。任务书：`docs/2026-09-05-E2-Windows-Engine启动阻断修复-任务书.md`。

**E2 没有让 Windows 两 role 通过。** 本轮在 Windows 11 x64 原生实跑，复现了原 3 项失败，定位并修复了
**第一层阻断（NUL）**，又在同一台机器上用隔离子进程和无沙箱对照定位出**第二层（stdin）与第三层
（Mojo 命名管道）两个此前未知的阻断**。第三层是 Chromium 浏览器进程与 R127 `WRITE_RESTRICTED`
主令牌的架构冲突，任务书禁止在本任务内改动令牌边界，故按“回传精确最小复现、主控继续裁决”处理。
Windows fidelity 仍为 `Degraded`，P1 / G5 / Alpha / Windows 发行范围一律不勾。

## 1. 基线与环境

- 分支：`fix/2026-09-05-windows-e-delivery-audit`，任务起始 SHA = `git rev-parse HEAD` =
  `ca8265b5090980fd37be4e5eec141c838ee22aa2`（与主控给出的值相等，无差异）。
- `git rev-parse HEAD:grok-bot` = `86f5a85f560f721677fa7e587a67ac0ffc036cb5`，未改动。
- 宿主：Microsoft Windows 11 Home China，`10.0.26200`，x64 原生（非 WSL / 非 Wine）。
- `rustc 1.98.0 (88d9e12ae 2026-08-18)` / host `x86_64-pc-windows-msvc` / LLVM `22.1.8`；
  `cargo 1.98.0`；`git version 2.53.0.windows.3`；MSVC 生成工具与 Windows SDK `10.0.26100.0`。
- 环境变量按任务书设置 `CARGO_INCREMENTAL=0`、`CARGO_PROFILE_DEV_DEBUG=0`、`CARGO_PROFILE_TEST_DEBUG=0`。
- 代码/协议保持 epoch 4，未改协议、shim、epoch、manifest、中央台账、第一真源、Cargo 依赖、
  品牌、证书、Actions。

## 2. Engine 工件：本机独立复算

主控复核版曾记为“未取得原始字节，不称已独立重放”。本轮在本机真下载、真组装并逐个复算：

| 项 | 本机实测 | 与仓内 pin / macOS 记录 |
|---|---|---|
| `electron-v43.3.0-win32-x64.zip` | `144,396,349 B`，sha256 `18528bedc6a9b04bdc5efb7b803cbc3cb0e5ea6415d54046e23d464d89a00da9` | 与 `tools/engine-pins.toml` 的 size/sha256 逐字相等 |
| `--version` 探针 | `v43.3.0` | 与 pin 相等 |
| `acosmi-engine-fixture.exe` | `225,441,792 B`，sha256 `6a205cc474e151893422f36f474b593708f68efd31746fb1b87ccb0d4a7587ee` | 与原 E 报告相等 |
| `resources/app.asar` | `30,069 B`，sha256 `3141a5521ea819a21ee4e8b8f93948af6ca2baa11d4f458f05653672c5f5e11f` | 与原 E 报告相等 |
| ASAR header sha256 | `d795c804b80b9ac20f3c40ebc44dc61fe423cbee1e03c1c3ce95c2bfa6f926d1` | 与 R190 的 macOS 记录相等 |
| `manifest.json` | sha256 `857f1490b1841b6839fc092742e1eb9730330f2b104534f002975af154041ac6` | 与原 E 报告相等 |
| fuse wire / sentinels / release epoch | `000011001` / `1` / `4` | 与 R185/R190 相等 |
| 协议描述符 sha256 | `a2bd3a3978a650e199294437aba1661c7bb0ed0c8861ade85a09ff9a49a1252c` | `engine protocol --check` 通过 |
| shim | `files=3`，非空 LOC `595/600`，grok-bot 外 `package.json=1` | 与 R190 相等 |

交付结束时 bundle 已复原为未插桩状态，`engine verify` 重跑通过，ASAR 仍为 `30,069 B` /
`d795c804…`；诊断期间的临时 shim 改动没有进入任何提交。

## 3. 补强后的成对 probe：复现成立

两条 paired probe 均在受限令牌下真实执行，先由正常权限对照跑同一条命令：

- ACL：正常权限同时写出 `profile\allowed.txt` 与 `outside\escaped.txt`，字节恰为 `allowed\r\n` /
  `escaped\r\n`；清标记后受限 child 只写出 profile 文件，`outside\escaped.txt` 不存在且退出码非零。
- NUL：正常权限完成整条命令并写出 `before` 与 `allowed` 两个标记；清标记后受限 child 写出
  `before` 标记后以非零退出码结束，`allowed` 缺失。**这只表示诊断复现成立，不是产品成功。**
- 受限 NUL 访问没有成功，因此原归因不需要改写；本轮另用系统调用把它精确化（§4.1）。
- 两次 root 目录都由本轮新建，没有删除任何非本轮创建的目录。

## 4. 三层阻断的定位

诊断用的隔离子进程只回传布尔、Win32 码、计数与摘要；不回传当前用户 SID、机器名、账户名、
完整环境或生产数据。临时诊断产物只落在本任务 private 目录，未入仓、未上传原始文件。
诊断改动（`inheritable_null` 的 stdio 重定向开关、`launch_with` 的子进程状态打印、shim 的退出码
插桩、独立探针二进制）与待提交产品改动始终分开，交付前已全部回退，`git status` 只剩产品改动。

### 4.1 第一层：`WRITE_RESTRICTED` 拒绝 NUL 的写打开（已修复）

隔离探针在**同一条 `spawn_restricted` 路径**上直接调用 `CreateFileW` 与 Rust `OpenOptions`：

| 打开方式 | 正常令牌 | 受限令牌 |
|---|---|---|
| `NUL` read | ok，win32=0 | ok，win32=0 |
| `NUL` write | ok，win32=0 | **失败，win32=5** |
| `NUL` read+write | ok，win32=0 | **失败，win32=5** |
| `\\.\NUL` 三种同上 | 同上 | 同上 |

同一探针另报：`token restricted=true`、`token integrity_rid=0x2000`（Medium）、`job member=true`，
`stdout write ok=true`，`stdin bytes=22 newline_terminated=true first_char_is_brace=true`。
即**受限 child 确实能读到父进程写入的实际 stdin boot 字节**，令牌、完整性类别与 Job 成员状态
均符合 R127 设计。设备侧只记录类型事实：读打开成立、写打开被拒，与
`CreateRestrictedToken(WRITE_RESTRICTED)` 只对写访问再做 restricting SID 检查的语义一致。

真实 Electron 的首个失败类型由重定向到本任务 private 文件的 child stderr 取得，逐字为：

```
[FATAL:electron\shell\common\node_bindings.cc:735] Unable to open nul device needed for
initialization,aborting startup. As a workaround, try starting with --no-stdio-init
```

固定版本源码核对（`v43.3.0`）：`shell/common/platform_util_win.cc::IsNulDeviceEnabled` 就是
`_open("nul", _O_RDWR)`；`shell/common/node_bindings.cc` 在 `kNoStdioInit` 未给出时才做这次
fail-fast 检查，给出时改为置 `node::ProcessInitializationFlags::kNoStdioInitialization`。

**采用的修复**：Windows 专属地在 `spawn_engine` 的参数尾部追加 `--no-stdio-init`，其余平台参数
逐项不变。判据来自钉死版本的源码而不是猜测：在 Node `v24.18.1` 的 `src/node.cc` 中，
`kNoStdioInitialization` 在 Windows 上只关掉三件事 ——
(1) 上述 fail-fast 探测；
(2) `PlatformInit` 里“fd 0-2 无效或 `FILE_TYPE_UNKNOWN` 时才 `_open("nul", _O_RDWR)`”的修补循环；
(3) `atexit(ResetStdio)`，而 `ResetStdio` 的 Windows 主体只有 `uv_tty_reset_mode()`。
`spawn_restricted` 始终通过 `STARTF_USESTDHANDLES` 交付三个有效句柄（boot 管道 + 两个由父进程打开
的 NUL），且从不附加控制台，因此三项对本项目都是 no-op，stdin boot 协议不变。Electron 自己在
`shell/browser/electron_browser_client.cc` 的 `kCommonSwitchNames` 里把该开关复制给 renderer 与
utility 子进程，所以令牌、Job、目录 ACL 与协议全部保持 R127/R199 原样。

修复后重跑：child stderr 中的 FATAL 消失，进程继续推进到下一阶段（§4.2）。

### 4.2 第二层：Electron 在 Windows 把 `process.stdin` 换成已结束的流（未修复，超出本任务范围）

去掉 NUL 阻断后，child 仍然 `ConnectTimeout`，且退出码为 1、stderr 为空。用退出码把 shim
`bootstrap()` 的各步区分开后测得：失败点是 `readBootCapability()`，且 `process.stdin` 交付
**0 字节**即结束，`process.stdin.constructor.name` 长度为 8（`Readable`），`isTTY` 为假。

这不是本项目沙箱造成的。三次**无任何限制**的对照直接运行同一 bundle 均得到同一结果：

| 对照 | 结果 |
|---|---|
| 普通 shell 管道 stdin + `--no-stdio-init` | 同样 0 字节 |
| 普通 shell 管道 stdin + **不带** `--no-stdio-init` | 同样 0 字节 |
| 磁盘文件重定向 stdin + `--no-stdio-init` | 同样 0 字节 |

另有 CRT 层对照：受限 child 中 `GetStdHandle(STD_INPUT_HANDLE)` 与 `_get_osfhandle(0)` 都有效、
同一句柄、`GetFileType=3`（`FILE_TYPE_PIPE`），stdout/stderr 为 `FILE_TYPE_CHAR`；console 与
GUI 两种 subsystem 的探针结论相同。即 OS/CRT 层交付是正确的。

根因在 Electron 自身。钉死 bundle 的可执行文件内含如下逐字代码：

```js
"win32"===process.platform){const{Readable:e}=r("stream"),t=new e;t.push(null),
Object.defineProperty(process,"stdin",{configurable:!1,enumerable:!0,get:()=>t})}
```

即 Electron 在 Windows 上无条件把 `process.stdin` 替换为已 `push(null)` 的 `Readable`，且属性
`configurable:false`，shim 无法再重定义。**因此 stdin boot 协议在 Windows 的 Electron 主进程里
不可能读到数据，与令牌、Job、ACL 无关。**

**已验证但未提交的候选修复**：fd 0 本身仍是父进程的管道，只是 `process.stdin` 这个流对象被换掉。
把 shim 的 boot 读取改为在 Windows 上直接包住 fd 0（仅用现有允许的 `node:net`，不新增 import、
不改 wire、不改 epoch）：

```js
const boot_stdin = process.platform === "win32"
  ? new net.Socket({ fd: 0, readable: true, writable: false })
  : process.stdin;
for await (const chunk of boot_stdin) {
```

本机实测该改动可用：`electron-shim-check` 仍通过（非空 LOC 599/600），双管道握手完成，
`launch + peer credential + ready` 不再报错，失败点前移到 `start`（见 §4.3）。

**没有提交它**，理由是任务书明确把 shim 排除在可修改范围外，并且它会改变所有平台的 ASAR 字节与
`asar_header_sha256`（`d795c804…` 会变），使 R185/R190 已钉死的 macOS 工件证据失效；加上第三层
阻断仍在，提交它并不能让 Windows 通过。请主控裁决是否接受该改动以及随之而来的跨平台工件重钉。

### 4.3 第三层：Chromium Mojo 无法在 `WRITE_RESTRICTED` 主令牌下建立 IPC 通道（未修复，需主控裁决）

在 §4.2 的候选修复下继续跑，`start` 阶段失败为 `EngineReported("load_timeout")`，child stderr 逐字为：

```
[FATAL:mojo\public\cpp\platform\platform_channel.cc:108] Check failed: . : 拒绝访问。 (0x5)
```

对照固定版本 Chromium `150.0.7871.212` 的 `mojo/public/cpp/platform/platform_channel.cc`：
`CreateChannel()` 先用**空安全描述符**（`nullptr`）`CreateNamedPipeW` 建立本端，再立刻用
`GENERIC_READ | GENERIC_WRITE` `CreateFileW` 打开同一管道取得远端，第 108 行即
`PCHECK(remote_endpoint->is_valid())`。

隔离探针在同一受限令牌下精确复现，并读回对象自身的 DACL（SID 只保留众所周知者，其余脱敏）：

| 观察 | 正常令牌 | 受限令牌 |
|---|---|---|
| `CreateNamedPipeW`（空安全描述符） | ok | ok |
| 以 Mojo 的 flags 重开 read+write | ok，win32=0 | **失败，win32=5** |
| 以普通 flags 重开 read+write | ok，win32=0 | **失败，win32=5** |
| 重开 read-only | （管道已被占用，不作判据） | ok，win32=0 |

管道对象实际 DACL 为 5 条 ACE：`S-1-5-18`、`S-1-5-32-544`、创建者 各 `0x001f01ff`
（`FILE_ALL_ACCESS`），`S-1-1-0` 与登录会话 SID 各 `0x00120089`（`FILE_GENERIC_READ`）。
**其中没有任何一条命名 `S-1-5-12`（Restricted Code）。** 而同一进程的令牌默认 DACL 确实是
`S-1-5-18` / 当前用户 / `S-1-5-12` 三条 `GENERIC_ALL`（即 `set_restricted_default_dacl` 生效）。
结论是 NPFS 不把令牌默认 DACL 复制给新建管道，而是使用它自己的默认描述符；`WRITE_RESTRICTED`
的 restricting SID 检查只作用于写访问，于是读打开成立、读写打开被拒。

这与第一层是同一机制的两处表现（`\Device\Null` 与 NPFS 默认描述符都不命名 restricting SID）。
差别在于：第一层可以用 Electron 官方开关绕开，第三层发生在 Chromium 自己的 IPC 骨干里，
本项目既不能改 Chromium 的调用，也不允许（任务书明列禁止）移除 token/Job/ACL、启 breakaway、
改宿主设备/NPFS ACL、加入宽泛 restricting SID，或把限制降级为日志告警。因此
**“Chromium 浏览器进程能否在 `WRITE_RESTRICTED` 主令牌下运行”是 R127 Windows 令牌裁决层面的问题**，
本任务按任务书回传最小复现，不擅自放宽安全条件。

该最小复现已落为成对回归（正常权限先证明两次写打开都成立，受限 child 必须自己跑完并写出观察）：

```powershell
cargo test -p openbot-windows-sandbox --locked restricted_write_process_cannot_reopen_default_security_kernel_objects -- --ignored --nocapture --test-threads=1
```

## 5. 本轮改动

产品改动只有两处，均在任务书的可修改范围内：

1. `crates/openbot-computer/src/engine/process.rs`
   - 新增 `#[cfg(windows)] fn windows_engine_args()`：在共享 `engine_args` 之后追加且只追加
     `--no-stdio-init`，Windows `spawn_engine` 改用它；非 Windows 参数集合逐项不变。
   - 新增 `#[cfg(windows)]` 单元回归 `windows_engine_args_append_only_the_no_stdio_init_switch`：
     共享集合不得含该开关，Windows 集合恰为共享集合加尾部一项。
2. `crates/openbot-windows-sandbox/src/windows.rs`
   - 新增成对回归 `restricted_write_process_cannot_reopen_default_security_kernel_objects` 与其
     `#[ignore]` 子进程半场 `default_security_kernel_open_probe_child`，把 §4.1 与 §4.3 的
     设备/NPFS 读写不对称钉成机器判据（正常权限对照 → 清结果 → 受限 child 必须跑完并自报）。
   - `#[cfg(test)]` 之前的生产代码与任务起始 SHA 逐字节相等；R199 九键封闭环境、令牌、Job、ACL、
     peer/generation 判据均未改动。

`crates/openbot-windows-sandbox/SECURITY.md`、`engine_conformance.rs`、`engine_bundle.rs` 本轮
没有需要同步的已验证事实，故未改。

## 6. 最终验证：逐条实际退出码

全部在本机 Windows 原生执行。

| 命令 | 退出码 | 结果 |
|---|---|---|
| `cargo xtask engine fetch` | 0 | 下载 `144,396,349 B`，sha256 与 pin 相等，`--version=v43.3.0` |
| `cargo xtask engine bundle` | 0 | `app.asar=30069`，header `d795c804…`，fuse sentinels=1，epoch=4 |
| `cargo xtask engine verify` | 0 | raw archive + `--version` + bundle verify 全通过 |
| `cargo xtask engine protocol --check` | 0 | `version=4`，sha256 `a2bd3a39…` |
| `cargo xtask electron-shim-check` | 0 | `files=3`，非空 LOC `595/600`，协议 hash 匹配 |
| `cargo test -p openbot-windows-sandbox --locked -- --nocapture` | 0 | **7 passed / 0 failed / 5 ignored** |
| `cargo test -p openbot-windows-sandbox --locked restricted_write_process_ -- --ignored --nocapture --test-threads=1` | 0 | **3 passed / 0 failed / 0 ignored** |
| `cargo test -p openbot-computer --locked` | 0 | lib **67/0/0**；`engine_conformance` **5/0/3 ignored** |
| `cargo test -p openbot-computer --test engine_conformance --locked -- --include-ignored --nocapture --test-threads=1` | **101** | **5 passed / 3 failed / 0 ignored** |
| `cargo clippy -p openbot-windows-sandbox -p openbot-computer --all-targets --locked -- -D warnings` | 0 | 无警告 |
| `cargo clippy -p openbot-testkit --features xtask --bin xtask --locked -- -D warnings` | 0 | 无警告 |
| `cargo fmt --all --check` | 0 | 无差异 |
| `git diff --check` | 0 | 无空白问题 |
| `cargo xtask parity-check` | 0 | **0 违反**；parity `886/826/1712`，fixtures `40/20/60`，overlay `carry=1234 revalidate=470 split=2 superseded=6` |
| `cargo xtask grok-inventory --check` | 0 | `files=2110`，tree `bb00e636…` 未变 |

三项真实 Engine 失败逐条：

| 用例 | 结果 | 错误 |
|---|---|---|
| `browser_role_start_frame_stop_has_no_debug_listener_or_orphan` | failed | `launch + peer credential + ready: ConnectTimeout` |
| `component_role_start_frame_stop_has_no_debug_listener_or_orphan` | failed | `launch + peer credential + ready: ConnectTimeout` |
| `both_roles_pause_on_last_viewer_and_resume_the_same_document` | failed | `engine: ConnectTimeout` |

失败点已从“NUL FATAL”前移到 §4.2 的 stdin 层；在提交的代码上（shim 未改）表现仍是
`ConnectTimeout`。`cargo xtask ci` 与 GitHub Actions 本轮均未运行（未获授权）。

## 7. 明确缺口：没有取得的判据

任务书要求的运行期判据一项都没有成立，全部保留为缺口：

- 两个 role 的握手（Windows 上未完成）。
- 输入 / 帧 / ACK、pause/resume（未执行到）。
- 无 TCP listener 的负对照（未执行到）。
- renderer Job membership、renderer OS sandbox 真实状态（未执行到）。
- 关闭后进程树与 profile lock 清理（未执行到）。
- main/renderer creation-time 绑定的运行期验证（未执行到）。
- Credential Manager 真机 ignored 用例（本任务范围外，会写操作者自己的凭据库，未跑）。
- 原生 Desktop 产品旅程、签名 / Authenticode、Windows golden 截图：均未做、不声称。

Windows sandbox fidelity 保持 `Degraded`。本交付没有改中央 manifest / parity 状态，没有关闭
P1、G5、G7、Alpha 或 Windows 发行范围。

## 8. 需要主控裁决的两项

1. **shim boot 读取**（§4.2）：是否接受用 `net.Socket({ fd: 0 })` 在 Windows 上包住 fd 0。它不改
   wire、不改 epoch、不新增 shim import，但会改动所有平台的 ASAR 字节与 `asar_header_sha256`，
   需要同批重钉 macOS 工件记录。本机已验证其确实让双管道握手成立。
2. **R127 Windows 令牌**（§4.3）：Chromium 的 Mojo 通道要求浏览器进程能重开自建的默认安全描述符
   命名管道并取得写访问，这在 `WRITE_RESTRICTED` 主令牌下由 restricting SID 检查拒绝。可选方向
   （均超出本任务授权，未实施、未评估安全代价）：改用非 write-restricted 的 lockdown 方案、
   为 Engine 目录以外的对象另设创建策略、或接受 Windows 采用与 macOS/Linux 不同的主进程约束层次。
   在裁决之前，Windows Engine 运行期保持阻断。
