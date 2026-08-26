# Batch 30 WIP：Channel Detail + AppSidebar

> 日期：2026-08-26。分支 `codex/2026-08-26-G6-app-sidebar-channel-shell`；
> base = Batch29 正式 head `be56a9c0a7bce695f5cf7fc1c668445e7c9fd7b9`。
> 只跑本地定向测试；不运行 `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`，
> 不修改/暂存/提交并行出现的 `docs/assets/`。

## 本批生产闭环

1. 修复 G1 时代遗留的 `ChannelRepo.thread_id=None`：production list/detail 以
   AuthContext deployment/tenant/actor 回查当前 native channel thread，不读 legacy
   `intelligence_channel_mappings`；
2. typed `GetVisibleChannel` 经唯一 ApplicationService 到 PostgreSQL，未知/非member/错scope统一
   404；`GET /api/channels/{channel_id}` 只回 closed channel DTO；
3. Leptos `/channel/:channel_id` 读取真实 detail 并形成可达 destination shell；本批不冒充
   transcript/composer/stop/steer/screen route journey 已完成；
4. `shell::app_sidebar` 与 `features::app_sidebar::channel` 使用同一Sidebar children，真实
   `/api/channels` keyset分页、可见字段搜索、channel socket reconnect-refetch、production
   session status/sign-out；
5. 只有真实 host/WASM/浏览器/AX/键盘与后端PG证据全成立后，才裁决 T-API-0034、
   T-UI-0037/0038；T-ROUTE-0009、T-API-0031/0033、channel create/new与完整chat保持todo。

## 构造性边界

- channel list/detail 只按materialized membership；member身份不从URL/body/frame读取；
- native thread投影必须同时匹配deployment、tenant、channel anchor与当前thread membership；
- realtime event只触发/refetch roster，NOTIFY不是真源；malformed/error/close均不得静默保持陈旧；
- search只匹配row真实显示的name/last-message；分页不能在renderer伪造“已加载全部”；
- single-user不显示无效sign-out；multi-user只在Server 204后离开；
- channel route没有真实能力的部分明确显示不可用状态，不画可点击假composer/control。
