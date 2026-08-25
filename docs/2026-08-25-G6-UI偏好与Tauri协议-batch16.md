# G6 Batch 16：UI 偏好持久化与 Tauri custom protocol

> 日期：2026-08-25。基线为 Batch 15 文档 head `ad1137e`；本批首个实现 checkpoint
> `17d43eb`，目标图审计 checkpoint `9a4853157d4ade71e9f9f994934f953301288ac7`。
> 本批关闭列出的子项，不代表 G6 整关通过。

## 1. 本批关闭的生产子项

- [x] shared contract：`UiTheme=system/light/dark`、`UiLocale=en/zh-CN`，stored fields 独立
  optional，partial update 至少一项；wire 不含 actor/deployment/tenant/time；
- [x] 唯一 ApplicationService port：`GetUiPreferences` / `UpdateUiPreferences`；空 update 在
  port 前拒绝，无 port 时 fail-closed；
- [x] native 0021：`user_ui_preferences` 以 deployment/tenant/actor 为复合 PK，closed
  theme/locale、nonempty row、actor cascade、DB clock、COALESCE 原子 partial merge；
- [x] Server `GET/PUT /api/me/preferences`：authenticated GET；PUT 的 trusted Origin 在 body
  parse 前；no-store；closed HttpOnly/SameSite=Lax cookie，Secure 只取既有 public URL 配置；
- [x] Leptos 启动 read + serialized/coalescing partial PUT；theme/locale 立即生效，远端写失败
  显示本地化 `role=alert`，不静默；
- [x] Desktop Local settings：三行 closed format、最大 256 bytes、Unix 0600、同目录临时文件
  + fsync + rename + directory fsync；默认 typed in-process lane 不引 JSON codec；
- [x] opt-in production Tauri adapter：按 webview label 绑定 Rust `AuthContext`，未绑定窗口连
  asset 都 401；preferences/approval 只经 typed `InProcessTransport`；fresh approval 是单调
  deadline；custom protocol 只发 canonical local asset、8 MiB cap、closed MIME/extension、strict
  CSP，并由 Rust 用本地偏好/OS locale 改写首帧；
- [x] Tauri 只进入第一真源指定的 macOS/Windows Desktop target；Linux Server/Web 与 WASM
  graph 无 Tauri/Wry/GTK；13 个真实 build.rs 与 9 个 WebView2 payload 已 exact audit/guard；
- [x] parity ledger：新增偏好 GET/PUT、native table/fixture 均 done；LocaleSwitch 据真实
  Chromium 键盘/ARIA/reload 证据从 todo 改 done。

## 2. 构造性边界

### 2.1 Server 与 Desktop 不共享存储实现，只共享 contract

Server 的跨设备偏好由 PostgreSQL adapter 承担；Desktop Local 由本机 closed file adapter
承担。二者都实现同一个 application port，因此 theme/locale 合并、空 update、错误域只有一份
业务语义。Axum/Tauri 只负责认证、framing、大小和错误映射。

### 2.2 首帧与运行期同一真源

- Server 首帧从 `openbot-ui` closed cookie 改写 `<html class lang>`；
- Desktop 首帧从 local settings / `sys-locale 0.3.2` 改写同一 marker；
- WASM 启动后读取已认证 stored preference；若用户在 GET 返回前已交互，以
  `interaction_revision` 拒绝旧读覆盖新选择；
- partial PUT 只有一个串行队列，保存中继续操作只合并最新字段，不并发丢更新。

### 2.3 Tauri window identity 由 host 铸造

custom protocol request 不接受 renderer 自报 actor/role/fresh。host registry 以 webview label
映射 Rust `AuthContext`；没有映射的 label 统一 401。approval 决策仍由 Batch 14 的 durable
binding/PG 负责，Tauri 只调用同一 typed transport，不生成第二套授权逻辑。

## 3. 本机证据

| 面 | 结果 |
| --- | --- |
| contracts UI wire | 1/0/0 |
| application empty-update boundary | 1/0/0 |
| Server preferences framing/cookie/origin | 1/0/0 |
| Desktop local settings | 2/0/0 |
| Desktop typed lane no-codec | 1/0/0 |
| Tauri custom protocol handler | 2/0/0 |
| native 0020 historical boundary + 0021 | 4/0/0（PG17.11 TCP SCRAM） |
| UI | 19/0/0；WASM/Clippy 绿 |
| Desktop Tauri Clippy | all-targets `-D warnings` 绿 |
| strict recount | 157/157/0 |

`schema-0021.json` = 43 表 / 404 列 / 295 NOT NULL / 222 约束 / 86 索引 /
4 trigger / 4 enum / 1 public function / 0 extension；4775 行；SHA-256
`fab4e148cb4f847e2f7079eae95b158ff7d4d0ed740ca2007fbb2de8ab7e3531`。

Browser fixture 只用于 UI/host framing：system/zh-CN 经键盘切到 dark/en，保存期间无 alert；
reload immediate/settled 均为 `class=dark/lang=en`。真实 production handler 的 loopback 请求实得
closed `Set-Cookie`，手工 cookie 请求 root 实得预期 `<html class lang>`。它不冒充真 PG browser
approval；Batch 14/本批 PG 测试分别承担 durable backend 与 preference store 证据。

最新台账：API `40/121/161`、tables `56/0/56`、UI `4/148/152`、fixtures
`15/22/37`；全 parity `440/1232/1672`，violations/warnings `0/0`。

## 4. 供应链状态

`./tools/check-tauri-dependencies.sh` 与 `./tools/check-deny-release-targets.sh` 全绿；六个发行目标
bans/sources 均绿。真实红灯仍为：

- macOS/Windows 各 5 个 MPL-2.0，尚未完成 license allow + NOTICE/SPDX/source offer；
- macOS/Windows 各 5 个 UNIC runtime unmaintained、`patched=[]`，尚未裁决；
- Cargo Vet：既有 no-all-features target 基线 181；Tauri all-features macOS 270（净增 89）、
  Windows 269（净增 88）；未改 config、未加 exemption。

Cargo.lock-only audit 额外看见的 Linux GTK/proc-macro-error/glib 十条在六个发行 target 都不可达，
机器 negative guard 已锁；不能再把 glib unsound 写成“Linux Desktop 阻塞”，因为第一真源没有
Linux Desktop。完整审计见 `docs/2026-08-25-G6-Tauri供应链目标图delta-batch16.md`。

## 5. 仍未完成（不打勾）

- [ ] 对外产品名、bundle id、deep-link scheme 未提供；因此没有伪造 `tauri.conf.json` 或可发布
  binary；
- [ ] 真实 window lifecycle/multi-window ACL integration、macOS arm64/Windows x64 原生发行构建；
- [ ] `/settings` route、其余 23 个 primitive ledger 条目、45 个业务组件、31 route journey；
- [ ] Web 110 张、macOS/Windows 各 54 张、zh-CN 27 张 golden 与完整 AX/键盘矩阵；
- [ ] MPL/UNIC/Cargo Vet 红灯；
- [ ] Desktop Local installed-app OAuth、browser/file/shell、Screen/Handover 等分别属于后续
  G4/G5/G7，不能借 Tauri host adapter 冒充。

本批未运行 `cargo xtask ci`，未派发 GitHub Actions，未处理 `grok-bot`，分支尚未 push/建 PR。
