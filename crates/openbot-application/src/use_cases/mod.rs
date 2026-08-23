//! use case —— 每个文件一个用例，一个用例一条编排。
//!
//! G1 从下面两个起步；W-3a people 与 W-3b tool 编排分别在 `people` / [`crate::tool`]：
//!
//! | 用例 | 标注 | parity 出处 |
//! | --- | --- | --- |
//! | [`list_visible_channels`] | parity | `server/src/routes/channels/routes.ts::list`（ledger `api-channels-list-get`） |
//! | [`health`] | 新增 | 上游无对应路由；它是 typed 边界的最小活性证据 |
//!
//! 用例是**函数**不是结构体：它们没有需要跨调用保存的状态，依赖由参数传入（port 的
//! 引用 + `AuthContext`）。做成 struct 只会让「这次调用用了哪些依赖」从签名里消失。

pub mod health;
pub mod list_visible_channels;
pub mod people;

pub use health::{DEFAULT_HEARTBEAT_PERIOD, health, health_stream};
pub use list_visible_channels::{DEFAULT_CHANNEL_PAGE, list_visible_channels};
pub use people::{
    DEFAULT_PEOPLE_PAGE, MAX_PEOPLE_PAGE, admin_status, change_person_access, change_person_role,
    current_user, list_people,
};
