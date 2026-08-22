//! `openbot-domain` —— 纯领域层：领域状态、不变量、policy 类型、Agent reducer。
//!
//! # 所有权边界（v3 §5.1 / §7.2 / §8.1）
//!
//! 负责：
//!
//! - 领域状态与不变量本身（thread / run / tool call / lease / generation 的合法迁移）。
//! - policy 类型：决策输入、决策结果、refusal 原因的类型化表达（判定引擎的宿主在
//!   `openbot-agent`，CEL 求值器在 `openbot-infra`）。
//! - **Agent reducer 必须 pure**（v3 §7.2 / CLAUDE.md §4）：`reduce(state, event) -> (state, effects)`。
//!   DB、provider、MCP、browser、file、shell 全都是 effect —— reducer 只描述它们，永不执行。
//! - 工具执行管线（v3 §8.1）里属于纯判定的那几段：validation、effect 分类、outcome 与
//!   `commit_state` 的状态迁移规则。
//!
//! 明确**不**负责：
//!
//! - 任何 I/O、时钟、随机数、线程与 async runtime。纯函数是可确定性重放（v3 §20.1 shadow
//!   Agent 重放录制 stream）的前提，一旦掺进环境依赖，golden trace 就不可信。
//! - use case 编排与事务边界 —— 在 `openbot-application`。
//! - SQL、连接池、HTTP、provider 协议细节 —— 在 `openbot-infra`。
//! - 用户可见文案与本地化（CLAUDE.md §4a）。
//!
//! # Phase 0 状态
//!
//! 刻意为空。领域类型是 G1 的产物；Phase 0 只冻结证据与骨架。
