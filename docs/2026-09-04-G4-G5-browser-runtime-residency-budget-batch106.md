# Batch106：Browser runtime residency budget 核心与 Rust owner

日期：2026-09-04

selector implementation：`2385493d5e2c6472b4470e39d167e60f272c4f6f`

manager implementation：`b92c401a540281460c98542a8f29583343746578`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§7.4、§10.1、§10.6、§11.1–§11.3、§17.2–§17.3、§18、§24 G4/G5、§28.1 R181

固定产品上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 本批结论与边界

Batch106关闭固定上游`browser-eviction.test.ts`的13条LRU/idle纯判据，并新增Rust-owned
`BrowserRuntimeManager`，把选择结果落实为owned driver handle的实际close，而不是只返回建议列表。

当前已证明：

- 默认live cap=`8`、idle timeout=`1,800,000 ms`；
- cap在冷启动成功后以最近使用次序选择非活动LRU，刚启动/刚使用的browser不被逐出；
- 同一完整scope并发冷启动只调用driver一次；
- 全部slot仍被activity lease占用时，新增launch之前返回稳定`computer_runtime_busy`；
- idle恰在cutoff时关闭，但持lease的handle不关闭；
- full `ComputerSecurityScope` digest、`ComputerId`与`ComputerGeneration`共同绑定instance；stale generation与
  同scope不同computer identity分别稳定拒绝；新generation先退休旧handle；
- launch失败恢复此前预留的LRU victim；显式stop与shutdown实际consume driver handle；
- manager错误Display/Debug只输出stable code，不复制driver canary/prose。

本批**没有**把manager接入Server/Desktop production assembly或真实`EngineProcess`，也没有实现
`COMPUTER_MAX_BROWSERS`/`COMPUTER_BROWSER_IDLE_MS`配置入口、CPU/RSS/pids/disk quota、Supervisor/runsc、
Browser action、ScreenHub或file/shell。因此只关闭selector parity与manager-owned residency子项；完整
Computer runtime budget、ComputerManager、G4/G5与P1/P2继续todo。

## 2. 固定上游来源

三份来源均从固定commit经GitHub Contents API读取并独立计算：

| 文件 | Git blob | bytes | SHA-256 |
|---|---|---:|---|
| `agent-computer/src/browser-eviction.ts` | `8fa9bcc4899961ebac7bc29414cbbd5fb74d673f` | 2,489 | `01e4912c6301dbe01badbb3ca715139371157b4367c475923e04dda4b2bcedf8` |
| `agent-computer/src/profiles.ts` | `b0b2e714e6f720bd22f6c330e842b4746ab146db` | 18,499 | `16dbc32e19bf4c1f7d99e2b5c5ee8499c55d7f78b4a3f8a2e8e6c228200070df` |
| `agent-computer/tests/browser-eviction.test.ts` | `218178c9a41e8c256b443ba2bfe49e60c0f34e0a` | 5,240 | `5dc110dd32b3053cb7c29d4ec990e7ba5d5aad4c71906c17fb6176e3abb4cfd4` |

上游observable state machine是：live map记录每个Bot的`usedAt`；cold launch去重；launch成功后执行cap；
cap按稳定LRU关闭`len-max`项；idle以`usedAt <= now-timeout`为边界；stop保留on-disk profile。Rust不照搬
Bot-only key，而按v4 §10.1把key增强为完整security scope。

## 3. Rust manager不变量

`BrowserRuntimeManager<D>`对所有driver具体类型保持同一闭包：

1. live handle只存在于manager state或临时lifecycle owner，不交给renderer/transport；
2. `BrowserLease`存活期间计数非零，cap/idle/generation/stop都不能选择该handle；
3. 所有cold launch与retirement经一个lifecycle mutex串行，已有browser operation仍可并发；
4. cap满且无非活动victim时effect前busy，不能临时再起第`max+1`个不可回收browser；
5. launch失败把尚未close的victim原样恢复；close消费handle，driver契约要求Drop为forced cleanup；
6. touch order checked-add，耗尽或Mutex poison/Arc ownership漂移统一`computer_runtime_invariant`，不wrap；
7. shutdown先永久关闭new-lease入口；仍有lease就busy，lease drop后重复调用完成全部retirement；
8. scope轴不进入公开wire；Debug只显示digest标记、computer ID与generation。

## 4. 闭合fixture与机械证据

`fixtures/computer/browser-runtime-budget.json`：1,579 bytes，SHA-256
`1dc491809ec8f7b84919349f492cfbff955ce034923ccc4b15858aea5c6e53ce`。

fixture固定defaults与七类case，并显式写：

- `pureSelector=true`；
- `managerOwnsDriverHandles=true`；
- `serverOrDesktopEngineAssembly=false`；
- `cpuRssPidsDiskBudget=false`。

最终证据：

- `browser::eviction`=`15/0/0`：13条固定上游映射 + stable equal-time + overflow负向；
- `manager`=`10/0/0`：fixture shape/hash、budget、secret-free errors与八条lifecycle/concurrency case；
- `openbot-computer --all-features --locked`=`43/0/2 ignored`；两条ignored仍是真Electron host
  conformance，未冒充运行；
- all-target/all-feature locked Clippy `-D warnings`、fmt、diff-check；
- `cargo xtask parity-check --json`：parity=`861/849/1710`，tests=`470/577/1047`，
  fixtures=`26/22/48`，overlay=`1286/416/2/6`，0 violation/warning。

按R63未运行`cargo xtask ci`，未派发GitHub Actions。未跑strict recount（当前没有固定上游目录）；没有
schema/native/API/route/UI/env/dependency/Cargo.lock/workflow变化，`grok-bot`、零npm与唯一非Grok
`package.json`不变量不变。
