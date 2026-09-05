# Batch110：正式 Screencast、Rust latest buffer 与 Frame ACK

日期：2026-09-04

implementation：`c5f22f424efec517f851c3b495d3de215be1e957`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§11.2–§11.3、§12.1–§12.6、§19.1 P2、§24 G5/G7、§28.1 R185

固定产品上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 结论与未完成边界

Batch110以正式`Page.startScreencast`替换Batch109的per-input `Page.captureScreenshot`过渡证据，并把
ACK authority从shim上收到Rust：frame只有经过完整header/payload/scope/generation/sequence校验并发布进
size-one latest buffer后，Rust才沿独立control write发送`frame_ack`，shim再调用
`Page.screencastFrameAck`。

macOS arm64的BrowserComputer与SandboxedComponent两个真实Seatbelt Electron role都完成
start→连续frames→slow consumer→input effects→stop→exact stop replay→shutdown。T-BROP-0030–0032与
既有T-FIX-0025据此done。

本批仍**不**关闭完整Screen/G7：

- 尚无Desktop viewer ticket/loopback WS、Server authenticated WSS或多viewer fan-out；
- 尚未落实10fps passive/15fps driving、component≤5fps及p95/p99 latency预算；
- `captureScreenshot` 2fps capability fallback尚未实现；
- ScreenIngress尚未接Server/Desktop production Computer composition；
- Windows protocol-v3 runtime与Ubuntu runsc/Xvfb未跑；P1仍红，不能宣称P2进入/通过；
- secret typed effect、坐标DPI/zoom/letterbox、drag完整序列与viewer跨scope注入仍todo。

## 2. 为什么升级 protocol v3

CDP `ScreencastFrameMetadata`官方字段包含timestamp、page scale、device width/height、scroll X/Y，但不含
device scale；v4 §12.3又要求device scale。因此shim只通过既有固定`Runtime.evaluate` probe额外读取
`window.devicePixelRatio`，不开放任意表达式。

官方协议依据：<https://chromedevtools.github.io/devtools-protocol/tot/Page/#event-screencastFrame>、
<https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-startScreencast>、
<https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-screencastFrameAck>。

frame wire不兼容扩展显式升级为protocol/release epoch=`3/3`，而不是继续篡改v2：

- magic=`OBFRAME2`，fixed header=`76 bytes`；
- header逐字段为protocol/role/mime/length、generation、sequence、captured-at-ms、device width/height、
  device/page scale、scroll X/Y、CDP screencast session ID、computer/tab长度；
- command新增fire-and-forget `frame_ack`，sequence按decimal string保全u64；
- `stopped`回传canonical decimal received/acknowledged counters与`replayed`；
- malformed timestamp/scale/scroll/header/payload在正文暴露前拒绝。

descriptor还单源固定screencast参数：JPEG、quality70、1280×800、everyNthFrame1、
maxFramesInFlight1、sendLastFrame=true。generated protocol SHA-256=
`16cc1f4d35aa8d0437ae43f7d81595eed378dcb864bc7bd190be075232f3f8bf`。

## 3. Rust latest buffer与ACK时序

`ScreenIngress`后台任务独占frame read half与stateful decoder；`watch`只保留最新一个`Arc<EngineFrame>`。
发布与consumer sequence在同一Mutex内比较：上一published尚未consume时，新frame替换它并将drop counter
checked-add；consumer永远读最新值，不形成无界队列。

时序严格为：

1. shim收到并校验CDP event，写一帧binary pipe；
2. Rust在分配前验长度，再验全部metadata/scope/generation/sequence/JPEG；
3. Rust在size-one buffer内原子publish并计算是否drop旧帧；
4. Rust发送绑定computer/generation/tab/frame-sequence/CDP-session的`frame_ack`；
5. shim exact-match pending frame后调用literal `Page.screencastFrameAck`。

maxFramesInFlight=1确保Chrome在第5步前不再推进。input ACK与frame channel继续独立，Screen生产者不会被
慢UI阻塞。任何frame decoder/ACK writer/task join失败都会使ScreenIngress显式失败；stop不能用部分stats
冒充成功。

## 4. 真机背压与停止语义

固定conformance页的CSS hover动画制造连续frame，测试在400ms内故意不消费。多次重跑中每个role均满足：

- received=acknowledged，观测范围49..52帧；
- dropped-before-consume范围37..40且>0；
- consumer随后拿到最新帧，内存面仍恒为1；
- frame timestamp>0，device/page scale>0，scroll与尺寸有限；page wheel后scrollY真实增加；
- Page.stopScreencast后先撤event listener、排空已写frame与ACK，再detach/destroy；
- 同tab第二次stop不重复CDP effect，返回相同冻结stats与`replayed=true`；
- 主进程/全部后代TCP LISTEN=0，shutdown后PID/profile lock=0。

动态帧数受调度影响，fixture只钉`received=ack`与`dropped≥1`，不把某次精确数伪装成稳定oracle。

## 5. Bundle与实跑矩阵

官方Electron macOS arm64 archive仍为`122102881 B`、SHA-256 `ee939d1564…`、version43.3.0。
protocol-v3 bundle：`app.asar=29806 B`、header SHA-256
`0297234048a52c164c34f3c7c9e31749208411972dfa0e73ee6f65110f289772`、fuses=`000011001`；
signature/embedded integrity/rebrand/manifest verify全绿。shim仍3文件、唯一package、零npm，
`596/600 LOC`；literal CDP allowlist已从capture替换为Page start/stop/ack。

| 检查 | 结果 |
|---|---|
| Contracts | `104/0/0` |
| Computer all-features | lib=`58/0/0`，fixture=`2/0/0`，host=`0/0/2 ignored` |
| 真机 `engine_conformance --include-ignored` | `4/0/0`；真实role=`2/0/0` |
| xtask | `103/0/0` |
| native Contracts/Computer与testkit Clippy | 通过 |
| Windows Computer all-target/all-feature Clippy | 通过；runtime未跑 |
| protocol/shim/bundle/verify | v3 hash、596/600、ASAR/fuses/integrity/signature全绿 |
| parity | `871/839/1710`；browser=`17/33/50`；overlay=`1273/429/2/6`；0 violation/warning |
| fixtures | `29/21/50`；T-FIX-0025由todo转done，不新增重复ID |
| non-strict recount | `71/0/89 skipped`；无上游目录，strict未跑 |

首次shim为628/600，被预算闸门拒绝后在不删判据的前提下压至596；首次正式真机wheel测试把内层overflow
滚动误当page scroll且读到先前同图frame，实得`1/2`，改为页面级滚动并等待distinct frame后才绿；首次
台账修改又误把T-BROP-0001–0003置done，parity按R124判红后恢复并将证据移到正确0030–0032。三类首次
失败均未计通过。

本批无schema/native/API/route/UI/Web bundle/dependency/Cargo.lock/env变化；Cargo package仍829，
未运行R63禁止的`cargo xtask ci`，未派发Actions。

## 6. 下一步

接ScreenHub多viewer owner、Desktop/Server ticket与binary WS，并把同一ScreenIngress装入production
Computer composition；随后实测fps/latency/last-viewer 2秒stop、resize/navigation/tab切换与跨scope注入。
Windows和runsc必须各自用protocol-v3 bundle重跑，不得由本次macOS证据外推。
