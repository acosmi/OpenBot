# Batch107：Golden RGBA / PNG 清单比对与正式闸门

日期：2026-09-04

外部候选：`ccd97df2945a84a5b79f85675dcc4289670685c8`

择取后 comparator implementation：`79d849f0db7a4b822a56ec4369e638b229345c8a`

PNG gate implementation：`a69b3efcf41edd1c7dd0e089885fbd6143fa02a7`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§16.3、§19.3、§24 G0/G6/G8、§25、§28.1 R182；GUI 第一真源
`docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md` §10

## 1. 本批结论与边界

Batch107只关闭Golden比较器与可执行闸门本身：`openbot-testkit::golden`实现已解码RGBA的确定性比较，
`cargo xtask golden`再接PNG解码、清单校验、exact 245-path inventory、reviewed mask与diff输出。

当前已证明：

- 任一RGBA通道差严格`>16`才算差异；差异像素比例严格`>0.1%`或出现任一`8×8`全差异块即失败；
- mask同时退出比较分子、分母与全差异块，完全mask、stride/尺寸不一致和算术溢出均fail-closed；
- `fixtures/ui/seed.json::pages_covered`机械导出137 Web、54 macOS arm64、54 Windows x64，共245条
  exact relative path；只凑245个任意文件、错名、重复、漏页、额外页或viewport尺寸不符均失败；
- 输入必须是≤16 MiB的真实PNG，宽高≤4096，decoder allocation≤64 MiB；不跟随matrix内symlink；
- mask sidecar只接受manifest reviewed allowlist中的页面与selector，最多64个resolved矩形；
- 失败时只写ignored的`fixtures/ui/golden/_diff/`确定性PNG，CI不得自动覆盖baseline；
- formal `verify`必须先有固定`sha256:<64hex>`容器digest、非TBD CJK包版本，以及baseline/actual完整相等
  inventory。当前`ready=false`只能让`check-manifest`证明结构正确，不能让正式verify变绿。

本批**没有**生成245张baseline，没有接Web CDP截图、macOS/Windows `xcap`，没有固定Ubuntu x64容器或
`fonts-noto-cjk`版本，也没有完成AX/键盘/reduced-motion矩阵。因此T-FIX-0004–0008、各页面formal
golden、G6与G8继续todo；“闸门按预期判红”不得冒充视觉基线通过。

## 2. 外部候选审计与择取

候选分支基于旧R172，主控没有合并整分支，只把单一候选commit择取到R181 coordination基线。逐项复核
GUI §10.4后确认其pure core保持以下精确边界：

1. 阈值比较使用绝对通道差且边界值16仍相等，17才不同；
2. ratio只在`different/comparable > 0.001`时失败，等于0.001不失败；
3. 8×8全差异块独立判红，不能被全图比例稀释；
4. mask为半开区间并在比较前规范化；越界、重叠与完全覆盖有确定语义；
5. 所有像素数、buffer长度和坐标运算使用checked路径，不允许wrap或panic替代错误。

择取后的`79d849f…`保留18条core测试；未把旧分支上的其它内容或台账一并带入。

## 3. Manifest v2 与文件名真源

`fixtures/ui/golden/MANIFEST.toml`升为schema v2。页面键不是人工第二份清单，而由seed route逐segment生成：

- 根路由固定`home`；普通segment原样进入key；segment间用`--`连接；
- 动态segment按同一page的`golden_param`替换；TanStack `$key_`尾下划线只作路由消歧，参数名仍为`key`；
- page key只允许ASCII字母数字、连字符和下划线；画廊固定`design-gallery`且只用1440×900；
- Web en为27页×2主题×2视口，加画廊×2，共110；Web zh-CN为27；Desktop每平台54；总计245；
- filename末尾的`<width>x<height>`既是清单身份，也是PNG真实尺寸的强制断言。

manifest还同步把已经裁决的CSS hard/warn预算校正为128/120 KiB，并把“crate仍为空”“当前是Windows”
等历史阻塞语句改成当前真实缺口。

## 4. 依赖与供应链边界

- 新增exact `image 0.25.10`，关闭default features且只启`png`；它是
  `openbot-testkit`的optional dependency，只由`xtask` feature启用；
- Cargo.lock只新增`image 0.25.10`、`byteorder-lite 0.1.0`、`moxcms 0.8.1`、`pxfm 0.1.30`四包，
  workspace lock package总数829；四包均无`build.rs`；
- `tools/check-ui-dependencies.sh`锁exact edge、features、Cargo checksum、四包license和零build.rs；
  Server/Desktop及默认testkit产品图均不含这四包；
- `provenance/sources.spdx.json`把image sourceInfo绑定Cargo.lock checksum
  `85ab80394333c02fe689eaf900ab500fbd0c2213da414687ebf995a65d5a6104`；
- 六发行target的`bans/sources`守卫保持绿。全仓`cargo deny check licenses bans sources`仍因既有
  Tauri MPL/build-script范围判红；`cargo vet check --locked`当前373个unvetted（含本批四包），未添加
  exemption，故不得写成供应链整关通过。

## 5. 实跑证据

| 检查 | 结果 |
|---|---|
| `cargo test -p openbot-testkit --lib --locked` | `19/0/0`；其中Golden core 18条，既有fintech 1条 |
| `cargo test -p openbot-testkit --bin xtask --features xtask --locked golden_gate` | Golden gate `7/0/0` |
| `cargo test -p openbot-testkit --all-features --locked` | 聚合`137/0/10 ignored` |
| `cargo clippy -p openbot-testkit --all-targets --all-features --locked -- -D warnings` | 通过 |
| `cargo xtask golden check-manifest` | `matrix=245; masks=0; ready=false` |
| 1024×1024真实PNG self-compare | `1048576` comparable、`0` different、match=true |
| `cargo xtask golden verify --actual-root <empty>` | exit 1；先因容器/font provenance TBD拒绝，符合预期 |
| `bash tools/check-ui-dependencies.sh` | PNG-only/testkit-only与四包license/build.rs/checksum guard通过 |
| `bash tools/check-deny-release-targets.sh` | 六发行target bans/sources与workspace shape通过 |
| R124 revalidation | T-TEST-1040由core全套覆盖；T-TEST-0652/0657经PG17.11 SCRAM真实OpenAI HTTP stream测试`1/0/0` |
| `cargo xtask parity-check --json` | parity=`861/849/1710`，tests=`470/577/1047`，fixtures=`26/22/48`，overlay=`1283/419/2/6`，0 violation/warning |
| `cargo xtask recount` | `71 passed / 0 mismatch / 89 skipped`；无固定上游目录，strict未运行 |

xtask完整单测另实得`102/0/0`。没有运行R63禁止的`cargo xtask ci`，没有派发GitHub Actions。

## 6. 台账变化与后续入口

- T-FIX-0003保持done，但done evidence升级为可执行manifest/PNG gate；fixture总数没有凭空增加；
- 修正继承的机械recount漂移：`fixtures/MANIFEST.yaml`实际48/26/22，`parity/tests.yaml`实际todo577；
- 因本批真实重跑三条R124目标，overlay从`1286/416/2/6`变为`1283/419/2/6`，parity总数不变；
- 下一步应先固定Ubuntu x64 Web golden容器与CJK包版本，再接release bundle的CDP capture并完成双录；
  macOS/Windows Desktop分别等正式发行窗口与钉版xcap。任何平台只对自己的baseline比对，不做
  WKWebView/WebView2/Chromium跨引擎逐像素相等。
