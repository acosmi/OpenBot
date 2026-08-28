# G4 Google Drive REST 生产面（Batch 13）

日期：2026-08-25  
第一真源：后端方案 §2.4、§5.2、§8.1–§8.6、§9.2–§9.5、§14.3、§15.3、§24 与 R76  
固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`  
实施提交：`7114a7698a677c71f8492272f3c9d1c9042869eb`  
堆叠 PR：[#30](https://github.com/acosmi/OpenBot/pull/30)，base=`feat/2026-08-25-G4-mcp-oauth-runtime`，head=`codex/2026-08-25-G4-drive-rest-runtime`

## 1. 本批结论

本批闭合第一真源 §9.5 的 Google Drive 后端生产面，不把 Google Workspace Developer Preview MCP endpoint 当成 GA，也没有用 fake/test-only runtime 冒充生产：

- production Server main 真实装配 `GoogleDriveRestTransport` 与 `GoogleDriveOAuthClient`；
- Drive 与 RMCP 由 native 0019 的 closed transport identity 分派，但共享同一 catalog/grant、CEL、decision、attempt、capability、outcome 与 audit 管线；
- asker's per-user OAuth 从管理员登记、begin、code callback、每 operation refresh、401 单次 retry 到 disconnect/revoke 均走 PostgreSQL/Vault 生产实现；
- search/recent/metadata/read 四条工具使用 GA Drive v3 REST；结果带 HTTPS vendor link 与 first-party provenance；
- 客户文档正文只存在于一次 bounded 调用内，不写 PostgreSQL、不创建本地 ACL/index。

因此 §24.1 的 Drive 子项已按机器证据打勾；G4 整关仍未通过。

## 2. 第一性原理裁决

### 2.1 Drive 不是 MCP

固定上游当前目录明确选择 `https://www.googleapis.com/drive/v3` 和 `google-drive-rest`。Rust 版将协议身份写入 `mcp_servers.transport`，闭集只有：

- legacy/remote `mcp`；
- `google_drive_rest`。

未知字符串由 PostgreSQL CHECK 和 Rust enum 双重拒绝。transport 同时进入 catalog fingerprint；切换协议不能继承旧 grant。

### 2.2 Google refresh 不伪造“必须轮换”

MCP OAuth 继续要求 refresh rotation；Google web-server OAuth 的正常 refresh 响应通常只返回短期 access token。共用 credential store 新增显式 exchanger contract：

- MCP 缺 replacement refresh 仍失败；
- Google 可保留原 refresh，且不伪写 `credential.rotated`；
- code callback 仍必须取得 refresh token，才能形成 durable connection；
- stored/request/response scope 均固定为 `drive.readonly`，偏离即 fail-closed。

### 2.3 正文不能成为第二数据真源

adapter 只投影当次调用结果。真实 PG 测试用正文 canary 复核：audit payload 命中为 0，public schema 中 Drive 专用 relation 为 0。vendor link 只接受无 userinfo/password/fragment 的 HTTPS URL。

## 3. 生产实现

| 面 | 落地 |
| --- | --- |
| schema | native 0019 给 `mcp_servers` 追加 nullable `transport` 与闭集 CHECK；legacy NULL 解释为 MCP |
| 固定目录 | 唯一 key=`google-drive`，固定 Google/first-party/GA base/user OAuth/read-only scope；未知 key 无 fallback |
| REST | SafeDialer-only；30s/request、8MiB wire、25 files、20,000 Unicode scalar model-visible cap |
| 工具 | `search_files`、`list_recent_files`、`get_file_metadata`、`read_file_content` 四条静态 schema |
| 查询 | Drive q 对反斜杠与 apostrophe 顺序转义；recent 只用 `modifiedTime desc` |
| 内容 | Docs/Sheets/Slides 分别 export text/plain、text/csv、text/plain；普通文本 `alt=media`；binary 只读 metadata |
| OAuth | 固定 auth/token/revoke；S256、offline consent、`client_secret_post`、exact callback/issuer/scope |
| actor | catalog 与执行时都要求 exact actor 的 active `mcp_user_credentials`；无 deployment fallback |
| Agent | closed transport dispatcher；第一次 Drive 401 才 refresh 并重试一次，第二次结果终止 |
| 撤权 | 本地 tombstone/join delete/audit 先 commit；Google revoke 失败进入 SKIP LOCKED reconciliation |
| HTTP | typed `POST /api/plugins/servers` 只接受 key；unknown URL 字段 400；Drive refresh 走 typed ApplicationService |
| 组装 | Server main 同时注入 RMCP、Drive REST、两类 OAuth exchanger；无额外 HTTP client |

