# Batch73：Desktop structured-event actual open/close commands

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G6-desktop-event-commands`
>
> base：`31c75efe51ac1fbc27e375fb2070540fcaf6e8db`（PR #55 merge commit）
>
> implementation：`61689c4a0169e25eda700bb3d4db668656fa44b8`
>
> 第一真源：v4 §5.1–§5.3、§13.1–§13.4、§15.3、§21.1、§24 G4/G6、§28.1 R146/R147。

## 1. 结论

本批把 Batch72 的 Rust host bridge 接成 Tauri Wry **actual commands**：

- `openbot_structured_events_open` 从 Tauri 注入的真实 `Webview` 取得 label，不接受 renderer 自报 label；
- open 先经 bound authority 建立 subscription，再 spawn 后台 Tauri Channel pump，立即返回 host-minted `subscriptionId`；
- `openbot_structured_events_close` 只在调用 Webview 自己的 actual label 内关闭该 ID；
- another-window、unknown、already-finished ID 统一返回 `false`，不泄露哪一种；
- invoke error 只返回 stable code，`WindowAlreadyOpen` 的内部 injective label 不跨 IPC；
- 两个 command 名、closed frame、open receipt 与 close request 单源位于 wasm-safe `openbot-contracts::desktop`；
- Wry registration 一次完成同一 `DesktopTauriProtocol` Arc 的 managed state、exact handler 与 custom scheme；
- 二项公开 command audit allowlist 与实际 `#[tauri::command]` 数量、函数名、`generate_handler!` 逐项机械 join。

本批没有让 WASM 调用这些命令，也没有运行真实 Tauri event loop/Webview。因此它关闭的是 actual host command registration，不是 Desktop realtime/native-window journey。

## 2. 根因

R146 的 `pump_structured_events` 若直接作为一个长寿命 invoke，会到 stream terminal 才返回。组件卸载时 renderer 尚未拿到可关闭句柄；更关键的是，JavaScript callback 被 unregister 后，Tauri 2.11.5 的 Rust Channel 仍可能成功执行 `Webview::eval(runCallback)`，不能把“callback 不存在”当成 host sink error。结果是 pump 可一直活到整个 window unbind。

正确生命周期必须是：

1. open command 用 host authority 建立 stream；
2. host 持有 subscription 并启动 pump；
3. open 立即返回不可伪造为 authority 的 opaque ID；
4. renderer unmount/reconnect 时调用 exact close；
5. close 再用 host-observed Webview label 收窄，ID 单独不具备 authority。

同时，WASM 下一批必须消费同一 frame/receipt；把 Batch72 的 serde enum 留在 `openbot-desktop` 会逼 UI 复制 wire，因此本批先上收到 contracts。

## 3. 实现面

### 3.1 Shared wire

新增 `openbot-contracts::desktop`：

- exact open/close command name constants；
- `DesktopStructuredStreamKind` 与 exact-thread event family validation；
- gap/terminal/delivery-class 与 `DesktopStructuredEventFrame`；
- `DesktopStructuredSubscriptionOpened`；
- `DesktopStructuredSubscriptionCloseRequest`。

所有输入 DTO 均 `deny_unknown_fields`；frame 中仍没有 window、actor、tenant、role、auth generation 或 internal label。Contracts 保持零 I/O，并在 `wasm32-unknown-unknown` 单独编译通过。

### 3.2 Exact close

`DesktopStructuredEventBridge::close_subscription(actual_window, id)` 在同一 registry mapping 内同时匹配 actual label 与 ID，移除登记后同步关闭 internal route。`DesktopTauriProtocol` 再要求该 host window 当前仍有 authority binding。

### 3.3 Actual commands

`register_tauri_protocol` 现在是 Wry production registration，并按固定顺序：

- manage 同一个 `Arc<DesktopTauriProtocol>`；
- `generate_handler!` 显式列出 open/close 两项；
- 注册 caller-selected closed custom scheme。

open command 在返回 receipt 前已经把完整 subscription move 进后台 pump；close command 的 label 只来自 Tauri `Webview` command argument。没有 renderer 可传的 label/actor/tenant 字段。

