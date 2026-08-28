# Batch 40：Components Gallery / Quote Slice

> 日期：2026-08-27。分支：`codex/2026-08-27-G6-gallery-foundations`。
> base：Batch39证据head `bf4ebe1`；WIP恢复点：`ed19544`；
> implementation：`d5ca010231d1cfb6cc0950bddb84e7be7f651abf`。
> 固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批关闭compiled component的完整只读目录/Settings Gallery journey，并落第一个真实Rust renderer
`showQuote`及共享Frame。没有运行`cargo xtask ci`，没有派发Actions，没有生成formal golden；既有未跟踪
`docs/assets/`未修改、未暂存、未提交。

## 1. 第一真源与范围裁决

固定上游Settings Gallery并不是元数据列表：index/detail都渲染真实`ComponentPreview`；只列
`published=true`；旧build留下但当前无renderer的published row显示“This build cannot draw this”；
unpublished在个人Gallery按不存在处理。build首次announce目录时missing row默认published，已有治理row
绝不被build覆盖。

Batch39后本仓只有兼容表row/repo，没有component ApplicationService、HTTP、renderer或route。直接先画
Gallery会是假完成。因此Batch40选择一条可闭合竖切：

- 编译期manifest只登记当前真正实现的`showQuote`，不伪报另外12个上游renderer；
- API仍列数据库全部compiled governance row，使旧renderer缺失事实可见；
- `showQuote`参数schema/metadata与renderer同批落地，但conversation tool registration、per-Bot
  withholding、data-function与tool-call-time decision尚未落，故`T-CMP-0007`继续todo；
- `RefusedCard`已提供共享组件，但compiled+sandboxed两条生产拒绝路径未接，`T-CMP-0008`继续todo；
- 视觉以GUI第一真源为准：中性无边框chrome、语义色只落文字/状态点，不复制上游彩色背景/边框。

## 2. authority 与持久化

- contracts新增closed `CompiledComponentKind(chart/card/decision)`、完整`ComponentRecord`、catalogue
  request/reply与`showQuote` manifest/schema；全部WASM-safe、deny unknown；
- UI registry与contract manifest单测双向精确相等，当前恰`showQuote`；删renderer或多报manifest都会红；
- 浏览器保留上游`components[{name,title,kind,description}]` wire，但Application要求每个字段与Server
  编译期manifest逐字相等，拒unknown/tampered/duplicate；renderer不能自报新能力或改metadata；
- `PostgresComponentAdministration`列全部治理row，并在单条查询中排序聚合`withheldFrom/functions`；
  unknown kind、坏publication shape与越界字段失败关闭；
- catalogue transaction只`INSERT missing ON CONFLICT DO NOTHING`；首次默认published，已有draft/
  publication/updatedBy/grants零改；每个真实insert与`component.published` hash-chain audit同事务；
- Axum GET/PUT均no-store，PUT用trusted Origin-before-body；Desktop Tauri custom protocol复用同一
  `AppCommand`且no-store，manifest专属body cap 256KiB；
- production Server main注入同一个PG adapter；fixture起始published stale + unpublished，页面首次PUT
  添加Quote，确定性覆盖全部可见性分支。

## 3. GUI 与 renderer

- `GalleryFrame`保留optional title/caption/action/children和figure/figcaption结构；`GalleryTone`精确
  neutral/positive/caution/negative，`GalleryBadge`用文字+点承载语义；
- `QuoteCard`实现Quotation标题、optional context、quote与attribution；空quote有明确空态；参数schema
  required恰quote/attribution，context optional，additionalProperties=false；
- `ComponentPreview`只渲染registry真实成员；未知/stale row用本地化诚实fallback；preview统一
  `aria-hidden=true`、pointer-events none，真实tile里无嵌套interactive；
- 新增`/settings/components-gallery`与`/:name`；index先best-effort exact announce，再权威GET；只过滤
  published且有publishedDescription；
- detail显示published description、Kind与Called as；stale published仍有facts+fallback；unpublished/
  unknown统一No such component且零preview/facts；
- Gallery route真实后才进入SettingsShell；Back、General exact、Connected prefix、Gallery prefix顺序
  保持上游，另有v3新增Memory exact；由此关闭SettingsSidebar业务条目。

