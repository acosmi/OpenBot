# macOS Helper 加载器与发布边界

日期：2026-09-05。v4 §10.5 / §11.2–11.3 / §16.2 / §24 / R206。
本批承接并完成既有helper-loader在制差量的编译修复与定向验收，不包含前端重设计或其Cargo依赖变更。

## 修复范围

原macOS主进程的精确文件/管道规则不等于被释放父profile继承的helper已经安全。
保留的旧版同探针日志表明：受控scope中的动态库可经helper执行，在邻接测试目录写出marker。
本批为诊断包采用显式hardened ad-hoc签名，清除原继承的宽松entitlements；固定只保留JIT与
诊断需要的library-validation例外，未开启DYLD环境、unsigned executable memory、debugger或
executable-page-protection例外。全部main/helper均核对signature、hardened/ad-hoc flags和精确entitlement集合。

该library-validation例外仍是诊断限制，不能冒充真实发行身份或完整Engine compromise边界。
参考：[Apple Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime)、
[DYLD entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.allow-dyld-environment-variables)、
[library validation](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.disable-library-validation)。

包装epoch从4升为5、manifest schema从1升为2，操作协议仍为4。
manifest新增signing_profile以及5个macOS helper的hash；runtime在spawn前核对完整文件集合及内容hash。
本批复核时另补齐打包CLI的同一集合要求，并限制executable/fuse/ASAR为固定产品布局：缺少helper hash、
把helper改为主入口、额外路径或修改helper字节都不能通过验证。CLI不会按额外manifest路径读取根外文件。

`engine verify`明确仅验证local fixture；`engine verify --release`在完成原始包与诊断包校验后明确拒绝。
当前还没有生产签名profile实现，不能以省略flag、诊断签名或局部测试将该包记成可发行。
Engine中的fixture名仍属于诊断资产，Wrok Bot正式产品身份及整体更名继续属于首版安装交付工作。

## 本轮证据

本机macOS arm64；不能外推Intel/Windows/Linux。命令均locked、关闭incremental/dev/test debug，
不运行CI/Actions。原始日志位于忽略目录 `target/qa/macos-engine-20260905/`。

- 新增的旧schema/缺失helper/篡改helper用例首先暴露缺失Digest trait导入，修正后真实fixture测试1/0/0；
  同一隔离副本先正向加载，再修改独立复制的helper字节而拒绝。其余只读hardlink不被修改。
  后加的helper冒充主入口负向同样通过。首次Clippy发现Copy类型冗余clone，已修正并复跑。
- 实际helper加载器探针1/0/0：动态库在无保护的测试二进制中能写出预设marker作为正向；
  当前helper在生产父profile释放模型下marker=false。当前helper独立调用退出非成功，
  因而另以完整真实Engine旅程证明兼容，不把该退出本身描述为业务启动成功。
  旧marker=true来自保留的历史失败日志，不伪称本轮重新签回旧包复现。
- 真实两role Engine conformance 9/0/0：环境隔离、input/frame、停流/恢复、退出清理与无监听/孤儿。
  冻结的需求生命周期fixture仍保留采集时epoch4；当前epoch5由实际bundle/handshake验证。
  仅应用本批14文件的独立审查工作树另跑9/0/0，fmt与parity-check均通过。
  该独立首跑8/1暴露需求旅程未使用外部fixture目录；统一两条旅程的路径解析后复跑全部通过，
  该路径错误未当作Engine运行失败，也未忽略失败用例。
- Computer库67通过、0失败、5项native/subprocess用例默认忽略；上面明确点名的原生用例已另跑，
  没有把全部5个ignore自动算作通过。打包单测5/0/0，含完整清单与路径拒绝。
- Computer/Testkit all-target/all-feature Clippy `-D warnings`通过；protocol生成物check通过，
  protocol SHA-256=`c6f5a90c9a549e70c49121ab5b5fd322d4cdf03f27449b7ee7860f0434427e60`。
- 实际bundle manifest SHA-256=`b15d1e58a3b9c91bcecc34ff4fdfdd72ee632761961ff221920c5527ff50240a`，
  ASAR header=`1f636d249fd2c0fbaa8b6a3fc896730089be936329a1b2657aed4359501d1601`。
- 原始Electron SHA与`--version=v43.3.0`、local fixture校验通过；`--release`最终exit1，
  原因为诊断包未满足可信发行身份/签名和完整闸门，这是预期拒绝，**发布闸门仍红**。
  首次受限环境在原始`--version`处被终止，已在允许的原生环境重跑，未用那次结果冒充release拒绝。

本批不新增parity/fixture完成量，不关闭G5E/G7/G8或macOS A0–A7。
生产ComputerManager/出口网关/实际导航与原生电脑接线仍待完成；全部helper执行方式和网络边界、
正式产品签名与发行仍需独立证据。前端反馈的JPEG真实像素尺寸结构校验另批补齐，当前不扩张图像预算。
