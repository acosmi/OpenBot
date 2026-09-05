# 外部任务 E：Windows 真机 Engine 验收 —— 交付

任务标签：**E**。执行日期：**2026-09-05**（本机时区已是 America/Los_Angeles）。
文件名沿用任务书 §范围 指定的 `docs/2026-09-04-Windows-Engine真机-外部交付.md`，该日期是任务书签发日，不是执行日；
按 CLAUDE.md §8 的命名规则本应为 09-05，但任务书把允许新增的文件名钉死，故不自行改名。

## 0. 一句话结论

Windows x64 真机首次跑通了 **bundle 侧与 boundary 侧的全部判据**，并把 Batch54 从未执行过的受限
token/ACL 负对照修成真正可判定；但**两个 role 的 conformance 没有通过**，原因是一条真机才暴露的
硬阻断：Electron 43 的 Node 启动会无条件打开 `nul` 设备，而 R127 的 `WRITE_RESTRICTED` token 拒绝该写。
本任务**不修**这条阻断（它要改 R127 的 token 裁决，超出授权），只交最小复现与建议。
**因此不得记 Windows 真机通过，P1 的 Windows 部分仍红。**

## 1. 标签 / 分支 / 路径 / 基线

| 项 | 值 |
| --- | --- |
| 标签 | E |
| 分支 | `feat/2026-09-04-P1-windows-runtime-audit` |
| 主机平台 | Windows 11 家庭版 中文版，`10.0.26200` build `26200`，x64 原生（`AMD64`，Intel64 Family 6 Model 140 Stepping 1） |
| 绝对工作树路径 | `D:\OpenBot-Windows-E` |
| `handoff_base` | `de8d0ecc7d71e88e41e9b3b6bf7884c9d89c53fd` |
| 代码/协议基线 R196 | `87d84bb85d0056dfa4dcc2b35be4c2a610a55ae3` |
| `grok-bot` tree | `86f5a85f560f721677fa7e587a67ac0ffc036cb5`（零改动） |
| Rust | `rustc 1.98.0 (88d9e12ae 2026-08-18)`，host `x86_64-pc-windows-msvc`，LLVM `22.1.8`，与 `rust-toolchain.toml` 钉值一致 |
| git | `2.53.0.windows.3` |

WSL / Wine / macOS cross-check **均未使用**；全部命令在 Windows x64 原生 shell 执行。

macOS 绝对路径按本 checkout 同名相对路径映射：任务书里的 `/Users/…/OpenBot/<相对路径>` 一律读作
`D:\OpenBot-Windows-E\<相对路径>`；本机没有、也没有创建任何 macOS 目录。

检出核验（交接说明 §Windows 同事开始方式 的四条，全部实跑通过）：

```
git rev-parse HEAD                        -> de8d0ecc7d71e88e41e9b3b6bf7884c9d89c53fd
git merge-base --is-ancestor 87d84bb… HEAD -> exit 0
git diff --name-only 87d84bb… HEAD         -> 恰 3 个 docs 文件
git rev-parse HEAD:grok-bot                -> 86f5a85f560f721677fa7e587a67ac0ffc036cb5
```

克隆按交接说明执行；因本机沙箱禁止删除 `D:\OpenBot*` 前缀路径，第一次被中断的半成品 clone
（仅 `.git`、0 refs、无工作树）无法删除，故就地续完同一次 clone —— 其 `.git/config` 已由那条命令写入
`autocrlf=false` / `longpaths=true` / 单分支 refspec，与重新 clone 等价。

## 2. 已验证（本轮真机实跑，逐条带退出码）

### 2.1 Engine 包与 bundle（任务书验收 2）

