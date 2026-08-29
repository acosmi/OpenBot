# OpenBot G6 Admin Shell + Audit Batch59

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-admin-audit`

基线：Batch58 PR #41 已以 merge commit `a6b1b9856df975e985cf17e51155d8648a83ffce` 合入 `main`。

implementation：`f071fc807a8a886a97d8cc45670de7e9af6fe097`

## 1. 结论

本批把既有production admin status/audit API接成第一批真实Admin GUI：

- `/admin`：受admin gate保护的implemented-only落地页；
- `/admin/audit`：typed、50条keyset分页的审计页；
- `/admin/playground`：纳入同一200px secondary shell；
- 非admin经真实PG session/role/generation显示统一NotFound、global Admin link隐藏，child page不构造。

只关闭`T-ROUTE-0004/0012/0013`。`T-UI-0028`代表完整上游AdminSidebar，当前People/IdP/Credentials/Skills/Plugins/Components/Computers等route仍缺，故保持todo；`T-UI-0132/0139`是formal page golden，当前仍0张，同样保持todo。

## 2. Admin gate与secondary shell

`AdminShell`先请求既有`GET /api/admin/status`：

- 200 + closed `{status:"ok"}`才构造children与secondary nav；
- 401/403/404统一渲染NotFound，不把“无权”与“不存在”分开；
- network/invalid/5xx显示本地化gate error，不降级放行；
- nav只列当前真实destination：Overview、Audit、Playground；未实现页不画断链；
- 复用Settings已验证的200px tokenized shell，global AppSidebar仍是唯一外层sidebar/main owner。

真实PG负向先通过testkit session bootstrap取得生产cookie，再在一个事务语义中删admin role并把users/sessions generation同步到2。浏览器访问`/admin`与`/admin/audit`均实得NotFound、admin nav0、audit row0、console0；因此不是只靠前端伪造一个user对象。

## 3. Audit transport与投影

UI helper只发：

```text
GET /api/admin/audit-events?limit=50[&cursor=<encoded opaque cursor>]
```

same-origin credentials、no-store request、redirect error；响应再检查：

- 每页≤50；event id页内唯一；跨页append仍拒绝duplicate；
- id/event type/actor/target均非空、bounded、无control字符；
- payload必须是≤64KiB JSON object；
- next cursor≤2048且无control字符。

页面按Server顺序原样追加，不在renderer重新排序。每行展示event type、RFC3339时间、actor/system、target与可展开的结构化facts；不把payload变链接、HTML或自由class/id。

GUI fixture按固定顺序给52条脱敏记录，只实现empty filters与50行页长；其它查询fail-closed。production PostgreSQL reader/ACL/keyset/audit retention继续由既有PG17与Server证据承担，本fixture不冒充它们。

## 4. Release浏览器证据

管理员正向：

- `/admin`：h1=`管理`，global/admin nav共2，admin current恰1；secondary href精确`/`、`/admin`、`/admin/audit`、`/admin/playground`；console0；
- global AppSidebar由同一admin probe条件显示Admin入口；admin时link1且整个`/admin/*` section current，user时link0；
- `/admin/audit`：首屏50行，首行含system actor/target/RFC3339；Load more enabled；
- click后52行、按钮0、duplicate id0、alert0、console0；payload展开为`{"outcome":"granted","sequence":0}`；
- hard reload回50行，main1/nav2/h1审计/current1、horizontal overflow0；
- Playground route保留唯一h1并有admin current1。

非管理员负向见§2；两route均不创建AdminSidebar或Audit child。

## 5. 机械证据

| 面 | 结果 |
| --- | --- |
| `openbot-ui` | `136/0/0` |
| `openbot-server` all-features（宿主loopback） | lib `213/0/0`；fixture bin `2/0/0`；其余非PG tests绿 |
| UI/Server all-target/all-feature Clippy | exit 0 |
| UI wasm32 check | exit 0 |
| release/offline/locked Trunk build | exit 0 |
| i18n/design/CSS | 570 leaf keys；92 Rust files/74 icons；296 class literals |
| bundle | wasm gzip `1,439,871/3,670,016`；CSS `99,181/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| parity | routes=`11/21/32`；UI仍=`87/65/152`；总=`698/996/1694`；overlay=`1597/95/2/0`；0违反 |
| strict recount | `159/0/0`，固定上游`891df72f…`，skip0 |

## 6. 明确未做

- 未实现完整AdminSidebar剩余destination；`T-UI-0028`保持todo。
- 未生成Admin Home/Audit formal golden；`T-UI-0132/0139`保持todo。
- 未增加audit搜索/过滤编辑UI；当前只完成按时间浏览与keyset分页。
- 未把fixture audit rows当成production PG audit reader证据。
- 未运行`cargo xtask ci`或Actions（R63 manual-only）。
- P1 Windows/runsc仍红，未进入P2；`grok-bot/`零改动，无npm。
