# 外部任务 G：安全流式 Markdown 正文

先读同目录 `2026-09-04-v4第二轮外派任务-总则.md` 和列出的第一真源，全部共同约束适用。
固定基线 `87d84bb85d0056dfa4dcc2b35be4c2a610a55ae3`；分支 `feat/2026-09-04-G6-streaming-markdown`。
本任务可在现有 Rust/WASM 与真实浏览器开发环境实施，不依赖 live provider。

## 任务与范围

GUI 第一真源 §6.3–§6.4、§8–§10 是完整要求；对应 `T-UI-0122`，关联正文组件 `T-UI-0042`。
现有 `ChannelConversation` 已有正文/流式事件，尚无 Markdown 模块。本任务把正文接入安全 Rust renderer，
不重写 Composer、tool boundary、Screen 或 run 状态机，不以单组件完成关闭整个 channel route。

允许修改：

- 新增 `crates/openbot-ui/src/features/channels/markdown/` 及本模块测试。
- `crates/openbot-ui/src/features/channels/mod.rs`、`conversation.rs`：只增加模块与正文挂载/消息身份和必要测试，保留其它事件/权限/队列行为。
- `Cargo.toml`、`Cargo.lock`、`crates/openbot-ui/Cargo.toml`：只增加已裁决 `pulldown-cmark =0.13.4`、`syntect =5.3.0` 的必要闭包；不升级已有包或启用 native oniguruma。新增图、许可、unsafe/build.rs/发行影响逐项报告，不加 vet exemption。
- 新增有固定来源/许可/摘要的 24 语言语法资产与语法打包模块，位置限 `crates/openbot-ui/design/markdown/`；不用上游自带主题，不引入 npm/shiki。
- locale 仅新增 `channels.markdown_*`；`crates/openbot-server/src/bin/openbot-ui-fixture.rs` 仅新增本任务场景，不覆盖既有处理。
- 新增 `fixtures/ui/markdown/`、`docs/2026-09-04-流式Markdown正文-外部交付.md`。

## 验收

1. TABLES/STRIKETHROUGH/TASKLISTS、嵌套列表、引用、围栏代码、链接和图片按 §6.4 实现。raw/inline HTML 作为文本节点，不使用 innerHTML 执行模型内容。
2. 流式完整/不规则/逐字符输入最终树与一次性解析逐节点相等；覆盖未闭合 fence、列表/表格形成、UTF-8、后到 reference definition 对早先块的影响。缓存以消息身份和块身份隔离，不能只测不会影响旧块的样本；若某类 Markdown 必须使旧块失效，明确说明并提交一致性证据。
3. 按规定 24 种语言构造 SyntaxSet，unknown language/plain text 有界回退；scope 只映射六个设计 token。复制按钮有中英名称、键盘可用，不向日志/模型发送正文。
4. 链接统一显示域名并防危险 scheme/userinfo/control chars；Web noopener/noreferrer。Desktop 只能走既有受控宿主开链能力，若缺该能力，显示明确不可操作状态并单列宿主缺口，禁止放宽 Tauri allowlist 或在 WebView 内导航。
5. 远程图片只渲染链接芯片，真实浏览器证明请求为零；仅有已授权、同源/已认可 custom protocol 附件可内联。不能把任意相对 URL 当“应用附件”，附件权威缺失时按链接显示并如实登记。
6. 巨大正文/代码/嵌套输入有可解释边界，取消/离页释放缓存；跨消息、切换 channel、旧 run 响应不得污染当前正文。表格与代码只在自身容器滚动。
7. `streaming_render_equals_batch_render`、恶意 URL/HTML/图片 canary 与边界测试；UI 定向 tests、Clippy、WASM check；锁定工具 offline release bundle，再跑 i18n/design/css/bundle/dependency gates。
8. 真实 release 浏览器验证中英、两主题、1024×640 与1440×900、键盘复制/链接、流式替换、页面横向溢出0、console错误0、远程图片网络0。无法控制视口或网络检查时如实保留缺口；人工截图不冒充正式 golden/AX 全关。

交付实际 bundle 增量与依赖/资产来源、NOTICE/SPDX 精确增补建议。不要仅交纯 parser 或静态示例；必须有既有频道正文消费。
完整 Desktop/Wry、所有附件协议、正式 golden 或任何未实测部分保持未完成，由主控决定 T-ID 最终状态。
