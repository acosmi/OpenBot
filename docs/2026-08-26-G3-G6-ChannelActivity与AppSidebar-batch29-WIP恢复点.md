# Batch 29 WIP 恢复点：Channel Activity + AppSidebar

> 日期：2026-08-26。分支 `codex/2026-08-26-G3-channel-activity-app-sidebar`，
> base = Batch28 正式 head `b8df5d661956aff1031bef8409944db8b83bda9a`。
> 只跑本地定向测试；不运行 `cargo xtask ci`，不派发 Actions，不处理
> `grok-bot`，不修改/暂存/提交并行出现的 `docs/assets/`。

## 本批唯一生产闭环

1. PostgreSQL roster 真源：channel-anchored user message/assistant terminal 在原事务内单调更新
   `channels.last_message*`，预览折一行/C0·C1清理/最200 code points；
2. PostgreSQL `openbot_channel_activity` NOTIFY：仅作低延迟优化，不是真源；事务回滚不发；
3. typed `SubscriptionRequest::ChannelActivity`→`ApplicationService::subscribe`→
   `ThreadDirectory::subscribe_channel_activity`，每个通知回查 actor membership 后才出流；
4. `GET /api/channels/events` same-origin WebSocket，固定
   `openbot.channel-activity.v1`，只读，client Text/Binary 以1008关；
5. `openbot-ui::shell::app_sidebar` + `features::app_sidebar::channel`：真 `/api/channels`
   keyset分页、可见字段搜索、socket reconnect-refetch、同一children Sidebar、生产session status/sign-out；
6. 只在上述全绿后勾 T-API-0030、T-UI-0037/0038；App shell layout/route/golden 是否另勾
   按其自己 journey 证据裁决，不跟组件捆绑冒充。

## 构造性边界

- socket event 不携 member IDs；内部通知也只携 bounded activity，订阅方以当前PG membership回查；
- reconnect 必须先refetch；丢NOTIFY只损延迟，不损数据；
- user/assistant 更新只许时间前进；stale report 零通知；
- AppSidebar 不接触PG/模型/业务规则，不显示single-user登出，不用test-only列表冒充生产API；
- channel 创建/详情route、完整ChatTranscript/Composer、global app-shell route journey 仍独立todo。
