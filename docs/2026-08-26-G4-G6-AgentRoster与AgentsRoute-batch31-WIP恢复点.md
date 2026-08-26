# Batch 31 WIP：Agent Roster + `/agents`

> 日期：2026-08-26。分支 `codex/2026-08-26-G4-agent-roster`；
> base = Batch30正式head `4a805a6ee8f43a8a44d1dc1af155750cb8fe06e1`。
> 只跑本地定向测试；不运行`cargo xtask ci`，不派发Actions，不处理`grok-bot`，
> 不修改/暂存/提交`docs/assets/`。

## 本批生产闭环

1. typed Agent roster/detail经唯一ApplicationService与PostgreSQL；
2. public/private、owner/admin、soft-delete与per-user hidden roster逐项按第一真源裁决；
3. `GET /api/agents`与`GET /api/agents/{agent_id}`，不可见/不存在统一404；
4. Leptos `/agents`真实roster destination，并把已存在的Agents链接接入AppSidebar；
5. 只在PG/Axum/in-process/WASM/浏览器证据成立后关闭T-API-0019/0020及精确测试/UI项。

## 明确不冒充

- Agent create/edit/duplicate/hide/unhide/delete仍按各自API与journey保持todo；
- channel-new只在本批读面完成后具备真实recipient来源，本批不提前关闭；
- AppSidebar总项须继续等待new-channel/skills/settings/admin其余destination；
- remote Agent customer auth、recorded vendor trace与完整lifecycle仍不因roster存在而完成。
