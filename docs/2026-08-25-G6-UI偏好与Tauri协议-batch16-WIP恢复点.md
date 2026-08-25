# Batch 16 WIP 恢复点：UI 偏好与 Tauri custom protocol

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-ui-preferences-desktop`，基线为 Batch 15
> 文档 head `ad1137e`，首个 WIP commit `17d43eb`。本文件是可断电恢复的 checkpoint，
> **不是 G6 完成证书**；旧的 Cargo.lock 全联合数字已按第一真源发行矩阵复核并在下文更正。

## 已落源码

- [x] `openbot-contracts::ui`：封闭 `system/light/dark` 与 `en/zh-CN`，stored fields 独立
  optional；partial update 至少一项，actor/deployment/tenant 无 wire 输入；
- [x] `ApplicationService`：`GetUiPreferences` / `UpdateUiPreferences` 与唯一
  `UiPreferenceAdministration` port；空 update 在 port 前 400；
- [x] native 0021：`user_ui_preferences` 以 `(deployment,tenant,actor)` 为 PK，theme/locale closed
  CHECK、nonempty CHECK、user cascade；partial upsert 以 COALESCE 原子合并并用 DB clock；
- [x] Server `GET/PUT /api/me/preferences`：GET authenticated，PUT same-origin guard 在 body 前；
  no-store；镜像 cookie 只有 closed theme/locale，`Path=/; Max-Age=31536000; SameSite=Lax;
  HttpOnly`，`Secure` 仍只由 public URL 的既有配置事实决定；
- [x] UI 启动读取 stored preference；主题/语言即时生效，partial writes 单队列串行+合并；保存失败
  显示本地化 `role=alert`，不静默；浏览器 reload 前后 class/lang/ARIA 保持；
- [x] Desktop local settings：三行 256-byte closed file、0600（Unix）、temp+fsync+rename+目录 fsync，
  不把 JSON codec 引进默认 typed in-process lane；
- [x] opt-in `tauri-host` production adapter：精确 Tauri 2.11.5/Wry，custom protocol 按
  webview label 读取 host-bound
  `AuthContext`；未绑定窗口连 asset 都 401；preferences/approval 经 typed
  `InProcessTransport`；fresh approval 用单调时钟 deadline，不是永久 bool；index 从本地偏好/
  OS locale Rust 改写并带 strict CSP；asset canonical path、8 MiB cap、闭合 MIME/extension；
- [ ] 可发布 binary/`tauri.conf.json` 尚未创建：对外产品名/bundle id/deep-link 仍无输入，不能
  用含 OpenBot 的内部代号当发行身份。

Tauri API 依据官方 2.11.5 文档：

- <https://docs.rs/tauri/2.11.5/tauri/struct.Builder.html#method.register_asynchronous_uri_scheme_protocol>
- <https://docs.rs/tauri/2.11.5/tauri/struct.UriSchemeContext.html#method.webview_label>

## 当前机器证据

- contracts UI wire：1/0/0；application UI preference：1/0/0；Server preference framing/cookie：
  1/0/0；
- Desktop local preference：2/0/0；default in-process no-codec：1/0/0；Tauri handler：2/0/0；
- native 0020 历史边界 + native 0021：**4/0/0**（PG17.11 TCP SCRAM）；0021 证明并发 theme/
  locale partial merge、deployment/tenant 隔离、closed checks 与 actor cascade；
- `fixtures/db/schema-0021.json`：43 表 / 404 列 / 295 NOT NULL / 222 约束 / 86 索引 /
  4 trigger / 4 enum / 1 public function / 0 extension；4775 行；SHA-256
  `fab4e148cb4f847e2f7079eae95b158ff7d4d0ed740ca2007fbb2de8ab7e3531`；
- Tauri 2.11.5 macOS native all-targets check、Tauri handler test 与 Desktop Clippy
  `-D warnings` 通过；contracts/application/infra/server/fixture targeted Clippy 全绿；
- UI 19/0/0、WASM/Clippy 绿；Trunk gates：262 i18n keys、19 Rust files/74 icons、44 CSS
  classes、WASM gzip 364812、CSS 27137、fonts 740216、external/inline=1/0；
- 真实 Chromium fixture：system/zh-CN → keyboard dark/en，800ms 内无 alert；reload 的 immediate
  与 settled 均为 `class=dark/lang=en`，radio=`false,false,true`、trigger=English；
- loopback GET preferences 实得 `Set-Cookie: openbot-ui=v1.dark.en; ... SameSite=Lax;
  HttpOnly`，手工 cookie 请求根 HTML 实得 `<html lang="zh-CN" class="light">`；
- Tauri dependency guard 绿：Linux x64/arm64 与 WASM 图无 Tauri/Wry/GTK，macOS/Windows
  精确图存在；13 个真实 build.rs、Swift dormant toolchain、5 份 MPL 证据、5 份 UNIC
  advisory record 与 9 个 WebView2 payload 均锁 version/path/SHA；
- target-aware cargo-deny：六个发行目标的 bans/sources 全绿；联合扫描仍单独检查 workspace
  dependency/wildcard/duplicate，未把 cargo-deny 的跨 target parent/child 伪组合当发行图；
- 本轮复跑 Desktop Tauri handler `2/0/0`、local preference `2/0/0`、typed no-codec `1/0/0`，
  Desktop Tauri all-targets Clippy `-D warnings`、fmt、两份 shell guard syntax 全绿；
- 临时 PG17 已停止，明文测试口令文件已删除；未运行 `cargo xtask ci`，未派发 Actions。

## 供应链目标图复核（旧联合口径作废）

加入第一真源钉定的 `tauri = 2.11.5` + `wry` 后，workspace Cargo.lock 从 640 增至
**822 packages（+182）**。但 Cargo.lock 是“所有 target 可能包”的并集，不是任何发行物的
依赖图。GUI 第一真源 §10.1 只指定 macOS arm64 / Windows x64 Desktop；Linux 是 Server OCI
与 Web golden runner，**没有 Linux Desktop**。因此：

- [x] `tauri` 及 host-only http/serde/sys-locale 已共同限定到 macOS/Windows，Linux 两架构和
  WASM 的 `openbot-desktop --features tauri-host` 图均无 Tauri/Wry/GTK；
- [x] 旧“33 个 build.rs 未审”作废：真实 macOS=11、Windows=10、并集=13，已逐份通读、
  写行为说明和 exact hash guard；`TEST_SWIFT_RS=false` 由 `.cargo/config.toml force=true`
  构造性封死；六目标 bans/sources 全绿；
- [ ] macOS/Windows 各仍有同 5 个 MPL-2.0：`cssparser 0.36.0`、
  `cssparser-macros 0.6.1`、`dtoa-short 0.3.5`、`option-ext 0.2.0`、
  `selectors 0.36.1`。尚未写 license allow/NOTICE/source-offer；
- [ ] target-aware advisory 各只剩同 5 个 runtime UNIC unmaintained record：
  `RUSTSEC-2025-0075/0080/0081/0098/0100`，均 `patched=[]`，不是已知漏洞但维护性风险真实；
- [x] `cargo audit` 既有三条 ignore 后仍报 15 条的原因已精确拆开：其中 10 条是 lock-only
  扫到的 Linux GTK/proc-macro-error/glib 包，六个发行 target 全部不可达；`glib` unsound
  **不在发布图**，不能再写成“Linux Desktop 阻塞”；剩余 5 条就是上一项 UNIC；
- [ ] cargo-vet 不是 367 条真实发布阻塞：按 target 为 macOS `270`、Windows `269`；关闭
  all-features 的既有基线均为 `181`，故 Tauri 净增分别 `89/88`。没有修改
  `supply-chain/config.toml`，没有生成 exemption，仍保持红；
- [ ] 外部发行身份、`tauri.conf.json` capability/CSP、binary、真实窗口 lifecycle、macOS/
  Windows 各自原生发行构建仍未完成。

完整表格与命令见 `docs/2026-08-25-G6-Tauri供应链目标图delta-batch16.md`。以上红项不能写绿，
但按第一真源 R61/R63 的实施裁决，它们只阻止 G6 勾关/发布，不阻止继续实现其它生产 UI。

## 恢复顺序

1. `git status --short --branch`，确认 HEAD 至少为 `17d43eb`；当前安全审计改动尚未正式
   ledger/R79；
2. 运行 `./tools/check-tauri-dependencies.sh` 与
   `./tools/check-deny-release-targets.sh`；二者应绿；
3. 继续不依赖外部发行身份/供应链豁免的 G6 production route、primitive、component、golden/
   AX harness；已完成条目必须逐条据机器证据打勾；
4. 发布 Desktop 前再闭合 MPL notice/source offer、5 条 UNIC 决策与 target-specific Cargo Vet；
   不得直接 `cargo vet regenerate exemptions`；
5. 有 reviewed external identity 后补 `tauri.conf.json`、binary 与窗口 lifecycle，只要求第一真源
   指定的 macOS arm64/Windows x64 Desktop 原生证据，不虚构 Linux Desktop；
6. 仍不运行 `cargo xtask ci`，不派发 Actions，不碰 `grok-bot`。

另外：Batch 15 分支已推送且 Actions=0；公开 Draft PR 创建被安全审查要求用户明确授权，
尚未创建，未绕过。
