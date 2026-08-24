# G3 History 与 Explicit Memory batch 3

> 第一真源修订：v3 §28.1 R66；R63 继续有效，只运行本机定向测试，不派发 Actions / 不跑
> `cargo xtask ci`。

## 1. 完成项

- [x] `GetThreadHistory` typed command/port + production PostgreSQL sequence projection；
- [x] `/api/copilotkit/threads/{thread_id}/messages?agentId=...` compatibility facade；
- [x] unknown/new/invisible/deleted/empty thread 均 200 `{"messages":[]}`；
- [x] `agentId` 只保 wire，不参与 ACL；坏 role/content/toolCallId 显式依赖错误；
- [x] Explicit memory WASM DTO 不含 owner/origin/createdBy input；
- [x] `MemoryAdministration` + production PostgreSQL adapter；
- [x] Remember/list/correct/forbid/delete/recall 六条 HTTP，trusted Origin 写 guard 先于 body；
- [x] Fact provenance、Thread/Bot scope 可见性、owner keyset、correct/supersede、内容擦除；
- [x] User + exact Bot/thread PostgreSQL simple FTS + structured-tag AND recall；无 pgvector；
- [x] message-only 正向对照 memory 行数 0；没有 background extraction job；
- [x] lifecycle event 末段失败整事务回滚；
- [ ] Built-in `remember` tool 尚未接 §8.1，Memory GUI 尚未实现。

## 2. 关键裁决

1. History 的“空”是业务成功值，数据库/结构错误不是空；旧客户端虽 fail-soft，Server 仍诚实 5xx。
2. History `agentId` 不能成为第二份 scope 真源；native thread 自身 + AuthContext membership 才是。
3. GUI remember 只能铸 `user_action`；`remember_tool` 与 `verified_import` 没有反序列化入口。
4. Correct 不是 UPDATE content：新记录指向 `supersedes_id`，旧记录保留 provenance 且不再召回。
5. Forbid/delete 都必须同事务擦除 content；状态/event 不同，重复动作幂等零新 event。
6. Recall 默认含 user scope，仅在 caller 提供且权威可见时加入 exact Bot/thread scope；请求 tags
   先排序去重，再以“记录包含全部请求 tag”的 AND 语义收窄 FTS，空 tags 不改变结果集。
7. 普通 owner 写不要求 admin/fresh，但必须可信 Origin；读/recall 无 CSRF side effect。

## 3. 本机证据

- contracts memory：**2/0/0**；
- application memory：**5/0/0**；thread history use case 包含在线程定向 **10/0/0**；
- Server memory：**4/0/0**；Server thread（含 history）：**14/0/0**；
- Axum / typed in-process memory 对拍：**1/0/0**；
- PG17/SCRAM memory admin：**2/0/0**；thread history：**1/0/0**；
- API：**25 done / 130 todo / 155**；tests：**169/878**；
- 总 parity：**287/1365/1652**，violations/warnings 0；fixtures **10/22/32**；
- GitHub Actions：本分支未派发。

## 4. 未冒充边界

本批没有 Memory GUI，也没有 built-in Agent `remember` tool，所以“两种显式写入口”只完成 GUI
对应的后端 API，不宣称整条用户旅程完成。WebSocket、outbox consumer、lease renew/stale-running
recovery、terminal/chunk writer、Intelligence importer/checksum 仍是 G3/G4 后续项，G3 整关不勾。
