# Batch 31：G4/G6 Agent Roster、GET API 与 `/agents`只读资料

> 日期：2026-08-26。分支 `codex/2026-08-26-G4-agent-roster`；
> base = Batch30正式head `4a805a6ee8f43a8a44d1dc1af155750cb8fe06e1`；
> implementation = `55bc2f18d1f6108272864dee8fba6d22c637305a`。
>
> 本批只运行本地定向测试；**没有**运行`cargo xtask ci`，没有派发GitHub Actions，
> 没有处理`grok-bot`，没有修改、暂存或提交`docs/assets/`。

## 1. 本批完成项

- [x] T-API-0019：`GET /api/agents`；
- [x] T-API-0020：`GET /api/agents/{agent_id}`；
- [x] T-TEST-0298–0303：Agent profile六条纯领域权限判据；
- [x] T-TEST-0305、0328–0329、0332：private list/get、DTO/permission、admin ownership
  分离与missing 404；
- [x] T-UI-0029：`abstract-avatar`收敛到统一Avatar；
- [x] T-UI-0030：AgentCard；
- [x] AppSidebar的Agents destination局部接线；AppSidebar总项仍不勾；
- [x] GUI第一真源中的Inter相对路径按最终Trunk产物修为`/fonts/*`。

## 2. 权限、tenant与DTO边界

固定上游的profile规则没有留在SQL或handler里复制：

- active public：所有已认证actor可access/run；
- active private：只准owner/admin access/run；
- active user profile：只准owner/admin manage；
- system public：所有actor可access/run，任何人不可manage；
- deleted：access/manage/run全拒；
- `can_run_agent`以`pub use`成为`can_access_agent`的构造性alias，不维护第二份函数。

`AgentReadScope`由Application从权威`AuthContext`铸造，只含tenant、actor、admin。SQL先按
package tenant、visibility/owner/admin、deleted与current-user hidden收窄；每一行decode后再过
domain终判，SQL漂移不能静默扩大授权。物理表没有user-created profile的tenant列，因此本批不
伪造不存在的scope；package-backed Agent则必须匹配当前tenant，cross-tenant detail统一404。

浏览器DTO固定13字段：id/name/title/roleDescription/avatarSeed/visibility/endpoint、三个has/hidden
布尔与systemOwned/canManage/mine。ownerUserId、configuration、credential/token明文均不穿边界。
`hasAuth`只在configuration同时具有合法header与credentialId字符串时为真；callback只看hash列。

## 3. HTTP与GUI

两个GET都先认证，再解析query/path，经唯一ApplicationService dispatch。成功响应显式
`Cache-Control: no-store`；unknown query或malformed id为400，缺失/不可见/删除/跨tenant为404，
port unavailable为503。错误正文不回数据库值。

`/agents`逐字保留固定上游分组：

- Your coworkers = `mine`；
- Explore coworkers = `!mine && visibility == public`；
- admin可见但不属于自己的private profile不被错误归入“mine”，仍可用直接detail URL读取。

AgentCard保持144×180、4:5、name与三行role；链接只用bounded percent-encoded同源query。
profile由URL拥有，hard reload会重新GET detail；close清query并返焦roster。页面只显示当前真实读
能力，不画create/edit/duplicate/hide/unhide/delete/start-channel按钮。卡片和profile已有同名
文字，Avatar wrapper因此AX隐藏，避免链接/资料重复朗读名称。

第一次最终产物浏览器检查发现CSS仍请求`/assets/fonts/*`，而Trunk的copy-dir实际输出
`/fonts/*`。原因是源码CSS位于design子目录，编译CSS却位于dist根；相对路径语义已经改变。
本批把两处`@font-face`改为根同源路径并同步GUI第一真源，最终
`document.fonts.check('14px "Inter Variable"') == true`。

## 4. 本机证据

