# Batch111：Engine-backed ScreenHub 与 viewer ticket 核心

日期：2026-09-04（America/Los_Angeles）

分支：`feat/2026-09-04-G7-screen-hub-ticket-core`

implementation：`ddf49ee5b2b09b940528614330a228ab68077c0a`

第一真源：`CLAUDE.md`；v4 §10.1、§11.1–§11.3、§12.2–§12.4、§12.6、§19.1 P2、§24 G5/G7、§28.1 R186

## 1. 本批结论与边界

Batch111把Batch110的单consumer `ScreenIngress`推进为真实Engine来源的Rust `ScreenHub`核心：同一
BrowserComputer或SandboxedComponent engine session只能交出一个`EngineScreenSource`；Hub为多个viewer
共享同一份latest frame，并以actor/auth-generation/stream/origin/window绑定的128-bit一次性ticket授权。

本批**没有**实现Server authenticated WSS或Desktop loopback WS，也没有接production Computer composition、
auth-generation事件hook、每连接帧大小/带宽/idle限制、fps/p95、最后viewer断开2秒停流或
`captureScreenshot` fallback。因此`T-BROP-0027`、P1/P2、G5、G7继续todo；不能把“ticket/Hub纯核心”写成
“完整Screen transport已完成”。

## 2. Authority与唯一Engine来源

- `EngineLaunchConfig`现在强制携带Rust authority铸造的`ScreenAudience`，固定tenant、actor与
  `AuthGeneration`；component role另在任何目录/进程副作用前校验actor与role完全相同，tenant两role都
  必须相同。audience不进入engine wire，也不交给renderer。
- `ScreenStreamKey`绑定完整scope digest、ComputerId、ComputerGeneration与TabId；renderer只看到原有
  opaque engine协议，不获得这些Rust authority字段。
- active session只能调用一次`take_screen_source()`；第二次稳定返回
  `engine_screen_source_already_issued`。Hub只接受该不可Clone source，避免第二个consumer绕过viewer授权。
- 复核中发现首版source clone消费watch frame却未推进共享`consumed_sequence`，会在生产只走Hub时把已消费
  帧误计为drop。最终实现把同一`ScreenIngressState`带入source，每次latest/next都推进共享消费序列并在
  ingress失败时关闭；真实测试把engine慢consumer与viewer慢consumer拆成两层独立证据。

## 3. Ticket、binding与撤权

- ticket由OS CSPRNG直接生成16 bytes，发行对象以`SecretBytes`持有并在drop时清零；Debug只显示
  `[REDACTED]`。Hub只保存SHA-256 digest，不保存raw ticket。
- TTL固定30秒，`expires_at <= now`即过期；成功握手后digest立即删除，重放稳定
  `screen_ticket_invalid`。错误actor/generation/origin/window返回同一`screen_not_visible`，错误binding不会
  烧掉合法ticket。
- host API把非秘密base protocol `openbot.screen.v1`与秘密requested protocol
  `obot_screen_<32 lower-hex>`分开；ticket不进入URL/query，未来upgrade response只能选择base protocol。
  真实HTTP/WS选择仍属下一批transport，不由本核心冒充。
- Server binding固定已验证origin；Desktop binding再固定window label与非零window-binding generation。
  ticket同时绑定tenant、actor、auth generation及完整stream key。
- `invalidate_actor`删除该actor所有旧generation ticket/stream，向已有viewer发布显式revoked并停止source
  task。当前这是可执行核心primitive；production session/role变更事件尚未接线，fixture明确为false。

## 4. 多viewer latest与viewer binary

- Hub cap显式为1..=256；active viewer与pending ticket共同占用cap，不能靠批量预签票超订。
- Engine ingress与Hub各只保留一份latest，总计每tab最多两份；所有viewer的watch receiver共享同一
  `Arc<ScreenViewerFrame>`，没有按帧率增长的per-viewer queue。