| 命令 | 退出码 | 实得 |
| --- | --- | --- |
| `cargo xtask engine fetch` | 0 | 下载 `electron-v43.3.0-win32-x64.zip`，sha256 `18528bedc6a9b04bdc5efb7b803cbc3cb0e5ea6415d54046e23d464d89a00da9` 与 `tools/engine-pins.toml` 逐字相等；`--version=v43.3.0` |
| `cargo xtask engine bundle` | 0 | **首次真实生成 windows-x64 bundle**；`app.asar=30069 B`、`header_sha256=d795c804b80b9ac20f3c40ebc44dc61fe423cbee1e03c1c3ce95c2bfa6f926d1`、`fuse_sentinels=1`、`release_epoch=4` |
| `cargo xtask engine verify` | 0 | raw archive + `--version` + bundle verify；Windows 分支同轮读回 PE `Integrity`/`ElectronAsar` 资源并逐字节比对 |
| `cargo xtask engine protocol --check` | 0 | `version=4`，`sha256=a2bd3a3978a650e199294437aba1661c7bb0ed0c8861ade85a09ff9a49a1252c` |
| `cargo xtask electron-shim-check` | 0 | `files=3`；非空 LOC `595/600`；非 grok `package.json` 恰 1；protocol hash match |

bundle 摘要（`fixtures/computer/windows-runtime-audit/windows-engine-runtime-2026-09-05.json` 存完整表）：

- `manifest.json` sha256 `857f1490b1841b6839fc092742e1eb9730330f2b104534f002975af154041ac6`
- `acosmi-engine-fixture.exe` sha256 `6a205cc474e151893422f36f474b593708f68efd31746fb1b87ccb0d4a7587ee`（已含写入的 PE 资源）
- `resources/app.asar` sha256 `3141a5521ea819a21ee4e8b8f93948af6ca2baa11d4f458f05653672c5f5e11f`
- `fuse_wire=000011001`、`release_epoch=4`、`protocol_version=4`、`platform=windows-x64`

**跨平台一致性旁证**：ASAR 字节数与 header sha256 与 `engine-pins.toml` 里 Batch115 记录的 macOS 值
（`ASAR30069 B/header d795c804…/shim595`）逐字相等，说明 ASAR 与协议本身平台无关，Windows 侧差异
只落在 PE 资源与 executable 重命名上。

### 2.2 Boundary（任务书验收 3）

| 命令 | 退出码 | passed/failed/ignored |
| --- | --- | --- |
| `cargo test -p openbot-windows-sandbox --locked -- --nocapture` | 0 | **5 / 0 / 3** |
| `cargo test -p openbot-windows-sandbox --locked restricted_write_process_writes_profile_but_not_medium_outside -- --ignored --nocapture` | 0 | **1 / 0 / 0** |

默认 5 条里有两条是 Batch54 只交叉编译、从未在真机执行过的：

- `named_pipe_peer_identity_binds_pid_and_exact_creation_time` —— 真实 Named Pipe 上
  `GetNamedPipeClientProcessId` 等于 spawn PID，且 `GetProcessTimes` 的 100 ns creation FILETIME 逐位相等；
- `pe_resource_update_round_trips_exact_bytes_without_loading_code` —— 真实 `BeginUpdateResourceW`
  事务写入后以 `AS_DATAFILE_EXCLUSIVE | AS_IMAGE_RESOURCE` 重读，字节逐一相等。

**受限 token/ACL 负对照现在是真的成立的**（详见 §4 的三条修复）：同一个 child 在受控 profile 写出
逐字节 `allowed\r\n`，在 medium 标签的 outside 写失败且文件不存在，进程退出码非 0。
残留目录的实际 SDDL 复核（测试 panic 时不清理，故可直接读）：

```
profile : D:P(A;OICI;FA;;;RC)(A;OICI;FA;;;SY)(A;OICI;FA;;;<当前用户>)
outside : D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;<当前用户>)          ← 无 RC ACE
```

### 2.3 Job / breakaway（任务书验收 5 的可静态判定部分）