本批还修复一个由真实小连接池测试暴露的死锁：catalog refresh 提交事务后原先仍持有 pool connection，curated add 的外层又持有一条，随后 `granted_tools_any` 在 pool size=2 时等待第三条。现在两层都在下一次 acquire 前显式释放已提交连接。

## 4. 本机证据

没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

| 定向证据 | 结果 |
| --- | ---: |
| `cargo test -p openbot-infra --test google_drive_runtime --locked` | 30 passed / 0 failed / 1 ignored |
| 同文件 PG17/SCRAM 端到端（`--include-ignored --exact`） | 1 / 0 / 0 |
| `native_0019` PG17/SCRAM | 2 / 0 / 0 |
| 既有 `mcp_oauth_runtime` PG17/SCRAM | 2 / 0 / 0 |
| 既有 `mcp_protocol`（含 PG） | 5 / 0 / 0 |
| 既有 `plugin_user_credential` PG17/SCRAM | 11 / 0 / 0 |
| Server plugin framing | 4 / 0 / 0 |
| Application MCP admin | 2 / 0 / 0 |
| infra Drive/OAuth pure | 3 / 0 / 0 |
| native migration guards | 3 / 0 / 0 |
| 五 crate `--all-targets --all-features` Clippy `-D warnings` | 通过 |
| contracts/UI `wasm32-unknown-unknown` | 通过 |
| `cargo xtask parity-check --json` | 0 violations / 0 warnings |
| `cargo xtask recount --require-upstream` | 154 passed / 0 mismatch / 0 skipped |

真实端到端覆盖：curated add → OAuth client registration → authorization URL/state/PKCE → code callback → static catalog → explicit grant → actor A 可见/actor B 不可见 → Agent decision/attempt/capability → access-1 401 → access-2 成功 → outcome/audit → disconnect 本地立即 deny → revoke 503 pending → reconciler revoke success。

schema 0019 fixture：41 tables / 369 columns / 268 NOT NULL / 200 constraints / 80 indexes / 4 triggers；文件 SHA-256=`8e0170ca5893c86d7131c01f62a93ea84caf371bfae2fe6d2e4f4edd8060d4d1`。

## 5. 台账变化

- tests：`236/811/1047` → `265/782/1047`（+29 done）；
- API：`34/123/157` → `35/122/157`（+1 done）；
- parity：`394/1267/1661` → `424/1237/1661`（+30 done）；
- fixtures：`12/22/34` → `13/22/35`（新增并完成 schema 0019）；
- G2 专项队列不含本批文件，仍为 `155/79/234`。

## 6. 明确未完成

- 未使用真实 Google credential，不能声称 live Google tenant 已验收；
- `drive.readonly` 是 restricted scope，Google 外部 verification/security assessment 仍是发布前置，本机代码无法替代；
- Desktop Local installed-app client、system browser、随机 `127.0.0.1` callback 尚未实现；
- custom MCP、MCP private egress、通用 authenticated refresh、grant/effect 管理 API 与完整 GUI 尚未闭合；
- 真实 human approval、run/user cancel、browser/file/shell executor 及 G5–G8 仍未完成。

## 7. 外部协议依据

- [Drive files.list](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list)
- [Drive search terms](https://developers.google.com/workspace/drive/api/guides/ref-search-terms)
- [Download/export files](https://developers.google.com/workspace/drive/api/guides/manage-downloads)
- [Export MIME formats](https://developers.google.com/workspace/drive/api/guides/ref-export-formats)
- [Drive OAuth scopes](https://developers.google.com/workspace/drive/api/guides/api-specific-auth)
- [Google web-server OAuth and revoke](https://developers.google.com/identity/protocols/oauth2/web-server)

## 8. Git / PR 证据

- 实施提交：`7114a7698a677c71f8492272f3c9d1c9042869eb`；
- PR #30：OPEN / CLEAN / MERGEABLE；
- base 是 Batch 12 head 分支，不是 `main`；
- `statusCheckRollup=[]`，该 head 的 Actions run 列表为空；
- 合并时仍须按堆叠 `baseRefName` 依赖顺序使用 merge commit。
