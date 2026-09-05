# Batch117：Plugins 管理界面与权限呈现

日期：2026-09-04；分支：`feat/2026-09-04-G6-admin-plugins-ui`；起点：`22945bb`（R192）。
依据：v4 §9、§24、§25、§28.1 R193；GUI 第一真源 §5、§9、§15。

> R195更正：Batch117当时的OAuth密码仍采用RwSignal<String>，不符合§6.4；旧局部浏览器绿不证明密钥所有权。当前已由Batch119改为DOM输入和零化请求，失败保留只适用于非secret字段，密钥再次发送必须重填。

## 本批实际实现

Server Web 的 `/admin/plugins`、`/admin/plugins/:key`、`/admin/plugins/:key/tools/:tool`
共用 `AdminPluginsPage` 和既有 `AdminShell`。配置列表、目录、自定义 HTTPS/CIDR 服务器、
启用/移除、OAuth client 表单、本人账户连接、工具刷新、作用说明和逐 Agent grant 已接现有 typed API。
列表同时纳入当前可见与本人隐藏的 Agent；授权不取代每次调用的策略、账户权限和审计。

`McpAdminServer.authentication` 由 Rust 从 transport 与 credential kind 投影。
匿名 MCP、部署级 Bearer、个人 OAuth 是封闭三态；尚无 client credential 的 Google Drive 也明确是 OAuth。
GUI 不从 `hasCredential`、vendor 文案或服务器名称推测认证方式，不接收已有 secret 明文。
表单刚输入的 client secret 只在当前编辑期间存在，关闭/成功/离页清除，不进入 App-owned 写状态。

读响应有总字节、server/tool/schema/grant 数量与长度约束，并拒绝跨 server ref、重复 grant、
未知认证/错误类型和不安全链接。写入在 App owner 上串行；离开 route 不将已提交写入伪装成取消。
失败及 202 unknown commit 只请求最新状态，不自动重放；开关先保留权威旧值，再按读回状态刷新。
表单错误只属于本次编辑，键盘开关在读回后恢复焦点；关闭模态框后恢复触发按钮，跨 route 不抢焦点。

## 本机证据

| 验证 | 本轮结果 | 证据边界 |
|---|---|---|
| UI lib | 187 passed / 0 failed / 0 ignored | 含 4 个不可信响应/receipt 测试及 custom form 校验；不是浏览器证明 |
| Contracts / Server lib | 105 / 233 passed，均 0 failed / 0 ignored | typed DTO 与已有 Server 行为回归 |
| 独立 PostgreSQL SCRAM + loopback | `google_drive_runtime` 31、`mcp_protocol` 11、`plugin_admin_runtime` 1，全部通过且 0 ignored | 真实数据库、事务、审计与本地协议；OAuth vendor 是测试对端，不是真实 Google 用户凭据 |
| Clippy | Contracts/Infra/Server/UI all-target/all-feature `-D warnings`；UI WASM 与最终 fixture binary 另跑通过 | 编译/静态验证不冒充 Desktop 或 Windows runtime |
| release 浏览器 | macOS 内置浏览器，1280×720，中英、匿名服务器/CIDR、OAuth 表单、grant/revoke、删除/启用、失败/unknown/离页、键盘焦点均实走 | 明确使用 testkit-only 可丢弃内存服务；不是生产 DB→GUI 整体旅程或官方 golden |
| 设计与供应链局部 | i18n 869 叶键及占位符集合相等；design-lint、373 class css-check、tools verify、UI/Tauri dependency guards 通过 | Tauri guard 仍列 MPL/UNIC/Vet 未闭合，不代表全供应链绿 |
| release bundle | wasm gzip 2007787 B；CSS 116942 B；字体 740216 B；external script 1 / inline 0 | 均在固定预算内；无 npm |
| 台账与冻结 | strict recount 160 / 0 / 0；parity-check 0 违反；shim 595/600；非 Grok package.json 恰 1 | Grok inventory 同步 2110 文件；冻结 Git tree 不变 |

