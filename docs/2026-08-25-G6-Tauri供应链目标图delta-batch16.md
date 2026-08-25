# G6 Batch 16：Tauri 2.11.5 供应链目标图 delta 审计

> 日期：2026-08-25。范围只含 GUI 第一真源 §10.1 的 Desktop 发行目标：macOS arm64 与
> Windows x64。Linux x64/arm64 是 Server/Web 面，WASM 是共同 GUI bundle；它们不是 Linux
> Desktop。本文记录机器事实，不是 G6 完成证书，也不把待裁决项写绿。

## 1. 为什么 Cargo.lock 全联合不能直接当发行图

Tauri/Wry 进入后 `Cargo.lock` 从 640 增至 822 package。Cargo.lock 必须保留跨平台解析所需的
所有包，因此它会同时含 macOS Objective-C、Windows WebView2 和 Linux GTK/WebKitGTK。把
这 822 个包直接拼成一张图，会构造出“macOS parent → Linux-only child”这种任何一次 Cargo
构建都不可达的组合。

本批把 `openbot-desktop` 的 Tauri host-only 五条依赖共同放入：

```toml
[target.'cfg(any(target_os = "macos", target_os = "windows"))'.dependencies]
```

`tauri_host` 模块与 re-export 同样以 feature + target 双门控。机器反向证明：Linux x64、
Linux arm64、WASM 三图均无 Tauri/Wry/GTK；macOS 与 Windows 均精确存在
`tauri 2.11.5`、`tauri-runtime 2.11.3`、`tauri-runtime-wry 2.11.4`、`wry 0.55.1`。

## 2. 真实 build.rs 面

macOS 真实集合 11 个，Windows 真实集合 10 个，并集 13 个；旧全联合的 33 个不是发行图。

| package | 实际 build target | 行 | SHA-256 | 行为与目标 |
| --- | --- | ---: | --- | --- |
| indexmap 1.9.3 | `build.rs` | 8 | `558b4d0b9e9b3a44f7e1a2b69f7a7567ea721cd45cb54f4e458e850bf702f35c` | autocfg 探测 |
| objc2 0.6.4 | `build.rs` | 39 | `f13d2effabc1cfa07fa5018c78eadc645914d676c034adf78acb24b8b419ce7a` | Apple target/cfg |
| objc2-exception-helper 0.1.1 | `build.rs` | 45 | `6c338b9ad9f2d47c6c9d4e3d9d604334828da36dfda4bc4d999b99aab005ceba` | `cc` 编译包内 `try_catch.m` |
| schemars 0.8.22 | `build.rs` | 25 | `5ef3c87640a839e95aa892c4dbc9557d8b6437caa697b53a598954cf471e2303` | target/cfg |
| selectors 0.36.1 | `build.rs` | 77 | `36ba09a8d2089d0cae8e310829ecf0e94bcbaa87e775a6578c7d2f0459a5b6ca` | 包内表 → OUT_DIR PHF Rust |
| swift-rs 1.0.8 | `src-rs/test-build.rs` | 21 | `82941bdb037e5479003967346ae2f1932391770c8515f4b10cc90569a9b171a1` | 默认 no-op；只在 `TEST_SWIFT_RS=true` 进 toolchain |
| tauri 2.11.5 | `build.rs` | 504 | `62d1a1e16affe3c9b59d6766159a568255e385cab969ce4149ad6081276715bd` | desktop cfg、OUT_DIR permission、确定性 docs；移动端/上游 workspace 分支不可达 |
| tauri-runtime 2.11.3 | `build.rs` | 19 | `68b2727346e58a9963803a75ac29695b500aaa7d0673e18551465502b60cbf11` | desktop/mobile cfg |
| tauri-runtime-wry 2.11.4 | `build.rs` | 19 | `68b2727346e58a9963803a75ac29695b500aaa7d0673e18551465502b60cbf11` | 同上 |
| vswhom-sys 0.1.3 | `build.rs` | 23 | `3adb4b0f64aa6af4ca91aa3b0bacf81eb75e98b5587a625568ed825eb18a6f17` | Windows 编译包内 C++，链接 Ole |
| web_atoms 0.2.6 | `build.rs` | 130 | `8b50922bbb295e90a26edc3e2ab34f068fee930cefd73ae03cca210cf08f9d89` | 包内静态表 → OUT_DIR Rust |
| webview2-com-sys 0.38.2 | `build.rs` | 91 | `ea73d2566f434a25e8172d0fb9eaad5fa29ee687a8f05b9c2be02b19e4366e16` | 按 Windows target 拷贝包内 loader |
| wry 0.55.1 | `build.rs` | 118 | `3c3153deae92302ed707b06ecfafffbf256f0ef4157b813c3120887c8010d7db` | Desktop 只 emit/link；Android Kotlin 分支不可达 |

`swift-rs` 同包的 574 行 dormant `src-rs/build.rs` 也锁为
`e18db702ab5655fa7659047b0892b4caff44bc5a497ccc4fa3c6c0246a7a6a19`；其中含
Swift/xcrun/clang/nm/llvm-objcopy。`.cargo/config.toml` 以
`TEST_SWIFT_RS = { value = "false", force = true }` 构造性封锁环境注入。

