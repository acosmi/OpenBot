# AG-UI 0.0.57 官方事件fixture：候选交付与主控修正

日期：2026-09-04。原候选：`dd7febe7f03b01f2c3ffa515cfcd752aacfcb864`，基线R188
`8a91b2d5606891ee28db744c8ad7909a5a68b96e`。

## 实际验收

主控从官方Git tree独立核对发布commit、六份上游文件的路径/blob/bytes；本地逐字节计算的Git blob和SHA-256
与官方记录相等。官方事件枚举、fixture和Rust常量三者均为33项且集合相等。
原候选fixture测试5/0/0、Agent lib57/0/0、all-target/all-feature Clippy和fmt均独立重跑通过。
这些结果只证明协议fixture，不是PG/GUI/SafeDialer/vendor recorded/live证据。

原候选不能原样接纳，主控在自己的分支作了以下修正，未改外部工作树：

- 额外package.json违反仓库只有一个非Grok package.json的要求。按原字节保存为
  `package-manifest.json`，本地文件只作schema资料，upstream_path仍登记真实上游路径。
- 原报告多处字节数、scenario范围、run ID与文件不符；现在以provenance的机器实值为准。
- 原测试只机械比较fixture与Rust两者；现在加入官方EventType枚举的三向比较。
- provenance完整性检查增加exact文件集合、regular-file限制和逐项byte count。
- 去掉fixture中的宿主工作区路径，补显式初始state；数据仍是本项目protocol corpus，不冒充实录。
- MIT版权行按官方原样保留；不凭package作者名发明版权人。SLSA URL只作参考，签名验证明确false。

## 固定来源

官方仓库：`https://github.com/ag-ui-protocol/ag-ui`。
发布schema commit：`54f13419055b4d0f442c71e1efab18b310982ce1`；包版本由同commit
`sdks/typescript/packages/core/package.json`核对为`@ag-ui/core@0.0.57`。
来源文件、Git blob、SHA、字节数及本地改名关系均在
`fixtures/agui/official-event-family.provenance.json`，没有替换v4原始Git oracle的声明。

官方根LICENSE为MIT，原文版权行只有`Copyright (c) 2025`；本项目保存原文，G8关于权利人归属文字
的进一步确认不被本次fixture完成状态代替。没有运行npm/npx或增加Cargo依赖。

## Fixture边界

`official-event-family.jsonl`共39行，三个scenario分别为1–35、36–37、38–39；
成功、错误和interrupt终态分开。合计33个独立EventType，不能把“33种类型”写成“33个独立run”。
修正后4426 bytes，SHA-256
`6c13dad31a54a85a7c38b2d8e7623b9799a31d546eb0502d58000244708ec5ab`。
测试保留decoder各variant的独立语义验证与RunAgentInput/resume编码验证，scope为协议层。

T-FIX-0011 malformed、T-FIX-0012 transport interruption及G4/G6/G8均不随本次关闭；
没有修改production decoder/provider、Cargo.toml/Cargo.lock或外部原始候选。

## 主控命令

- `cargo test -p openbot-agent --test agui_official_fixture --locked`：5/0/0。
- `cargo test -p openbot-agent --lib --locked`：57/0/0。
- `cargo clippy -p openbot-agent --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- `cargo xtask electron-shim-check`：595/600 LOC、非Grok package.json=1。

中央台账由主控在本分支完成后同步；原始外部候选的测试通过本身不构成中央done。
