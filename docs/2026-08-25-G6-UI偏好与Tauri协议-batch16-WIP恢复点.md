# Batch 16 WIP 恢复点：UI 偏好与 Tauri custom protocol

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-ui-preferences-desktop`，基线为 Batch 15
> 文档 head `ad1137e`。本文件是需要用户许可/安全裁决前的 checkpoint，**不是完成证书**。

## 已落源码

- `openbot-contracts::ui`：封闭 `system/light/dark` 与 `en/zh-CN`，stored fields 独立
  optional；partial update 至少一项，actor/deployment/tenant 无 wire 输入；
- `ApplicationService`：`GetUiPreferences` / `UpdateUiPreferences` 与唯一
  `UiPreferenceAdministration` port；空 update 在 port 前 400；
- native 0021：`user_ui_preferences` 以 `(deployment,tenant,actor)` 为 PK，theme/locale closed
  CHECK、nonempty CHECK、user cascade；partial upsert 以 COALESCE 原子合并并用 DB clock；
- Server `GET/PUT /api/me/preferences`：GET authenticated，PUT same-origin guard 在 body 前；
  no-store；镜像 cookie 只有 closed theme/locale，`Path=/; Max-Age=31536000; SameSite=Lax;
  HttpOnly`，`Secure` 仍只由 public URL 的既有配置事实决定；
- UI 启动读取 stored preference；主题/语言即时生效，partial writes 单队列串行+合并；保存失败
  显示本地化 `role=alert`，不静默；浏览器 reload 前后 class/lang/ARIA 保持；
- Desktop local settings：三行 256-byte closed file、0600（Unix）、temp+fsync+rename+目录 fsync，
  不把 JSON codec 引进默认 typed in-process lane；
- opt-in `tauri-host`：精确 Tauri 2.11.5/Wry，custom protocol 按 webview label 读取 host-bound
  `AuthContext`；未绑定窗口连 asset 都 401；preferences/approval 经 typed
  `InProcessTransport`；fresh approval 用单调时钟 deadline，不是永久 bool；index 从本地偏好/
  OS locale Rust 改写并带 strict CSP；asset canonical path、8 MiB cap、闭合 MIME/extension；
- 没有创建可发布 binary/`tauri.conf.json`：对外产品名/bundle id/deep-link 仍无用户裁决，不能
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
- 临时 PG17 已停止，明文测试口令文件已删除；未运行 `cargo xtask ci`，未派发 Actions。

## 真实阻塞（不得绕过）

加入第一真源钉定的 `tauri = 2.11.5` + `wry` 后，workspace Cargo.lock 从 640 增至
**822 packages（+182）**，并产生以下机器红灯：

1. `cargo deny licenses`：5 个 MPL-2.0（file-level copyleft）未获许可白名单裁决：
   `cssparser 0.36.0`、`cssparser-macros 0.6.1`、`dtoa-short 0.3.5`、
   `option-ext 0.2.0`、`selectors 0.36.1`。项目默认闭源，不能由实现者替用户接受新
   copyleft 义务；
2. `cargo audit --no-fetch --deny warnings`：新增 **15** 条 denied warnings：GTK3/ATK 8 条
   unmaintained（RUSTSEC-2024-0411/412/413/415/416/418/419/420）、
   `proc-macro-error` unmaintained（2024-0370）、UNIC 5 条 unmaintained
   （2025-0075/0080/0081/0098/0100），以及 `glib 0.18.5` 的
   **RUSTSEC-2024-0429 unsound**（patched >=0.20，Tauri Linux GTK3 图固定 0.18）；
3. `cargo deny bans`：33 个新 build.rs 尚未逐份 delta audit，当前 errors=33；
4. `cargo vet --locked`：**367 unvetted**，仍未改 `supply-chain/config.toml`，未生成任何新
   exemption。

`glib::VariantStrIter` 在 Tauri/Wry/GTK/WebKit 消费源码外零直接符号命中，但这只支持后续
reachability 审查，不自动等于可接受 unsound waiver。上述四项与对外产品身份是明确用户/
security/legal 决策；在裁决前，Tauri feature 保持 WIP，G6 不勾，deny/audit/vet 不写绿。

## 恢复顺序

1. `git status --short --branch`；当前改动尚未正式 ledger/R79；
2. 先取得用户对 MPL-2.0、15 条 RustSec（尤其 unsound）、33 build.rs 审计方向与
   cargo-vet 367 项“真实审计 vs 精确非审计 exemption”的明确裁决；
3. 若继续 Tauri：逐文件审 33 build.rs、MPL 原件/NOTICE/SPDX、Linux GTK native 闭包与
   advisory reachability，写 exact guard；不得直接 `cargo vet regenerate exemptions`；
4. 再补 reviewed external product identity、`tauri.conf.json` strict CSP/capability 与真实窗口
   lifecycle assembly；macOS/Windows/Linux 原生构建分别取证；
5. 供应链闭合后才更新 parity/API/table/fixture/UI、R79、正式 Batch 16 文档和堆叠 PR；仍不
   运行完整 CI/Actions。

另外：Batch 15 分支已推送且 Actions=0；公开 Draft PR 创建被安全审查要求用户明确授权，
尚未创建，未绕过。
