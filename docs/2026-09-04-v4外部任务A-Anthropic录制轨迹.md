# 外部任务 A：Anthropic 官方 recorded trace

状态：Batch121/R197 已通过主控独立验收；原候选7903e61，本地择取442b6fe，T-FIX-0056；Google和完整G4仍未闭合。日期：2026-09-04。

先读取以下第一真源，不依赖旧聊天快照：

- `/Users/fushihua/Desktop/OpenBot/CLAUDE.md`
- `/Users/fushihua/Desktop/OpenBot/docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md`，尤其§24、§25、§28.1与任务相关章节。
- `/Users/fushihua/Desktop/OpenBot/docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md`
- `/Users/fushihua/Desktop/OpenBot/docs/2026-09-04-v4并行实施预留台账.md`

固定基线为R188 `8a91b2d5606891ee28db744c8ad7909a5a68b96e`。在自己的独立worktree/checkout创建下述分支，必须核验基线；不要切换、rebase、修改主控工作树或Personal Skills工作树。主控当前Screen分支的未提交改动不属于本任务。

全程零npm，不运行npm/npx或引入node_modules/锁文件；保持`grok-bot` tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`和仅一个非Grok `package.json`。禁止`cargo xtask ci`、Actions dispatch、擅自合并PR；不要改Cargo依赖/lock、中央parity、`fixtures/MANIFEST.yaml`、v4、CLAUDE、README、移交指南或预留台账。中央证据状态由主控独立验收后统一更新。

只提交本任务允许范围，最终一个候选commit即可；不得用整分支合并交付。发现超出允许范围的生产缺陷，先交最小可复现证据和修复建议，由主控处理，不自行扩范围。

## 任务

分支：`feat/2026-09-04-G4-anthropic-recorded-trace`。
建议独立路径：`/Users/fushihua/Desktop/OpenBot-G4-anthropic-recorded-trace`；若已存在先检查，不覆盖。

为G4取得Anthropic Messages的一份或多份**官方录制HTTP/SSE响应**，并经production `AnthropicProvider` + `SafeDialer`离线loopback回放。参照已有`crates/openbot-infra/tests/openai_recorded_trace.rs`的证据方法；不得照抄OpenAI事件作为Anthropic trace。

只允许修改：

- `crates/openbot-infra/tests/anthropic_recorded_trace.rs`
- `fixtures/provider/anthropic-*.sse`
- `fixtures/provider/anthropic-*.provenance.json`
- `crates/openbot-infra/src/provider/anthropic.rs`：仅当真实回放揭示兼容缺口时最小修复，不重构其它provider。
- `docs/2026-09-04-Anthropic-recorded-trace-外部交付.md`：任务证据与许可/中央NOTICE需增补的精确文本。

必须证明源数据来自Anthropic官方仓库的recorded/replay资产；官方手写单元测试、文档示例、自造JSON都不能称recorded。必须固定raw response字节与来源；提取不得擅改事件语义。没有可证明的录制资产时交“来源证据不足”，不伪造完成。

验收：

1. 尽可能覆盖text/tool-use/usage/terminal，thinking只在真实录制包含时报告；不足的族明确列出，不补手写事件冒充recorded。
2. 同一production adapter经SafeDialer和真实本机HTTP/SSE监听，整块、非规则分块、逐字节分块的标准化输出一致；usage不倒退、terminal恰一次。
3. key只在固定header，不进URL；坏帧/error body不回显vendor正文。测试改造样本必须标为negative mutation，与原始recorded文件分离。
4. fixture与provenance一对一；无真实credential/request/customer标识；每个保留的vendor response/call ID说明非secret依据。
5. 不把此结果外推为live调用、三家齐备或T-FIX-0013等全部provider fixture完成。

必跑：`cargo test -p openbot-infra --test anthropic_recorded_trace --locked`、Anthropic相关现有单测、`cargo clippy -p openbot-infra --all-targets --all-features --locked -- -D warnings`、`cargo fmt --all -- --check`。loopback bind权限不足时应使用宿主允许的权限运行，不能跳过网络路径。

## 交付与验收

执行到底，给出候选SHA，不停在“未提交但测试通过”。交付：

1. 任务名、分支、worktree绝对路径、基线SHA、候选SHA。
2. `git diff --name-status 8a91b2d5606891ee28db744c8ad7909a5a68b96e..<candidate>`、`git diff --check`、`git status --short`结果。
3. 官方source URL、exact commit/blob、原始与提取后字节数/SHA-256、LICENSE/copyright、确定性提取规则；消毒前提与canary扫描结果。不得交API key、customer请求或可验证secret hash。
4. 每个验收命令、退出码、passed/failed/ignored计数与原始输出位置。若基线已有失败，须给差异复现；不能只写“全部通过”。
5. 明确未运行、未完成和证据不足项。候选通过不等于对应T-ID、G4、G6或v4整关关闭。

主控收到后重新核来源/许可/字节、逐文件审计、独立重跑，审计通过才逐commit择取并更新台账。不要改动其它工具的任务范围。