- viewer在连接时记录当前sequence，之后按gap累计`skipped_frames`；慢viewer取到最新值时可机械证明旧帧
  已被coalesce。单测证明viewer A从1直接到3时skip=1，viewer B逐帧消费时skip=0。
- viewer wire为`OBSCRN01`、version1、68-byte little-endian header + JPEG；只含generation、sequence、
  timestamp、尺寸、scale与scroll。scope digest、computer/tab ID、ticket及内部CDP screencast session ID
  构造性不写入binary。

## 5. Fixture与真实Electron证据

新增`fixtures/computer/screen-hub-ticket-core-v1.json`：2368 bytes，SHA-256
`8559fec9d7442990a0e416a09c2b5e86d54799559c6ba39902c94b3d36f4c83f`。T-FIX-0051只关闭
Engine-backed source、latest/multi-viewer、ticket与generation invalidation核心；十项生产/性能/平台余面
逐项为false。

macOS arm64重新下载并验证官方Electron 43.3.0：archive SHA-256
`ee939d1564d83d61032b3b3cb23af4e46005a4900c91f0695f7ed793f0ce6e83`；protocol/epoch仍3/3，ASAR
29806 bytes、header SHA-256
`0297234048a52c164c34f3c7c9e31749208411972dfa0e73ee6f65110f289772`、fuses=`000011001`。

BrowserComputer使用Server binding，SandboxedComponent使用两个不同Desktop window binding；两种真实
Seatbelt role都验证：错误actor发票拒绝、错误binding不烧票、重放拒绝、双viewer初始frame相同、完整input
矩阵后两viewer取得同sequence且`Arc::ptr_eq`、各自skip>0、auth generation推进后两者均closed。最终Browser
实得`received=acknowledged=68`、Component为69，两者engine ingress slow-consumer drop均为37；主/全部后代
TCP LISTEN=0，shutdown后PID/profile lock=0。

## 6. 本轮验证与台账

| 检查 | 本轮结果 |
|---|---|
| `openbot-computer --all-features` | lib=`60/0/0`；fixture=`3/0/0`；host=`0/0/2 ignored` |
| 真实`engine_conformance --include-ignored` | `5/0/0`；真实role=`2/0/0` |
| native/Windows all-target/all-feature Clippy | `-D warnings`通过；Windows runtime未跑 |
| Linux target | `x86_64-unknown-linux-gnu` all-feature check通过；runsc/Xvfb runtime未跑 |
| protocol/shim/bundle/verify | v3 hash=`16cc1f4d…`；shim=`596/600 LOC`；ASAR/fuses/integrity/signature通过 |
| parity | `871/839/1710`；browser=`17/33/50`；overlay=`1273/429/2/6`；0 violation/warning |
| fixtures | `30/21/51`；新增T-FIX-0051，既有todo未冒进 |
| recount | non-strict=`71/0/89 skipped`；未配置上游，strict未跑 |
| 硬约束 | Grok parent tree=`86f5a85f…`；2,110文件inventory绿；非Grok package.json=1；零npm lock；Actions manual-only |

本批无schema/native/API/route/UI/Web bundle/dependency/Cargo.lock/env变化；Cargo package仍829。未运行R63
禁止的`cargo xtask ci`，未派发GitHub Actions。

## 7. 下一步

从本核心接Server authenticated WSS与Desktop loopback WS：由各自可信transport验证cookie/origin或Tauri
window binding，只选择base protocol，并把viewer frame写入binary WS；同时接production auth-generation
失效hook、frame-size/bandwidth/idle cap和最后viewer 2秒停流。之后才能在production composition上跑
fps/p95、DPI/zoom/letterbox、resize/navigation/tab切换、restart/replay/跨scope注入与component ≤5 fps矩阵。
Windows真机和Ubuntu runsc/Xvfb仍必须各自运行，不能从本次macOS证据外推。