## 4. 测试矩阵

- contracts command name/receipt 精确 wire 与 unknown authority field 拒绝；
- shared stream family 对 wrong event/wrong thread fail-closed；
- terminal wire 省略非 overflow class 且 authority 字段 0；
- exact close 只关闭目标 internal route，另一 subscription/window 不受影响；
- protocol close 以 host label 收窄，跨 window ID/unknown/unbound 分支精确；
- stable error 把带私有 internal label 的 duplicate route 投影成固定 `desktop_subscription_conflict`；
- Wry builder 可实际安装 protocol/managed state/handler；
- production source 中 `#[tauri::command]` 恰 2，audit allowlist、函数名和 `generate_handler!` 恰好一一对应；
- Batch72 的 EOF/terminal、wrong family/thread、window rebind、sink close 等回归保持。

## 5. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| `cargo fmt --all` + `git diff --check` | 通过 |
| Contracts tests | `98/0/0` |
| Desktop default tests | `80/0/0`；doc-test 1 ignored 显式可见 |
| Desktop all-feature tests | `102/0/0`；doc-test 1 ignored 显式可见 |
| Contracts WASM | `wasm32-unknown-unknown` check 通过 |
| Clippy | Contracts/Desktop all-target/all-feature，`-D warnings` 通过 |
| macOS host | Desktop all-target/all-feature check 通过 |
| Windows target | `x86_64-pc-windows-msvc` all-target/all-feature check 通过；compile-only，不冒充 runtime |
| Tauri dependency guard | Linux host graph absent；13 build scripts；9 WebView2 payload；既有 MPL/UNIC/Vet blockers 不变 |
| parity | `813/881/1694`，0 violation；fixtures `17/22/39`；overlay `1445/241/2/6` |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff 0；inventory 2,110 files |
| invariants | Cargo.lock/workflow diff 0；非 Grok 恰一个 `package.json`；本批 package/lock/npm diff 0 |

### 5.1 revalidate 19

本批新增 `openbot-contracts/src/desktop.rs` 并在 contracts `lib.rs` 导出，overlay 的路径前缀算法因此以粗粒度 `openbot_contracts` 命中 19 条既有 done：

- Agent contracts：T-TEST-0321/0322/0323/0325/0326/0327/0328/0330/0331/0333/0335；
- thread identity：T-TEST-0978–0985。

机械复核结果：`agent.rs`、`ids/thread.rs`、Application/Server/Infra 对应生产模块 diff 均为 0；本轮 full Contracts 98/0/0 已重跑上述 contracts 测试。T-TEST-0328 原 done evidence 还含真实 PG directory，本轮没有改该实现、也没有重跑 PG，故只保留历史 PG 证据并明确不冒充本轮重跑。parity-check 最终 0 违反，输出 `diff required revalidate=19`。

本批无 UI/CSS/locale/Cargo dependency 变化，没有重跑 Trunk、Browser、Engine 或 golden。没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

## 6. 未闭合边界

- WASM 仍使用 Web EventSource/WebSocket，尚未探测 `window.isTauri` 或调用这两条 command；
- Tauri Channel callback 的 index ordering、end、invoke reject 与 unmount exact close 尚未在 WASM 实现；
- actual Tauri binary/tauri.conf/capability、verified session → `bind_window`、window destroyed → `unbind_window` assembly仍不存在；
- 当前每个 host subscription 仍有独立 internal broker route；单 Webview 多 stream 的 aggregate queue 尚未证明满足 §13.2“每窗口 256”；
- Windows 只有 cross compile，没有真实 WebView2 command/channel/window runtime；
- Screen loopback binary WebSocket、formal Desktop golden与T-UI-0126仍todo；
- 无新增/关闭 T-ID，G4/G6整关不勾。

下一批应实现 WASM 的 Tauri 2.11.5 Channel adapter与三条 realtime host selection，并以 command receipt 在unmount/reconnect时exact close；在真实 native assembly 前仍不得声称 Desktop journey完成。
