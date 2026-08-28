# G6 UI 供应链 delta 审计（Batch 15）

> 日期：2026-08-25。范围只覆盖 Batch 15 因第一真源钉定的 Leptos/ICU GUI、Axum
> static files 与本地构建工具新增的供应链面。本文不是第三方完整源码审计，也不解除 G2
> 独立安全外审；尤其不把 cargo-vet exemption 写成 full audit。

## 1. 输入与负向基线

GUI 第一真源固定 `leptos = 0.8.19`、`leptos_router = 0.8.13`、`leptos_meta =
0.8.6`、`leptos_i18n(_build) = 0.6.2`，且 i18n 必须开启 `csr/plurals/
format_datetime/format_nums/icu_compiled_data`。本批未以“减依赖”为由删掉这些功能。

加入真实 GUI 图后，机器先红再审：

- `cargo deny check licenses`：CC0-1.0、BSL-1.0 两项 rejected；
- `cargo deny check bans`：30 份 `build-script-not-allowed`，另发现 2 份可执行维护脚本；
- `cargo audit --no-fetch --deny warnings --ignore RUSTSEC-2023-0071`：新增两条
  unmaintained warning；
- `cargo vet --locked`：**185 unvetted dependencies**，全缺 `safe-to-deploy`。

没有把红灯隐藏为“工具问题”：下面每项都从本机 registry 原件读取，裁决后才加入精确
allow/ignore；cargo-vet 仍保持红色，未生成任何新 exemption。

## 2. 新许可

| crate / 路径 | 许可 | 原件 SHA-256 | 消费与裁决 |
|---|---|---|---|
| `base16 0.2.1 / LICENSE-CC0` | CC0-1.0 | `a2010f343487d3f7618affe54f789f5487602331c0a8d03f49e9a7c547cf0499` | 只经 `wasm_split_macros → leptos`；CC0 最大范围放弃著作权，失效时给商业使用/修改/分发 fallback，无 copyleft |
| `xxhash-rust 0.8.18 / LICENSE` | BSL-1.0 | `c9bff75738922193e67fa726fa225535870d2aa1059f91452c411736284ad566` | 只经 `server_fn(_macro) → leptos`；Boost 1.0 是 OSI/FSF 宽松许可，源形式须保留声明，机器目标码例外；仍进入 NOTICE/provenance |

`tools/check-ui-dependencies.sh` 同时锁版本、原件 hash 与 `deny.toml` 的两项许可决定。

## 3. build.rs 逐文件结果

全部 30 份合计 961 行。危险模式复扫只有：rustc/version probe、OUT_DIR 写、两份 Windows
crate 的本包 `lib/` link-search；`TcpStream/UdpSocket/reqwest/ureq/curl/wget/download`
零执行命中。URL 只在注释或 `leptos-use` 的 warning 文案。

| crate（同组表示脚本逐字同 hash） | 行数 | build.rs SHA-256 | 实际副作用 |
|---|---:|---|---|
| `camino 1.2.5` | 104 | `5bc29910c9644c320a7cceed121474915f7e832f484f1bf694dec80a45182aa0` | 执行 `$RUSTC --version`，只 emit cfg |
| `cookie 0.18.2` | 5 | `75c45e6b8566ca721dd5759b6ef16e365d5cb201660eee5ec04e278f9d1eefe2` | `version_check` 探 `doc_cfg`，只 emit cfg |
| `crossbeam-deque 0.8.7` | 14 | `e40cf96d7d7b1650f9f53a3f578633a178324dbea1d905b3f71a75b45d3982a1` | 只读 sanitizer env，emit cfg |
| `erased-serde 0.4.10` | 30 | `cc81259cd7861fc7c4b054656fc50bc381b5e96f22501e1c77f045ca93d41f77` | `$RUSTC --version`，只 emit cfg |
| `icu_{calendar,datetime,decimal,locale,plurals}_data 2.2.0` + `icu_time_data 2.2.1` | 各 11 | `c2d446772e3d766a804963dbf36e51729f910920f91f4b68c0c199fe6ca0853e` | 六份逐字相同；只读 `ICU4X_DATA_DIR`，emit cfg |
| `leptos 0.8.19` | 15 | `e212d639297796bd51e905411d3d9c77bf99de7e187769e420118df5d01f7cd4` | rustc channel + target cfg；WASM 固定 getrandom backend |
| `leptos-use 0.18.3` | 17 | `bcec3171f169950ffc50fbcc6c7de59f3636e61299f3c7958cde27202263b26d` | 只读 feature/target，错误组合时 warning |
| `leptos_macro 0.8.17`、`leptos_router 0.8.13`、`reactive_graph 0.2.14`、`server_fn 0.8.13`、`server_fn_macro 0.8.10`、`tachys 0.2.18` | 各 8 | `11ccc42a6266f3bb42677ff796dd8f456a72460f0a20a83cce22bb29dfec534e` | 六份逐字相同；rustc channel 探测，只 emit nightly cfg |
| `matrixmultiply 0.3.11` | 28 | `70108cb12936fdbe2123b2018c42899a531c7cb0007b3e402a8fdea7411e88a7` | autocfg 试编 rustc 版本/AVX512，零外部数据 |
| `mime_guess 2.0.5` | 196 | `bc413487e376b343b65089a9a897f4bb3c9d5fbaa5a6833e87db1d3c18c462d8` | 从包内静态 MIME 表生成 `OUT_DIR/mime_types_generated.rs` |
| `paste 1.0.15` | 38 | `dba46ae4291317fb644ba2143f44eaf54a8ab946ba1367a33d055d694715f68a` | `$RUSTC --version`，只 emit cfg |
| `proc-macro2-diagnostics 0.10.1` | 7 | `66fcc487972086f42011c84a1949861799dc7cfde1e56201d22cf8e71b59b8b1` | `version_check`，只 emit nightly cfg |
| `rayon-core 1.13.0` | 7 | `fa31cb198b772600d100a7c403ddedccef637d2e6b2da431fa7f02ca41307fc6` | 只有 rerun-if-changed；无链接动作 |
| `rustix 1.1.4` | 286 | `74cb32e64aa6fe99c2496a425b016e22f4e43c438a8237966b8acae04a98eaf9` | 读 target/env；只调用 rustc/wrapper 做 metadata 试编，输出只进 OUT_DIR |
| `slotmap 1.1.1` | 17 | `fa4b3bd978b8f9c9a619b6fd61b4471a9b0a386335dd3b1fc8997daa8a16c4ff` | `version_check`，只 emit cfg/warning |
| `typeid 1.0.3` | 33 | `688afbcaa398ea159c3481b26d74fde6ce3a675d48364d772557c8e91100de46` | `$RUSTC --version`，只 emit cfg |
| `wasmparser 0.239.0` | 27 | `ba7ab1735d3642c53562d1223a6eb54c2392e619ade3005e0deee3fc4229feea` | `$RUSTC --version`，只 emit cfg |
| `windows_x86_64_{gnu,msvc} 0.52.6` | 各 8 | `6d40bd2c0ed4cbea5126dfcd89d72f229c7d986540cbf0dc34acc1017f1de20f` | 两份逐字相同；只把同 crate 的 `lib/` 加入 Windows link-search |
| `zip 2.4.2/src/build.rs` | 7 | `8a048f0daacc5e4067f432b107cafe331426f6aecd4f76759a8be42d5556027e` | 只读 feature，弃用组合时 warning |