- **无 breakaway：负向** `grep -rn 'BREAKAWAY' crates/` 命中 **0**；**正向对照**同一条命令能命中确实存在的
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_JOB_MEMORY`
  （`openbot-windows-sandbox/src/windows.rs::create_job`）。所以"没有 breakaway"是证出来的，不是假设。
- **suspended 阶段附着**：Job 句柄在 `spawn_restricted` 里先进 `AttributeList::new(&inherited, raw(&job))`
  （`PROC_THREAD_ATTRIBUTE_JOB_LIST`），`CreateProcessAsUserW` 带 `CREATE_SUSPENDED`，`ResumeThread`
  在其后才调用 —— 顺序在源码上闭合。
- **renderer 属于同 Job 的运行期判据**由 `IsProcessInJob` 在 host 侧 fail-closed，但它只在 role 跑起来后才
  执行；本轮因 §3 的阻断**没有取得该运行期证据**。

### 2.4 其它闸门（任务书验收 7）

| 命令 | 退出码 | 实得 |
| --- | --- | --- |
| `cargo test -p openbot-computer --locked` | 0 | lib `66/0/0`；`engine_conformance` 默认 `5/0/3 ignored` |
| `cargo clippy -p openbot-windows-sandbox -p openbot-computer --all-targets --locked -- -D warnings` | 0 | — |
| `cargo clippy -p openbot-testkit --features xtask --bin xtask --locked -- -D warnings` | 0 | — |
| `cargo fmt --all --check` | 0 | — |
| `git diff --check` | 0 | — |
| `cargo xtask parity-check` | 0 | parity `886/826/1712`、fixtures `35/20/55`、overlay `carry=1234 revalidate=470 split=2 superseded=6`，0 违反 |
| `cargo xtask recount` | 0 | 通过 71 / 失配 0 / **跳过 89**（本机没有固定上游，未设 `OPENBOT_UPSTREAM_DIR`） |
| `cargo xtask grok-inventory --check` | 0 | `files=2110`，tree 未变 |

parity / fixtures / overlay 四组数字与 R196 登记的 `parity886/826/1712`、`fixtures35/20/55`、
`overlay1234/470/2/6` 逐字相符，说明本轮改动没有移动任何台账计数。

未运行：`cargo xtask ci`、完整 workspace 测试、GitHub Actions、runsc（均按总则禁止或无环境）。
未安装任何 npm/node；未使用 WSL；未关闭 sandbox。

## 3. 真实阻塞：Electron 43 的 `nul` 设备与 R127 受限 token 不相容

### 3.1 现象

`cargo test -p openbot-computer --test engine_conformance --locked -- --include-ignored --nocapture --test-threads=1`
退出码 **101**，`5 passed; 3 failed`：

| 用例 | 结果 | 说明 |
| --- | --- | --- |
| `demand_fixture_keeps_protocol_and_production_boundary_explicit` | passed | fixture |
| `engine_input_fixture_locks_protocol_and_unfinished_platform_boundaries` | passed | fixture |
| `screencast_fixture_locks_ack_order_latest_buffer_and_remaining_screen_boundary` | passed | fixture |
| `screen_hub_fixture_locks_ticket_and_production_transport_boundary` | passed | fixture |
| `screen_coordinate_fixture_locks_units_journeys_and_hardware_boundary` | passed | fixture |
| `browser_role_start_frame_stop_has_no_debug_listener_or_orphan` | **failed** | `launch + peer credential + ready: ConnectTimeout` |
| `component_role_start_frame_stop_has_no_debug_listener_or_orphan` | **failed** | `launch + peer credential + ready: ConnectTimeout` |
| `both_roles_pause_on_last_viewer_and_resume_the_same_document` | **failed** | `engine: ConnectTimeout` |

无一条被跳过；三条失败都停在 boot 握手之前，因此 role 内部的帧、ordinary input、latest/ACK、viewer
生命周期**一次都没有执行到**，没有任何可记录的实际结果。

### 3.2 根因（真机取证，不是推断）

产品代码把 child 的 stdout/stderr 接到 `NUL`，所以默认看不到任何原因。临时把这两个句柄换成真实文件后，
子进程 stderr 给出确定答案：

```
[FATAL:electron\shell\common\node_bindings.cc:735] Unable to open nul device needed for
initialization,aborting startup. As a workaround, try starting with --no-stdio-init
```

机制：R127 的
`CreateRestrictedToken(DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED)` 只带一个
restricting SID（Restricted Code，`S-1-5-12`）。`WRITE_RESTRICTED` 让**所有写访问检查**必须同时被普通 SID
集合与该 restricting SID 放行，而 `\Device\Null` 的安全描述符里没有 `S-1-5-12` 的 ACE，于是 Electron 在
加载 app 之前就 abort。

三组对照把结论钉死：

1. **负向**（受限 token）：`echo x>nul && echo allowed>profile\allowed.txt` 之后 `allowed.txt` 不存在 ——
   `&&` 短路证明 `>nul` 失败。
2. **正向**（普通 token，同一 shell、同一条命令行）：`exit=0`，文件被创建。
3. **排除 stdio 有效性**：把三个 std 句柄全部换成真实文件（而不是 `NUL` 句柄）后 FATAL 依旧出现，
   说明 Electron 是**无条件**打开 `nul`，不是因为某个 fd 看起来无效才去补。

### 3.3 为什么不用 Electron 给的 workaround

试过 `--no-stdio-init`：FATAL 确实消失，但三条 role 仍然 `ConnectTimeout`，且子进程 stdout/stderr
文件**一个字节都没有**（同轮还开了 `ELECTRON_ENABLE_LOGGING`）。它关掉的是整个 stdio 初始化，而
R119/R127 的 boot capability 恰恰靠 stdin 送 4 KiB boot token —— 换掉这条通道等于改协议，
总则明确禁止。故该 workaround 已在本轮**验证并否决**，产品代码里没有留下它。

### 3.4 建议（由主控裁决，本任务不实施）

这条阻断落在 R127 的 token 裁决本身，任务书禁止我删 restricted token、Job 或 ACL，也禁止自行改协议，
所以只交建议：

- 不能靠加 restricting SID 绕开：`\Device\Null` 授的是 `Everyone`，把 `Everyone` 或当前用户加进
  restricting 列表等于取消写限制，会掏空 R127 的全部意义；
- 不能改 `\Device\Null` 的安全描述符：那是宿主级持久改动，正是 R127 当初否决 `sandboxrs-windows` 的理由；
- 可考虑的方向（都需要新 R 行）：main 进程改用 Job + 完整性标签 + ACL 而不带 `WRITE_RESTRICTED`
  （renderer 仍由 Chromium 自沙箱按 R127 现状约束）、或引入 broker 预开句柄、或钉一个不在 Node
  启动路径上开 `nul` 的 Electron 版本。三者都改变安全边界的口径，必须由主控立裁决并重跑本任务。

最小复现已经落成一条可执行的 ignored 测试，不需要 Electron：

```
cargo test -p openbot-windows-sandbox --locked restricted_write_process_cannot_open_the_nul_device -- --ignored --nocapture
```

它当前 **passed**，即"受限 child 打不开 `nul`"这一事实成立。阻断被修好之后这条测试会转为失败，
断言消息里写明了届时应当连同 R127 的 token 裁决一起退休它。

## 4. 改动与理由

只改了一个文件、且只在其 `#[cfg(test)]` 模块内；**产品代码零改动**。

