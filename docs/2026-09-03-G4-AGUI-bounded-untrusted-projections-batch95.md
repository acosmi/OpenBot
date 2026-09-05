# G4 AG-UI Bounded Untrusted Projections（Batch95）

> 日期：2026-09-03（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §2.4、§7.5、§13.2、§15.3、§16.5、§17.2、§24、§25、§28.1
> 基线：R168 / Batch94，`80a50e2a0b9519895204c57424df350c0a05f858`
> implementation：`20331887e7260c5be3749b6bb955dda7931bb6f8`

## 1. 本批关闭的范围

固定 `@ag-ui/core@0.0.57` decoder早已能验证state、messages、activity、step与raw/custom，但production `RemoteAguiSession`此前把这些事件全部静默丢弃。只有decoder单测不证明真实remote Agent、RunRuntime、PostgreSQL replay与UI消费边界支持它们。

本批关闭：

- `T-EVT-0004` STATE_SNAPSHOT / STATE_DELTA；
- `T-EVT-0005` MESSAGES_SNAPSHOT；
- `T-EVT-0006` ACTIVITY_SNAPSHOT / ACTIVITY_DELTA；
- `T-EVT-0007` STEP_STARTED / STEP_FINISHED；
- `T-EVT-0009` RAW / CUSTOM。

本批不关闭 `T-EVT-0003 tool-call-result`：虽然同一个projection类型已有正向单测，但还没有带权威offered tool、run assertion与callback事实的production PG正向。也不关闭 `T-EVT-0010 interrupt-resume`，它仍需durable human lifecycle。

## 2. 类型与安全边界

Application新增私有字段、non-serde `ProviderRemoteProjection`，只允许八个本地closed family：state、messages、activity、step_started、step_finished、tool_result、raw、custom。

持久化payload的结构由本地构造，remote不能覆盖：

```json
{
  "kind": "remote_agui_projection",
  "source": "remote_ag_ui",
  "family": "state",
  "untrusted": true,
  "untrustedKey": null,
  "untrustedType": null,
  "untrustedValue": {}
}
```

所有remote-controlled值只能位于`untrusted*`字段，不能形成actor、scope、grant、decision、capability或本地tool outcome。构造器迭代检查JSON key/value中的NUL，单projection编码上限1MiB；`RemoteAguiSession`再以checked累加限制每session最多4096项/8MiB。任何shape/size/overflow失败都只产生`provider_invalid_response`，Debug只打印closed family和字节数。

state/activity delta继续先在decoder clone上逐操作原子应用RFC6902并每步重验上限，projection只携最终完整值。messages snapshot只作为untrusted projection，绝不替换PostgreSQL `messages`权威transcript。raw/custom里的伪permission/instruction没有消费者。

## 3. Durable sequence与保留期

`DurableTextRun::append_remote_projection`先flush待写text，再与semantic chunk/tool checkpoint共用同一expected sequence。PostgreSQL以`event_type=checkpoint`、`kind=remote_agui_projection`写入，same sequence/same payload精确replay，不同payload conflict；run/thread cursor同步推进。

conversation snapshot此前把任意checkpoint都当成tool-exchange分段，新增projection会错误截断active streaming text。本批将lateral subquery收紧为只认`payload.kind=tool_exchange`；真PG证明projection前的`hel`与之后的`lo`仍组成`hello`。

projection只在active run可重放。任一completed/failed/cancelled/reconciliation terminal通过统一事务把内容收敛为：

```json
{
  "kind": "remote_agui_projection",
  "source": "remote_ag_ui",
  "retained": false,
  "untrusted": true
}
```

事件行、sequence、cursor和terminal不删除不改变。Batch95之前production没有这种checkpoint，因此不新增历史data migration；native latest仍为0027。和R167相同，这只是数据库当前可见值的逻辑清除，不冒充WAL/backup物理擦除。

## 4. UI quarantine

Web/Tauri现有thread event transport原样承载durable checkpoint。Conversation只接受上述exact full wrapper或terminal marker：source/family/untrusted/键集合漂移即判坏事件；合法projection只推进cursor，不触发snapshot reload、不进入streaming text、transcript或authority。伪`untrusted=false`测试明确失败。

