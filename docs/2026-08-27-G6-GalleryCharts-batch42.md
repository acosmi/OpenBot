# Batch 42：Gallery Charts

> 日期：2026-08-27。分支`codex/2026-08-27-G6-gallery-charts`；base `5f0b542`；
> WIP `7fba049`；implementation `18a080cd2749cb958adcbfd12d06af11468d8ae8`；
> 固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批按固定上游实现`showBarChart/showPieChart/showLineChart/showAreaChart/showProgress`五个独立
renderer/schema，零图表运行时依赖；不运行`cargo xtask ci`/Actions，不生成formal golden，不触碰
`docs/assets/`。

## 实施

- manifest/registry按name稳定排序从5扩为10；未实现renderer无manifest入口；
- Bar/Pie point schema required label/value；Progress额外required target；全部nested
  `additionalProperties=false`；Line/Area返回同一schema，series required name/values；
- Bar按最大值归一并保留可见最小高度；Donut total<=0为空态；Line/Area共用`plot_geometry`，Area只增加
  polygon；Progress target<=0为0%、其它clamp 0..100；
- chart palette只循环GUI token `chart-1..5`，模型无颜色输入，普通series不借用success/danger；
- SVG/图形aria-hidden，label/legend/facts仍由文本DOM承载；空数据是本地化句子而非空axis；
- fixture保留unpublished future row；PG用例升级为十entry，forced-audit十row0、成功added10/audit10、
  重复0及tamper/admin治理/unknown kind边界保持。

## 证据

| 面 | 结果 |
| --- | --- |
| contracts / application / Agent / Server / UI | **80 / 140 / 28 / 204 / 120** |
| infra / Desktop / transport | **306 / 78 / 8**，均0失败 |
| PostgreSQL 17.11 SCRAM | 十entry catalogue **1 / 0 / 0** |
| Clippy / WASM | 七crate `-D warnings`；contracts/UI WASM绿 |
| i18n / design / CSS | **498** leaf；**83 Rust / 74 icons**；**265** class literals |
| bundle | WASM gzip **1,169,672 B**；CSS **91,428 B**；fonts **740,216 B**；external/inline **1/0** |
| parity / recount | **652/1026/1678**（本批不改done）；strict **157/157/0**；0 warning/violation |

浏览器实得11个published tile（10真实+1 stale）、unpublished0：Bar三条高度100/66.6667/37.5且
chart token色；Donut三circle=48/26/26；Line polyline1/polygon0；Area polyline1/polygon1；Progress两行
90/100；nested interactive0。Bar detail Kind=Chart/Called as=showBarChart，hard reload保持；overflow/
alerts/console均0。

`T-CMP-0003`仍todo：conversation registration、published/per-Bot withholding、data grant与call-time
decision未完成。Decisions/Activity、Refused生产接线、admin/sandbox/Desktop renderer与formal golden继续
todo；完整G4/G6不勾。