### 4.1 三条修复（都在 `crates/openbot-windows-sandbox/src/windows.rs` 的测试模块）

Batch54 的 `restricted_write_process_writes_profile_but_not_medium_outside` 从未在真机执行过，
本轮一跑就发现它**三处都错**，任何一处都会让这条负对照变成空转：

| # | 缺陷 | 真机表现 | 修复 |
| --- | --- | --- | --- |
| E-2 | argv[0] 用 `PathBuf::join("System32/cmd.exe")` 构造，含**正斜杠**；`encode_command_line` 对不含空格的 argv[0] 不加引号（这对 `CommandLineToArgvW` 消费者是正确的），于是 `cmd.exe` 解析自己的原始命令行时把 `/cmd.exe` 当成开关 | 子进程 stderr `The syntax of the command is incorrect.`，exit 1，**命令行一条都没执行** | 新增 `probe_shell()`，用 `.join("System32").join("cmd.exe")` 走原生分隔符 |
| E-3 | 命令串用 `format!` 拼了绝对路径并加引号，经 `quote_argument` 变成 CRT 的 `\"` 转义；而 `cmd.exe /S` 只剥最外层一对引号、其余按字面读，`\"` 直接落进重定向目标 | 两个写目标都非法 | 命令串改为**不含任何引号**、相对 spawn policy 工作目录（即 `root`）的路径 |
| E-4 | 断言 `allowed.txt` 内容为 `allowed\r\n`，但命令行在 `&` 前留了空格，`cmd.exe` 把重定向摘出后 echo 的是 `allowed `（带尾空格） | 实得 `"allowed \r\n"` | 去掉 `&` 前空格，保留逐字节断言 |

