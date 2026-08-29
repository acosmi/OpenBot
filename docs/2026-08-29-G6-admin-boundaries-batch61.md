# OpenBot G2/G6 Admin Boundaries Batch61

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-admin-boundaries`

基线：Batch60 PR #43 已以 merge commit `aab3f1dbd1504100f872571710cda2223a44bae0` 合入 `main`。

implementation：`ae8e9c73f46b17d97c54380a29931feb612dd5ca`

## 1. 结论

本批把既有 production PolicyStore、typed GET/PUT 和 CEL policy 接成受 AdminShell 保护的 `/admin/boundaries` 真实旅程：

- 未配置仍是 `policy:null` / default-deny，页面不会猜一份策略；
- 首次向导没有预选项，管理员必须明确选严格基线或固定上游兼容基线；
- 已配置后可切 enforce/dry-run，按顺序添加 preset/自定义 deny、移除单条，并在两个基线间显式切换；
- 迁移来的 custom allow 逐字只读保留，页面不提供覆盖 preset 的按钮；
- non-admin 在 child 构造前统一 NotFound，policy GET/PUT 为 0。

只关闭 `T-ROUTE-0014`。完整 AdminSidebar `T-UI-0028` 与 formal Boundaries golden `T-UI-0133` 继续 todo。

## 2. 首次设置与共享契约

固定上游 GET 总会返回默认 `enforce / deny=[] / allow=["true"]`；v4 §8.3 已裁决新安装不得有该隐式 allow，必须由 admin 完成首次设置。因此本批定义两个唯一可判定 preset，二者都必须点击后才 PUT：

```text
严格基线：   mode=enforce, deny=[], allow=[]
上游兼容：   mode=enforce, deny=[], allow=["true"]
```

选择前所有 acting tool 继续 deny。选择后两种 exact baseline 都有显式反向按钮；若 allow 不是空表也不是单个 `true`，页面判为 custom，只读显示且零 baseline 按钮，不把迁移策略压成二值。

`MAX_ACTION_POLICY_EXPRESSION_BYTES=4096` 从 domain 上收到 wasm-safe contracts，domain parser re-export 同一个值，GUI 新增规则也消费它。共享 `ActionPolicyResponse { policy: Option<ActionPolicyDocument> }` 后，Server/UI 不再各写一份 response envelope。

PUT 始终替换完整有序文档；只有 response 中存在 policy 且逐字段等于请求时，页面才更新。actor、admin、freshness、Origin、updated_by 均不进入 body。

## 3. 规则编辑不变量

- 新增输入走共享 ECMAScript TrimString；空、超过 4096 UTF-8 bytes、exact duplicate 均在请求前拒绝；
- fixed-upstream 三条 preset 原文逐字保留，最长表单规则 143 bytes；
- existing 空/坏/超限规则仍显示并可移除，因为 production fail-closed 语义不能被 GUI 隐藏；
- row key 含 index+rule，remove 只删一个 index，不按字符串一次删掉全部重复行；
- deny 顺序不变，allow 自定义顺序不变；
- load/save worker 固定到 `AdminBoundariesPage` stable owner。

浏览器发现 duplicate draft alert 在成功移除对应规则后仍残留；此时错误已经不符合 current policy。修复为任何成功 authoritative PUT receipt 都清 draft validation error；失败仍保留输入与提示。新 bundle/新 host/新 tab 复验后，remove 使 `aria-invalid=false`、alert=0，draft 留在输入中且可再次合法提交。

## 4. Release memory fixture 浏览器证据

memory PolicyStore 从 `None` 开始：

- 首屏只发一次 policy GET；first-setup article=2、mode controls=0；
- 分别在两次全新 fixture 中选择兼容与严格 preset，均只有点击后进入配置页；
- enforce↔dry-run、default-deny↔allow true 双向成立；
- preset 与 Enter 自定义规则按顺序保存，duplicate 保持行数且显示本地化 alert；
- 按 index remove 后 stale alert 清除；
- hard reload 保留 dry-run、固定 preset deny 和 allow true；
- zh-CN/English 的 h1、三个 section、mode、rule label、Audit link 全部切换；
- 600×900 form/preset 单列，1280×900 双列；horizontal overflow=0；
- main=1、nav=2、h1=1、admin current=1、duplicate DOM id=0、runtime console error=0。

Chromium 固定报告 modulepreload integrity 不适用的 1 条 warning；没有把它伪报为 0 warning。

## 5. 真实 PostgreSQL/session 浏览器竖切

一次性 PostgreSQL 17.11 仅监听 `127.0.0.1:55461`，host auth 为 SCRAM-SHA-256；数据库名满足既有 `openbot_ui_approval_fixture_` guard。PG fixture 使用 production keyed `SessionTokenHash` / `PostgresSessionAuthResolver` / `PolicyStore`，并预置：

```text
mode=enforce
deny=[]
allow=['actor.id == "fixture-actor"']
updated_by=fixture-seed
```

前置实得无 cookie policy=401、testkit bootstrap=303。浏览器经 host-only HttpOnly/Lax cookie 进入后：setup=0、custom allow 原文可见、custom notice=1、baseline button=0。添加 password preset 并切 dry-run，数据库精确为：

```text
mode    | deny                                                        | allow                             | updated_by     | current rows
dry-run | [intent == "type" && contains(element.name, "password")]  | [actor.id == "fixture-actor"]   | fixture-actor  | 1
```

hard reload 后 mode/deny/custom allow 与零覆盖按钮保持。它证明 GUI 写面穿过真实 session、ApplicationService 和 PG store，不是浏览器本地状态。

随后原子删除当前 actor 的 admin role，并同步 user/session generation=2。全新 tab 只发生 global probe + shell gate 两次 `/api/admin/status` 403；policy GET/PUT=0，localized NotFound，global Admin/admin nav/mode/setup 全部 0。两条 403 是预期 network console 事件，WASM/runtime error=0。

fixture 停止时 approval waiter 因 actor generation/role 已变而 denied，符合 fail-closed。PG/data/socket/log/password 全清，55461 无响应。

## 6. “先试再拒”的 production 语义证据

本轮亲跑：

- fixed-upstream policy matrix `28/0/0`，含 dry-run forward 与 enforce 拒绝；
- CEL corpus `6/0/0`，69 条 corpus 与 6 条分歧台账不漂；
- Application tool pipeline `1/0/0`：enforce refusal 先审计、零 attempt；同规则 dry-run 记录 refusal 后仍执行；
- Server policy route `4/0/0`：exact deployment route、fresh admin/Origin、非 admin 前置拒绝；
- Axum/in-process same Arc `1/0/0`。

因此“记录并放行”不是一句 UI 文案，既有 acting pipeline 的真实语义也在本轮复验。

## 7. 机械证据

| 面 | 结果 |
| --- | --- |
| contracts / domain policy / UI | `88/0/0`；policy filtered `67/0/0`；UI `143/0/0` |
| Server | lib `213/0/0`；main `7/0/0`；migrate `3/0/0`；fixture `4/0/0`；PG-only suites按定义 ignored |
| policy focused | upstream `28/0/0`；CEL `6/0/0`；Application `1/0/0`；Server `4/0/0`；transport `1/0/0` |
| Clippy / wasm | 六 crate all-target/all-feature `-D warnings`；UI wasm32 locked check均通过 |
| GUI build | pins verify + release/offline/locked Trunk build通过；零 npm |
| i18n/design/CSS | 637 leaf keys；94 Rust files/74 icons；313 class literals |
| bundle | wasm gzip `1,527,597/3,670,016`；CSS `103,671/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| parity | routes=`13/19/32`；UI=`87/65/152`；总=`700/994/1694`；overlay=`1595/97/2/0`；0违反 |
| strict recount | `159/0/0`，固定上游 `891df72f…`，skip 0 |
| Grok/shim | tree `86f5a85f…`；inventory 2,110；shim `405/600`；单 package/零 npm锁守卫通过 |

用户要求清理本仓编译垃圾后，本轮先删除 `target/` 17.0 GiB、`target-xtask/` 1.3 GiB 与 Trunk dist；源码零损。为完成证据按 pins 重建必要工具/目标，最终交付前再次清理可重建产物。一次 Trunk 命令误在仓库根执行，工具在编译前以“找不到目标 crate”拒绝；随后在 `crates/openbot-ui` 正确目录重跑并以最终 bundle/gates 为证据，不把失败尝试写成通过。

## 8. 明确未做

- 未实现完整 AdminSidebar 其余 destination；`T-UI-0028` 保持 todo。
- 未生成 Boundaries formal golden；`T-UI-0133` 保持 todo。
- 未新增第三种 policy mode、自由 policy language 或自动 allow 默认。
- 未把 testkit 303 冒充 OIDC/SAML 登录。
- 未运行 `cargo xtask ci`，未派发 GitHub Actions（R63 manual-only）。
- P1 Windows/runsc runtime 仍红，未进入 P2；`grok-bot/` 零改动，无 Grok 文本/能力进入本批。
