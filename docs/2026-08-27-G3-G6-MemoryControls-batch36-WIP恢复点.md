# Batch 36 WIP：Memory Controls 与 `/settings/memory`

> 日期：2026-08-27。分支 `codex/2026-08-27-G3-memory-controls`；base = Batch35交付head
> `5bd272d5e602162660ea1ade433ee8be8e07d78c`。固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 只跑本机定向测试；不运行`cargo xtask ci`，不派发Actions，不处理`grok-bot`，不修改/
> 暂存/提交既有`docs/assets/`与误置构建缓存。

## 第一真源裁决

- v3 §3.1条7、§4.3条8–11与§24 G3明确要求用户可查看、纠正、禁止、删除memory，并有
  **31 route之外**的新增`/settings/memory`页面与全局“禁用记忆写入”控制；
- 已有R66六条memory API与PG真事务只能证明单条记录旅程，尚无global write control与GUI，不能用
  backend存在冒充Memory GUI；
- 全局开关是runtime数据治理，不藏进`user_ui_preferences`，也不伪造成一条特殊memory。新增
  `user_memory_controls(tenant_id,actor_user_id,writes_enabled,updated_at)`，scope与既有memory真源一致；
- 无控制行等价`writes_enabled=true`，保持升级兼容。显式关闭后阻断GUI remember、remember tool与
  correct三个会增加/替换 retained content 的路径；list/recall/forbid/delete继续可用，用户始终能查看
  和擦除已有数据。verified importer是一次性迁移边界，不被runtime开关暗中改写；
- disabled拒绝投影为固定`policy_refused` + rule id `memory_writes_disabled`。GUI只本地化stable code，
  tool executor返回closed `memory_writes_disabled`，不把用户文案或memory内容写进错误；
- `/settings/memory`只消费typed Server DTO；响应`no-store`，写请求trusted Origin先于body。页面不做
  optimistic删除/纠正，只有Server提交成功才替换权威行；commit unknown不自动重发。

## 本批实施范围

1. native 0022、post-schema fixture与迁移/表注册机械闸门；
2. closed MemoryControl contract、typed ApplicationService/port与GET/PUT HTTP；
3. GUI remember、remember tool、correct三条生产写路径同事务检查开关；
4. `/settings/memory`：enabled switch、owner keyset/list/load-more、correct、forbid、delete、状态/scope/
   provenance呈现与AppSidebar真实destination；
5. deterministic fixture、PG17.11 SCRAM、Server/UI/WASM、真实release浏览器、i18n/design/CSS/bundle、
   parity/recount证据；机器证据成立后才更新两份第一真源、CLAUDE、移交指南与正式Batch36文档。

## 明确不在本批冒充

- 不做后台抽取、隐式learning、pgvector/customer document index；
- 不把“禁用写入”偷换成自动删除或停止recall，existing data生命周期仍由用户逐条控制；
- 不处理actual legacy exporter/production bundle三次演练；该项继续阻止G3整关勾选；
- 不倒算Markdown/tool boundary/Screen、完整Composer/AppSidebar其余destination、golden/Tauri binary，
  G6整关继续不勾。