同时把 spawn+等待抽成 `run_probe_to_completion()`，供两条 ignored 探针共用；三处坑各自写了说明注释，
避免再次回归。

### 4.2 一条新增（同文件、同测试模块）

`restricted_write_process_cannot_open_the_nul_device`（`#[ignore]`）：§3 阻断的最小复现，不依赖 Electron。

### 4.3 刻意没有做的改动

- **没有**把 `encode_command_line` 改成给 argv[0] 无条件加引号。它当前的行为对生产消费者
  （Electron，走 `CommandLineToArgvW`/CRT 规则）是正确的，misparse 的是 `cmd.exe` 这个测试专用宿主；
  而且我无法端到端验证改动后的 Electron 行为（被 §3 阻断）。作为观察记录在此，不作为改动交付。
- **没有**改 `crates/openbot-computer/src/engine/process.rs`：`--no-stdio-init` 试过并否决（§3.3），已完全还原，
  `git diff` 对该文件为空。
- **没有**改 `SECURITY.md`：任务书只允许用它"同步已修复边界"，而 §3 是未修复的阻断，写进去会读成已接受。
- **没有**改 `engine_conformance.rs`：本轮没有需要修的 Windows 探针缺陷，跨平台断言与阈值一字未动。
- **没有**改 `engine_bundle.rs`：Windows PE/bundle 路径本轮全绿，无需修复。
- **没有**改中央 `parity/`、`fixtures/MANIFEST.yaml`、两份第一真源、`CLAUDE.md`、`README`、`NOTICE`、SPDX、
  移交指南与预留台账。

### 4.4 新资产 / 依赖

**无**。`Cargo.lock` 未改动，新增 package 0，没有引入任何第三方来源，因此中央 `NOTICE` / SPDX
**不需要任何增补**。新增文件只有本文档与一份无 secret 的 JSON 证据。

## 5. 未验证 / 不得据本轮宣称

- Windows 两个 role 的 conformance（被 §3 阻断）；
- 运行期的 renderer OS sandbox 状态、main/renderer creationTime exact 绑定、renderer 的 Job 成员资格、
  全进程树 TCP `LISTEN=0`、退出后进程树与 profile lock 清理 —— 这些判据全部在 role 起来之后才执行；
- kernel 级网络或可执行路径 allowlist；原生 Desktop 产品旅程；签名发行 / Authenticode；Windows golden；
- `credential_manager_generic_round_trip_and_delete`（另一条 ignored 真机测试）：**未运行**。任务书验收 3
  只点名默认测试与 ACL 负对照，且它会向操作者本人的 Credential Manager 写入条目，超出本任务范围。
  `SECURITY.md` 已登记它仍是 Windows 运行期缺口，本轮不改变该结论；
- `recount` 的 89 条上游复算（本机无固定上游克隆）。

Windows fidelity 如实保持 **`Degraded`**：代理参数与 write-restricted token 不构成网络或可执行路径
allowlist，也不抵抗同 UID 的恶意进程。本轮没有任何证据改变这一口径。

## 6. 一条程序性发现（E-5）

