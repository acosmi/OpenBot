# E2：Windows Engine 启动阻断修复与原生复验

日期：2026-09-05。用户已授权将本任务说明和主控修正推送到分支，由 Windows 同事运行并回传。

## 开始方式

仓库为 `https://github.com/acosmi/OpenBot.git`，本次执行分支为
`fix/2026-09-05-windows-e-delivery-audit`。它从已合入的 R201 主线 `ec7a6c4` 派生，择取并修正
E 原候选 `6c4cb00863dd669219deb7c636cdd920079327fc`。包含 R199 Windows 封闭环境修复，
**代码/协议仍为 epoch 4**；不包含 macOS 未提交的 epoch 5、前端重设计或本机启动在制改动。

使用新目录保留原 E 现场；不要强制 reset、clean 或改写原 E 分支：

```powershell
git clone --config core.autocrlf=false --config core.longpaths=true --single-branch --branch fix/2026-09-05-windows-e-delivery-audit https://github.com/acosmi/OpenBot.git WrokBot-Windows-E2
cd WrokBot-Windows-E2
git fetch origin main:refs/remotes/origin/main
git status --short
git rev-parse HEAD
git rev-parse HEAD:grok-bot
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
```

在开始报告里登记上面取得的完整 HEAD，后续不追 main、不 rebase。本任务执行者只在该 checkout 修改。
先读 `CLAUDE.md`、v4 第一真源的 §10–§12、§24、§25、§28.1 R127/R184–R201，
以及 `docs/2026-09-04-Windows-Engine真机-外部交付.md` 的主控复核版。
主控给出的准确任务提交 SHA 应与 `git rev-parse HEAD` 相等，若不等先报告差异。

## 要交付什么

目标是让**本项目** Windows x64 的 Browser/Component 两 role 通过真实启动、输入、帧、停流与清理。
不能仅交一个复现 NUL 失败的绿色测试，就称为 Engine 修复完成。

先运行补强后的两条 paired probe：

```powershell
cargo test -p openbot-windows-sandbox --locked -- --nocapture
cargo test -p openbot-windows-sandbox --locked restricted_write_process_ -- --ignored --nocapture --test-threads=1
```

- ACL：正常权限可写 profile 和 outside；清掉标记后，受限 child 只写 profile，outside 不存在且非零退出。
- NUL：正常权限同命令完成；受限 child 的 before 标记必须存在且内容准确，after 不存在、退出非零。
  这表示诊断复现成立，**不是产品成功**。若受限 NUL 访问成功，应记录事实并检查原归因，不能改成任意失败即通过。
- 根目录创建失败时保留现场并查原因；不要删除一个并非本轮创建的目录以让测试继续。

随后运行当前 epoch 的真实 bundle/conformance：

```powershell
cargo xtask engine fetch
cargo xtask engine bundle
cargo xtask engine verify
cargo xtask engine protocol --check
cargo xtask electron-shim-check
cargo test -p openbot-computer --test engine_conformance --locked -- --include-ignored --nocapture --test-threads=1
```

## 如何定位并修复 NUL/stdio 阻断

官方 Electron 的 `NodeBindings::Initialize` 会调用 `IsNulDeviceEnabled`，后者以读写方式打开 NUL。
R127 的 write-restricted token 是已有安全边界；主控尚未批准改变该边界。

请用隔离测试子进程核对以下事实，并只记录脱敏的布尔、错误码、计数和摘要：

1. 正常/受限 token 对 NUL 的 read、write、read+write 打开结果和 Win32 错误码，不能只依赖 cmd 重定向。
2. 实际 token 的受限状态/完整性类别与 Job 成员状态。设备 ACL 的所需权限判定只记录类型事实，
   不回传当前用户 SID、机器名、账户名、完整环境或生产数据。
3. Electron 真实 stderr 中的首个失败类型；临时诊断 stdout/stderr 文件只放本任务 private 目录，
   不把原始文件上传。将诊断改动和待提交产品改动分开。
4. 若调查 `--no-stdio-init`，必须先证明同一 spawned child 能读取实际 stdin boot bytes、连接两条 pipe，
   并观测退出码/存活状态。仅 ConnectTimeout 不能推断 stdin 原因。诊断 flag 不自动进入生产。
5. 优先寻找保留既有限制与 stdin 协议的 Windows 专属兼容修复；若没有可验证方案，回传精确最小复现，
   主控继续裁决。不得为了成功启动移除 token/Job/ACL、启 breakaway、修改宿主 NUL ACL、加入宽泛 restricting SID，
   或把限制改成仅日志告警。

## 可修改范围

- `crates/openbot-windows-sandbox/src/windows.rs`、`command_line.rs`、`lib.rs`：本任务必要的 Windows 修复和成对回归。
- `crates/openbot-windows-sandbox/SECURITY.md`：仅同步已实际验证的事实，不提前更改 fidelity。
- `crates/openbot-computer/src/engine/process.rs`、`tests/engine_conformance.rs`：Windows 专属路径；
  不放宽跨平台断言，不让失败跳过。R199 九键环境与其余 scope/peer/generation 不得回退。
- `crates/openbot-testkit/src/xtask/engine_bundle.rs`：仅实际复现的 Windows PE/bundle 缺陷。
- 新增 `docs/2026-09-05-E2-Windows-Engine原生复验-交付.md` 与
  `fixtures/computer/windows-runtime-audit/windows-engine-runtime-e2.json`；旧 E JSON 不覆盖。

不改 main、GUI、中央台账/manifest/第一真源、Cargo 依赖、协议/epoch/shim、品牌、证书/密钥、Actions。
需要越出上述边界时先回传最小复现和具体方案，不擅自放宽安全条件。

## 最终验证和回传

```powershell
cargo test -p openbot-windows-sandbox --locked -- --nocapture
cargo test -p openbot-windows-sandbox --locked restricted_write_process_ -- --ignored --nocapture --test-threads=1
cargo test -p openbot-computer --locked
cargo test -p openbot-computer --test engine_conformance --locked -- --include-ignored --nocapture --test-threads=1
cargo clippy -p openbot-windows-sandbox -p openbot-computer --all-targets --locked -- -D warnings
cargo clippy -p openbot-testkit --features xtask --bin xtask --locked -- -D warnings
cargo fmt --all --check
git diff --check
cargo xtask parity-check
cargo xtask grok-inventory --check
```

最后报告每条命令的实际退出码、passed/failed/ignored，两个 role 的握手、输入/帧/ACK、pause/resume、
无 TCP listener、Job/renderer sandbox、关闭后进程树及 profile lock。缺任一项就保留明确缺口。
保留 Windows `Degraded`；不声称完成原生 Desktop 产品旅程、签名/Windows golden 或完整 P1/G5/Alpha。

形成任务起始 SHA 之后恰 1 个实现提交；普通 push 到本任务分支后，回传完整候选 SHA、
`git diff --stat <任务起始SHA>..HEAD`、报告路径及未解决问题。不 force-push、不合并、不派发 Actions，
不改其它外派分支。主控收到用户通知后独立验收，通过后用 merge commit 合入。