## 4. PostgreSQL 17.11 SCRAM

真库用例`1/0/0`证明：

- 初始published/unpublished两row的稳定顺序、withheld/function排序与derived dirty flag；
- 人为给`audit_events`加拒绝`component.published`的trigger后，sync返回Unavailable且showQuote row=0；
- 移除trigger后首次`added=[showQuote]`、重复`added=[]`、audit恰1且actor正确；
- 篡改manifest title在port前/port内均拒绝；
- 管理员把showQuote改成unpublished+admin draft后，重复build sync不覆盖任何治理字段；
- durable kind改成unknown后list精确返回`Corrupt { field: "kind" }`。

实例实测`17.11 (Homebrew)`、`password_encryption=scram-sha-256`、role hash为SCRAM；测试后临时
cluster已停止删除。

## 5. 本机机械证据

| 面 | 结果 |
| --- | --- |
| contracts / application / Agent / Server / UI | **80 / 140 / 28 / 204 / 118**，均0失败 |
| infra / Desktop | **306 / 78**，均0失败 |
| Server Components HTTP | **1 / 0 / 0** |
| Axum/in-process transport | **8 / 0 / 0**；Components另有Axum/Tauri逐字段专项 |
| PostgreSQL 17.11 SCRAM | Component Catalogue **1 / 0 / 0** |
| Clippy / WASM | 七crate all-targets/all-features `-D warnings`；contracts/UI WASM通过 |
| i18n / design / CSS | **491** leaf；**81 Rust / 74 icons**；**251** source class literals |
| release bundle | WASM gzip **1,099,850 B**；CSS **84,507 B**；fonts **740,216 B**；external/inline **1/0** |
| parity | API **54/113/167**；components **1/21/22**；routes **8/24/32**；UI **86/66/152** |
| total parity / fixtures | **652/1026/1678**；fixtures **16/22/38** |
| strict fixed-upstream recount | **157 / 157 / 0** |
| parity violations / warnings | **0 / 0** |

`Cargo.lock`与package集合零变化。最终Trunk构建使用钉版工具并以`--release --offline --locked`成功；
bundle预算与strict CSP external/inline约束全绿。

## 6. release浏览器

- 首进index触发exact catalogue PUT，再GET得到三row；页面只列published的Legacy widget与Quotation，
  unpublished Notice正文/卡片均0；
- Legacy tile/detail显示renderer unavailable；Quotation tile/detail渲染真实Quote，preview的AX name不
  污染tile link，`aria-hidden=true`且nested interactive=0；
- Quote detail实得published description、`Kind=Card`、`Called as=showQuote`与完整sample，hard reload保持；
- `/showNotice`显示No such component，preview/facts=0，返回gallery链接正确；
- 篡改title的catalogue PUT实得400 malformed；精确重复PUT实得200 no-store+`added=[]`；GET 200
  no-store且closed wire无source/arguments/secret；
- 中英切换后h1/description/nav同步；Settings nav顺序General/Connected/Gallery/Memory，Gallery current1；
- 最终release固定1280×720表面：subnav200、main/nav/h1=`1/2/1`、overflow0、duplicate IDs0、
  visible alerts0、console0。

浏览器表面仍不提供viewport resize，因此不生成/关闭formal responsive golden；Batch38既有四视口
SettingsShell证据只用于shell持续回归，不冒充本批page golden。

## 7. 台账与未完成边界

- 关闭`T-API-0037/0038`、`T-ROUTE-0027/0028`、`T-CMP-0005`、`T-UI-0066`；总parity
  `646/1032 → 652/1026`；
- `T-CMP-0007 showQuote`仍todo：缺conversation registration、published/per-Bot withholding、
  data-function grant与tool-call-time decision整链；
- 另外12个compiled renderer、activity no-preview、共享Refused生产接线、Admin governance与全部
  sandboxed/Desktop renderer仍todo；
- Settings Gallery formal golden、完整compiled component参数/render/action golden仍todo；
- G4/G6整关、Tauri binary/window lifecycle、Approval完整集成、browser/file/shell与经许可legacy
  production drills继续未完成。