交接说明的克隆命令带 `--single-branch`，因此 checkout 里既没有 `origin/main` 也没有 `main`，
`cargo xtask parity-check` 的 v4 overlay 会在 `changed_target_prefixes` 直接报
`无法计算 git diff target 前缀：origin/main 与 main 都不可解析` 而判红 —— **这不是台账问题**：
同一次运行里 9 份 parity 台账 + fixtures 台账的 8 条规则全部通过。

处理方式：`git fetch origin main:refs/remotes/origin/main`。它只增加一个只读远程引用，
不改 HEAD、不改分支、不 rebase、不追主线；`changed_target_prefixes` 用的是
`merge-base HEAD origin/main`（实得正是 R196 `87d84bb…`）再对 `crates` 求差，所以只会算进本任务自己的
改动。补上引用后 `parity-check` 退出码 0、0 违反。建议后续外派的交接说明把这一步写进去。

## 7. 原始输出与复算

原始平台日志、诊断用的临时 stderr 捕获文件与探针脚本**留在本机**，不入交付；本文件与
`fixtures/computer/windows-runtime-audit/windows-engine-runtime-2026-09-05.json` 只保留消毒后的
判定事实（无用户名、机器名、SID 字面量、账号或客户内容）。

全部结论可由下列命令在同一 checkout 复算：

```powershell
cargo xtask engine fetch
cargo xtask engine bundle
cargo xtask engine verify
cargo xtask engine protocol --check
cargo xtask electron-shim-check
cargo test -p openbot-windows-sandbox --locked -- --nocapture
cargo test -p openbot-windows-sandbox --locked restricted_write_process_writes_profile_but_not_medium_outside -- --ignored --nocapture
cargo test -p openbot-windows-sandbox --locked restricted_write_process_cannot_open_the_nul_device -- --ignored --nocapture
cargo test -p openbot-computer --locked
cargo test -p openbot-computer --test engine_conformance --locked -- --include-ignored --nocapture --test-threads=1
cargo clippy -p openbot-windows-sandbox -p openbot-computer --all-targets --locked -- -D warnings
cargo clippy -p openbot-testkit --features xtask --bin xtask --locked -- -D warnings
cargo fmt --all --check
git diff --check
git fetch origin main:refs/remotes/origin/main   # 见 §6，只读引用
cargo xtask parity-check
cargo xtask recount
cargo xtask grok-inventory --check
```

要复现 §3.2 的 FATAL 原文，需要临时把 `spawn_restricted` 里的两个 `inheritable_null()` 换成真实文件句柄
（stdout/stderr 各一个），跑任一条 role 用例后读该文件；该改动是诊断用途，**未包含在候选 commit 里**。

## 8. 提交与计数

- 候选 commit 建在首次 `handoff_base` `de8d0ecc7d71e88e41e9b3b6bf7884c9d89c53fd` 之上，恰 1 条。
- `git rev-list --count de8d0ec..<candidate>` = 1；`git rev-list --count 87d84bb..<candidate>` = 2
  （主控准备文档 1 条 + 本实现 1 条），符合总则〈E 任务远程交接补充〉的计数例外。
- **已 push 到 `origin/feat/2026-09-04-P1-windows-runtime-audit`**。总则默认禁止执行方 push，本次是
  用户在 2026-09-05 收到上述结论后**明确指示"提交推送到分支"**，该授权只覆盖这一条分支的普通 push。
- 仍**未** force-push、未开 / 未合并 PR、未派发 GitHub Actions、未改动其它任何分支。
- 候选 commit 的完整 SHA 无法写进它自己包含的文件，随交付消息报出，也可由
  `git log --format=%H -1 origin/feat/2026-09-04-P1-windows-runtime-audit` 取得；主控异机取内容时
  另有按交接说明导出的单一 Git 补丁。
- 本节从"未 push"改为上述表述后，用 `git commit --amend` 并入同一条候选 commit，因此
  `handoff_base..HEAD` 仍恰为 1，计数例外不变。
