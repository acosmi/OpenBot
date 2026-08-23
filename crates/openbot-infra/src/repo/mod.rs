//! repository —— `openbot-application` 里各 port 的 PostgreSQL 实现。
//!
//! 依赖方向是 `openbot-infra -> openbot-application`：port 由 application 定义，
//! 适配器在这里实现。application 只依赖 contracts，所以整条链无环。
//!
//! 每个 repo 的落点由 `parity/tables.yaml` 对应表条目 notes 里的 `repo=` 钉死，
//! 不由本模块自行命名。
//!
//! 本层只做「SQL ↔ 类型化行」的翻译：可见性、定序、游标判据落在 SQL 里，
//! 业务规则与编排在 application。**不接受来自 transport 的任意 query**（v3 §5.2）。

pub mod channels;

pub use channels::ChannelRepo;