本批没有为上游零使用的开放payload新造视觉卡片，也没有让remote messages替代权威历史；“UI支持”在这里是安全接收、校验、隔离与cursor replay，不是展示任意remote HTML/指令。

## 5. 真实 production vertical

同一production链路 `Remote Agent → SafeDialer/SSE → AguiDecoder → RemoteAguiProvider → BuiltInAgentRuntime → PostgresRunRuntime` 在服务端发送RUN_FINISHED前暂停。测试只有在数据库观察到以下事实后才放行terminal：

- 9条active projection按序为`step_started,state,state,messages,activity,activity,raw,custom,step_finished`；
- 9/9均为本地`source=remote_ag_ui`且`untrusted=true`；
- 第二条state为RFC6902后的`phase=done`；
- 第二条activity为patch后的`done=true`；
- raw canary在active期恰1，证明正向链路不是“从未保存”。

RUN_FINISHED后：projection marker恰9、reasoning marker恰1，visible reasoning、encrypted reasoning、raw projection与remote error canary在messages/run_events/audit联合扫描全部为0；assistant text仍为`remote answer`，terminal仍为completed，package provider调用0。

## 6. 验证

- Application：`161/0/0`。
- Agent：`50/0/0`，含全family顺序、tool-result offered约束、session 8MiB fail-closed。
- Infra lib宿主：`324/0/0`。
- UI完整：`181/0/0`。
- Testkit默认：`17/0/9 ignored`；完整真实PG Agent矩阵另跑`9/0/0`。
- PostgreSQL 17.11 run-runtime + conversation：`5+1/0/0`。
- Application/Agent/Infra/Server/Desktop/Testkit/UI all-target/all-feature Clippy `-D warnings`：通过。
- `cargo fmt --all -- --check`与UI wasm32 all-feature compile：通过。
- 钉版`tools fetch/verify`：Tailwind 4.3.3、Trunk 0.21.14、wasm-opt version_132、wasm-bindgen 0.2.127。
- production `trunk build --release --offline --locked`：通过；同源第二次构建八文件SHA逐字相同。
- i18n=`799` keys；design=`104 Rust files/74 icons`；CSS=`365` class literals。
- bundle：wasm gzip=`1,874,461/3,670,016`、CSS=`115,524/131,072`、fonts=`740,216/819,200`、external/inline script=`1/0`。
- parity=`830/874/1704`、events=`42/46/88`、fixtures=`21/22/43`、overlay=`1293/403/2/6`，0 violation。
- recount=`71/0/89 skipped`；89条均因未设置固定`OPENBOT_UPSTREAM_DIR`，strict未冒充通过。
- `grok-inventory --check`=2,110 files；Git tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`。
- 非Grok `package.json`仍恰1；无npm、无Cargo.lock/依赖、无schema/API/env变化；Actions仍manual-only。
- 临时PG17.11已fast stop并删除精确data/socket根，55495无listener。

## 7. 首跑与外部状态

- 第一次Trunk启动继承宿主`NO_COLOR=1`，被0.21.14按bool参数拒绝；仅对构建进程移除该环境变量后，offline/locked原命令通过。
- Batch93/94/95 push未完成：本机Git HTTPS无可用credential，`gh auth status`显示两个已登记账号token均invalid。没有伪称已push/建PR，也没有派发Actions；本地提交完整保留，重新登录后再推送。

## 8. 未声称完成

- `T-EVT-0003 tool-call-result`与`T-EVT-0010 interrupt-resume`仍todo；不以unit-only代码冒充production证据。
- 没有关闭完整AG-UI、G3、G4、G6或G8；computer runtime budget、三家recorded/live trace、Browser/file/shell、Desktop OAuth、MCP private egress/admin、外审/KMS、Windows/runsc、发行与golden仍未完成。
- 本批不展示或信任任意remote payload，不证明WAL/backup物理擦除。
- 按R63未运行`cargo xtask ci`，未派发GitHub Actions。