13 份脚本均无网络或下载。`deny.toml` 只允许这 13 个名称；任何版本/入口/字节变化先由
`tools/check-tauri-dependencies.sh` 判红。

## 3. WebView2 包内二进制

`webview2-com-sys 0.38.2` 带三架构各三份 Microsoft loader。Windows x64 发行实际消费 x64，
但 crates.io archive 包含全部九份，故全部精确 bypass + SHA guard，不按目录宽放：

| arch/file | SHA-256 |
| --- | --- |
| arm64 `WebView2Loader.dll` | `df5816669f5123595c475d97929240d7d0e04f0bdc7dbe18af1dda42348b73a6` |
| arm64 `WebView2Loader.dll.lib` | `d70271daa44507865ca0696cd1c1ede5e58e694eec74cb2d06e17dbbe205e9a2` |
| arm64 `WebView2LoaderStatic.lib` | `506ffde430bee7f91f2ce1a078effb5289b7cec3b0c7283647f0842def524ab4` |
| x64 `WebView2Loader.dll` | `8427b1fc58ec707813e5c0a51eb5d69397bb333250a7b891be4d3b123f1e0f1c` |
| x64 `WebView2Loader.dll.lib` | `bfc8ccaaa056be95243a5b66a827e5849d2bb39676fca4dcc2053796d8e15c6d` |
| x64 `WebView2LoaderStatic.lib` | `0659b741bde6348d4c4a6ec4ceb9af50e3d0048ed9cd3c8659bccbb61fde55ee` |
| x86 `WebView2Loader.dll` | `44ab92c2246ebfb5f98aa5726626fb44beb61543f2ef1803338af9fd295e63f0` |
| x86 `WebView2Loader.dll.lib` | `a3ec0ee539d58fe72391f2e89cb814f96fab721c6d8d30953d152ea95dffad49` |
| x86 `WebView2LoaderStatic.lib` | `6649ce9ca24e7a5693ee54178f42e0378004ce537d82d15354e6c9adb467bc16` |

## 4. 仍为红灯的真实集合

### 4.1 MPL-2.0

macOS/Windows 各精确 5 个，Linux/WASM 为 0：

- `cssparser 0.36.0`、`cssparser-macros 0.6.1`；
- `dtoa-short 0.3.5`；
- `option-ext 0.2.0`；
- `selectors 0.36.1`。

原件/声明已 hash guard，但尚未写 `deny.toml` license allow、NOTICE、SPDX/source offer，因此
`cargo deny licenses` 正确保持失败。MPL 2.0 是 file-level copyleft；Mozilla 原文与 FAQ：
<https://www.mozilla.org/en-US/MPL/2.0/>、<https://www.mozilla.org/en-US/MPL/2.0/FAQ/>。

### 4.2 RustSec

target-aware cargo-deny 在 macOS/Windows 各只报 5 个 `informational="unmaintained"`、
`patched=[]`，Linux/WASM 为 0：

- `RUSTSEC-2025-0075` unic-char-range 0.9.0；
- `RUSTSEC-2025-0080` unic-common 0.9.0；
- `RUSTSEC-2025-0081` unic-char-property 0.9.0；
- `RUSTSEC-2025-0098` unic-ucd-version 0.9.0；
- `RUSTSEC-2025-0100` unic-ucd-ident 0.9.0。

它们经 `urlpattern 0.3.0 → tauri-utils 2.9.3` 进入 runtime，不可谎称 compile-only；也没有
已知漏洞或可升级修复版。

既有三条 ignore 后，lock-only `cargo audit` 仍报 15：除上述五条外的十条为
`proc-macro-error 1.0.4`、GTK/ATK 八包和 `glib 0.18.5` unsound。它们只属于 Cargo.lock 中
未发行的 Linux Tauri/GTK 分支；六个真实 target 图均无这些 package，negative guard 已锁。
这解释 lock-only 输出，不等于给五条真实 UNIC 自动豁免。

### 4.3 Cargo Vet

| 图 | unvetted | 对 no-all-features 基线 181 的净增 |
| --- | ---: | ---: |
| macOS arm64 all-features | 270 | 89 |
| Windows x64 all-features | 269 | 88 |

`supply-chain/config.toml`、audits/imports 零改动，未调用 regenerate/add-exemption。故 Cargo Vet
仍红，G6 不勾。

## 5. 可复算命令与结果

```bash
./tools/check-tauri-dependencies.sh
# ok: Linux host graph absent; 13 build scripts; 9 WebView2 payloads

./tools/check-deny-release-targets.sh
# 联合结构 bans ok；Linux x64/arm64、macOS x64/arm64、Windows x64、WASM bans/sources 全 ok
```

另以 `cargo deny --target <target> --all-features --locked --offline` 分别读 license/advisory，
以及 `cargo vet --cargo-arg=--filter-platform=<target>` 统计上表。测试只在本机定向执行；未运行
`cargo xtask ci`，未派发 GitHub Actions。