Windows 两份真实 import archive 另锁：GNU `.a` =
`33f0f658b3d2108a4b7ba7809e2dcb5ad0431d9c474be5adc5efa2944f24f665`，MSVC `.lib` =
`24d8cbc445955b0d48041948a3c71ce2cddb948c089b25ec7106fadd9f3efde0`。它们只按精确
crate@version + 单一路径 bypass，不能称为纯源码输入。

另有两份 cargo-deny 先判红的维护者脚本：

- `cookie 0.18.2/scripts/test.sh`：只被该 crate 的 `.travis.yml` 引用，依赖构建不可达；
  SHA `a76191d56d96c32efcb6883e0983e86beb4c6842e6e5c5a8bfded4c8183ff6f6`；
- `leptos-use 0.18.3/template/createfn.sh`：README 明示贡献者手动运行的 ffizer scaffold，
  本仓不装/不调用 ffizer；SHA `b5cc498eca3f1bc4d9cf75e6ce30b37c59788e9030fb030c72184178097c820b`。

两者只放行精确路径；新增脚本仍会判红。

## 4. 两条停止维护通告

| ID | crate | 机器事实 | 窄裁决 |
|---|---|---|---|
| RUSTSEC-2024-0436 | `paste 1.0.15` | `informational="unmaintained"`，`patched=[]`；crate 自身 `proc-macro=true` | 仅编译期；第一真源钉定 Leptos 传递引入；版本、proc-macro 身份或 patched 状态变化即先红 |
| RUSTSEC-2026-0173 | `proc-macro-error2 2.0.1` | `informational="unmaintained"`，`patched=[]`；四个直接消费者全部是 proc-macro | 仅编译期；消费者集合或 patched 状态变化即先红 |

这两条不是漏洞修复豁免，也不等于风险消失。直接换 pastey/manyhow 会改动第一真源钉定框架
的上游源码/依赖，不能在没有独立 delta 的情况下擅自做。manual-only workflow 的 cargo audit
显式列出三条 ignore（含既有 R44 RSA），guard 锁闭合前提。

## 5. 工具链边界

`cargo xtask tools verify` 对发行构建用四工具精确通过：Tailwind 4.3.3、Trunk 0.21.14、
Binaryen version_132、wasm-bindgen-cli 0.2.127。Trunk 自己的上游安装 lock 仍含 yanked
`crossbeam-channel 0.5.14` 与 `zip 2.6.1`；wasm-bindgen-cli 构建还报告 `buf_redux 0.8.4`、
`multipart 0.18.0` future-incompat。它们不进入 workspace Cargo.lock 或产品发行物，但仍是
构建机供应链风险，本文如实保留，不能写成“所有工具依赖已审”。

## 6. 本机最终证据与未闭合项

- `bash tools/check-ui-dependencies.sh`：ok（30 build scripts、2 licenses、2 compile-time
  unmaintained advisories、2 Windows archives、2 unreachable maintainer scripts）；
- `cargo deny --offline --format json check --hide-inclusion-graph`：exit 0；四段 errors=0；
  30 条 multiple-version warning，另 1 条 OFL asset 不在 Cargo graph 的 warning；
- `cargo audit --no-fetch --deny warnings --ignore RUSTSEC-2023-0071 --ignore
  RUSTSEC-2024-0436 --ignore RUSTSEC-2026-0173`：加载 1225 advisory，扫描 640 个依赖，
  exit 0；
- `cargo vet --locked`：**185 unvetted**，仍红。没有改 `supply-chain/config.toml`，没有自动
  exemption；因此本批不宣称供应链整关通过，也不勾 G6。
- 未运行 `cargo xtask ci`，未派发 GitHub Actions。
