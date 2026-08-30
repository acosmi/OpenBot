# Batch76：品牌无关 native window lifecycle primitive

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G6-native-window-lifecycle`
>
> base：`e382ebc767925e50e6477fd2638623fe96143d54`（PR #58 merge commit）
>
> implementation：`c424228a85b088f5167cdaa64493c6f6cf6e1b53`
>
> 第一真源：v4 §6.1、§13.1–§13.3、§15.3、§23.4、§24 G6、§28.1 R147–R150；GUI v2 §15.1。

## 1. 结论

本批把 Batch69–75 的手工 host protocol 测试推进为可复用的 actual Tauri window lifecycle primitive：

- `VerifiedDesktopWindowAuthority` 只包装 Rust 已经从可信 session source 得到的 `AuthContext`；renderer 没有可序列化输入面；
- lifecycle 在 Webview build 前绑定 host label；actual `WebviewWindowBuilder` build 或本地 registry 失败时先 destroy/unbind 回滚；
- 主 Webview 固定 `devtools(false)`，拒绝 `window.open` 与 download；
- top-level navigation 只接受当前internal custom origin，拒绝remote scheme/host、userinfo与非默认port；
- macOS 使用 `<scheme>://localhost`；Windows另精确接受钉版Tauri/Wry默认映射`http://<scheme>.localhost`；
- production protocol registration 同时 manage lifecycle 并挂全局 `on_window_event`；只有 `WindowEvent::Destroyed` exact unbind，Focused等其他event不提前撤权；
- shutdown primitive可一次撤销全部actual window authority和其structured subscriptions。

本批没有创建 `tauri.conf.json`、capability、binary、session verifier或发行identity；MockRuntime证据不是真实Wry/WebView2运行时。

## 2. 根因

Batch73 已把actual open/close command注册到Tauri Builder，Batch74/75又闭合WASM选择和同窗预算，但所有测试仍直接调用`DesktopTauriProtocol::bind_window`。缺少的owner职责包括：

1. 谁保证authority早于首个local request；
2. actual window build失败后谁撤销binding；
3. 哪个真实Tauri event代表窗口已经不存在；
4. 主Webview是否能导航到remote origin、开新窗、下载或开devtools；
5. macOS与Windows custom protocol URL形态如何闭集。

这些不能推迟到最终品牌文件才第一次实现。相反，产品名/bundle ID/deep-link没有reviewed输入，按§23.4又不能由代码批次擅自发明。因此本批只落品牌无关primitive，并把发行assembly继续留红。

## 3. 实现面

### 3.1 Verified authority 与 build rollback

`VerifiedDesktopWindowAuthority::from_verified_session` 的命名本身要求调用点指出session验证来源；字段私有且不serde。`create_verified_window`先调用既有`bind_window`，再构造Tauri `WebviewWindowBuilder`。invalid label或平台build错误时，先`unbind_window`再返回stable `desktop_window_build_failed`；若本地registry异常，则destroy actual window并撤权。

### 3.2 Closed Webview surface

builder固定：

- devtools=false；
- `on_new_window`一律Deny；
- `on_download`一律false；
- navigation要求无userinfo、无非默认port且origin恰为当前internal protocol。

钉版Tauri 2.11.5源码明确：macOS/iOS/Linux为`<scheme>://localhost/path`，Windows/Android默认由Wry映射成`http://<scheme>.localhost/path`。本批只发行macOS/Windows，因此测试同时固定两种闭集；Windows没有启`use_https_scheme`，HTTPS workaround不被误放。

### 3.3 Destroyed lifecycle

`register_tauri_protocol`现在创建并manage同一`DesktopWindowLifecycle`，再安装`on_window_event`。handler只对`Destroyed`调用`unbind_window`；`CloseRequested`可能被prevent，不能当成销毁证据。unbind继续复用Batch75的binding cancellation、pending subscribe cancel、registered route exact close。

`shutdown_authority`用于未来binary退出路径；本批未伪造不存在的App run-loop assembly。

## 4. MockRuntime 与负向证明

- dependency-free Tauri `test` feature只作为macOS/Windows target-scoped dev dependency，production feature图不变；
- MockRuntime actual build `openbot://localhost/` WebviewWindow，managed lifecycle state存在、label与start URL逐字相等，authority已绑定；
- Focused(false)不unbind；Destroyed unbind且重复Destroyed幂等false；
- invalid Tauri label让build失败，先前authority确实回滚；
- 两个actual MockRuntime window由shutdown authority精确2→0；
- navigation正向覆盖macOS native形态与Windows HTTP workaround；负向覆盖HTTPS workaround、错host、remote scheme、userinfo、非默认port与file URL；
- 既有custom protocol/commands/structured budget/stale binding测试全部回归。

## 5. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` + `git diff --check` | 通过 |
| Desktop all-feature最终 | `109/0/0`；doc-test 1 ignored显式可见 |
| Clippy | 首跑仅`needless_lifetimes`失败，消除后Desktop all-target/all-feature `-D warnings`通过 |
| macOS host | Tauri MockRuntime actual builder与all-feature unit通过；不是Wry真运行时 |
| Windows target | 中间compile暴露平台enum dead-code warning；改编译期bool后`x86_64-pc-windows-msvc` all-target/all-feature最终无warning通过；compile-only |
| Tauri dependency guard | Linux host graph absent；13 build scripts；9 WebView2 payload；既有MPL/UNIC/Vet blockers不变 |
| parity | `813/881/1694`，0 violation；fixtures `17/22/39`；overlay `1445/241/2/6`；diff required revalidate=0 |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff 0；inventory 2,110 files |
| invariants | Cargo.lock/workflow diff 0；非Grok恰一个`package.json`；本批新增package/npm=0 |

本批无contracts/UI/T-ID/CSS/locale变化，没有重跑Trunk、Browser、Engine或golden。没有运行`cargo xtask ci`，没有派发GitHub Actions。

## 6. 未闭合边界

- 没有实现Desktop Local“当前OS用户+本地app instance”或Desktop Remote session verifier；caller仍需提供已验证`AuthContext`；
- 没有`tauri.conf.json`、capability allowlist、reviewed产品名/bundle ID/deep-link、main binary或setup/run-loop；
- MockRuntime不执行macOS WKWebView/Windows WebView2；没有真实load、command/Channel、Destroyed、XSS或shutdown journey；
- 仅撤window authority，不替代未来App退出时`InProcessTransport::shutdown`的5秒收拢证据；
- Tauri/Wry callback scheduler背压、Windows真机、formal Desktop golden/AX和供应链blocker仍todo；
- G6整关不勾，T-UI-0126不勾。

下一批优先审计Desktop Local single-user authority source与可测试setup输入；reviewed发行identity缺失时仍不得创建伪`tauri.conf`或对外binary。
