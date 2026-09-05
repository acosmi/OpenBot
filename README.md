# OpenBot

本仓库用于实施 OpenBot 的全量 Rust 重写。

## 入口文档

- **接手实施先读这份**：[实施移交指南](docs/2026-08-23-OpenBot-实施移交指南.md) —— 环境搭建、闸门跑法与它照不到的地方、台账现状、下一批工作单、待裁决项与已知的坑。
- **仓库纪律**：[CLAUDE.md](CLAUDE.md) —— 红线、固定基线、发布级不变量、闸门清单、协作约定。**阶段进度的真源也在这里（§1）**，逐条写明什么已达成、什么明确未闭合。
- **实施真源（后端）**：[OpenBot 全量 Rust 重写：终版研究、前置审计与实施方案](docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md)（v4 = v3 就地修订至 §28.1 R194，2026-09-04；R115–R125 是范围/参考源/Engine阶段裁决，R126–R194 是后续实施裁决，实施前必读；R180覆盖RMCP旧wire，R182固定Golden分界，R183–R188固定CDP pure/live、正式screencast、ScreenHub、viewer coordinate与Server/production分界）
- **实施真源（GUI）**：[GUI 设计系统与视觉规格](docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md)（v2，2026-08-28 同 PR 修订）
- **历史文档**：`docs/2026-08-28-OpenBot-TauriGUI-ElectronChromium-GrokBot大面积Rust迁移-v4修订计划-用户裁决版.md` 已被 R115–R125 吸收，不是实施依据；`grok-bot/` 是参考树（定位与方法见后端方案 §11.5），不是产品代码。

两份方案冲突时：视觉归设计系统文档，架构归后端方案；与 `CLAUDE.md` 冲突时以方案为准，并同 PR 修订 `CLAUDE.md`。

## 当前状态

实施按 Go/No-Go 闸门 G0–G8 分阶段推进（后端方案 §24）。**各阶段的达成项与未闭合项以 `CLAUDE.md` §1 为准**，不在本文件重复登记 —— 重复出来的第二份一定会漂。

后续开发必须遵守方案中的固定源码基线、来源权属、功能对等台账和 Go/No-Go 闸门；任何闸门失败只能修复后重跑，不能以"后续补齐"进入下一阶段。

当前全范围执行顺序与外部可派发任务见[全范围闭合工作单](docs/2026-09-04-v4全范围闭合工作单.md)与
[预留台账](docs/2026-09-04-v4并行实施预留台账.md)。Batch114/R189仅增加Server Screen传输预算；
parity仍873/839/1712、fixtures33/21/54、strict160/0/0；G0–G8及十条DoD尚未全部满足，目标继续推进。

Batch115/R190补Screen需求驱动暂停/恢复，fixtures现为34/21/55；production装配与完整G7仍待闭合。

R191已确认[开源共建与渐进上线方向](docs/2026-09-04-OpenBot开源共建与渐进上线策略-研究方案.md)：受控Alpha先跑通工作台、Agent工具和浏览器/原生电脑三闭环，再逐步补全v4。当前根LICENSE尚未切换，不以研究方案宣称已开源或已发行。

Batch116/R192完成官方AG-UI事件fixture的主控验收，fixtures现35/20/55；完整v4与Alpha发布准入仍未完成。

Batch117/R193新增[Plugins管理的Server Web子面](docs/2026-09-04-G6-Plugins管理界面与权限呈现-batch117.md)，保留完整凭据产品面、Desktop、真实整旅程与golden/AX缺口；完整目标继续。

Batch118/R194补Desktop插件连接读取与目录启用的typed通道；实际Wry/Windows/Local OAuth与发行准入仍待完成。
