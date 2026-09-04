# Batch116：AG-UI官方事件fixture主控验收

日期：2026-09-04；分支：`feat/2026-09-04-G4-agui-official-fixture-audit`。
依据：v4 §7.5、§21.2、T-FIX-0010、§28.1 R192。

原候选`dd7febe7f03b01f2c3ffa515cfcd752aacfcb864`基于R188，11个新增文件；主控没有合并外部分支，
也没有修改外部工作树。先独立核官方Git tree、本地Git blob/SHA/bytes、测试5/0/0、Agent57/0/0及Clippy/fmt。

候选存在三个实质问题：多出第二份非Grok package.json；测试未机械读取官方枚举作三向对比；
交付报告的文件尺寸/scenario与实物不符。主控在自己的分支改为惰性manifest文件名，补exact库存、
regular-file/byte count和三向集合检查，去除宿主路径，修正报告及归属表述。未更改六份官方源文件字节。

官方0.0.57发布schema来源为`54f13419055b4d0f442c71e1efab18b310982ce1`，与原v4仓库oracle
`e42bdbed…`分别登记，不能悄悄替换。官方Registry元数据与attestation payload的source commit已核实；
没有做Sigstore签名验证，provenance明确false。MIT原文完整保存，未从package作者信息编造版权人；
NOTICE原G8归属文字确认仍保留，fixture通过不等于发行许可全部闭合。

修正后fixture39行、33个type、3个独立scenario，4426B，SHA-256
`6c13dad31a54a85a7c38b2d8e7623b9799a31d546eb0502d58000244708ec5ab`。
三向集合相等；成功/error/interrupt lifecycle、各variant与RunAgentInput/resume经公共AguiDecoder/encoder回放。
它是确定性protocol corpus，不是实录，不证明PG/UI/网络、raw payload留存或调用本地工具。

验证：fixture5/0/0、Agent57/0/0、all-target/all-feature Clippy -D warnings、fmt/diff、
electron-shim-check595/600且非Grok package.json=1。SPDX增加一条独立schema source，package计数57。
T-FIX-0010转done；fixtures35/20/55，parity873/839/1712、overlay1273/431/2/6不变。
T-FIX-0011/0012、G4/G6/G8未闭合。无production代码/Cargo/schema/UI/Grok/workflow变化，无远端写入。

每份源文件的完整blob/bytes/SHA与原路径/本地改名关系见伴随provenance；详细修正见本批重写的外部交付报告。
