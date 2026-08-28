# Batch 41：Gallery Cards

> 日期：2026-08-27。分支：`codex/2026-08-27-G6-gallery-cards`。
> base：Batch40证据head `d6d9036`；WIP恢复点：`0102082`；
> implementation：`3173354d895110363850a4d8dcf6679fc90c332b`。
> 固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批按固定上游`cards.tsx`实现四个独立compiled renderer及其exact schema，扩展Batch40已经闭合的
manifest/PG/HTTP/Gallery通路。没有运行`cargo xtask ci`，没有派发Actions，没有生成formal golden；
既有未跟踪`docs/assets/`未修改、未暂存、未提交。

## 1. 身份与schema

四个name分别保持为独立catalogue/tool/grant键，未合并为带`kind`参数的通用卡：

- `showRecord`：required title/fields；field required label/value；value不截断；
- `showMetrics`：required title/metrics；nested label/value required；`maxItems=6`；
- `showChecklist`：required title/items；nested text/done required，note optional；
- `showNotice`：required title/body；points optional且有序；
- 四类所有nested object都`additionalProperties=false`；tone精确
  `neutral/positive/caution/negative`，说明文字与上游逐字同源。

manifest按稳定name顺序从1扩为5：Checklist、Metrics、Notice、Quote、Record。UI renderer registry单测
与contract manifest双向逐项相等；Server仍只接受逐字段exact browser复述，未实现组件没有入口。

## 2. renderer行为

- `RecordCard`复用GalleryFrame；optional subtitle/status/tone，字段保持输入顺序；label可截断，value
  `overflow-wrap:anywhere`不截断；
- `MetricsCard`最多渲染六项；值使用tabular nums，change用GalleryBadge文字+点表达tone；
- `ChecklistCard`是只读报告：done/total状态、完成项删除线、note；DOM中零button/input/checkbox；
- `NoticeCard`显示headline/body/optional tone与有序points；tone文案中英本地化；
- check/tone的success/caution只落glyph/text，所有GalleryBadge computed background透明；没有把语义色
  放进背景或边框；
- registry sample props与固定上游preview逐项一致；Gallery index/detail自动获得五个真实preview。

## 3. catalogue与fixture回归

Batch40 PG adapter无需改结构；真库用例升级为五entry：forced audit failure后五个name计数仍0；恢复后
首次added五项、audit五条且Quote恰一条；重复added空；tampered manifest拒绝；管理员unpublish/draft不被
覆盖；unknown kind仍失败关闭。

fixture把原unpublished `showNotice`改成未实现的`showFutureChart`，使首次sync能真实新增Notice并同时保留
“未来/未发布row不显示”的负向。最终数据库有7 row，Settings index只显示6个published：五真实renderer+
一个stale fallback。

## 4. 本机证据

| 面 | 结果 |
| --- | --- |
| contracts / application / Agent / Server / UI | **80 / 140 / 28 / 204 / 119**，均0失败 |
| infra / Desktop | **306 / 78**，均0失败 |
| PostgreSQL 17.11 SCRAM | 五entry catalogue **1 / 0 / 0** |
| Axum/in-process transport | **8 / 0 / 0** |
| Clippy / WASM | 七crate all-targets/all-features `-D warnings`；contracts/UI WASM通过 |
| i18n / design / CSS | **496** leaf；**82 Rust / 74 icons**；**257** source class literals |
| release bundle | WASM gzip **1,138,033 B**；CSS **86,968 B**；fonts **740,216 B**；external/inline **1/0** |
| parity / fixtures | **652/1026/1678**（本批不改done状态）；fixtures **16/22/38** |
| strict fixed-upstream recount | **157 / 157 / 0** |
| parity violations / warnings | **0 / 0** |

PG实例实测`17.11 (Homebrew)`、`password_encryption=scram-sha-256`、role hash为SCRAM，测试后停止
删除。`Cargo.lock`与package集合零变化；最终Trunk以`--release --offline --locked`成功。

## 5. release浏览器

- index恰六个published tile：Checklist、Headline figures、Legacy fallback、Notice、Quotation、Record；
  unpublished Future chart正文/卡片0；
- Record preview三字段，Amount/Raised/Owner值完整；
- Metrics preview三项，change分别`positive/caution`，computed semantic background计数0；
- Checklist进度`2/3`，done序列`true/true/false`，check/button/input/role=checkbox为0；
- Notice tone本地化“提醒”，points恰2且顺序保持；
- Metrics detail `Called as=showMetrics`、三项/两change，hard reload保持；
- 所有tile nested interactive0、overflow0、visible alerts0、console0。

## 6. 未完成边界

- 本批不改parity done计数：`T-CMP-0002`要求的conversation registration、published/per-Bot
  withholding、data-function grant与tool-call-time decision尚未闭合；
- 还缺五个Charts、两个Decisions、Activity、Preview no-preview runtime与Refused双路径接线；
- Admin governance、sandboxed/Desktop renderer、参数/render/action formal golden仍todo；
- 完整G4/G6、Tauri binary/window lifecycle及其余第一真源缺口继续未完成。
