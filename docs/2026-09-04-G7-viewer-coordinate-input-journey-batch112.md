# Batch112：Viewer 坐标映射与 IME/拖拽真实输入旅程

日期：2026-09-04（America/Los_Angeles）

分支：`feat/2026-09-04-G7-viewer-coordinate-journey`

implementation：`49da4d9c8da7db25ae631c332de5fdc517dba8a1`

第一真源：`CLAUDE.md`；v4 §12.3、§12.5–§12.6、§21.5、§24 G5/G7、§28.1 R187

## 1. 本批结论与边界

Batch112新增Rust `screen::coordinates`，把一个viewer实际显示的`object-fit: contain`画面从client CSS
坐标映射为CDP main-frame viewport CSS坐标，并另算document hit-test坐标；letterbox黑边、坏decoded比例、
NaN/Infinity、过小/过大decoded surface及无效canvas全部fail-closed。映射携带来源frame sequence，避免上层
把几何来源丢掉。

同一映射已在macOS BrowserComputer与SandboxedComponent两个真实Electron role驱动普通input：命中
letterbox后的button、执行`mousePressed → mouseMoved → mouseReleased`、以`Input.insertText`输入完成IME文本，
并把viewer wheel delta换算为CDP CSS delta后观察真实scrollY增加。

本批没有接Leptos canvas DOM事件、Server/Desktop WebSocket、stale displayed-frame transport拒绝或production
Computer assembly；非1 DPR、非1 pageScale和nonzero-scroll pointer只在pure矩阵，不是硬件实测。resize、
navigation、tab switch、Windows与runsc也未跑。因此不新增或关闭browser parity T-ID，不勾G5/G7整关。

## 2. 官方单位与公式纠偏

CDP官方定义钉死：

- `Page.ScreencastFrameMetadata.deviceWidth/deviceHeight`是DIP；`scrollOffsetX/Y`是CSS px；
- `Input.dispatchMouseEvent.x/y`相对main-frame viewport且单位为CSS px；wheel delta同为CSS px；
- `Input.insertText`明确用于“不来自按键”的文本，包括emoji keyboard或IME。

为避免只凭协议字段名猜公式，本轮继续核对ChromeDevTools官方前端：

- commit=`036dd84bc4fdfb0fd4be2a5ddb3fe37ef24939cd`；
- `InputModel.ts`：3982 bytes，Git blob=`cfa97617c47f1b01957429f1bfdc96ebd6fe07d7`；
- `ScreencastView.ts`：38774 bytes，Git blob=`978df09ac874f754df3e7a353cc544616cee5516`。

官方`InputModel`只以`screenZoom`把canvas点/轮滚换成viewport坐标；`pageScaleFactor`只在
`ScreencastView`的DOM hit-test中用于`viewport / pageScale + scroll`。本轮首版曾把pageScale同时除进CDP
input坐标；macOS实跑恰为pageScale=1，没有暴露，synthetic测试也一度把错误答案写成期望。追到官方实现
后改为：

```text
viewport_css = canvas_inside_image × device_dip / rendered_image_css
document_css = viewport_css / page_scale + scroll_css
wheel_css    = viewer_delta × device_dip / rendered_image_css
```

device scale在`physical = DIP × DPR`和`physical / DPR`之间恰消一次；Retina不能让同一个DOM target偏移。

## 3. 封闭几何与错误语义

- `CanvasRect`只接受finite坐标、正尺寸和绝对位置/尺寸≤1,000,000 CSS px；
- `DecodedFrameSize`每轴固定16..=16,384，且decoded与authenticated frame DIP aspect最多容忍一像素取整；
- `contain`实际图像rect从canvas与decoded intrinsic size唯一推导；上下/左右letterbox全部拒绝，right/bottom
  用exclusive边界，避免映到viewport宽/高之外；
- `MappedScreenPoint`同时携frame sequence、viewport与document坐标；wheel不加scroll、不除pageScale；
- stable error不包含client坐标或frame metadata，Debug没有额外页面内容。

## 4. Pure矩阵与真实旅程

四条pure测试覆盖：

1. 640×500 canvas内含1280×800 bitmap，上下各50px letterbox；client 140×184精确映射viewport
   80×168，scroll10×20时document为90×188；黑边与right edge拒绝；
2. DPR 1/2映射结果相同；pageScale=2保持viewport 640×400，只把document贡献减半为320×200再加scroll；
3. canvas 640×400时viewer wheel -10×25映射viewport CSS -20×50，scroll不参与delta；
4. zero/1px/超界decoded、坏aspect、NaN/Infinity及超界canvas位置全部拒绝。

真实macOS conformance使用同一个640×500 letterbox模型：

- client 140×184 → viewport 80×168，button hover/press命中；
- down后把pointer移到映射后的300×168再up，三条literal CDP input逐条收到exact ACK；
- 聚焦输入框后`日本語🔐`只经`Input.insertText`，截图SHA相对空值改变；没有IME protocol variant；
- client 550×450 → viewport 900×700；viewer deltaY=200 → CDP deltaY=400，随后frame scrollY增加；
- Browser/Component最终均`received=acknowledged=69`、slow ingress drop=37，TCP LISTEN/PID/profile lock=0。

## 5. Fixture、验证与台账

新增`fixtures/computer/screen-coordinate-input-journey-v1.json`：2707 bytes，SHA-256
`8a03fd6842ed19466519fb3f3f264253c8a43f9b3d1fdbd885f88c4b802714ef`。T-FIX-0052只锁pure
coordinate与macOS ordinary input组合journey，所有production/硬件/跨平台余面逐项为false。

| 检查 | 本轮结果 |
|---|---|
| coordinate pure | `4/0/0` |
| `openbot-computer --all-features` | lib=`64/0/0`；fixture=`4/0/0`；host=`0/0/2 ignored` |
| 真实`engine_conformance --include-ignored` | `6/0/0`；真实role=`2/0/0` |
| native/Windows all-target/all-feature Clippy | `-D warnings`通过；Windows runtime未跑 |
| Linux target | all-feature check通过；runsc/Xvfb runtime未跑 |
| protocol/shim/bundle/verify | v3 hash=`16cc1f4d…`；shim=`596/600 LOC`；原ASAR/fuses/integrity/signature不变且通过 |
| parity | `871/839/1710`；browser=`17/33/50`；overlay=`1273/429/2/6`；0 violation/warning |
| fixtures | `31/21/52`；新增T-FIX-0052，不改变既有parity状态 |
| recount | non-strict=`71/0/89 skipped`；无上游目录，strict未跑 |

首轮pure实得`1/3`：合法canvas原点0被误用“必须>0”的尺寸断言拒绝，修复后才4/0。其后虽然测试绿，
官方DevTools源复核又推翻pageScale错误公式；替换错误期望并重新跑完pure/真实/跨target闸门。两次中间结果均
不计最终通过。

本批无schema/native/API/route/UI/Web bundle/dependency/Cargo.lock/env变化；Cargo package仍829，Grok parent
tree与2,110文件inventory不变，非Grok package.json仍1、零npm、Actions manual-only。未运行R63禁止的
`cargo xtask ci`，未派发Actions。

## 6. 下一步

把`ScreenCoordinateMap`接真实Leptos canvas事件与ticket-bound viewer input时，必须在Rust transport侧重验
frame sequence/window/auth generation，不能相信renderer回传的“当前geometry”。随后实跑非1 DPR、非1
pageScale、nonzero scroll、resize/navigation/tab switch/close，并接Server/Desktop binary WS；之后才可把
本批pure/harness证据升级为production G7证据。secret typed effect与Browser snapshot/ref仍为独立后续。
