# Batch 42 WIP：Gallery Charts

> 日期：2026-08-27。分支`codex/2026-08-27-G6-gallery-charts`；base为Batch41证据head
> `5f0b542`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行`cargo xtask ci`，不派发Actions，不触碰`docs/assets/`。

## 范围与裁决

- 固定上游贡献`showBarChart/showPieChart/showLineChart/showAreaChart/showProgress`五个独立name；
  Line/Area必须共用同一schema与geometry scaler；
- 上游本就是手写SVG/HTML，不引入图表库；Rust版继续零chart dependency；
- series颜色只从GUI第一真源`chart-1..5` token循环，模型不能选色，拒绝/成功语义色不进入普通series；
- Empty必须是可见句子，不画空axis；Pie total<=0也走Empty；Metrics/label等文本仍在DOM，preview统一
  aria-hidden；
- 五renderer落地不等于runtime授权链完成，`T-CMP-0003`保持todo。

## 实施范围

1. contracts：五manifest entry；point/common/series/progress nested closed JSON Schema；Area与Line函数结果相等；
2. UI：Bar/Donut/Line/Area/Progress renderer、format/scale/geometry纯helper、五份固定sample；
3. fixture/PG test扩成十entry，existing unpublished future row继续不被sync覆盖；
4. WASM/Clippy/offline bundle/browser验证SVG数量、几何、legend、empty与token palette；
5. 不改parity done，不实现runtime decision/admin/sandbox/formal golden。
