# Batch118：Desktop 插件读取与目录启用

日期：2026-09-04；分支：`feat/2026-09-04-G6-desktop-plugins-framing`；起点：`4124380`（R193）。
依据：v4 §9、§24、§28.1 R194，接续 Batch117 的已知 Desktop transport 缺口。

## 变更与边界

`DesktopTauriProtocol` 新增 `GET /api/plugins/connections` 与 `POST /api/plugins/servers`，
分别派发既有 `ListMcpConnections`、`AddCuratedMcpServer`，身份仍来自宿主 window binding。
查询字符串不能提供 actor/key；读请求只允许 GET/空 body，写请求需宿主 fresh grant，body 限 16 KiB。
请求 payload 在所有新写路径解析后或拒绝前清零；只保留已解析的 reviewed catalogue key。
Application 的 admin gate 仍在 port 副作用之前，普通用户可读取自己的连接，但不能启用部署插件。

Server 与 Desktop 的目录选择请求共用 `McpCuratedServerSelection`，封闭字段只有 `key`，
拒绝 endpoint、actor、重复 key 等注入；不允许 renderer 通过目录启用 API 决定 vendor/transport。
Server 旧 `AddCuratedServerBody` 名称保留为 alias，wire 形状不变。

这关闭了 Batch117 loader 的连接读取 404 及目录启用 404 两个 framing 缺口。
没有实现 Local installed-app OAuth、register/connect/disconnect 的完整 Desktop 通道，也没有
启动实际 Wry 窗口或 GUI→native 整体旅程。T-ROUTE-0022–0024、G6 与受控 Alpha 仍未完成。

## 本机验证

- Desktop `tauri-host` lib：112 passed / 0 failed / 0 ignored；其中 Plugins 3 条（新增 2 条）。
  未绑定/解绑 window 401、伪造 actor header 无效、query 400、stale write 401 先于坏 body、
  普通用户写 403、unknown/duplicate field 400、oversize 413、错误 method 405，均观察到 port 写次数为 0；
  正向只分派一次 `admin/google-drive`，GET/POST 都是 no-store。
- Contracts lib 105、Server lib 233，全通过且 0 ignored。Server 首次沙箱运行的 13 条 socket 测试因
  `PermissionDenied` 失败；在允许 loopback 的宿主环境完整重跑 233/0/0，没有屏蔽测试或伪造成功。
- Desktop tauri-host all-target Clippy、Contracts/Server all-target/all-feature Clippy `-D warnings` 通过；
  Windows x64 MSVC `cargo check` 通过，明确仅交叉编译，不算 Windows 电脑控制或 P1 真机证据。
- `check-tauri-dependencies.sh` 局部 guard 通过；MPL/UNIC/Vet 原有发行阻塞仍保留。
- parity-check 0 违反；strict recount 160 / 0 / 0。T-API-0081、0086 记录本次 revalidation，
  parity 873 / 839 / 1712、fixtures 35 / 20 / 55 不变；overlay 1271 carry / 433 revalidate / 2 split / 6 superseded。

```bash
cargo test -p openbot-desktop --features tauri-host --lib --locked
cargo test -p openbot-contracts -p openbot-server --lib --locked
cargo clippy -p openbot-desktop --features tauri-host --all-targets --locked -- -D warnings
cargo clippy -p openbot-contracts -p openbot-server --all-targets --all-features --locked -- -D warnings
cargo check -p openbot-desktop --features tauri-host --target x86_64-pc-windows-msvc --locked
bash tools/check-tauri-dependencies.sh
cargo xtask parity-check
# OPENBOT_UPSTREAM_DIR 指向固定 891df72 上游检出：
cargo xtask recount --require-upstream
```

本批是 deterministic adapter 测试，使用明确命名的 FakeMcpConnections 检查调用身份与次数，
不宣称新 PG、vendor、GUI、Wry runtime、安装包或 golden 证据。GUI 验证与 bundle 数字仍是 Batch117。
Cargo.lock、native/schema、Engine/shim、Grok tree 和 Actions 均未改变；未派发 Actions、push 或合并 PR。