| 面 | 机器实得 |
| --- | --- |
| contracts Agent | **2 / 0 / 0**：closed camelCase/deny-unknown/secret-free + callback既有回归 |
| domain profile policy | **6 / 0 / 0** |
| application Agent use case | **2 / 0 / 0** |
| Server Agent module | **4 / 0 / 0**：含既有callback 2条与新增GET 2条 |
| UI Agent module | **3 / 0 / 0**：AgentPresence既有1条 + AgentCard/roster新增2条 |
| PostgreSQL 17.11 host SCRAM | AgentDirectory **1 / 0 / 0** |
| Clippy | contracts/domain/application/infra/agent/server/UI all-targets/all-features `-D warnings`绿 |
| WASM | UI wasm32 all-features绿（同时编译contracts Agent DTO） |
| GUI gates | i18n **391**；design **65 Rust/74 icons**；CSS **193** |
| bundle | WASM gzip **600783**；CSS **68843**；fonts **740216**；external/inline **1/0** |
| ledger | parity-check violation/warning **0/0**；strict recount **157/157/0** |

PG矩阵构造owner A、owner B、admin自有、public remote、system package、hidden、deleted与另一tenant
package Agent，逐项证明owner/other/admin list+get、mine/canManage分离、system protection、hidden双roster、
deleted/cross-tenant不可见，以及endpoint/hasAuth/hasCallbackToken投影。临时实例只监听127.0.0.1，
测试后停止并精确删除；没有连接用户数据库。

真实Chromium经testkit fixture复用production Axum static、HTTP framing与最终Leptos/WASM，实得：

1. 四卡精确mine2/explore2，卡片均144×180；AppSidebar Agents为current；
2. list/detail均200、`no-store`、13字段exact，forbidden wire key=0；
3. URL profile、直接hard reload、system badge、missing 404错误态、close后URL清理与返焦成立；
4. 1440×900/1024×640/900×640/600×800横向overflow均0；lg240、md48、compact Sheet保持；
5. Inter loaded=true，external resource=0，duplicate id=0，fake action=0，头像重复可访问名称=0；
6. 合法route最终console error=0；Chromium自身对modulepreload integrity有1条warning。首次浏览器还会
   请求尚不存在的favicon；brand icon仍是明确todo，因此本批不写“全局warning/error=0”。

fixture只证明GUI行为，不冒充生产PostgreSQL权限；后者由上述PG17/SCRAM承担。fixture与浏览器tab
均已停止/关闭，Playwright临时日志与非golden截图已删除。

## 5. 台账变化

| 口径 | Batch30 | Batch31 | 变化 |
| --- | ---: | ---: | ---: |
| API | 43 / 119 / 162 | **45 / 117 / 162** | +2 done |
| tests | 290 / 757 / 1047 | **300 / 747 / 1047** | +10 done |
| UI | 82 / 70 / 152 | **84 / 68 / 152** | +2 done |
| 全 parity | 546 / 1127 / 1673 | **560 / 1113 / 1673** | +14 done |
| fixtures | 15 / 22 / 37 | **15 / 22 / 37** | 0 |

## 6. 明确未完成

- [ ] T-TEST-0306：真正hide/unhide写事务与per-user移动；本批只验证既存偏好读面；
- [ ] Agent create/edit/duplicate/hide/unhide/delete的API、audit、并发package attachment与回滚；
- [ ] T-UI-0032完整AgentProfile动作、T-ROUTE-0007完整journey、T-UI-0126正式golden；
- [ ] `/channel/new`、首页`routeMessage`、四类fallback与create-time pinning事务；
- [ ] AppSidebar总项：new-channel/skills/settings/admin仍无真实destination；
- [ ] customer auth、三家recorded vendor trace、完整Agent事件UI、browser/file/shell等G4余面；
- [ ] G4/G6整关、brand favicon、31 route/golden/Tauri发行均未完成。

下一批应使用本批真实Agent roster作为recipient真源，闭合`POST /api/channels`与
`/channel/new`的create-time routing/fallback；不能先画fake composer或提前勾完整route。