浏览器先发现并修复了两个焦点缺陷：数据读回替换开关导致焦点掉到 body；程序控制的 Dialog
未注册 compound trigger，Escape 关闭后没有返回按钮。最终分别观察到
`plugin-grant-fixture-explore-public` 与 `plugin-remove` 为 active element；一个 main、一个 h1，
无页面横向溢出，最终浏览器 error log 为空。未采集全量 CDP AX、reduced-motion 或三平台 golden。

最终 bundle：WASM 原始 6419263 B，SHA-256 `2d49550470eb571a7e36aee4dee798e312f33483ec836b1a7e3691f92b05566c`；
CSS SHA-256 `e947094c8a4e17178edf0bb7259d0cbc02c4ef2eed3441a809861b63d5a2f7eb`。
另从干净夹具连接本人账户，再删除/重新启用，读回 connections 为空，操作计数恰好一次 remove、一次 add。

可复算命令（遵守 R63，不运行整仓 CI 驱动）：

```bash
cargo test -p openbot-ui --lib --locked
cargo test -p openbot-contracts -p openbot-server --lib --locked
# 在独立测试 PG 上设置 OPENBOT_TEST_DATABASE_URL，不把连接秘密写入文档或日志：
cargo test -p openbot-infra --locked --test google_drive_runtime --test mcp_protocol --test plugin_admin_runtime -- --include-ignored
cargo clippy -p openbot-ui -p openbot-contracts -p openbot-infra -p openbot-server --all-targets --all-features --locked -- -D warnings
cargo clippy -p openbot-ui --target wasm32-unknown-unknown --locked -- -D warnings
cargo xtask i18n-check
cargo xtask design-lint
cargo xtask css-check
cargo xtask bundle-budget
cargo xtask tools verify
bash tools/check-ui-dependencies.sh
bash tools/check-tauri-dependencies.sh
cargo xtask electron-shim-check
cargo xtask grok-inventory --check
cargo xtask parity-check
# OPENBOT_UPSTREAM_DIR 指向固定 891df72 上游检出：
cargo xtask recount --require-upstream
```

发布构建继续使用已校验的 standalone Trunk/Tailwind/wasm-bindgen/wasm-opt，`--release --offline --locked`。
本机环境的 `NO_COLOR=1` 不被 Trunk 的布尔参数接受，构建进程用 `env -u NO_COLOR` 清除该值。

浏览器复现：启动 `openbot-ui-fixture --dist <release dist> --port <空闲 loopback 端口>`；
打开 Plugins 三路由。testkit-only `/__fixture/plugins/control` 接受 0 普通、1 副作用前失败、
2 副作用后 unknown、3 延迟 1 秒；`/proof` 只记录操作类型、server id 和计数，不记录表单秘密。
先测 grant 的 Space/revoke，再测 mode 2 回读 checked 且仅一次 grant；OAuth mode 1 保留输入，
明确再次点击后才新增一次操作；custom form 拒 hostname CIDR，接受 numeric CIDR；mode 3 刷新期间
离开到 People，再回来无重放。移除的 Escape/Cancel 不写入，确认后才删除，重新启用不恢复工具 grants。
所有连接和清理只作用于夹具，不会替用户连接或断开真实外部账户。

## 保留的完整范围

三条 route 的 Server Web 子面已实现，但 T-ROUTE-0022–0024 **仍保持 todo**：尚未取得
production PostgreSQL/session→GUI 的整体权限旅程和 Desktop 实际宿主证据；Desktop 缺部分
connections/curated/OAuth custom protocol 路由，Local installed-app OAuth 另未完成。
插件详情还没有 Bearer 凭据的完整创建/编辑产品面，自定义表单目前只接受已有 credential UUID。
`/admin/skills`、独立 Personal Skills 候选验收、完整 AppSidebar、golden/AX 与 G6 整关保持未完成。

parity 仍 873 done / 839 todo / 1712；fixtures 35 / 20 / 55；overlay 1273 / 431 / 2 / 6。
G0–G8、十条 DoD 和受控 Alpha A0–A7 均未因本批获得整体完成结论。无 Cargo.lock/schema/native
migration/engine/Grok/workflow 修改，无 Actions 派发、远端 push 或 PR 合并。
